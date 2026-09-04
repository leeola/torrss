use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use topcoat::{
    Error, Result,
    context::Cx,
    context::app_context,
    router::{
        content::RawForm,
        error::{
            RouterErrorExt, SeeOther, bad_request, internal_server_error, not_found, see_other,
        },
        page, path_param, query_params, route,
    },
    runtime::{Event, procedure, shard},
    view::Unescaped,
    view::{class, component, view},
};
use tracing::error;
use url::Url;

use crate::{
    feed::registry::{self, FeedRegistry},
    grab,
    parser::form as parser_form,
    parser::{Field, Parser},
    rules::Engine,
    ruleset::form::{EditorRows, RulesetForm},
    ruleset::registry::{Rulesets, SaveError},
    ruleset::{Condition, Diff, Ruleset},
    server::{
        components::{self, Claimant, Grabbed, ItemDetails},
        format, held,
        listing::{self, Standing},
        matches::{self, Edits, Match, PatternError, Rules},
        query::IdList,
        verdict,
    },
    services::Services,
    store::grabs::{self, Grab},
    store::{self, StoredItem, library},
    torrent::scan::{self, ScanState},
};

path_param!(ruleset_id);
path_param!(feed_id);

/// The selection the feed page keeps while the listing re-renders.
///
/// The set is the one source of truth in the browser, so an id checked under
/// one filter survives a re-render that lists other rows. The signal beside
/// it only mirrors the set for the server.
///
/// Every selection operation returns the list the grab procedure receives, so
/// one handler writes one signal from one call.
///
/// The view operations keep the address bar in step with the signals that
/// drive the listing, so a reader shares the view they found. They replace
/// the entry rather than push one, because restoring the signals on a back
/// step needs a handler of its own and a shared link is what the URL is for.
const FEED_ACTIONS: &str = r"
window.torrssFeed = {
  chosen: new Set(),
  view: new URLSearchParams(location.search),
  boxes: () => [...document.querySelectorAll('#listing input[name=\'item\']')],
  list: () => [...window.torrssFeed.chosen].join(','),
  toggle: (value) => {
    const box = window.torrssFeed.boxes().find((one) => one.value === value);
    if (box && box.checked) {
      window.torrssFeed.chosen.add(value);
    } else {
      window.torrssFeed.chosen.delete(value);
    }

    return window.torrssFeed.list();
  },
  all: () => {
    for (const box of window.torrssFeed.boxes()) {
      box.checked = true;
      window.torrssFeed.chosen.add(box.value);
    }

    return window.torrssFeed.list();
  },
  clear: () => {
    window.torrssFeed.chosen.clear();
    for (const box of window.torrssFeed.boxes()) {
      box.checked = false;
    }

    return '';
  },
  count: () => window.torrssFeed.chosen.size,
  show: (feed) => {
    window.torrssFeed.view.set('feed', feed);
    window.torrssFeed.sync();
  },
  flip: () => {
    window.torrssFeed.view.set('all', window.torrssFeed.view.get('all') === '1' ? '' : '1');
    window.torrssFeed.sync();
  },
  sync: () => {
    for (const key of ['feed', 'all']) {
      if (!window.torrssFeed.view.get(key)) {
        window.torrssFeed.view.delete(key);
      }
    }

    const query = window.torrssFeed.view.toString();
    history.replaceState(null, '', query ? '/?' + query : '/');
  },
};
";

/// Controls which stored items the listing shows.
#[query_params(error = bad_request)]
struct FeedView {
    /// [`FeedEntry::id`](crate::feed::registry::FeedEntry::id) of the only
    /// feed to list, or absent for all.
    feed: Option<String>,

    /// `1` to list every stored item, or absent for the wanted ones alone.
    all: Option<String>,
}

impl FeedView {
    fn active(&self) -> Option<&str> {
        self.feed.as_deref().filter(|id| !id.is_empty())
    }

    fn show_all(&self) -> bool {
        self.all.as_deref() == Some("1")
    }
}

/// Names the feed a stored row came from.
///
/// A row outlives its registration, because the registry empties on a restart
/// while the rows do not. The host, then the whole URL, stands in so the row
/// still reads as something rather than as a blank column.
fn feed_name(registry: &FeedRegistry, item: &StoredItem) -> String {
    registry.name_of(&item.feed_url).unwrap_or_else(|| {
        item.feed_url
            .host_str()
            .map_or_else(|| item.feed_url.to_string(), str::to_owned)
    })
}

/// Builds everything the feed page shows about one release.
///
/// The standing arrives decided, because the page needs it before this to
/// work out which rows to list at all. The claimant list is a second pass
/// over the same rulesets: a listing runs to tens of rows, so repeating the
/// match costs less than threading one result through two shapes.
fn item_details(
    engine: &Engine,
    registry: &FeedRegistry,
    standing: &Standing,
    grabs: &HashMap<i64, Grab>,
    now: DateTime<Utc>,
    item: &StoredItem,
) -> ItemDetails {
    let title = &item.item.title;

    ItemDetails {
        rulesets: engine
            .claimants(title)
            .into_iter()
            .filter_map(|id| engine.ruleset(&id))
            .map(|ruleset| Claimant {
                id: ruleset.id.clone(),
                name: ruleset.name.clone(),
            })
            .collect(),
        values: standing
            .parsed()
            .map(|parsed| listing::parsed_values(engine, parsed))
            .unwrap_or_default(),
        hidden: standing.hidden_label(),
        feed_name: feed_name(registry, item),
        size: format::size(item.item.size),
        age: format::age(now, item.item.published),
        grab: grabs.get(&item.id).map(|grab| Grabbed {
            error: grab.error.clone(),
            age: format::age(now, Some(grab.at)),
            rulesets: grab.rulesets.clone(),
        }),
    }
}

#[page("/")]
async fn feed(cx: &Cx) -> Result {
    let view = query_params::<FeedView>(cx)?;
    let active_id = view.active().unwrap_or_default().to_owned();
    let show_all = view.show_all();

    view! {
        signal filter = active_id;
        signal all = show_all;
        signal selected = String::new();
        signal kept = String::new();
        signal count = 0.0;
        signal version = 0.0;
        signal fetching = false;
        signal grabbing = false;

        <script>(Unescaped::new_unchecked(FEED_ACTIONS))</script>

        <h1 class="text-2xl font-semibold tracking-tight">"Feed results"</h1>

        <div
            id="listing"
            // Every control the shard renders is caught here, where the
            // signals live.
            @change=$(|e: Event| {
                if e.target.name == "item" {
                    selected.set(raw!(
                        "cx.hydrate(window.torrssFeed.toggle(String(${e}.target.value)))",
                        String::new()
                    ));
                }

                count.set(raw!("cx.hydrate(window.torrssFeed.count())", 0.0));
            })
            // A view change re-renders the rows the shard lists. `kept` takes
            // the selection first and changes in the same tick, so the shard
            // checks the same rows again from one request.
            @click=$(|e: Event| {
                if e.target.name == "feed-filter" {
                    kept.set(selected.get());
                    filter.set(e.target.value);
                    raw!("window.torrssFeed.show(String(${e}.target.value))");
                }

                if e.target.name == "show-all" {
                    kept.set(selected.get());
                    all.toggle();
                    raw!("window.torrssFeed.flip()");
                }
            })
        >
            <div
                id="results"
                class="mt-6 flex scroll-mt-24 flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3"
            >
                <div class="flex flex-wrap items-center gap-3">
                    <button
                        type="button"
                        class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                        @click=$(|_e: Event| {
                            selected.set(raw!("cx.hydrate(window.torrssFeed.all())", String::new()));
                            count.set(raw!("cx.hydrate(window.torrssFeed.count())", 0.0));
                        })
                    >
                        "Select all"
                    </button>
                    <span class="text-sm text-slate-300">
                        $(if count.get() == 0.0 { "Nothing selected" } else { "" })
                        <span :hidden=$(count.get() == 0.0)>$(count.get()) " selected"</span>
                    </span>
                    <button
                        type="button"
                        :hidden=$(count.get() == 0.0)
                        class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                        @click=$(|_e: Event| {
                            selected.set(raw!("cx.hydrate(window.torrssFeed.clear())", String::new()));
                            count.set(0.0);
                        })
                    >
                        "Clear"
                    </button>
                </div>

                <div class="flex items-center gap-2">
                    <button
                        type="button"
                        :disabled=$(fetching.get())
                        class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200 disabled:cursor-not-allowed disabled:text-slate-600"
                        @click=$(async |_e: Event| {
                            fetching.set(true);
                            fetch_feeds().await;
                            fetching.set(false);
                            // A pass adds rows, so the listing refetches. The
                            // selection goes with it, because the rows the
                            // reader picked are still there.
                            kept.set(selected.get());
                            version.increment();
                        })
                    >
                        $(if fetching.get() { "Fetching..." } else { "Fetch now" })
                    </button>

                    <button
                        type="button"
                        :disabled=$(count.get() == 0.0)
                        class="rounded-md bg-sky-400 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-sky-300 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
                        @click=$(async |_e: Event| {
                            grabbing.set(true);
                            grab_items(selected.get()).await;
                            grabbing.set(false);
                            // clear() empties the browser set and returns the
                            // empty list, so one call does both.
                            selected.set(raw!("cx.hydrate(window.torrssFeed.clear())", String::new()));
                            kept.set("".to_owned());
                            count.set(0.0);
                            version.increment();
                        })
                    >
                        $(if grabbing.get() { "Grabbing…" } else { "Grab" })
                        <span :hidden=$(count.get() == 0.0)>
                            " " $(count.get()) " "
                            $(if count.get() == 1.0 { "release" } else { "releases" })
                        </span>
                    </button>
                </div>
            </div>

            feed_listing(
                filter: $(filter.get()),
                all: $(all.get()),
                kept: $(kept.get()),
                version: $(version.get()),
            )
        </div>
    }
}

/// The stored rows under the chosen filter, and what the page knows of each.
///
/// `kept` is the selection the browser holds, which the rows read their
/// checked state from. `version` is unread here and exists so a grab forces
/// a re-render once the rows it took are gone.
#[shard]
async fn feed_listing(cx: &Cx, filter: String, all: bool, kept: String, version: f64) -> Result {
    // Read for its change alone: a grab bumps it so the rows it took leave
    // the listing.
    let _ = version;

    let active = Some(filter.as_str()).filter(|id| !id.is_empty());
    let selection = IdList::new(Some(&kept));

    let registry = app_context::<Arc<FeedRegistry>>(cx);
    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    // Named `registered` rather than `feeds`, because the `#[page]` attribute
    // on the `/admin/feeds` page below puts a unit struct named `feeds` in
    // this scope.
    let registered = registry.entries();

    // A bookmark outlives the registration it names, because a restart empties
    // the registry while the rows stay. An id that names nothing lists nothing.
    // Falling back to every feed instead reads as that feed's whole contents.
    let chosen = active.and_then(|id| registry.get(id));
    let items = if active.is_some() && chosen.is_none() {
        Vec::new()
    } else {
        store::items(&services.db, chosen.as_ref().map(|entry| &entry.url)).await?
    };

    let owned = library::identities(&services.db).await?;
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let enabled = engine
        .rulesets()
        .filter(|ruleset| ruleset.enabled)
        .map(|ruleset| ruleset.id.clone())
        .collect();

    let standings: Vec<Standing> = items
        .iter()
        .map(|item| listing::standing(&engine, &enabled, &owned, &item.item.title))
        .collect();

    let owned_count = standings
        .iter()
        .filter(|standing| matches!(standing, Standing::Owned(_)))
        .count();
    let disabled_count = standings
        .iter()
        .filter(|standing| matches!(standing, Standing::Disabled(_)))
        .count();
    let unmatched_count = standings
        .iter()
        .filter(|standing| matches!(standing, Standing::Unmatched))
        .count();
    let hidden_count = owned_count + disabled_count + unmatched_count;

    let mut listed: Vec<(&StoredItem, Standing)> = items.iter().zip(standings).collect();
    if !all {
        listed.retain(|(_, standing)| standing.is_wanted());
    }

    let ids: Vec<String> = listed.iter().map(|(item, _)| item.id.to_string()).collect();

    let grabbed = grabs::all(&services.db).await?;
    let details: Vec<ItemDetails> = listed
        .iter()
        .map(|(item, standing)| item_details(&engine, registry, standing, &grabbed, now, item))
        .collect();

    view! {
        // The chips belong to the shard, because only a re-render presses the
        // one the reader picked. A component takes concrete values, so a chip
        // reads no signal of the page's own.
        //
        // No chip carries a selection. The browser holds it, so a filter
        // change re-renders the rows and leaves the set alone.
        <nav class="mt-6 flex flex-wrap gap-2">
            components::filter_chip(value: "", label: "All", current: active.is_none())
            for entry in &registered {
                components::filter_chip(
                    value: entry.id.as_str(),
                    label: entry.name.as_str(),
                    current: active == Some(entry.id.as_str()),
                )
            }
        </nav>

        <p class="mt-3 text-sm text-slate-400">
            if all {
                (format::count(ids.len(), "item", "items"))
            } else {
                (format::count(ids.len(), "wanted release", "wanted releases"))
            }
            " from " (format::count(registered.len(), "feed", "feeds"))
            if hidden_count > 0 {
                if all { ", " } else { ", hidden: " }
                (owned_count) " owned, "
                (disabled_count) " disabled, "
                (unmatched_count) " unmatched"
            }
            "."
            " "
            <button
                type="button"
                name="show-all"
                class="underline decoration-slate-700 underline-offset-2 hover:text-slate-200"
            >
                if all { "Show wanted only" } else { "Show all" }
            </button>
            if registered.is_empty() {
                " "
                <a
                    href="/admin/feeds"
                    class="underline decoration-slate-700 underline-offset-2 hover:text-slate-200"
                >
                    "Add a feed to get started."
                </a>
            }
        </p>

        if listed.is_empty() {
            <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                if all { "No item in this feed yet." } else { "No wanted release yet." }
            </p>
        } else {
            <ul class="mt-4 flex flex-col gap-2">
                for ((item, _), (id, shown)) in listed.iter().zip(ids.iter().zip(&details)) {
                    components::item_row(
                        engine: &engine,
                        item: item,
                        details: shown,
                        selected: selection.contains(id),
                    )
                }
            </ul>
        }
    }
}

/// Grabs every selected item and reports how many it took.
///
/// One failure never stops the rest. Each grab records its own outcome, so
/// the loop discards the result and the reader learns which release failed
/// from its badge rather than from a page that refuses to render.
///
/// The library is rescanned once at the end rather than once per item. A scan
/// lists the client's whole queue, so a call per release would repeat that
/// listing for the same answer.
#[procedure]
async fn grab_items(cx: &Cx, selected: String) -> Result<f64> {
    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let mut taken = 0.0;

    for entry in IdList::new(Some(&selected)).entries() {
        // A selection crosses the network like any other argument, so an
        // entry that is not an id, or names a row since removed, is skipped
        // rather than failing the whole call.
        let Ok(id) = entry.parse::<i64>() else {
            continue;
        };
        let Some(item) = store::item(&services.db, id).await? else {
            continue;
        };

        // A feed removed while its items remain leaves no credentials to
        // find, and none is the only honest answer for a download then.
        let auth = app_context::<Arc<FeedRegistry>>(cx)
            .auth_of(&item.feed_url)
            .unwrap_or_default();

        let _ = grab::grab(
            &engine,
            &services.db,
            services.downloads.as_ref(),
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &item,
            &auth,
        )
        .await;

        taken += 1.0;
    }

    scan::scan(
        app_context::<Arc<ScanState>>(cx),
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &engine,
    )
    .await;

    Ok(taken)
}

/// Flips one ruleset between enabled and disabled, and reports the new state.
///
/// The caller renders from the returned flag rather than from what it sent,
/// so the label always shows what the store holds.
///
/// The engine snapshot drops before the write. Reading the state and flipping
/// it are two steps either way, and holding the snapshot across the write
/// widens the gap between them for nothing.
#[procedure]
async fn switch_ruleset(cx: &Cx, id: String) -> Result<bool> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let enabled = rulesets.engine().ruleset(&id).ok_or_not_found()?.enabled;

    rulesets
        .set_enabled(&id, !enabled)
        .await
        .map_err(internal_server_error)?;

    Ok(!enabled)
}

/// Checks every registered feed now and reports how many it passed over.
///
/// This runs the same pass the poll task runs, so a reader who just added a
/// feed sees its items without waiting out the interval.
///
/// A pass that overlaps the poll task is harmless. Ingest upserts inside a
/// transaction keyed on the feed and the item's guid, so two passes converge
/// on the same rows rather than doubling them.
#[procedure]
async fn fetch_feeds(cx: &Cx) -> Result<f64> {
    let registry = app_context::<Arc<FeedRegistry>>(cx);
    let services = app_context::<Services>(cx);

    registry::check_all(
        registry,
        &services.db,
        services.feeds.as_ref(),
        services.clock.as_ref(),
    )
    .await;

    Ok(registry.entries().len() as f64)
}

#[page("/admin/rulesets")]
async fn ruleset_index(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-2xl font-semibold tracking-tight">"Rulesets"</h1>
                <p class="mt-1 text-sm text-slate-400">
                    "A ruleset picks a parser and decides which of the names it reads are wanted.
                    A disabled ruleset filters nothing, so its releases stay out of the feed."
                </p>
            </div>
            <a
                href="/admin/rulesets/new"
                class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
            >
                "New ruleset"
            </a>
        </div>

        if engine.rulesets().next().is_none() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No ruleset is declared."
            </p>
        } else {
            <ul id="rulesets" class="mt-6 flex scroll-mt-24 flex-col gap-3">
                for ruleset in engine.rulesets() {
                    components::ruleset_card(
                        ruleset: ruleset,
                        parser: engine.parser_of(ruleset),
                    )
                }
            </ul>
        }
    }
}

#[page("/admin/feeds")]
async fn feeds() -> Result {
    view! {
        signal version = 0.0;
        // The two inputs are bound rather than posted, so an accepted add
        // clears them and a refused one keeps what the reader typed.
        signal entry_name = String::new();
        signal entry_url = String::new();
        signal add_error = String::new();

        <h1 class="text-2xl font-semibold tracking-tight">"Feeds"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Every registered feed is polled on the configured interval."
        </p>

        // A Remove button is rendered by the shard, so its click is caught
        // here, where the signal lives.
        <div @click=$(async |e: Event| if e.target.name == "remove-feed" {
            let id = e.target.value;
            remove_feed_now(id).await;
            version.increment();
        })>
            feed_list(version: $(version.get()))
        </div>

        // The form carries no action. Its submit is caught here and stopped,
        // so Enter in either field reaches the procedure like the button.
        <form
            class="mt-6 flex flex-col gap-4 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4"
            @submit=$(async |e: Event| {
                e.prevent_default();

                let outcome = add_feed_now(entry_name.get(), entry_url.get()).await;

                if outcome.is_ok() {
                    entry_name.set("".to_owned());
                    entry_url.set("".to_owned());
                    add_error.set("".to_owned());
                    version.increment();
                } else {
                    add_error.set(outcome.unwrap_err());
                }
            })
        >
            <div>
                <label for="name" class="block text-xs text-slate-500">"Name"</label>
                <input
                    id="name"
                    type="text"
                    :value=$(entry_name.get())
                    @input=$(|e: Event| entry_name.set(e.target.value))
                    placeholder="Public Wave series"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <div>
                <label for="url" class="block text-xs text-slate-500">"Feed URL"</label>
                <input
                    id="url"
                    type="text"
                    :value=$(entry_url.get())
                    @input=$(|e: Event| entry_url.set(e.target.value))
                    placeholder="https://tracker.example/rss"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <p :hidden=$(add_error.get().is_empty()) class="text-xs text-rose-300">
                $(add_error.get())
            </p>

            <button
                type="submit"
                class="cursor-pointer self-start rounded-md bg-sky-400 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-sky-300"
            >
                "Add feed"
            </button>
        </form>
    }
}

/// Every registered feed, with the controls that read and drop one.
///
/// Test stays a link, because it opens a page rather than writing anything.
#[shard]
async fn feed_list(cx: &Cx, version: f64) -> Result {
    // This is read for its change alone. A removal bumps it so the row the
    // reader dropped leaves the list.
    let _ = version;

    let entries = app_context::<Arc<FeedRegistry>>(cx).entries();

    view! {
        if entries.is_empty() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No feed is registered."
            </p>
        } else {
            <ul class="mt-6 flex flex-col gap-2">
                for entry in &entries {
                    <li class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
                        <div class="min-w-0">
                            <p class="text-sm text-slate-200">(&entry.name)</p>
                            <p class="mt-0.5 font-mono text-xs break-all text-slate-500">
                                (entry.url.as_str())
                            </p>
                        </div>

                        <div class="flex items-center gap-2">
                            components::link_button(
                                href: format!("/admin/feeds/{}/test", entry.id),
                                label: "Test",
                            )
                            <button
                                type="button"
                                name="remove-feed"
                                value=(&entry.id)
                                class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200"
                            >
                                "Remove"
                            </button>
                        </div>
                    </li>
                }
            </ul>
        }
    }
}

#[page("/admin/client")]
async fn client() -> Result {
    view! {
        signal version = 0.0;
        signal scanning = false;
        // The feed the current check reads. It disables that one button and
        // leaves the others alive.
        signal busy = String::new();

        <h1 class="text-2xl font-semibold tracking-tight">"Client"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "What this application talks to, and how each one last answered."
        </p>

        <h2 class="mt-6 text-sm font-semibold text-slate-300">"Torrent client"</h2>

        <div class="mt-2 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
            client_status(version: $(version.get()))

            <button
                type="button"
                :disabled=$(scanning.get())
                class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200 disabled:cursor-not-allowed disabled:text-slate-600"
                @click=$(async |_e: Event| {
                    scanning.set(true);
                    scan_now().await;
                    scanning.set(false);
                    version.increment();
                })
            >
                $(if scanning.get() { "Scanning..." } else { "Scan now" })
            </button>
        </div>

        <h2 class="mt-8 text-sm font-semibold text-slate-300">"Torrents"</h2>
        <p class="mt-1 text-sm text-slate-400">
            "What the client holds that a ruleset claims, and when a grab moved it there."
        </p>

        client_torrents(version: $(version.get()))

        <h2 class="mt-8 text-sm font-semibold text-slate-300">"Feeds"</h2>
        <p class="mt-1 text-sm text-slate-400">
            "How each feed answered when it was last checked."
        </p>

        // A Test button is rendered by the shard, so its click is caught
        // here, where the signals live.
        <div @click=$(async |e: Event| if e.target.name == "check-feed" {
            let id = e.target.value;
            busy.set(id.clone());
            check_feed_now(id).await;
            busy.set("".to_owned());
            version.increment();
        })>
            feed_checks(version: $(version.get()), busy: $(busy.get()))
        </div>
    }
}

/// How the torrent client answered, and what the last scan found in it.
///
/// The block sits beside the Scan now button rather than holding it, because
/// a shard cannot reach the signals its caller declared.
#[shard]
async fn client_status(cx: &Cx, version: f64) -> Result {
    // This is read for its change alone. A scan bumps it so the counts below
    // report the pass that just ran.
    let _ = version;

    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    // Bound as `checked` rather than `client`, because the `#[page]`
    // attribute above puts a unit struct named `client` in scope.
    let checked = services.torrents.check().await;
    let scanned = app_context::<Arc<ScanState>>(cx).last();

    view! {
        <div class="min-w-0">
            <div class="flex flex-wrap items-center gap-2">
                <span class="text-sm text-slate-200">"qBittorrent"</span>
                <span class=(class!(
                    "rounded-full px-2 py-0.5 text-xs",
                    "bg-emerald-500/15 text-emerald-300" if checked.is_ok()
                        else "bg-rose-500/15 text-rose-300",
                ))>
                    if checked.is_ok() { "ok" } else { "failed" }
                </span>
            </div>

            <p class="mt-0.5 font-mono text-xs break-all text-slate-500">
                match &checked {
                    Ok(info) => (&info.version),
                    Err(error) => (error.to_string()),
                }
            </p>

            <p class="mt-1 text-xs text-slate-500">
                match &scanned {
                    None => "never scanned",
                    Some(last) => match &last.outcome {
                        Ok(report) => {
                            (report.matched) " of "
                            (format::count(report.torrents, "torrent", "torrents"))
                            " matched a ruleset, scanned "
                            (format::age(now, Some(last.at)))
                        },
                        Err(error) => {
                            "scan failed: " (error) ", "
                            (format::age(now, Some(last.at)))
                        },
                    },
                }
            </p>
        </div>
    }
}

/// The torrents the client holds that a ruleset claims, grabbed first, then
/// by the time the client added them. Each row carries what the ruleset read
/// out of the name.
///
/// The list is read from the client rather than from the library table,
/// because the state and the progress live only in the client, and the block
/// beside it already asks the client live.
#[shard]
async fn client_torrents(cx: &Cx, version: f64) -> Result {
    // This is read for its change alone. A scan bumps it so the list reports
    // what the pass that just ran left in the client.
    let _ = version;

    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    let accepted = grabs::accepted(&services.db).await?;
    let listed = match services.torrents.list().await {
        Ok(torrents) => Ok(held::held(&engine, torrents, &accepted)),
        Err(error) => Err(error.to_string()),
    };

    // The claimant and the age are resolved here rather than in the view,
    // because a row borrows both and an argument built inline dies before
    // the component reads it.
    let rows = listed.as_ref().map(|entries| {
        entries
            .iter()
            .map(|entry| {
                let claimant = Claimant {
                    id: entry.parsed.ruleset.clone(),
                    // A ruleset removed since the grab shows by its id, as a
                    // grabbed row does. The record is of what ran.
                    name: engine.ruleset(&entry.parsed.ruleset).map_or_else(
                        || entry.parsed.ruleset.clone(),
                        |ruleset| ruleset.name.clone(),
                    ),
                };

                let values = listing::parsed_values(&engine, &entry.parsed);
                let age = entry.grabbed_at.map(|at| format::age(now, Some(at)));

                (&entry.torrent, claimant, values, age)
            })
            .collect::<Vec<_>>()
    });

    view! {
        match &rows {
            Err(error) => <p class="mt-2 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "the client did not list its torrents: " (error)
            </p>,
            Ok(entries) if entries.is_empty() => <p class="mt-2 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No torrent in the client matches a ruleset."
            </p>,
            Ok(entries) => <ul class="mt-2 flex flex-col gap-2">
                for (torrent, claimant, values, age) in entries {
                    components::torrent_row(
                        torrent: torrent,
                        ruleset: claimant,
                        values: values.as_slice(),
                        ingested: age.as_deref(),
                    )
                }
            </ul>,
        }
    }
}

/// Every registered feed, with how it last answered and a control to ask now.
///
/// `busy` names the one feed a check is reading, so its button alone goes
/// dead while the others stay live. The label is plain server text, because
/// a write to `busy` re-renders the whole block anyway.
#[shard]
async fn feed_checks(cx: &Cx, version: f64, busy: String) -> Result {
    // This is read for its change alone. A finished check bumps it so the
    // row reports the answer that just arrived.
    let _ = version;

    let entries = app_context::<Arc<FeedRegistry>>(cx).entries();
    let now = app_context::<Services>(cx).clock.now();

    view! {
        if entries.is_empty() {
            <p class="mt-2 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No feed is registered. "
                <a
                    href="/admin/feeds"
                    class="underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                >
                    "Add a feed."
                </a>
            </p>
        } else {
            <ul class="mt-2 flex flex-col gap-2">
                for entry in &entries {
                    <li class="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
                        <div class="min-w-0">
                            <div class="flex flex-wrap items-center gap-2">
                                <span class="text-sm text-slate-200">(&entry.name)</span>
                                components::check_badge(check: entry.check.as_ref())
                            </div>

                            <p class="mt-0.5 font-mono text-xs break-all text-slate-500">
                                (entry.url.as_str())
                            </p>

                            <p class="mt-1 text-xs text-slate-500">
                                match &entry.check {
                                    None => "never checked",
                                    Some(check) => match &check.outcome {
                                        Ok(ingest) => {
                                            (format::count(ingest.items, "item", "items"))
                                            ", " (ingest.added) " new, checked "
                                            (format::age(now, Some(check.at)))
                                        },
                                        Err(error) => {
                                            "failed: " (error) ", checked "
                                            (format::age(now, Some(check.at)))
                                        },
                                    },
                                }
                            </p>
                        </div>

                        <button
                            type="button"
                            name="check-feed"
                            value=(&entry.id)
                            disabled=(busy == entry.id)
                            class="cursor-pointer rounded-md border border-slate-700 bg-slate-800/40 px-3 py-1.5 text-sm text-slate-400 transition-colors hover:border-slate-600 hover:text-slate-200 disabled:cursor-not-allowed disabled:text-slate-600"
                        >
                            if busy == entry.id { "Checking..." } else { "Test" }
                        </button>
                    </li>
                }
            </ul>
        }
    }
}

/// Scans the library from the torrent client now, and reports what matched.
///
/// This runs the same pass the scan task runs, so a reader who just added a
/// torrent by hand sees it counted without waiting out the interval.
///
/// A pass that overlaps the scan task is harmless. Each pass rewrites the
/// whole table in one transaction, so two passes converge on the snapshot
/// the client reported last.
///
/// A failed pass reports zero. The block the caller re-renders carries the
/// reason, which is what the reader reads. This number only tells the button
/// the call ended.
#[procedure]
async fn scan_now(cx: &Cx) -> Result<f64> {
    let state = app_context::<Arc<ScanState>>(cx);
    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    scan::scan(
        state,
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &engine,
    )
    .await;

    let matched = state
        .last()
        .and_then(|last| last.outcome.ok())
        .map_or(0, |report| report.matched);

    Ok(matched as f64)
}

/// Checks one feed now, and reports whether one is registered under that id.
///
/// A feed that fails to answer still counts as checked, so a `false` reports
/// a missing feed rather than a failed fetch. The recorded outcome is what
/// carries the failure to the page.
#[procedure]
async fn check_feed_now(cx: &Cx, id: String) -> Result<bool> {
    let services = app_context::<Services>(cx);

    Ok(registry::check(
        app_context::<Arc<FeedRegistry>>(cx),
        &services.db,
        services.feeds.as_ref(),
        services.clock.as_ref(),
        &id,
    )
    .await)
}

/// Fetches one feed now and shows what it carries.
///
/// This stores nothing and records no check, so `feed_items` and the client
/// page read the same after a test as before it. A fetch that fails renders
/// on the page rather than as an error status, because the request itself
/// succeeded: the tracker is what did not answer.
///
/// An id that names no feed is a 404.
#[page("/admin/feeds/{feed_id}/test")]
async fn test_feed(cx: &Cx) -> Result {
    let registry = app_context::<Arc<FeedRegistry>>(cx);
    let services = app_context::<Services>(cx);
    let id = path_param::<FeedId>(cx);

    let entry = registry.get(id).ok_or_not_found()?;
    let now = services.clock.now();

    let outcome = registry::preview(registry, services.feeds.as_ref(), id)
        .await
        .ok_or_not_found()?;

    view! {
        <nav class="text-sm text-slate-500">
            <a href="/admin/feeds" class="hover:text-slate-300">"Feeds"</a>
            " / "
            <span class="text-slate-300">(&entry.name)</span>
        </nav>

        <h1 class="mt-3 text-2xl font-semibold tracking-tight">(&entry.name)</h1>
        <p class="mt-1 font-mono text-xs break-all text-slate-500">(entry.url.as_str())</p>
        <p class="mt-1 text-sm text-slate-400">"Fetched just now. Nothing here was stored."</p>

        // Bound as `fetched` rather than `feed`, because the `#[page]` attribute
        // on the feed page puts a unit struct named `feed` in this scope, and
        // an arm naming it matches that struct instead of binding.
        match &outcome {
            Err(error) => <p class="mt-6 rounded-lg border border-rose-500/40 bg-rose-500/5 px-4 py-3 text-sm text-rose-300">
                "failed: " (error.to_string())
            </p>,
            Ok(fetched) => <div>
                <p class="mt-6 text-sm text-slate-300">
                    (format::count(fetched.items.len(), "item", "items"))
                </p>

                if fetched.items.is_empty() {
                    <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                        "The feed answered with no item."
                    </p>
                } else {
                    <ul class="mt-4 flex flex-col gap-2">
                        for item in &fetched.items {
                            <li class="rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
                                <span class="font-mono text-sm break-all">(&item.title)</span>

                                <div class="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1 text-xs text-slate-500">
                                    <span>
                                        if item.link.scheme() == "magnet" {
                                            "magnet"
                                        } else {
                                            "torrent file"
                                        }
                                    </span>
                                    <span>(format::size(item.size))</span>
                                    <span>
                                        if let Some(seeders) = item.seeders {
                                            (format::count(seeders as usize, "seeder", "seeders"))
                                        } else {
                                            "seeders unknown"
                                        }
                                    </span>
                                    <span>(format::age(now, item.published))</span>
                                </div>

                                <p class="mt-1 font-mono text-xs break-all text-slate-500">
                                    (item.link.as_str())
                                </p>
                            </li>
                        }
                    </ul>
                }
            </div>,
        }
    }
}

/// Registers a feed, and reports its id or why it was refused.
///
/// A blank name falls back to the URL host, then to the whole URL, so a
/// registration always has something to show in a chip and a row.
///
/// A refusal arrives inside [`Ok`] rather than as an error. A procedure's
/// [`Err`] reaches no expression in the browser, so a caller that reads one
/// never learns why the list gained no row.
///
/// A failed write reports one sentence and logs the cause. A database that
/// refuses the row is nothing the reader acts on, so the message names what
/// did not happen rather than how.
#[procedure]
async fn add_feed_now(cx: &Cx, name: String, url: String) -> Result<Result<String, String>> {
    let Ok(url) = Url::parse(url.trim()) else {
        return Ok(Err("the feed URL is not valid".to_owned()));
    };

    let name = match name.trim() {
        "" => url
            .host_str()
            .map_or_else(|| url.to_string(), str::to_owned),
        named => named.to_owned(),
    };

    // The form never carries credentials, so it passes none and the feed
    // keeps whatever a configuration file gave it.
    match app_context::<Arc<FeedRegistry>>(cx)
        .add(name, url, None)
        .await
    {
        Ok(id) => Ok(Ok(id)),
        Err(error) => {
            error!(error = %error, "add failed");

            Ok(Err("the feed was not stored".to_owned()))
        }
    }
}

/// Removes a feed, and reports whether one was registered under that id.
///
/// The stored items outlive the registration, because they record what a
/// tracker announced rather than who watched for it.
///
/// An id that names nothing reads as `false` rather than as an error. The
/// list the caller re-renders is the answer either way, and a row already
/// gone is what the reader asked for.
#[procedure]
async fn remove_feed_now(cx: &Cx, id: String) -> Result<bool> {
    let removed = app_context::<Arc<FeedRegistry>>(cx)
        .remove(&id)
        .await
        .map_err(internal_server_error)?;

    Ok(removed)
}

#[page("/admin/rulesets/new")]
async fn new_ruleset(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    view! {
        editor(engine: &engine, ruleset: None)
    }
}

#[page("/admin/rulesets/{ruleset_id}")]
async fn ruleset_editor(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let ruleset = engine
        .ruleset(path_param::<RulesetId>(cx))
        .ok_or_not_found()?;

    view! {
        editor(engine: &engine, ruleset: Some(ruleset))
    }
}

/// The one page that writes a ruleset, whether or not one is stored.
///
/// A ruleset being created and one being edited differ in what the page
/// already knows, not in what the reader does, so both read the same form.
/// [`None`] shows Create and no switch, because a ruleset nothing has saved
/// has nothing to switch on.
#[component]
async fn editor(engine: &Engine, ruleset: Option<&Ruleset>) -> Result {
    let name = ruleset
        .map(|ruleset| ruleset.name.clone())
        .unwrap_or_default();

    let ruleset_id = ruleset
        .map(|ruleset| ruleset.id.clone())
        .unwrap_or_default();

    let stored_id = ruleset_id.clone();
    let enabled_now = ruleset.is_some_and(|ruleset| ruleset.enabled);

    let parsers: Vec<&Parser> = engine.parsers().collect();

    // A new ruleset starts on the first parser the index lists, because the
    // select shows that one and a draft has to agree with what it shows.
    let named_parser = ruleset
        .map(|ruleset| ruleset.parser.clone())
        .or_else(|| parsers.first().map(|parser| parser.id.clone()))
        .unwrap_or_default();

    // What the browser posts on the first keystroke, so the draft starts
    // where the render left off.
    let initial_draft = RulesetForm {
        name: name.clone(),
        parser: named_parser.clone(),
        conditions: ruleset
            .map(|ruleset| ruleset.conditions.clone())
            .unwrap_or_default(),
        tests: ruleset
            .map(|ruleset| ruleset.tests.clone())
            .unwrap_or_default(),
    }
    .encode();
    let initial_rows = initial_draft.clone();

    view! {
        signal draft = initial_draft;
        signal rows = initial_rows;
        signal diff = String::new();
        signal enabled = enabled_now;
        // The id the switch and the save name. A handler outlives the render
        // that built it, so the argument comes from a signal rather than
        // from a capture.
        signal switch_id = stored_id;
        signal saving = false;
        signal save_error = String::new();
        signal saved = 0.0;

        // The row buttons the shard renders reach the signals above through
        // this, which one delegated handler on the form below calls.
        <script>(Unescaped::new_unchecked(components::ROW_ACTIONS))</script>

        <nav class="text-sm text-slate-500">
            <a href="/admin/rulesets" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">
                if name.is_empty() { "New" } else { (&name) }
            </span>
        </nav>

        <form
            id="ruleset-fields"
            data-rows="true"
            method="post"
            // Only a create posts the form itself. A save runs through the
            // procedure below, and Delete names its own action, so the editor
            // of a stored ruleset carries none. `method` stays either way,
            // because Delete posts through it.
            if ruleset.is_none() {
                action="/admin/rulesets"
            }
            // The parser decides which fields the condition and test rows
            // list, so picking one re-renders them. A keystroke moves the
            // draft alone, because re-rendering a row under the cursor takes
            // the focus with it. A `raw!` result enters the signal as the
            // JavaScript value it is, and the shard dehydrates every argument
            // before it fetches, so a plain string has to be hydrated on the
            // way in.
            @input=$(|e: Event| {
                if e.target.name == "parser" {
                    rows.set(raw!(
                        "cx.hydrate(window.torrssRows.serialize())",
                        String::new()
                    ));
                }
                draft.set(raw!(
                    "cx.hydrate(window.torrssRows.serialize())",
                    String::new()
                ));
            })
            // A row button is rendered by the shard, so its click is caught
            // here, where the signals live. The button names its action in
            // its own value, because the event vocabulary carries a target's
            // name and value and nothing structural. The guard keeps a click
            // on an input from re-rendering the rows under the cursor.
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
                    <div class="flex flex-wrap items-center gap-3">
                        <input
                            type="text"
                            name="name"
                            value=(&name)
                            placeholder="Coastal Ecology"
                            class="w-full max-w-sm rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-2xl font-semibold tracking-tight text-slate-100 focus:border-slate-600 focus:outline-none"
                        >
                        // The badge is bound rather than rendered by the
                        // component, because the switch below writes the
                        // signal and both have to agree without a reload.
                        // Each branch is one whole string, because the
                        // Tailwind scanner reads class names out of literals.
                        if ruleset.is_some() {
                            <span :class=$(if enabled.get() {
                                "rounded-full px-2 py-0.5 text-xs bg-emerald-500/15 text-emerald-300"
                            } else {
                                "rounded-full px-2 py-0.5 text-xs bg-slate-700/40 text-slate-400"
                            })>
                                $(if enabled.get() { "enabled" } else { "disabled" })
                            </span>
                        }
                    </div>
                    // The select carries no handler of its own: the form's
                    // delegated one above catches it, where the signals live.
                    // Every ruleset reads with a parser, so the choice is
                    // always shown and never empty while one exists.
                    <label for="parser" class="mt-3 block text-xs text-slate-500">
                        "Reads with"
                    </label>
                    <select
                        id="parser"
                        name="parser"
                        required=(true)
                        class="mt-1 w-full max-w-sm rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                    >
                        for parser in &parsers {
                            <option value=(&parser.id) selected=(parser.id == named_parser)>
                                (&parser.name) " (" (format::count(parser.fields.len(), "field", "fields")) ")"
                            </option>
                        }
                    </select>
                </div>

                <div class="flex flex-wrap items-center gap-2">
                    match ruleset {
                        // Create stays a form post. A ruleset with no id yet
                        // has nowhere to render into, and the redirect to its
                        // own editor is what the write is for.
                        None => <button
                            type="submit"
                            disabled=(parsers.is_empty())
                            class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white disabled:cursor-not-allowed disabled:bg-slate-700 disabled:text-slate-400"
                        >
                            "Create"
                        </button>,
                        Some(ruleset) => <div class="contents">
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

                                    let outcome = save_draft(
                                        switch_id.get(),
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
                            <button
                                type="button"
                                :title=$(if enabled.get() {
                                    "Stop this ruleset filtering feed results"
                                } else {
                                    "Let this ruleset filter feed results"
                                })
                                :class=$(if enabled.get() {
                                    "cursor-pointer rounded-md border px-3 py-1.5 text-sm transition-colors border-emerald-500/40 bg-emerald-500/10 text-emerald-300 hover:bg-emerald-500/20"
                                } else {
                                    "cursor-pointer rounded-md border px-3 py-1.5 text-sm transition-colors border-slate-700 bg-slate-800/40 text-slate-400 hover:border-slate-600 hover:text-slate-200"
                                })
                                @click=$(async |_e: Event| {
                                    let state = switch_ruleset(switch_id.get()).await;
                                    enabled.set(state);
                                })
                            >
                                $(if enabled.get() { "Disable" } else { "Enable" })
                            </button>
                            // A submit button naming its own action, because
                            // the editor's form already wraps this and HTML
                            // forbids a form inside a form. It skips
                            // validation, because a delete discards the draft
                            // rather than saving it.
                            <button
                                type="submit"
                                formaction=(format!("/admin/rulesets/{}/remove", ruleset.id))
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

            // A ruleset reads with a parser and there is none to pick, so the
            // page says where to make one rather than offering an empty
            // select and a Create that always fails.
            if parsers.is_empty() {
                <p class="mt-2 text-xs text-rose-300">
                    "No parser exists yet. Create one under Parsers first."
                </p>
            }

            <div
                id="conditions"
                class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40"
            >
                <div class="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                    <h2 class="text-sm font-semibold text-slate-100">"Conditions"</h2>
                    <button
                        type="button"
                        class="rounded-md border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-600 hover:text-slate-100"
                        name="row-action"
                        value="add-condition"
                    >
                        "Add condition"
                    </button>
                </div>
                <p class="px-4 pb-3 text-xs text-slate-500">
                    "The fields decide which titles have this shape, and the conditions decide
                    which of those the ruleset claims. A value compares in its normalized form.
                    An ordering compares numbers, so it needs a number, season, or episode
                    field. One of and none of take a comma-separated list, such as
                    720p, 1080p."
                </p>

                condition_rows(rows: $(rows.get()))
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
            // them submits the form around them, and the section holds no
            // input for the form's `@input` to read.
            //
            // The chips are rendered by the shard, so their click is caught
            // here, where the signal lives.
            <div @click=$(|e: Event| if e.target.name == "diff-filter" {
                diff.set(e.target.value);
            })>
                live_matches(
                    ruleset: $(ruleset_id),
                    diff: $(diff.get()),
                    draft: $(draft.get()),
                    saved: $(saved.get()),
                )
            </div>
        </form>
    }
}

/// Runs the saved rules and the edited rules over every stored title.
///
/// `before` is empty for a ruleset that is not saved yet, so every title the
/// draft claims reads as gained rather than unchanged.
///
/// The items arrive from the caller rather than being read here, because a
/// [`Match`] borrows the title it describes and cannot outlive the read.
pub(super) fn compute_matches<'a>(
    registry: &FeedRegistry,
    before: &Rules,
    after: &[&Field],
    conditions: &[Condition],
    items: &'a [StoredItem],
) -> (Vec<Match<'a>>, Vec<PatternError>) {
    let (after, errors) = matches::rules(after, conditions, &Edits::default());

    let matched = items
        .iter()
        .map(|item| {
            let diffed = matches::diff(before, &after, &item.item.title);

            Match {
                id: item.id,
                title: &item.item.title,
                segments: diffed.segments,
                values: diffed.values,
                diff: diffed.diff,
                feed: feed_name(registry, item),
            }
        })
        .collect();

    (matched, errors)
}
/// Re-renders the test rows from the draft the editor holds.
///
/// The rows follow the form rather than the save, as the field rows do, so a
/// test the reader just added carries an input for a field they added in the
/// same breath.
///
/// Only a structural change re-renders these. A keystroke takes the focus
/// out of the input under the cursor.
#[shard]
pub(super) async fn test_rows(cx: &Cx, rows: String) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    let posted = EditorRows::parse(&rows);

    // A draft that names no parser reads no value, so a test has nothing to
    // expect and the row carries no input.
    let fields = parser_fields(&engine, posted.parser.as_deref());

    view! {
        for (index, test) in posted.tests.iter().enumerate() {
            components::test_row(index: index, test: test, fields: &fields)
        }
    }
}

/// Re-renders the condition rows from the draft the editor holds.
///
/// The field select lists the draft's own fields, as the test row's inputs
/// do, so a field the reader just added is one a condition names in the same
/// breath.
#[shard]
async fn condition_rows(cx: &Cx, rows: String) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    let posted = EditorRows::parse(&rows);

    // A draft that names no parser reads no value, so the select offers
    // nothing to compare against.
    let fields = parser_fields(&engine, posted.parser.as_deref());

    view! {
        for (index, condition) in posted.conditions.iter().enumerate() {
            components::condition_row(index: index, condition: condition, fields: &fields)
        }
    }
}

/// Reports each saved test against the draft the editor holds.
///
/// The verdicts follow the draft rather than the rows, so a pattern the
/// reader is still typing is the one every test runs against. A test flips
/// between pass and failed under the cursor, with no save in between.
///
/// A draft that does not parse renders nothing. The Matches section reports
/// the same error on the same draft, and one message is enough.
#[shard]
pub(super) async fn test_results(cx: &Cx, draft: String) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    let Ok(posted) = RulesetForm::parse_draft(&draft) else {
        return view! {};
    };

    let fields = parser_fields(&engine, Some(&posted.parser));
    let rules = matches::rules(&fields, &posted.conditions, &Edits::default()).0;

    let judged = posted
        .tests
        .iter()
        .map(|test| (test, verdict::verdict(&rules, test)))
        .collect::<Vec<_>>();

    view! {
        components::test_verdicts(judged: &judged)
    }
}

/// The fields the parser named by `id` reads, or none when it names no
/// parser this engine carries.
///
/// The editor renders a draft the reader is still writing, so a parser they
/// have not picked yet is ordinary rather than an error. It reads nothing,
/// which is what an empty list says.
fn parser_fields<'a>(engine: &'a Engine, id: Option<&str>) -> Vec<&'a Field> {
    id.and_then(|id| engine.parser(id))
        .map(|parser| parser.fields.iter().collect())
        .unwrap_or_default()
}

/// Re-renders the Matches section against the draft the editor holds.
///
/// The draft is the form's own body, so what the reader typed reaches the
/// rules without a save. Every argument crosses the network and none of it
/// is trusted: the ruleset is looked up rather than taken, and a draft that
/// does not parse reports itself instead of matching anything.
#[shard]
async fn live_matches(cx: &Cx, ruleset: String, diff: String, draft: String, saved: f64) -> Result {
    // This is read for its change alone. A save bumps it so the diff measures
    // the draft against the rules the store now holds.
    let _ = saved;

    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    // An empty id names the ruleset the reader is still creating. Anything
    // else is looked up, so a spoofed id renders no ruleset but its own.
    let saved = match ruleset.as_str() {
        "" => None,
        id => Some(engine.ruleset(id).ok_or_not_found()?),
    };

    let posted = match RulesetForm::parse_draft(&draft) {
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

    let after = parser_fields(&engine, Some(&posted.parser));

    let services = app_context::<Services>(cx);

    let items = store::items(&services.db, None).await?;
    // A ruleset with nothing saved has no rules to lose, so the whole draft
    // reads as gained.
    let before = saved.map_or_else(Rules::default, |saved| {
        let fields = parser_fields(&engine, Some(&saved.parser));

        matches::rules(&fields, &saved.conditions, &Edits::default()).0
    });

    let (matched, errors) = compute_matches(
        app_context::<Arc<FeedRegistry>>(cx),
        &before,
        &after,
        &posted.conditions,
        &items,
    );

    let editor_path = format!("/admin/rulesets/{ruleset}");

    view! {
        components::match_section(
            editor: &editor_path,
            matched: &matched,
            errors: &errors,
            filter: Diff::from_slug(&diff),
        )
    }
}

/// Reads a posted ruleset, or answers 400 saying what to change.
///
/// The body arrives raw rather than through a typed form, because the editor
/// adds and removes field rows in the browser and the row count is not known
/// here.
fn posted(RawForm(body): &RawForm) -> Result<RulesetForm> {
    let body = str::from_utf8(body).map_err(|_| bad_request("the form is not valid UTF-8"))?;

    RulesetForm::parse(body).map_err(|error| bad_request(error.to_string()).into())
}

/// Reports a failed write to the reader.
///
/// A set that does not compile is the reader's own edit coming back, so it
/// reads as a 400 carrying the reason. Everything else is the application's
/// problem, not theirs.
fn write_failed(error: SaveError) -> Error {
    match error {
        SaveError::Engine { .. } | SaveError::InUse { .. } => bad_request(error.to_string()).into(),
        SaveError::Store { .. } => internal_server_error(error).into(),
    }
}

/// Creates a ruleset from the new-ruleset form, then opens its editor.
///
/// The id comes from the name, counting up past a slug already taken. It
/// never changes after, so the library rows and the grab records that carry
/// it survive every later rename.
///
/// A ruleset is stored enabled, so it filters the feed from its first save.
/// A reader who wrote a working rule meant it to run, and a ruleset that
/// claims nothing until they find the switch reads as a rule that failed.

#[route(POST "/admin/rulesets")]
async fn create_ruleset(cx: &Cx, form: RawForm) -> Result<SeeOther> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let posted = posted(&form)?;

    let id = {
        let engine = rulesets.engine();

        parser_form::unique_slug(&posted.name, |id| engine.ruleset(id).is_some())
            .ok_or_else(|| bad_request("the name has no letters or digits to build an id from"))?
    };

    rulesets
        .save(Ruleset {
            id: id.clone(),
            name: posted.name,
            enabled: true,
            parser: posted.parser,
            conditions: posted.conditions,
            tests: posted.tests,
        })
        .await
        .map_err(write_failed)?;

    Ok(see_other(format!("/admin/rulesets/{id}")))
}

/// Saves an edited ruleset, and reports its name or why it was refused.
///
/// The id and the enabled flag stay as they were. The draft carries neither,
/// because renaming a ruleset never moves it and saving an edit is not a
/// request to start or stop it.
///
/// A refusal arrives inside [`Ok`] rather than as an error. A procedure's
/// [`Err`] reaches no expression in the browser, so a caller that reads one
/// never learns the call ended and leaves its button reading Saving.
///
/// A failed write reports one sentence and logs the cause. A database that
/// refuses the row is nothing the reader acts on, so the message names what
/// did not happen rather than how.
#[procedure]
async fn save_draft(cx: &Cx, id: String, draft: String) -> Result<Result<String, String>> {
    let posted = match RulesetForm::parse(&draft) {
        Ok(posted) => posted,
        Err(error) => return Ok(Err(error.to_string())),
    };

    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let enabled = rulesets.engine().ruleset(&id).ok_or_not_found()?.enabled;
    let name = posted.name.clone();

    let saved = rulesets
        .save(Ruleset {
            id,
            name: posted.name,
            enabled,
            parser: posted.parser,
            conditions: posted.conditions,
            tests: posted.tests,
        })
        .await;

    match saved {
        Ok(()) => Ok(Ok(name)),
        Err(error @ (SaveError::Engine { .. } | SaveError::InUse { .. })) => {
            Ok(Err(error.to_string()))
        }
        Err(error) => {
            error!(error = %error, "save failed");

            Ok(Err("the ruleset was not stored".to_owned()))
        }
    }
}

/// Deletes a ruleset, then returns to the index.
#[route(POST "/admin/rulesets/{ruleset_id}/remove")]
async fn remove_ruleset(cx: &Cx) -> Result<SeeOther> {
    let removed = app_context::<Arc<Rulesets>>(cx)
        .remove(path_param::<RulesetId>(cx))
        .await
        .map_err(write_failed)?;

    if !removed {
        return Err(not_found().into());
    }

    Ok(see_other("/admin/rulesets"))
}
