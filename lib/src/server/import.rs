//! The page that turns the torrents a client holds into rulesets.
//!
//! The preview lists the client live and stores nothing, as the feed test
//! page does. Only the Import post writes, and it writes the shows the
//! reader left checked.
//!
//! The plan is computed again at post time rather than carried in the form.
//! The client is the source, and a change between the two requests costs one
//! stale row at most.

use std::collections::BTreeSet;
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

/// Lists every subject the client holds, and marks each one a ruleset
/// already names.
///
/// A client that does not answer renders its refusal on the page rather than
/// as an error status. The request itself succeeded, and the client is what
/// did not answer.
#[page("/admin/rulesets/import")]
async fn import_preview(cx: &Cx) -> Result {
    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let now = services.clock.now();

    let listed = match services.torrents.list().await {
        Ok(torrents) => Ok(import::plan(&engine, &torrents)),
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
        <nav class="text-sm text-slate-500">
            <a href="/admin/rulesets" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">"Import"</span>
        </nav>

        <h1 class="mt-3 text-2xl font-semibold tracking-tight">"Import from client"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Listed just now. Nothing is stored until you import."
        </p>

        match &rows {
            Err(error) => <p class="mt-6 rounded-lg border border-rose-500/40 bg-rose-500/5 px-4 py-3 text-sm text-rose-300">
                "failed: " (error)
            </p>,
            Ok(entries) if entries.is_empty() => <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "The client holds no show a parser reads."
            </p>,
            Ok(entries) => <form method="post" action="/admin/rulesets/import">
                <ul class="mt-6 flex flex-col gap-2">
                    for Row { suggestion, name, claimed, repeats } in entries {
                        <li>
                            <label class="block cursor-pointer rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4 transition-colors hover:border-slate-700">
                                <div class="flex flex-wrap items-center gap-3">
                                    <input
                                        type="checkbox"
                                        name="pick"
                                        value=(format!("{}|{}", suggestion.parser, suggestion.key))
                                        checked=(claimed.as_ref().is_none_or(|claimed| !claimed.same_parser))
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
                                </div>

                                <div class="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                                    <span>
                                        (format::count(suggestion.torrents, "torrent", "torrents"))
                                    </span>
                                    <span>(format::age(now, suggestion.newest))</span>
                                </div>

                                <div class="mt-3 flex flex-wrap items-center gap-2">
                                    for condition in &suggestion.conditions {
                                        <span class="rounded-full bg-slate-800/70 px-2 py-0.5 font-mono text-xs text-slate-400">
                                            (&condition.field) " " (condition.op.label()) " " (&condition.value)
                                        </span>
                                    }
                                </div>
                            </label>
                        </li>
                    }
                </ul>

                <div class="mt-6 flex flex-wrap items-center gap-3">
                    <button
                        type="submit"
                        class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                    >
                        "Import"
                    </button>
                    components::link_button(href: "/admin/rulesets", label: "Cancel")
                </div>
            </form>,
        }
    }
}

/// Creates a ruleset for each checked show, then returns to the index.
///
/// Every created ruleset is enabled, because a reader who imported a show
/// asked for its releases.
#[route(POST "/admin/rulesets/import")]
async fn import_rulesets(cx: &Cx, RawForm(body): RawForm) -> Result<SeeOther> {
    let picked = {
        let body = str::from_utf8(&body).map_err(|_| bad_request("the form is not valid UTF-8"))?;

        form_urlencoded::parse(body.as_bytes())
            .filter(|(key, _)| key == "pick")
            .map(|(_, value)| value.into_owned())
            .collect::<BTreeSet<_>>()
    };

    let services = app_context::<Services>(cx);
    let rulesets = app_context::<Arc<Rulesets>>(cx);

    let torrents = services
        .torrents
        .list()
        .await
        .map_err(internal_server_error)?;

    for suggestion in import::plan(&rulesets.engine(), &torrents) {
        if !picked.contains(&format!("{}|{}", suggestion.parser, suggestion.key)) {
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
