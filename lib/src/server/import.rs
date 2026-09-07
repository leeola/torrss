//! The page that turns the torrents a client holds into searches.
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

use crate::engine::Engine;
use crate::parser::form as parser_form;
use crate::search;
use crate::search::Condition;
use crate::search::import::{Collision, Suggestion};
use crate::search::registry::Searches;
use crate::search::{Search, import};
use crate::server::{components, format, handlers};
use crate::services::Services;
use crate::torrent::{Torrent, TorrentId};

/// Flips the review's checkboxes in a group, either the whole page or one
/// show's torrents.
///
/// Each operation returns the form serialized, which is what the `review`
/// signal takes and the shard re-plans from.
///
/// A disabled pick names a subject a search on the same parser already
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

    /// The `parser|key|field` of every condition the reader turned off.
    ///
    /// A chip's checkbox is checked when its condition is off, so the
    /// serialized form carries what was dropped rather than what was kept,
    /// and a field that starts to agree comes in on.
    dropped: HashSet<String>,
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
            dropped: HashSet::new(),
        };

        for (key, value) in form_urlencoded::parse(body.as_bytes()) {
            match &*key {
                "pick" => {
                    review.picked.insert(value.into_owned());
                }
                "torrent" => {
                    review.included.insert(TorrentId(value.into_owned()));
                }
                "drop" => {
                    review.dropped.insert(value.into_owned());
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

/// Names one condition of one suggestion, which is what a chip's checkbox
/// posts to turn it off.
fn condition_value(suggestion: &Suggestion, condition: &Condition) -> String {
    format!(
        "{}|{}|{}",
        suggestion.parser, suggestion.key, condition.field
    )
}

/// The conditions the imported search carries.
///
/// The subject leads, because [`crate::search::import`] places it first,
/// and it never drops. A search with no subject condition claims every
/// release its parser reads.
fn kept(suggestion: &Suggestion, review: Option<&Review>) -> Vec<Condition> {
    let Some((subject, rest)) = suggestion.conditions.split_first() else {
        return Vec::new();
    };

    let dropped = |condition: &Condition| {
        review.is_some_and(|review| {
            review
                .dropped
                .contains(&condition_value(suggestion, condition))
        })
    };

    [subject.clone()]
        .into_iter()
        .chain(rest.iter().filter(|one| !dropped(one)).cloned())
        .collect()
}

/// One suggestion with every name the row renders resolved.
struct Row<'a> {
    suggestion: &'a Suggestion,

    /// What the suggested search is called.
    name: String,

    /// The search that already names this subject, when one does.
    claimed: Option<Claimed>,

    /// What the parser of the earlier suggestion about this subject is
    /// called, when one precedes this.
    repeats: Option<String>,
}

/// The search a collision points at, named for the reader.
///
/// The engine is read once per row here, because a badge names the search
/// and the parser it reads with rather than their ids.
struct Claimed {
    id: String,
    search: String,

    /// What the search's parser is called, which only a collision on
    /// another parser renders.
    parser: String,

    same_parser: bool,
}

/// The page the reader reviews a client's torrents on.
///
/// The heading and the form shell stay put. Everything the review changes
/// lives in the shard below, so a click re-plans the list in place.
#[page("/searches/import")]
async fn import_preview() -> Result {
    view! {
        signal review = String::new();

        // The shard's checkboxes are rendered outside this render, so the
        // form is read back through the serializer rather than a capture.
        <script>(Unescaped::new_unchecked(components::ROW_ACTIONS))</script>
        <script>(Unescaped::new_unchecked(IMPORT_ACTIONS))</script>

        <nav class="text-sm text-slate-500">
            <a href="/searches" class="hover:text-slate-300">"Searches"</a>
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
            action="/searches/import"
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
                components::link_button(href: "/searches", label: "Cancel")
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
    let engine = app_context::<Arc<Searches>>(cx).engine();
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
                name: named(
                    &engine,
                    &suggestion.parser,
                    &kept(suggestion, review.as_ref()),
                ),
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
                                        href=(format!("/searches/{}", claimed.id))
                                        class="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs text-amber-300"
                                    >
                                        "already a search: " (&claimed.search)
                                    </a>,
                                    Some(claimed) => <span class="rounded-full bg-slate-800/70 px-2 py-0.5 text-xs text-slate-400">
                                        "also " (&claimed.search) ", read with " (&claimed.parser)
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
                                if let Some((subject, rest)) = suggestion.conditions.split_first() {
                                    // The subject never drops, so it stays a
                                    // span. A search without it claims every
                                    // release its parser reads.
                                    <span class="rounded-full bg-slate-800/70 px-2 py-0.5 font-mono text-xs text-slate-400">
                                        (&subject.field) " " (subject.op.label()) " " (&subject.value)
                                    </span>

                                    // The box is checked when the condition
                                    // is off, so the form carries what the
                                    // reader dropped rather than what they
                                    // kept.
                                    for condition in rest {
                                        <label class="cursor-pointer rounded-full bg-slate-800/70 px-2 py-0.5 font-mono text-xs text-slate-400 has-checked:bg-slate-900/40 has-checked:text-slate-600 has-checked:line-through">
                                            <input
                                                type="checkbox"
                                                name="drop"
                                                value=(condition_value(suggestion, condition))
                                                checked=(review.as_ref().is_some_and(|review| {
                                                    review
                                                        .dropped
                                                        .contains(&condition_value(suggestion, condition))
                                                }))
                                                class="sr-only"
                                            >
                                            (&condition.field) " " (condition.op.label()) " " (&condition.value)
                                        </label>
                                    }
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

/// Creates a search for each checked show, then returns to the index.
///
/// The posted form is the review, so the created searches carry only the
/// torrents the reader left checked. Every one is enabled, because a reader
/// who imported a show asked for its releases.
#[route(POST "/searches/import")]
async fn import_searches(cx: &Cx, RawForm(body): RawForm) -> Result<SeeOther> {
    let review = {
        let body = str::from_utf8(&body).map_err(|_| bad_request("the form is not valid UTF-8"))?;

        Review::parse(body).ok_or_else(|| bad_request("the form carries no review"))?
    };

    let services = app_context::<Services>(cx);
    let searches = app_context::<Arc<Searches>>(cx);

    let torrents = services
        .torrents
        .list()
        .await
        .map_err(internal_server_error)?;

    let excluded = excluded(&torrents, Some(&review));

    for suggestion in import::plan(&searches.engine(), &torrents, &excluded) {
        if !review
            .picked
            .contains(&format!("{}|{}", suggestion.parser, suggestion.key))
        {
            continue;
        }

        // The preview never offers a subject its own parser already has a
        // search for, so a post that names one is stale. It creates a second
        // search over the same releases.
        if suggestion
            .collision
            .as_ref()
            .is_some_and(|collision| collision.same_parser)
        {
            continue;
        }

        let conditions = kept(&suggestion, Some(&review));

        // The engine is read again per suggestion, because each save
        // rebuilds it and the next slug has to see the id just taken.
        let (id, name) = {
            let engine = searches.engine();
            let name = named(&engine, &suggestion.parser, &conditions);

            let id = parser_form::unique_slug(&name, |id| engine.search(id).is_some()).ok_or_else(
                || {
                    bad_request(format!(
                        "{} has no letters or digits to build an id from",
                        suggestion.show
                    ))
                },
            )?;

            (id, name)
        };

        searches
            .save(Search {
                id,
                name,
                enabled: true,
                parser: suggestion.parser,
                conditions,
                tests: Vec::new(),
            })
            .await
            .map_err(handlers::write_failed)?;
    }

    Ok(see_other("/searches"))
}

/// Resolves a collision's ids to the names the badge renders.
///
/// A search removed between the plan and the render leaves its id in place
/// of a name, which still tells the reader which one to look for.
fn claimed(engine: &Engine, collision: &Collision) -> Claimed {
    let found = engine.search(&collision.search);

    let parser = found
        .and_then(|search| engine.parser(&search.parser))
        .map_or_else(String::new, |parser| parser.name.clone());

    Claimed {
        id: collision.search.clone(),
        search: found.map_or_else(|| collision.search.clone(), |search| search.name.clone()),
        parser,
        same_parser: collision.same_parser,
    }
}

/// Returns the name the suggested search takes.
///
/// The conditions name it, as they name a search the reader saved with a
/// blank name, so an imported search reads the same as a hand-written one.
fn named(engine: &Engine, parser: &str, conditions: &[Condition]) -> String {
    let named = engine
        .parser(parser)
        .map_or(parser, |parser| parser.name.as_str());

    search::inferred_name(conditions, named)
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use chrono::{TimeZone, Utc};

    use super::{Review, excluded, kept};
    use crate::search::import::Suggestion;
    use crate::search::{Condition, Op};
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    fn equals(field: &str, value: &str) -> Condition {
        Condition {
            field: field.to_owned(),
            op: Op::Equals,
            value: value.to_owned(),
        }
    }

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
        let review = Review::parse(
            "reviewed=1&pick=series%7Ccoastal+ecology&torrent=abc&torrent=def\
             &drop=series%7Ccoastal+ecology%7Cresolution",
        )
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
        assert_eq!(
            review.dropped,
            HashSet::from(["series|coastal ecology|resolution".to_owned()]),
            "a dropped condition is named by its show and its field"
        );
    }

    #[test]
    fn a_dropped_condition_leaves_the_search_and_the_subject_stays() {
        let suggestion = Suggestion {
            parser: "series".to_owned(),
            key: "coastal ecology".to_owned(),
            show: "Coastal Ecology".to_owned(),
            members: Vec::new(),
            newest: None,
            conditions: vec![
                equals("show", "Coastal Ecology"),
                equals("resolution", "1080p"),
                equals("codec", "x265"),
            ],
            collision: None,
            repeats: None,
        };

        let review = Review::parse(
            "reviewed=1&drop=series%7Ccoastal+ecology%7Cshow\
             &drop=series%7Ccoastal+ecology%7Cresolution",
        )
        .expect("a body with the hidden input is a review");

        assert_eq!(
            kept(&suggestion, Some(&review)),
            [equals("show", "Coastal Ecology"), equals("codec", "x265")],
            "the subject stays whatever the review says, and the dropped field goes"
        );
    }

    #[test]
    fn a_form_with_nothing_checked_is_still_a_review() {
        assert_eq!(
            Review::parse("reviewed=1"),
            Some(Review {
                picked: BTreeSet::new(),
                included: HashSet::new(),
                dropped: HashSet::new(),
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
