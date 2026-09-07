//! The page a reader states their preference lists on.
//!
//! A search says which releases are wanted. These lists say which copy of
//! one is best, so the Results page keeps that copy and hides the rest.
//!
//! Every control here edits one field's list and writes it whole, because
//! that is the shape the store keeps. Each procedure reads the stored lists
//! before it edits them, so an edit acts on what is stored rather than on
//! what the browser last rendered.

use std::sync::Arc;

use topcoat::{
    Result,
    context::Cx,
    context::app_context,
    router::page,
    runtime::{Event, procedure, shard},
    view::view,
};
use tracing::error;

use crate::preference;
use crate::search::registry::Searches;
use crate::services::Services;

/// The page that edits every preference list.
#[page("/preferences")]
async fn preferences() -> Result {
    view! {
        signal version = 0.0;

        <h1 class="text-2xl font-semibold tracking-tight">"Preferences"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "These lists rank the results of every search. The first value
            wins, and a value no list names ranks below every value one does."
        </p>

        // Every control the shard renders is caught here, where the signal
        // lives. A bump re-reads the store, so the card shows what was
        // written rather than what the click assumed.
        <div
            // The three arms are exclusive, so the target moves into whichever
            // one runs. Separate `if` blocks each claim it instead.
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
                }
            })
            // The input carries its field in its id, because its value is
            // the value being added.
            @change=$(async |e: Event| if e.target.name == "new-value" {
                add_preference(e.target.id, e.target.value).await;
                version.increment();
            })
        >
            preference_cards(version: $(version.get()))
        </div>
    }
}

/// One card per field a list ranks by, with the values it holds.
///
/// A field no parser reads is listed after the rest when a list still names
/// it. A parser edit strands such a list, and a stranded one needs a way
/// out.
#[shard]
async fn preference_cards(cx: &Cx, version: f64) -> Result {
    // This is read for its change alone. An edit bumps it so the card shows
    // what the store now holds.
    let _ = version;

    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Searches>>(cx).engine();

    let stated = preference::store::all(&services.db).await?;
    let mut fields = preference::rankable_fields(&engine);

    let stranded: Vec<String> = stated
        .fields()
        .filter(|named| !fields.iter().any(|known| known == named))
        .map(ToOwned::to_owned)
        .collect();

    fields.extend(stranded);

    view! {
        if fields.is_empty() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No parser reads a field a list ranks by."
            </p>
        } else {
            <ul class="mt-6 flex flex-col gap-3">
                for field in &fields {
                    <li class="rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4">
                        <h2 class="text-sm font-semibold text-slate-100">(field)</h2>

                        if stated.values(field).is_empty() {
                            <p class="mt-2 text-xs text-slate-500">
                                "No preference. Every value ranks alike."
                            </p>
                        } else {
                            <ol class="mt-3 flex flex-col gap-2">
                                for (index, value) in stated.values(field).iter().enumerate() {
                                    <li class="flex flex-wrap items-center gap-3">
                                        <span class="w-6 text-xs text-slate-500">(index + 1)</span>
                                        <span class="min-w-0 flex-1 font-mono text-sm break-all text-slate-200">
                                            (value)
                                        </span>
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
                    </li>
                }
            </ul>
        }
    }
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
    let stated = preference::store::all(&services.db).await?;
    let mut values = stated.values(&field).to_vec();

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
    let stated = preference::store::all(&services.db).await?;
    let mut values = stated.values(field).to_vec();

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
    let stated = preference::store::all(&services.db).await?;
    let mut values = stated.values(field).to_vec();

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
