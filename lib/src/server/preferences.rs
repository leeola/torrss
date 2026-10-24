//! The page a reader states their preference lists and refused values on.
//!
//! A search says which releases are wanted. These lists say which copy of
//! one is best, so the home page keeps that copy and hides the rest. A
//! refused value hides every copy that reads it.
//!
//! Every control here edits one field's list or refused set and writes it
//! whole, because that is the shape the store keeps. Each procedure reads
//! the stored values, or the library's suggestion for a list with none,
//! before it edits. An edit therefore acts on what a fresh render shows
//! rather than on what the browser last rendered.

use std::collections::BTreeSet;
use std::sync::Arc;

use topcoat::{
    Result,
    context::Cx,
    context::app_context,
    router::page,
    runtime::{Event, procedure, shard, signal},
    view::{View, component, view},
};
use tracing::error;

use super::format;
use crate::preference;
use crate::preference::library::Library;
use crate::search::registry::Searches;
use crate::services::Services;
use crate::torrent::index;

/// The page that edits every preference list.
#[page("/preferences")]
async fn preferences(cx: &Cx) -> Result<impl View> {
    let version = signal(cx, || 0.0);

    Ok(view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Preferences"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "These lists rank the results of every search. The first value
            wins, and a value no list names ranks below every value one does.
            A field with no list follows the library, where the value the most
            completed torrents hold comes first. A value a card refuses hides
            every release that reads it, whatever the lists say."
        </p>

        // Every control the shard renders is caught here, where the signal
        // lives. A bump re-reads the store, so the card shows what was
        // written rather than what the click assumed.
        <div
            // The arms are exclusive, so the target moves into whichever one
            // runs. Separate `if` blocks each claim it instead.
            @click=$(async |e: Event| {
                let target = e.target.value;

                if e.target.name == "move-up" {
                    move_preference(target, true).await;
                    version.increment();
                } else if e.target.name == "move-down" {
                    move_preference(target, false).await;
                    version.increment();
                } else if e.target.name == "remove-value" {
                    remove_preference(target).await;
                    version.increment();
                } else if e.target.name == "keep-suggested" {
                    keep_suggested(target).await;
                    version.increment();
                } else if e.target.name == "remove-unwanted" {
                    remove_unwanted(target).await;
                    version.increment();
                }
            })
            // The input carries its field in its id, because its value is
            // the value being added.
            @change=$(async |e: Event| if e.target.name == "new-value" {
                add_preference(e.target.id, e.target.value).await;
                version.increment();
            } else if e.target.name == "new-unwanted" {
                add_unwanted(e.target.id, e.target.value).await;
                version.increment();
            })
        >
            preference_cards(version: $(version.get()))
        </div>
    })
}

/// One card per field a list ranks by, with the values it holds and the
/// values it refuses.
///
/// A field no parser reads is listed after the rest when a list or a
/// refused value still names it. A parser edit strands such a list or set,
/// and a stranded one needs a way out.
#[shard]
async fn preference_cards(cx: &Cx, version: f64) -> Result<impl View> {
    // This is read for its change alone. An edit bumps it so the card shows
    // what the store now holds.
    let _ = version;

    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Searches>>(cx).engine();

    let stated = preference::store::all(&services.db).await?;
    let refused = preference::store::unwanted(&services.db).await?;
    let library = Library::new(&engine, &index::all(&services.db).await?);
    let mut fields = preference::rankable_fields(&engine);

    // A set rather than a list, because a field that strands both a list and
    // a refused set still gets one card.
    let stranded: BTreeSet<&str> = stated
        .fields()
        .chain(refused.fields())
        .filter(|named| !fields.iter().any(|known| known == named))
        .collect();

    fields.extend(stranded.into_iter().map(ToOwned::to_owned));

    // One entry per card, built here rather than in the view, because a row
    // borrows its value for as long as the view lives.
    let suggestions: Vec<Vec<(String, usize)>> = fields
        .iter()
        .map(|field| {
            if stated.values(field).is_empty() {
                library.suggested(field)
            } else {
                Vec::new()
            }
        })
        .collect();

    Ok(view! {
        if fields.is_empty() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No parser reads a field a list ranks by."
            </p>
        } else {
            <ul class="mt-6 flex flex-col gap-3">
                for (field, suggested) in fields.iter().zip(&suggestions) {
                    <li class="rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4">
                        <h2 class="text-sm font-semibold text-slate-100">(field)</h2>

                        if stated.values(field).is_empty() {
                            if suggested.is_empty() {
                                <p class="mt-2 text-xs text-slate-500">
                                    "No preference. Every value ranks alike."
                                </p>
                            } else {
                                <p class="mt-2 text-xs text-slate-500">
                                    "Suggested from the library. Every value ranks by how many
                                    completed torrents hold it, and an edit starts from this
                                    order."
                                </p>
                                <ol class="mt-3 flex flex-col gap-2">
                                    for (index, (value, torrents)) in suggested.iter().enumerate() {
                                        list_row(
                                            field: field,
                                            index: index,
                                            value: value,
                                            torrents: Some(*torrents),
                                        )
                                    }
                                </ol>
                                <div class="mt-3">
                                    <button
                                        type="button"
                                        name="keep-suggested"
                                        value=(field)
                                        class="inline-block rounded-md border border-sky-400/50 bg-sky-400/10 px-2 py-1 text-xs text-sky-300 transition-colors hover:bg-sky-400/20"
                                    >
                                        "Keep this order"
                                    </button>
                                </div>
                            }
                        } else {
                            <ol class="mt-3 flex flex-col gap-2">
                                for (index, value) in stated.values(field).iter().enumerate() {
                                    list_row(field: field, index: index, value: value, torrents: None)
                                }
                            </ol>
                        }

                        <input
                            id=(field)
                            name="new-value"
                            type="text"
                            placeholder="Type a value and press Enter"
                            class="mt-3 w-full max-w-sm rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                        >

                        <h3 class="mt-4 text-xs font-semibold text-slate-400">"Unwanted"</h3>
                        if refused.values(field).next().is_none() {
                            <p class="mt-1 text-xs text-slate-500">"No value is refused."</p>
                        } else {
                            <ul class="mt-2 flex flex-col gap-2">
                                for (index, value) in refused.values(field).enumerate() {
                                    unwanted_row(field: field, index: index, value: value)
                                }
                            </ul>
                        }
                        <input
                            id=(format!("unwanted:{field}"))
                            name="new-unwanted"
                            type="text"
                            placeholder="Type a value to refuse and press Enter"
                            class="mt-3 w-full max-w-sm rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                        >
                    </li>
                }
            </ul>
        }
    })
}

/// One value of a card's list, with the controls that move and remove it.
///
/// `torrents` is how many completed torrents hold a value the library
/// suggests, and nothing for a value the reader stated.
#[component]
async fn list_row(
    field: &str,
    index: usize,
    value: &str,
    torrents: Option<usize>,
) -> Result<impl View> {
    Ok(view! {
        <li class="flex flex-wrap items-center gap-3">
            <span class="w-6 text-xs text-slate-500">(index + 1)</span>
            <span class="min-w-0 flex-1 font-mono text-sm break-all text-slate-200">
                (value)
            </span>
            if let Some(torrents) = torrents {
                <span class="text-xs text-slate-500">
                    (format::count(torrents, "torrent", "torrents"))
                </span>
            }
            <button
                type="button"
                name="move-up"
                value=(format!("{field}:{index}"))
                title="Move up"
                class="rounded-md border border-slate-700 px-2 py-1 text-xs text-slate-400 transition-colors hover:border-slate-500 hover:text-slate-200"
            >
                "\u{2191}"
            </button>
            <button
                type="button"
                name="move-down"
                value=(format!("{field}:{index}"))
                title="Move down"
                class="rounded-md border border-slate-700 px-2 py-1 text-xs text-slate-400 transition-colors hover:border-slate-500 hover:text-slate-200"
            >
                "\u{2193}"
            </button>
            <button
                type="button"
                name="remove-value"
                value=(format!("{field}:{index}"))
                class="inline-block rounded-md border border-sky-400/50 bg-sky-400/10 px-2 py-1 text-xs text-sky-300 transition-colors hover:bg-sky-400/20"
            >
                "remove"
            </button>
        </li>
    })
}

/// One value a card refuses, with the control that takes the refusal back.
#[component]
async fn unwanted_row(field: &str, index: usize, value: &str) -> Result<impl View> {
    Ok(view! {
        <li class="flex flex-wrap items-center gap-3">
            <span class="min-w-0 flex-1 font-mono text-sm break-all text-slate-200">
                (value)
            </span>
            <button
                type="button"
                name="remove-unwanted"
                value=(format!("{field}:{index}"))
                class="inline-block rounded-md border border-sky-400/50 bg-sky-400/10 px-2 py-1 text-xs text-sky-300 transition-colors hover:bg-sky-400/20"
            >
                "remove"
            </button>
        </li>
    })
}

/// Appends `value` to the list for `field`, and reports whether it landed.
///
/// A blank value and one the list already holds both add nothing. A list
/// that names one value twice ranks it by its first place alone, so the
/// second entry is noise rather than a statement.
#[procedure]
async fn add_preference(cx: &Cx, field: String, value: String) -> Result<bool> {
    let value = value.trim();

    if value.is_empty() {
        return Ok(false);
    }

    let services = app_context::<Services>(cx);
    let mut values = effective(cx, &field).await?;

    if values.iter().any(|held| held == value) {
        return Ok(false);
    }

    values.push(value.to_owned());

    Ok(write(services, &field, &values).await)
}

/// Removes one value by its place, and reports whether one was there.
#[procedure]
async fn remove_preference(cx: &Cx, target: String) -> Result<bool> {
    let Some((field, index)) = place(&target) else {
        return Ok(false);
    };

    let services = app_context::<Services>(cx);
    let mut values = effective(cx, field).await?;

    if index >= values.len() {
        return Ok(false);
    }

    values.remove(index);

    Ok(write(services, field, &values).await)
}

/// Swaps one value with its neighbor, and reports whether it moved.
///
/// A value at the end it is asked to move toward stays put, which is what
/// the `false` says.
#[procedure]
async fn move_preference(cx: &Cx, target: String, up: bool) -> Result<bool> {
    let Some((field, index)) = place(&target) else {
        return Ok(false);
    };

    let services = app_context::<Services>(cx);
    let mut values = effective(cx, field).await?;

    let Some(swap) = (if up {
        index.checked_sub(1)
    } else {
        Some(index + 1)
    }) else {
        return Ok(false);
    };

    if index >= values.len() || swap >= values.len() {
        return Ok(false);
    }

    values.swap(index, swap);

    Ok(write(services, field, &values).await)
}

/// Stores the order the library suggests for `field` as its list, and
/// reports whether it stored.
///
/// The card offers this only for a field with no list. A field that holds
/// one keeps it, so a card left open in another tab never replaces a list
/// the reader stated since.
#[procedure]
async fn keep_suggested(cx: &Cx, field: String) -> Result<bool> {
    let values = effective(cx, &field).await?;

    if values.is_empty() {
        return Ok(false);
    }

    Ok(write(app_context::<Services>(cx), &field, &values).await)
}

/// Refuses `value` for the field `id` names, and reports whether it landed.
///
/// `id` is the input's id, the field name after an `unwanted:` prefix. The
/// prefix keeps it apart from the card's list input, whose id is the bare
/// field name. An id without the prefix refuses nothing.
///
/// A blank value and one the set already holds both add nothing.
#[procedure]
async fn add_unwanted(cx: &Cx, id: String, value: String) -> Result<bool> {
    let Some(field) = id.strip_prefix("unwanted:") else {
        return Ok(false);
    };

    let value = value.trim();

    if value.is_empty() {
        return Ok(false);
    }

    let services = app_context::<Services>(cx);
    let mut values = refused(cx, field).await?;

    if values.iter().any(|held| held == value) {
        return Ok(false);
    }

    values.push(value.to_owned());

    Ok(write_unwanted(services, field, &values).await)
}

/// Takes back one refused value by its place, and reports whether one was
/// there.
///
/// The place counts in value order, which is the order the card lists the
/// values in.
#[procedure]
async fn remove_unwanted(cx: &Cx, target: String) -> Result<bool> {
    let Some((field, index)) = place(&target) else {
        return Ok(false);
    };

    let services = app_context::<Services>(cx);
    let mut values = refused(cx, field).await?;

    if index >= values.len() {
        return Ok(false);
    }

    values.remove(index);

    Ok(write_unwanted(services, field, &values).await)
}

/// Reads a `field:index` control value apart.
///
/// The split takes the last colon, so a field name carrying one still
/// resolves. A target that names no place resolves to nothing rather than
/// to the first one.
fn place(target: &str) -> Option<(&str, usize)> {
    let (field, index) = target.rsplit_once(':')?;

    Some((field, index.parse().ok()?))
}

/// Writes one field's list, and reports whether it stored.
///
/// A failed write logs the cause and reports `false`. The reader acts on
/// the card, which a re-render leaves as it was, so a refusal has nowhere
/// to put a message of its own.
async fn write(services: &Services, field: &str, values: &[String]) -> bool {
    match preference::store::replace(&services.db, field, values).await {
        Ok(()) => true,
        Err(error) => {
            error!(error = %error, "preference not stored");

            false
        }
    }
}

/// Writes the values one field refuses, and reports whether they stored.
///
/// A failed write logs the cause and reports `false`, as [`write`] does.
async fn write_unwanted(services: &Services, field: &str, values: &[String]) -> bool {
    match preference::store::replace_unwanted(&services.db, field, values).await {
        Ok(()) => true,
        Err(error) => {
            error!(error = %error, "unwanted value not stored");

            false
        }
    }
}

/// The list the card for `field` shows, which every edit starts from.
///
/// That is the stated list, or the order the library suggests when the
/// reader stated none. A first move on a suggested field therefore stores
/// the suggestion with that one move applied.
async fn effective(cx: &Cx, field: &str) -> Result<Vec<String>> {
    let services = app_context::<Services>(cx);
    let stated = preference::store::all(&services.db).await?;

    if !stated.values(field).is_empty() {
        return Ok(stated.values(field).to_vec());
    }

    let engine = app_context::<Arc<Searches>>(cx).engine();
    let library = Library::new(&engine, &index::all(&services.db).await?);

    Ok(library
        .suggested(field)
        .into_iter()
        .map(|(value, _)| value)
        .collect())
}

/// The values the card for `field` refuses, which every refusal edit starts
/// from, in the order the card lists them.
async fn refused(cx: &Cx, field: &str) -> Result<Vec<String>> {
    let services = app_context::<Services>(cx);
    let unwanted = preference::store::unwanted(&services.db).await?;

    Ok(unwanted.values(field).map(ToOwned::to_owned).collect())
}
