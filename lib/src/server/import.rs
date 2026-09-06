//! The page that turns the torrents a client holds into rulesets.
//!
//! The preview lists the client live and stores nothing, as the feed test
//! page does. Only the Import post writes, and it writes the shows the
//! reader left checked.
//!
//! The plan is computed again at post time rather than carried in the form.
//! The client is the source, and a change between the two requests costs one
//! stale row at most.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use topcoat::{
    Result,
    context::Cx,
    context::app_context,
    router::{
        content::RawForm,
        error::{SeeOther, bad_request, internal_server_error, see_other},
        page, route,
    },
    runtime::{Event, shard},
    view::Unescaped,
    view::view,
};
use url::form_urlencoded;

use crate::parser::form as parser_form;
use crate::rules::Engine;
use crate::ruleset;
use crate::ruleset::import::{Collision, Suggestion};
use crate::ruleset::registry::Rulesets;
use crate::ruleset::{Ruleset, import};
use crate::server::{components, format, handlers};
use crate::services::Services;
use crate::torrent::{Torrent, TorrentId};

/// Flips the review's checkboxes in a group, either the whole page or one
/// show's torrents.
///
/// Each operation returns the form serialized, which is what the `review`
/// signal takes and the shard re-plans from.
///
/// A disabled pick names a subject a ruleset on the same parser already
/// covers, so the page-wide operations leave it alone. `torrents` falls
/// back to every pick when it looks one up, because the reader still edits
/// the torrents of a subject the preview offers no pick for.
const IMPORT_ACTIONS: &str = r"
window.torrssImport = {
  boxes: (root, name) =>
    [...root.querySelectorAll('input[name=' + name + ']:not(:disabled)')],
  set: (root, names, on) => {
    for (const name of names) {
      for (const box of window.torrssImport.boxes(root, name)) {
        box.checked = on;
      }
    }

    return window.torrssRows.serialize();
  },
  all: () =>
    window.torrssImport.set(
      document.querySelector('#import-form'),
      ['pick', 'torrent'],
      true,
    ),
  none: () =>
    window.torrssImport.set(
      document.querySelector('#import-form'),
      ['pick', 'torrent'],
      false,
    ),
  torrents: (action) => {
    const cut = action.indexOf(':');
    const on = action.slice(0, cut) === 'all';
    const pick = action.slice(cut + 1);
    const form = document.querySelector('#import-form');

    const box =
      window.torrssImport.boxes(form, 'pick').find((one) => one.value === pick) ||
      [...form.querySelectorAll('input[name=pick]')].find((one) => one.value === pick);

    return window.torrssImport.set(box.closest('li'), ['torrent'], on);
  },
};
";

/// What the reader left checked in the preview.
///
/// The form is the review. A checkbox posts its value only when it is
/// checked, so the serialized form says what they kept and everything
/// absent from it is what they rejected.
#[derive(Debug, PartialEq, Eq)]
struct Review {
    /// The `parser|key` of every suggestion the reader wants created.
    picked: BTreeSet<String>,

    /// Every torrent the reader left in its suggestion's agreement.
    included: HashSet<TorrentId>,
}

impl Review {
    /// Reads a review out of a serialized form, or [`None`] when the body
    /// carries none.
    ///
    /// An empty body is the first render, where no show is picked and every
    /// torrent is in its agreement. The hidden `reviewed` input is what keeps
    /// a form the reader emptied from reading the same way.
    fn parse(body: &str) -> Option<Self> {
        if body.is_empty() {
            return None;
        }

        let mut review = Self {
            picked: BTreeSet::new(),
            included: HashSet::new(),
        };

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match &*key {
                "pick" => {
                    review.picked.insert(value.into_owned());
                }
                "torrent" => {
                    review.included.insert(TorrentId(value.into_owned()));
                }
                _ => {}
            }
        }

        Some(review)
    }
}

/// The ids of every listed torrent the review left out.
///
/// No review excludes nothing, because the first render keeps every torrent
/// in its agreement.
fn excluded(torrents: &[Torrent], review: Option<&Review>) -> HashSet<TorrentId> {
    let Some(review) = review else {
        return HashSet::new();
    };

    torrents
        .iter()
        .map(|torrent| torrent.id.clone())
        .filter(|id| !review.included.contains(id))
        .collect()
}

/// One suggestion with every name the row renders resolved.
struct Row<'a> {
    suggestion: &'a Suggestion,

    /// What the suggested ruleset is called.
    name: String,

    /// The ruleset that already names this subject, when one does.
    claimed: Option<Claimed>,

    /// What the parser of the earlier suggestion about this subject is
    /// called, when one precedes this.
    repeats: Option<String>,
}

/// The ruleset a collision points at, named for the reader.
///
/// The engine is read once per row here, because a badge names the ruleset
/// and the parser it reads with rather than their ids.
struct Claimed {
    id: String,
    ruleset: String,

    /// What the ruleset's parser is called, which only a collision on
    /// another parser renders.
    parser: String,

    same_parser: bool,
}

/// The page the reader reviews a client's torrents on.
///
/// The heading and the form shell stay put. Everything the review changes
/// lives in the shard below, so a click re-plans the list in place.
#[page("/admin/rulesets/import")]
async fn import_preview() -> Result {
    view! {
        signal review = String::new();

        // The shard's checkboxes are rendered outside this render, so the
        // form is read back through the serializer rather than a capture.
        <script>(Unescaped::new_unchecked(components::ROW_ACTIONS))</script>
        <script>(Unescaped::new_unchecked(IMPORT_ACTIONS))</script>

        <nav class="text-sm text-slate-500">
            <a href="/admin/rulesets" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">"Import"</span>
        </nav>

        <h1 class="mt-3 text-2xl font-semibold tracking-tight">"Import from client"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Listed just now. Check the shows to import. Nothing is stored until you do."
        </p>

        <form
            id="import-form"
            data-rows="true"
            method="post"
            action="/admin/rulesets/import"
            // A checkbox posts nothing when it is off, so the serialized
            // form is the review, and the shard re-plans from it.
            @change=$(|_e: Event| {
                review.set(raw!(
                    "cx.hydrate(window.torrssRows.serialize())",
                    String::new()
                ));
            })
            // A per-show button is rendered by the shard, so its click is
            // caught here, where the signal lives. The button names its show
            // in its own value, because the event vocabulary carries a
            // target's name and value and nothing structural.
            @click=$(|e: Event| if e.target.name == "torrents" {
                review.set(raw!(
                    "cx.hydrate(window.torrssImport.torrents(String(${e}.target.value)))",
                    String::new()
                ));
            })
        >
            // Without this a form the reader emptied serializes to nothing,
            // which reads as the first render and puts every torrent back.
            <input type="hidden" name="reviewed" value="1">

            <div class="mt-6 flex flex-wrap items-center gap-3">
                // These lead the list, because a reader who starts from
                // nothing reaches for them before reading a single show.
                // Both stay visible, because the page counts nothing and a
                // review with everything off is as reachable as one with
                // everything on.
                <button
                    type="button"
                    class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                    @click=$(|_e: Event| {
                        review.set(raw!(
                            "cx.hydrate(window.torrssImport.all())",
                            String::new()
                        ));
                    })
                >
                    "Select all"
                </button>
                <button
                    type="button"
                    class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                    @click=$(|_e: Event| {
                        review.set(raw!(
                            "cx.hydrate(window.torrssImport.none())",
                            String::new()
                        ));
                    })
                >
                    "Deselect all"
                </button>
            </div>

            import_suggestions(review: $(review.get()))

            <div class="mt-6 flex flex-wrap items-center gap-3">
                <button
                    type="submit"
                    class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                >
                    "Import"
                </button>
                components::link_button(href: "/admin/rulesets", label: "Cancel")
            </div>
        </form>
    }
}

/// Lists every subject the client holds, re-planned from what the reader
/// left checked.
///
/// A client that does not answer renders its refusal here rather than as an
/// error status. The request itself succeeded, and the client is what did
/// not answer.
///
/// The review crosses the network and none of it is trusted. Every id in it
/// is compared against what the client just listed, so an id naming nothing
/// excludes nothing.
#[shard]
async fn import_suggestions(cx: &Cx, review: String) -> Result {
    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let now = services.clock.now();

    let review = Review::parse(&review);

    let listed = match services.torrents.list().await {
        Ok(torrents) => {
            let excluded = excluded(&torrents, review.as_ref());

            Ok(import::plan(&engine, &torrents, &excluded))
        }
        Err(error) => Err(error.to_string()),
    };

    // The names are resolved here rather than in the view, because a row
    // borrows them and a value built inline dies before the row reads it.
    let rows = listed.as_ref().map(|suggestions| {
        suggestions
            .iter()
            .map(|suggestion| Row {
                name: named(&engine, suggestion),
                claimed: suggestion
                    .collision
                    .as_ref()
                    .map(|collision| claimed(&engine, collision)),
                repeats: suggestion.repeats.as_ref().map(|id| {
                    engine
                        .parser(id)
                        .map_or_else(|| id.clone(), |parser| parser.name.clone())
                }),
                suggestion,
            })
            .collect::<Vec<_>>()
    });

    view! {
        match &rows {
            Err(error) => <p class="mt-6 rounded-lg border border-rose-500/40 bg-rose-500/5 px-4 py-3 text-sm text-rose-300">
                "failed: " (error)
            </p>,
            Ok(entries) if entries.is_empty() => <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "The client holds no show a parser reads."
            </p>,
            Ok(entries) => <ul class="mt-6 flex flex-col gap-2">
                for Row { suggestion, name, claimed, repeats } in entries {
                    <li>
                        <div class="rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4">
                            <label class="flex cursor-pointer flex-wrap items-center gap-3">
                                <input
                                    type="checkbox"
                                    name="pick"
                                    value=(format!("{}|{}", suggestion.parser, suggestion.key))
                                    checked=(picked(review.as_ref(), suggestion))
                                    disabled=(claimed.as_ref().is_some_and(|claimed| claimed.same_parser))
                                    class="size-4 rounded border-slate-700 bg-slate-950"
                                >
                                <h2 class="text-sm font-semibold text-slate-100">(name)</h2>

                                match claimed {
                                    Some(claimed) if claimed.same_parser => <a
                                        href=(format!("/admin/rulesets/{}", claimed.id))
                                        class="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-300"
                                    >
                                        "already a ruleset: " (&claimed.ruleset)
                                    </a>,
                                    Some(claimed) => <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                                        "also " (&claimed.ruleset) ", read with " (&claimed.parser)
                                    </span>,
                                    None => "",
                                }

                                if let Some(repeats) = repeats {
                                    <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                                        "same show as the " (repeats) " suggestion"
                                    </span>
                                }
                            </label>

                            <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                                <span>
                                    (format::count(
                                        suggestion
                                            .members
                                            .iter()
                                            .filter(|member| member.included)
                                            .count(),
                                        "torrent",
                                        "torrents",
                                    ))
                                </span>
                                <span>(format::age(now, suggestion.newest))</span>

                                // A button names its show in its own value,
                                // because the event vocabulary carries a
                                // target's name and value and nothing
                                // structural. The prefix ends at the first
                                // colon, which no parser id carries.
                                <button
                                    type="button"
                                    name="torrents"
                                    value=(format!("all:{}|{}", suggestion.parser, suggestion.key))
                                    class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                                >
                                    "Select all"
                                </button>
                                <button
                                    type="button"
                                    name="torrents"
                                    value=(format!("none:{}|{}", suggestion.parser, suggestion.key))
                                    class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                                >
                                    "Deselect all"
                                </button>
                            </div>

                            <div class="mt-3 flex flex-wrap items-center gap-2">
                                for condition in &suggestion.conditions {
                                    <span class="rounded-full bg-slate-800/70 px-2 py-0.5 font-mono text-xs text-slate-400">
                                        (&condition.field) " " (condition.op.label()) " " (&condition.value)
                                    </span>
                                }
                            </div>

                            <ul class="mt-3 flex flex-col gap-1">
                                for member in &suggestion.members {
                                    <li>
                                        <label class="flex cursor-pointer items-center gap-2 text-xs text-slate-400">
                                            <input
                                                type="checkbox"
                                                name="torrent"
                                                value=(&member.torrent.id.0)
                                                checked=(member.included)
                                                class="size-3.5 rounded border-slate-700 bg-slate-950"
                                            >
                                            <span class="font-mono break-all">(&member.torrent.name)</span>
                                            <span class="text-slate-500">
                                                (format::age(now, member.torrent.added_at))
                                            </span>
                                        </label>
                                    </li>
                                }
                            </ul>
                        </div>
                    </li>
                }
            </ul>,
        }
    }
}

/// Whether the pick checkbox of `suggestion` renders checked.
///
/// Nothing is picked on the first render, so the reader picks the shows they
/// want rather than unpicking the ones they do not. After that the review
/// itself says what they kept.
fn picked(review: Option<&Review>, suggestion: &Suggestion) -> bool {
    review.is_some_and(|review| {
        review
            .picked
            .contains(&format!("{}|{}", suggestion.parser, suggestion.key))
    })
}

/// Creates a ruleset for each checked show, then returns to the index.
///
/// The posted form is the review, so the created rulesets carry only the
/// torrents the reader left checked. Every one is enabled, because a reader
/// who imported a show asked for its releases.
#[route(POST "/admin/rulesets/import")]
async fn import_rulesets(cx: &Cx, RawForm(body): RawForm) -> Result<SeeOther> {
    let review = {
        let body = str::from_utf8(&body).map_err(|_| bad_request("the form is not valid UTF-8"))?;

        Review::parse(body).ok_or_else(|| bad_request("the form carries no review"))?
    };

    let services = app_context::<Services>(cx);
    let rulesets = app_context::<Arc<Rulesets>>(cx);

    let torrents = services
        .torrents
        .list()
        .await
        .map_err(internal_server_error)?;

    let excluded = excluded(&torrents, Some(&review));

    for suggestion in import::plan(&rulesets.engine(), &torrents, &excluded) {
        if !review
            .picked
            .contains(&format!("{}|{}", suggestion.parser, suggestion.key))
        {
            continue;
        }

        // The preview never offers a subject its own parser already has a
        // ruleset for, so a post that names one is stale. It creates a second
        // ruleset over the same releases.
        if suggestion
            .collision
            .as_ref()
            .is_some_and(|collision| collision.same_parser)
        {
            continue;
        }

        // The engine is read again per suggestion, because each save
        // rebuilds it and the next slug has to see the id just taken.
        let (id, name) = {
            let engine = rulesets.engine();
            let name = named(&engine, &suggestion);

            let id = parser_form::unique_slug(&name, |id| engine.ruleset(id).is_some())
                .ok_or_else(|| {
                    bad_request(format!(
                        "{} has no letters or digits to build an id from",
                        suggestion.show
                    ))
                })?;

            (id, name)
        };

        rulesets
            .save(Ruleset {
                id,
                name,
                enabled: true,
                parser: suggestion.parser,
                conditions: suggestion.conditions,
                tests: Vec::new(),
            })
            .await
            .map_err(handlers::write_failed)?;
    }

    Ok(see_other("/admin/rulesets"))
}

/// Resolves a collision's ids to the names the badge renders.
///
/// A ruleset removed between the plan and the render leaves its id in place
/// of a name, which still tells the reader which one to look for.
fn claimed(engine: &Engine, collision: &Collision) -> Claimed {
    let found = engine.ruleset(&collision.ruleset);

    let parser = found
        .and_then(|ruleset| engine.parser(&ruleset.parser))
        .map_or_else(String::new, |parser| parser.name.clone());

    Claimed {
        id: collision.ruleset.clone(),
        ruleset: found.map_or_else(|| collision.ruleset.clone(), |ruleset| ruleset.name.clone()),
        parser,
        same_parser: collision.same_parser,
    }
}

/// Returns the name the suggested ruleset takes.
///
/// The conditions name it, as they name a ruleset the reader saved with a
/// blank name, so an imported ruleset reads the same as a hand-written one.
fn named(engine: &Engine, suggestion: &Suggestion) -> String {
    let parser = engine
        .parser(&suggestion.parser)
        .map_or(suggestion.parser.as_str(), |parser| parser.name.as_str());

    ruleset::inferred_name(&suggestion.conditions, parser)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use chrono::{TimeZone, Utc};

    use super::{Review, excluded};
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    fn torrent(id: &str) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: id.to_owned(),
            state: TorrentState::Seeding,
            size: 0,
            progress: 1.0,
            added_at: Utc.with_ymd_and_hms(2025, 3, 4, 12, 0, 0).single(),
        }
    }

    #[test]
    fn an_empty_body_is_no_review() {
        assert_eq!(
            Review::parse(""),
            None,
            "the first render posts nothing, and no show is picked there"
        );
    }

    #[test]
    fn a_review_reads_picks_and_torrents() {
        let review =
            Review::parse("reviewed=1&pick=series%7Ccoastal+ecology&torrent=abc&torrent=def")
                .expect("a body with the hidden input is a review");

        assert_eq!(
            review.picked.iter().map(String::as_str).collect::<Vec<_>>(),
            ["series|coastal ecology"],
            "the pick value is the parser and the key the checkbox posted"
        );
        assert_eq!(
            review.included,
            HashSet::from([TorrentId("abc".to_owned()), TorrentId("def".to_owned())]),
            "every checked torrent stays in its agreement"
        );
    }

    #[test]
    fn a_form_with_nothing_checked_is_still_a_review() {
        assert_eq!(
            Review::parse("reviewed=1"),
            Some(Review {
                picked: BTreeSet::new(),
                included: HashSet::new(),
            }),
            "the hidden input is what tells an emptied form from the first render"
        );
    }

    #[test]
    fn excluded_is_every_listed_torrent_the_review_left_out() {
        let torrents = [torrent("abc"), torrent("def"), torrent("ghi")];

        assert_eq!(
            excluded(&torrents, Review::parse("reviewed=1&torrent=def").as_ref()),
            HashSet::from([TorrentId("abc".to_owned()), TorrentId("ghi".to_owned())]),
            "what the reader left unchecked is what leaves the agreement"
        );
        assert_eq!(
            excluded(&torrents, None),
            HashSet::new(),
            "no review excludes nothing, because the first render keeps every torrent"
        );
    }
}
