//! The pages that list and write a parser.
//!
//! A parser is the ruleset editor's first half: the fields that read a
//! filename apart, with nothing that decides whether anyone wants the
//! release. So this page is that editor with the role, the base, the badge,
//! and the switch cut away, and it re-renders through the same row shards.

use std::sync::Arc;

use topcoat::{
    Error, Result,
    context::Cx,
    context::app_context,
    router::{
        content::RawForm,
        error::{
            RouterErrorExt, SeeOther, bad_request, internal_server_error, not_found, see_other,
        },
        page, path_param, route,
    },
    runtime::{Event, procedure, shard},
    view::Unescaped,
    view::{component, view},
};
use tracing::error;

use crate::feed::registry::FeedRegistry;
use crate::parser::form::{self as parser_form, ParserForm};
use crate::parser::{PRESETS, Parser};
use crate::ruleset::Diff;
use crate::ruleset::registry::{Rulesets, SaveError};
use crate::server::handlers::{compute_matches, draft_fields, field_rows, test_results, test_rows};
use crate::server::matches::{Edits, Rules};
use crate::server::{components, matches};
use crate::services::Services;
use crate::store;

path_param!(parser_id);

#[page("/admin/parsers")]
async fn parser_index(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-2xl font-semibold tracking-tight">"Parsers"</h1>
                <p class="mt-1 text-sm text-slate-400">
                    "A parser composes fields into one pattern that reads a filename apart. It
                    claims nothing by itself."
                </p>
            </div>
            <a
                href="/admin/parsers/new"
                class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
            >
                "New parser"
            </a>
        </div>

        if engine.parsers().next().is_none() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No parser is declared."
            </p>
        } else {
            <ul id="parsers" class="mt-6 flex scroll-mt-24 flex-col gap-3">
                for parser in engine.parsers() {
                    components::parser_card(parser: parser)
                }
            </ul>
        }
    }
}

#[page("/admin/parsers/new")]
async fn new_parser() -> Result {
    view! {
        parser_editor(parser: None)
    }
}

#[page("/admin/parsers/{parser_id}")]
async fn parser_editor_page(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let parser = engine
        .parser(path_param::<ParserId>(cx))
        .ok_or_not_found()?;

    view! {
        parser_editor(parser: Some(parser))
    }
}

/// The one page that writes a parser, whether or not one is stored.
///
/// A parser being created and one being edited differ in what the page
/// already knows, not in what the reader does, so both read the same form.
/// [`None`] shows Create, because a parser nothing has saved has nothing to
/// save over.
#[component]
async fn parser_editor(parser: Option<&Parser>) -> Result {
    let name = parser.map(|parser| parser.name.clone()).unwrap_or_default();

    let parser_id = parser.map(|parser| parser.id.clone()).unwrap_or_default();
    let stored_id = parser_id.clone();

    // What the browser posts on the first keystroke, so the draft starts
    // where the render left off.
    let initial_draft = ParserForm {
        name: name.clone(),
        fields: parser
            .map(|parser| parser.fields.clone())
            .unwrap_or_default(),
        tests: parser
            .map(|parser| parser.tests.clone())
            .unwrap_or_default(),
    }
    .encode();
    let initial_rows = initial_draft.clone();

    view! {
        signal draft = initial_draft;
        signal rows = initial_rows;
        // The id the save names. A handler outlives the render that built it,
        // so the argument comes from a signal rather than from a capture.
        signal save_id = stored_id;
        signal diff = String::new();
        signal saving = false;
        signal save_error = String::new();
        signal saved = 0.0;

        // The row buttons the shard renders reach the signals above through
        // this, which one delegated handler on the form below calls.
        <script>(Unescaped::new_unchecked(components::ROW_ACTIONS))</script>

        <nav class="text-sm text-slate-500">
            <a href="/admin/parsers" class="hover:text-slate-300">"Parsers"</a>
            " / "
            <span class="text-slate-300">
                if name.is_empty() { "New" } else { (&name) }
            </span>
        </nav>

        <form
            id="parser-fields"
            data-rows="true"
            method="post"
            // Only a create posts the form itself. A save runs through the
            // procedure below, and Delete names its own action, so the editor
            // of a stored parser carries none. `method` stays either way,
            // because Delete posts through it.
            if parser.is_none() {
                action="/admin/parsers"
            }
            // A keystroke moves the draft alone, because re-rendering a row
            // under the cursor takes the focus with it. A `raw!` result
            // enters the signal as the JavaScript value it is, and the shard
            // dehydrates every argument before it fetches, so a plain string
            // has to be hydrated on the way in.
            @input=$(|_e: Event| {
                draft.set(raw!(
                    "cx.hydrate(window.torrssRows.serialize())",
                    String::new()
                ));
            })
            // A row button is rendered by the shard, so its click is caught
            // here, where the signals live. The button names its action in
            // its own value, because the event vocabulary carries a target's
            // name and value and nothing structural.
            @click=$(|e: Event| if e.target.name == "row-action" {
                rows.set(raw!(
                    "cx.hydrate(window.torrssRows.apply(String(${e}.target.value)))",
                    String::new()
                ));
                draft.set(raw!(
                    "cx.hydrate(window.torrssRows.apply(String(${e}.target.value)))",
                    String::new()
                ));
            })
        >
            <div id="top" class="mt-3 flex scroll-mt-24 flex-wrap items-start justify-between gap-4">
                <div class="min-w-0 flex-1">
                    <input
                        type="text"
                        name="name"
                        value=(&name)
                        placeholder="Scene Episodes"
                        class="w-full max-w-sm rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-2xl font-semibold tracking-tight text-slate-100 focus:border-slate-600 focus:outline-none"
                    >
                </div>

                <div class="flex flex-wrap items-center gap-2">
                    match parser {
                        // Create stays a form post. A parser with no id yet
                        // has nowhere to render into, and the redirect to its
                        // own editor is what the write is for.
                        None => <button
                            type="submit"
                            class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                        >
                            "Create"
                        </button>,
                        Some(parser) => <div class="contents">
                            // A submit button that never submits. Enter inside
                            // a field activates the form's first submit
                            // button, and that has to be Save rather than
                            // Delete below it.
                            <button
                                type="submit"
                                :disabled=$(saving.get())
                                class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
                                @click=$(async |e: Event| {
                                    e.prevent_default();
                                    saving.set(true);
                                    save_error.set("".to_owned());

                                    let outcome = save_parser_draft(
                                        save_id.get(),
                                        draft.get(),
                                    ).await;

                                    if outcome.is_ok() {
                                        saved.increment();
                                    } else {
                                        save_error.set(outcome.unwrap_err());
                                    }

                                    saving.set(false);
                                })
                            >
                                $(if saving.get() { "Saving..." } else { "Save" })
                            </button>
                            // A submit button naming its own action, because
                            // the editor's form already wraps this and HTML
                            // forbids a form inside a form. It skips
                            // validation, because a delete discards the draft
                            // rather than saving it.
                            <button
                                type="submit"
                                formaction=(format!("/admin/parsers/{}/remove", parser.id))
                                formnovalidate=(true)
                                class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200"
                            >
                                "Delete"
                            </button>
                        </div>,
                    }
                </div>
            </div>

            <p
                :hidden=$(save_error.get().is_empty())
                class="mt-2 text-xs text-rose-300"
            >
                $(save_error.get())
            </p>

            <div
                id="field-rows"
                class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40"
            >
                <div class="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                    <h2 class="text-sm font-semibold text-slate-100">"Fields"</h2>
                    <p class="text-xs text-slate-500">
                        "Each field claims one run of the name, in the order they are listed."
                    </p>

                    <div class="flex flex-wrap items-center gap-2">
                        // No `name`, so `FormData` skips it and the draft
                        // never records which preset a row started from. An
                        // option carries the whole row as a form encoding,
                        // which the `add` action copies pair by pair.
                        <select
                            id="field-preset"
                            class="rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                        >
                            <option value="">"blank"</option>
                            for preset in PRESETS {
                                <option value=(parser_form::encode_preset(preset))>(preset.name)</option>
                            }
                        </select>

                        <button
                            type="button"
                            class="rounded-md border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-600 hover:text-slate-100"
                            name="row-action"
                            value="add"
                        >
                            "Add field"
                        </button>
                    </div>
                </div>

                field_rows(rows: $(rows.get()))
            </div>

            <div
                id="tests"
                class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40"
            >
                <div class="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                    <h2 class="text-sm font-semibold text-slate-100">"Tests"</h2>
                    <button
                        type="button"
                        class="rounded-md border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-600 hover:text-slate-100"
                        name="row-action"
                        value="add-test"
                    >
                        "Add test"
                    </button>
                </div>
                <p class="px-4 pb-3 text-xs text-slate-500">
                    "An expected value is compared in its normalized form: a number without
                    leading zeros, text lowercased with separators collapsed to spaces. An
                    empty value asserts nothing."
                </p>

                test_rows(rows: $(rows.get()))

                test_results(draft: $(draft.get()))
            </div>

            // The section sits inside the form so the form's own `@click`
            // catches a row's Save as test, which writes a test row. Every
            // control the section renders is a `type="button"`, so none of
            // them submits the form around them.
            //
            // The chips are rendered by the shard, so their click is caught
            // here, where the signal lives.
            <div @click=$(|e: Event| if e.target.name == "diff-filter" {
                diff.set(e.target.value);
            })>
                parser_matches(
                    parser: $(parser_id),
                    diff: $(diff.get()),
                    draft: $(draft.get()),
                    saved: $(saved.get()),
                )
            </div>
        </form>
    }
}

/// Re-renders the Matches section against the draft the editor holds.
///
/// The draft is the form's own body, so what the reader typed reaches the
/// fields without a save. Every argument crosses the network and none of it
/// is trusted: the parser is looked up rather than taken, and a draft that
/// does not parse reports itself instead of matching anything.
#[shard]
async fn parser_matches(
    cx: &Cx,
    parser: String,
    diff: String,
    draft: String,
    saved: f64,
) -> Result {
    // This is read for its change alone. A save bumps it so the diff measures
    // the draft against the fields the store now holds.
    let _ = saved;

    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    // An empty id names the parser the reader is still creating. Anything
    // else is looked up, so a spoofed id renders no parser but its own.
    let stored = match parser.as_str() {
        "" => None,
        id => Some(engine.parser(id).ok_or_not_found()?),
    };

    let posted = match ParserForm::parse_draft(&draft) {
        Ok(posted) => posted,
        Err(error) => {
            return view! {
                <section id="matches" class="mt-8 scroll-mt-24">
                    <h2 class="text-lg font-semibold tracking-tight">"Matches"</h2>
                    <p class="mt-2 text-xs text-rose-300">(error.to_string())</p>
                </section>
            };
        }
    };

    let after = draft_fields(None, &posted.fields);

    let services = app_context::<Services>(cx);
    let items = store::items(&services.db, None).await?;

    // A parser with nothing saved has no fields to lose, so the whole draft
    // reads as gained.
    let before = stored.map_or_else(Rules::default, |stored| {
        let fields = stored.fields.iter().collect::<Vec<_>>();

        matches::rules(&fields, &[], &Edits::default()).0
    });

    let (matched, errors) = compute_matches(
        app_context::<Arc<FeedRegistry>>(cx),
        &before,
        &after,
        &[],
        &items,
    );

    let editor_path = format!("/admin/parsers/{parser}");

    view! {
        components::match_section(
            editor: &editor_path,
            matched: &matched,
            errors: &errors,
            filter: Diff::from_slug(&diff),
        )
    }
}

/// Creates a parser from the new-parser form, then opens its editor.
///
/// The id comes from the name, counting up past a slug already taken. It
/// never changes after, so anything that later names the parser survives
/// every rename.
#[route(POST "/admin/parsers")]
async fn create_parser(cx: &Cx, form: RawForm) -> Result<SeeOther> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let posted = posted(&form)?;

    let id = {
        let engine = rulesets.engine();

        parser_form::unique_slug(&posted.name, |id| engine.parser(id).is_some())
            .ok_or_else(|| bad_request("the name has no letters or digits to build an id from"))?
    };

    rulesets
        .save_parser(Parser {
            id: id.clone(),
            name: posted.name,
            fields: posted.fields,
            tests: posted.tests,
        })
        .await
        .map_err(write_failed)?;

    Ok(see_other(format!("/admin/parsers/{id}")))
}

/// Saves an edited parser, and reports its name or why it was refused.
///
/// The id stays as it was, because renaming a parser never moves it.
///
/// A refusal arrives inside [`Ok`] rather than as an error. A procedure's
/// [`Err`] reaches no expression in the browser, so a caller that reads one
/// never learns the call ended and leaves its button reading Saving.
#[procedure]
async fn save_parser_draft(cx: &Cx, id: String, draft: String) -> Result<Result<String, String>> {
    let posted = match ParserForm::parse(&draft) {
        Ok(posted) => posted,
        Err(error) => return Ok(Err(error.to_string())),
    };

    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let name = posted.name.clone();

    let saved = rulesets
        .save_parser(Parser {
            id,
            name: posted.name,
            fields: posted.fields,
            tests: posted.tests,
        })
        .await;

    match saved {
        Ok(()) => Ok(Ok(name)),
        Err(error @ SaveError::Engine { .. }) => Ok(Err(error.to_string())),
        Err(error) => {
            error!(error = %error, "save failed");

            Ok(Err("the parser was not stored".to_owned()))
        }
    }
}

/// Deletes a parser, then returns to the index.
#[route(POST "/admin/parsers/{parser_id}/remove")]
async fn remove_parser(cx: &Cx) -> Result<SeeOther> {
    let removed = app_context::<Arc<Rulesets>>(cx)
        .remove_parser(path_param::<ParserId>(cx))
        .await
        .map_err(write_failed)?;

    if !removed {
        return Err(not_found().into());
    }

    Ok(see_other("/admin/parsers"))
}

/// Reads a posted parser, or answers 400 saying what to change.
///
/// The body arrives raw rather than through a typed form, because the editor
/// adds and removes field rows in the browser and the row count is not known
/// here.
fn posted(RawForm(body): &RawForm) -> Result<ParserForm> {
    let body = str::from_utf8(body).map_err(|_| bad_request("the form is not valid UTF-8"))?;

    ParserForm::parse(body).map_err(|error| bad_request(error.to_string()).into())
}

/// Reports a failed write to the reader.
///
/// A set that does not compile is the reader's own pattern, so it answers
/// 400 carrying the message. A database that refuses the row is not, so it
/// answers 500.
fn write_failed(error: SaveError) -> Error {
    match error {
        SaveError::Engine { .. } | SaveError::InUse { .. } => bad_request(error.to_string()).into(),
        SaveError::Store { .. } => internal_server_error(error).into(),
    }
}
