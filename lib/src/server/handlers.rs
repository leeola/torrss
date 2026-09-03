use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use topcoat::{
    Error, Result,
    context::Cx,
    context::app_context,
    router::{
        content::{Form, RawForm},
        error::{
            RouterErrorExt, SeeOther, bad_request, internal_server_error, not_found, see_other,
        },
        page, path_param, query_params, route,
    },
    runtime::{Event, shard},
    view::{class, component, view},
};
use url::Url;

use crate::{
    feed::registry::{self, FeedRegistry},
    grab,
    rules::Engine,
    ruleset::form::{self, RulesetForm},
    ruleset::registry::{Rulesets, SaveError},
    ruleset::{Diff, Field, Ruleset},
    server::{
        components::{self, Claimant, Grabbed, ItemDetails},
        format,
        listing::{self, Standing},
        matches::{self, Edits, Match, PatternError},
        query::{self, IdList},
    },
    services::Services,
    store::grabs::{self, Grab},
    store::{self, StoredItem, library},
    torrent::scan::{self, ScanState},
};

path_param!(ruleset_id);
path_param!(feed_id);

/// Controls which stored items show and which of them are selected.
#[query_params(error = bad_request)]
struct FeedView {
    /// [`FeedEntry::id`](crate::feed::registry::FeedEntry::id) of the only
    /// feed to list, or absent for all.
    feed: Option<String>,

    /// Comma-separated [`StoredItem::id`] values marked to grab.
    selected: Option<String>,

    /// `1` to list every stored item, or absent for the wanted ones alone.
    all: Option<String>,
}

impl FeedView {
    fn active(&self) -> Option<&str> {
        self.feed.as_deref().filter(|id| !id.is_empty())
    }

    fn selection(&self) -> IdList<'_> {
        IdList::new(self.selected.as_deref())
    }

    fn show_all(&self) -> bool {
        self.all.as_deref() == Some("1")
    }
}

/// Builds a feed URL carrying the feed filter, the selection, and the mode.
///
/// The parameter is `filter` rather than `feed`, because the `#[page]`
/// attribute below puts a unit struct named `feed` in this scope.
fn feed_url(filter: Option<&str>, selected: &str, all: bool, anchor: &str) -> String {
    query::url(
        "/",
        &[
            ("feed", filter.unwrap_or_default()),
            ("selected", selected),
            ("all", if all { "1" } else { "" }),
        ],
        anchor,
    )
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
    let active = view.active();
    let selection = view.selection();
    let show_all = view.show_all();

    let registry = app_context::<Arc<FeedRegistry>>(cx);
    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    // Named `registered` rather than `feeds`, because the `#[page]` attribute
    // on the admin list below puts a unit struct named `feeds` in this scope.
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

    // The selection, the select-all link, and the grab form all work from
    // the listed rows, so hidden rows leave the set entirely rather than
    // staying selectable behind a filter.
    let mut listed: Vec<(&StoredItem, Standing)> = items.iter().zip(standings).collect();
    if !show_all {
        listed.retain(|(_, standing)| standing.is_wanted());
    }

    let ids: Vec<String> = listed.iter().map(|(item, _)| item.id.to_string()).collect();

    let grabbed = grabs::all(&services.db).await?;
    let details: Vec<ItemDetails> = listed
        .iter()
        .map(|(item, standing)| item_details(&engine, registry, standing, &grabbed, now, item))
        .collect();

    // Selecting every listed item is one link, so the target is the whole
    // listed set rather than a toggle of what is already selected.
    let all_listed = ids.join(",");
    let selected_here = ids.iter().filter(|id| selection.contains(id)).count();
    let all_selected = selected_here == ids.len() && !ids.is_empty();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Feed results"</h1>
        <p class="mt-1 text-sm text-slate-400">
            if show_all {
                (format::count(ids.len(), "item", "items"))
            } else {
                (format::count(ids.len(), "wanted release", "wanted releases"))
            }
            " from " (format::count(registered.len(), "feed", "feeds"))
            if hidden_count > 0 {
                if show_all { ", " } else { ", hidden: " }
                (owned_count) " owned, "
                (disabled_count) " disabled, "
                (unmatched_count) " unmatched"
            }
            "."
            " "
            <a
                href=(feed_url(active, selection.as_str(), !show_all, "#results"))
                class="underline decoration-slate-700 underline-offset-2 hover:text-slate-200"
            >
                if show_all { "Show wanted only" } else { "Show all" }
            </a>
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

        <nav class="mt-6 flex flex-wrap gap-2">
            components::filter_chip(
                href: feed_url(None, selection.as_str(), show_all, "#results"),
                label: "All",
                current: active.is_none(),
            )
            for entry in &registered {
                components::filter_chip(
                    href: feed_url(Some(&entry.id), selection.as_str(), show_all, "#results"),
                    label: entry.name.as_str(),
                    current: active == Some(entry.id.as_str()),
                )
            }
        </nav>

        <div
            id="results"
            class="mt-6 flex scroll-mt-24 flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3"
        >
            <div class="flex flex-wrap items-center gap-3">
                components::checkbox(
                    href: feed_url(
                        active,
                        if all_selected { "" } else { all_listed.as_str() },
                        show_all,
                        "#results",
                    ),
                    checked: all_selected,
                    label: "Select every listed release",
                )
                <span class="text-sm text-slate-300">
                    if selection.is_empty() {
                        "Nothing selected"
                    } else {
                        (selection.len()) " selected"
                    }
                </span>
                if !selection.is_empty() {
                    <a
                        href=(feed_url(active, "", show_all, "#results"))
                        class="text-xs text-slate-500 underline decoration-slate-700 underline-offset-2 hover:text-slate-300"
                    >
                        "Clear"
                    </a>
                }
            </div>

            <div class="flex items-center gap-2">
                components::action_button(
                    action: query::url(
                        "/feeds/check",
                        &[("back", feed_url(active, selection.as_str(), show_all, "#results").as_str())],
                        "",
                    ),
                    label: "Fetch now",
                )

                <form method="post" action="/grab" class="contents">
                    <input type="hidden" name="selected" value=(selection.as_str())>
                    <input type="hidden" name="feed" value=(active.unwrap_or_default())>
                    <input type="hidden" name="all" value=(if show_all { "1" } else { "" })>
                    <button
                        type="submit"
                        disabled=(selection.is_empty())
                        class="rounded-md bg-sky-400 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-sky-300 disabled:cursor-not-allowed disabled:bg-slate-800 disabled:text-slate-500"
                    >
                        if selection.is_empty() {
                            "Grab selected"
                        } else {
                            "Grab " (format::count(selection.len(), "release", "releases"))
                        }
                    </button>
                </form>
            </div>
        </div>

        if listed.is_empty() {
            <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                if show_all { "No item in this feed yet." } else { "No wanted release yet." }
            </p>
        } else {
            <ul class="mt-4 flex flex-col gap-2">
                for ((item, _), (id, shown)) in listed.iter().zip(ids.iter().zip(&details)) {
                    components::item_row(
                        engine: &engine,
                        item: item,
                        details: shown,
                        toggle_href: feed_url(
                            active,
                            &selection.toggled(id),
                            show_all,
                            &format!("#item-{id}"),
                        ),
                        selected: selection.contains(id),
                    )
                }
            </ul>
        }
    }
}

/// What the feed page's grab form posts.
#[derive(Deserialize)]
struct GrabForm {
    /// The selection, in the comma-separated form the page carries it in.
    selected: String,

    /// The feed filter to return to, empty for every feed.
    feed: Option<String>,

    /// `1` when the grab came from the show-all view, so it returns there.
    all: Option<String>,
}

/// Grabs every selected item, then returns to the listing with the
/// selection cleared.
///
/// One failure never stops the rest. Each grab records its own outcome, so
/// the loop discards the result and the reader learns which release failed
/// from its badge rather than from a page that refuses to render.
///
/// The library is rescanned once at the end rather than once per item. A scan
/// lists the client's whole queue, so a call per release would repeat that
/// listing for the same answer. Doing it before the redirect means the
/// listing already shows what was just grabbed.
#[route(POST "/grab")]
async fn grab_selected(cx: &Cx, Form(input): Form<GrabForm>) -> Result<SeeOther> {
    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    for entry in IdList::new(Some(&input.selected)).entries() {
        // A selection is whatever arrived in the URL, so an entry that is
        // not an id, or names a row since removed, is skipped rather than
        // failing the whole submission.
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
    }

    scan::scan(
        app_context::<Arc<ScanState>>(cx),
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &engine,
    )
    .await;

    Ok(see_other(feed_url(
        input.feed.as_deref().filter(|id| !id.is_empty()),
        "",
        input.all.as_deref() == Some("1"),
        "#results",
    )))
}

/// Where a switch returns the reader after it flips a ruleset.
#[query_params(error = bad_request)]
struct SwitchReturn {
    back: Option<String>,
}

/// Flips one ruleset between enabled and disabled, then returns to `back`.
///
/// This posts rather than links because it writes the stored rulesets. It
/// redirects instead of rendering, so a reload never repeats the flip.
#[route(POST "/admin/rulesets/{ruleset_id}/enabled")]
async fn set_enabled(cx: &Cx) -> Result<SeeOther> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let id = path_param::<RulesetId>(cx);

    // The engine snapshot drops before the write. Reading the state and
    // flipping it are two steps either way, and holding the snapshot across
    // the write widens the gap between them for nothing.
    let enabled = rulesets.engine().ruleset(id).ok_or_not_found()?.enabled;

    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin".to_owned());

    rulesets
        .set_enabled(id, !enabled)
        .await
        .map_err(internal_server_error)?;

    Ok(see_other(back))
}

/// Checks every registered feed now, then returns to `back`.
///
/// This runs the same pass the poll task runs, so a reader who just added a
/// feed sees its items without waiting out the interval.
///
/// A pass that overlaps the poll task is harmless. Ingest upserts inside a
/// transaction keyed on the feed and the item's guid, so two passes converge
/// on the same rows rather than doubling them.
#[route(POST "/feeds/check")]
async fn check_feeds(cx: &Cx) -> Result<SeeOther> {
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/".to_owned());

    let services = app_context::<Services>(cx);

    registry::check_all(
        app_context::<Arc<FeedRegistry>>(cx),
        &services.db,
        services.feeds.as_ref(),
        services.clock.as_ref(),
    )
    .await;

    Ok(see_other(back))
}

/// Builds the action a ruleset's switch posts to.
fn switch_action(ruleset: &str, back: &str) -> String {
    query::url(
        &format!("/admin/rulesets/{ruleset}/enabled"),
        &[("back", back)],
        "",
    )
}

#[page("/admin")]
async fn admin(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    view! {
        <div class="flex flex-wrap items-end justify-between gap-4">
            <div>
                <h1 class="text-2xl font-semibold tracking-tight">"Rulesets"</h1>
                <p class="mt-1 text-sm text-slate-400">
                    "A ruleset decides which filenames it claims and which parts it pulls out of
                    them. A disabled ruleset filters nothing, so its releases stay out of the feed."
                </p>
            </div>
            <a
                href="/admin/rulesets/new"
                class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
            >
                "New ruleset"
            </a>
        </div>

        if engine.bases().next().is_none() {
            <p class="mt-6 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No ruleset is declared."
            </p>
        } else {
            <ul id="rulesets" class="mt-6 flex scroll-mt-24 flex-col gap-3">
                for base in engine.bases() {
                    components::ruleset_card(
                        ruleset: base,
                        parent: engine.parent(base),
                        nested: false,
                        enabled: base.enabled,
                    )

                    for child in engine.children(base) {
                        components::ruleset_card(
                            ruleset: child,
                            parent: Some(base),
                            nested: true,
                            enabled: child.enabled,
                        )
                    }
                }
            </ul>
        }
    }
}

#[page("/admin/feeds")]
async fn feeds(cx: &Cx) -> Result {
    let entries = app_context::<Arc<FeedRegistry>>(cx).entries();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Feeds"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Every registered feed is polled on the configured interval."
        </p>

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
                            components::action_button(
                                action: format!("/admin/feeds/{}/remove", entry.id),
                                label: "Remove",
                            )
                        </div>
                    </li>
                }
            </ul>
        }

        <form
            method="post"
            action="/admin/feeds"
            class="mt-6 flex flex-col gap-4 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4"
        >
            <div>
                <label for="name" class="block text-xs text-slate-500">"Name"</label>
                <input
                    id="name"
                    type="text"
                    name="name"
                    placeholder="Public Wave series"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <div>
                <label for="url" class="block text-xs text-slate-500">"Feed URL"</label>
                <input
                    id="url"
                    type="text"
                    name="url"
                    placeholder="https://tracker.example/rss"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <button
                type="submit"
                class="cursor-pointer self-start rounded-md bg-sky-400 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-sky-300"
            >
                "Add feed"
            </button>
        </form>
    }
}

#[page("/admin/client")]
async fn client(cx: &Cx) -> Result {
    let entries = app_context::<Arc<FeedRegistry>>(cx).entries();
    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    // Bound as `checked` rather than `client`, because the `#[page]`
    // attribute above puts a unit struct named `client` in scope.
    let checked = services.torrents.check().await;
    let scanned = app_context::<Arc<ScanState>>(cx).last();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Client"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "What this application talks to, and how each one last answered."
        </p>

        <h2 class="mt-6 text-sm font-semibold text-slate-300">"Torrent client"</h2>

        <div class="mt-2 flex flex-wrap items-center justify-between gap-3 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-3">
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

            components::action_button(
                action: "/admin/torrents/scan?back=/admin/client",
                label: "Scan now",
            )
        </div>

        <h2 class="mt-8 text-sm font-semibold text-slate-300">"Feeds"</h2>
        <p class="mt-1 text-sm text-slate-400">
            "How each feed answered when it was last checked."
        </p>

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

                        components::action_button(
                            action: format!(
                                "/admin/feeds/{}/check?back=/admin/client",
                                entry.id,
                            ),
                            label: "Test",
                        )
                    </li>
                }
            </ul>
        }
    }
}

/// Scans the library from the torrent client now, then returns to `back`.
///
/// This runs the same pass the scan task runs, so a reader who just added a
/// torrent by hand sees it counted without waiting out the interval.
///
/// A pass that overlaps the scan task is harmless. Each pass rewrites the
/// whole table in one transaction, so two passes converge on the snapshot
/// the client reported last.
#[route(POST "/admin/torrents/scan")]
async fn scan_library(cx: &Cx) -> Result<SeeOther> {
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin/client".to_owned());

    let services = app_context::<Services>(cx);
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    scan::scan(
        app_context::<Arc<ScanState>>(cx),
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &engine,
    )
    .await;

    Ok(see_other(back))
}

/// Checks one feed now, then returns to `back`.
///
/// A feed that fails to answer still counts as checked, so this reports a
/// missing feed rather than a failed fetch. The recorded outcome is what
/// carries the failure to the page.
#[route(POST "/admin/feeds/{feed_id}/check")]
async fn check_feed(cx: &Cx) -> Result<SeeOther> {
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin/client".to_owned());

    let services = app_context::<Services>(cx);

    let checked = registry::check(
        app_context::<Arc<FeedRegistry>>(cx),
        &services.db,
        services.feeds.as_ref(),
        services.clock.as_ref(),
        path_param::<FeedId>(cx),
    )
    .await;

    if !checked {
        return Err(not_found().into());
    }

    Ok(see_other(back))
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

/// What the add form posts.
#[derive(Deserialize)]
struct NewFeed {
    name: String,
    url: String,
}

/// Registers a feed, then returns to the list.
///
/// A blank name falls back to the URL host, then to the whole URL, so a
/// registration always has something to show in a chip and a row.
#[route(POST "/admin/feeds")]
async fn add_feed(cx: &Cx, Form(input): Form<NewFeed>) -> Result<SeeOther> {
    let url = Url::parse(input.url.trim()).map_err(|_| bad_request("the feed URL is not valid"))?;

    let name = match input.name.trim() {
        "" => url
            .host_str()
            .map_or_else(|| url.to_string(), str::to_owned),
        named => named.to_owned(),
    };

    // The form never carries credentials, so it passes none and the feed
    // keeps whatever a configuration file gave it.
    app_context::<Arc<FeedRegistry>>(cx)
        .add(name, url, None)
        .await
        .map_err(internal_server_error)?;

    Ok(see_other("/admin/feeds"))
}

/// Removes a feed, then returns to the list.
///
/// The stored items outlive the registration, because they record what a
/// tracker announced rather than who watched for it.
#[route(POST "/admin/feeds/{feed_id}/remove")]
async fn remove_feed(cx: &Cx) -> Result<SeeOther> {
    if !app_context::<Arc<FeedRegistry>>(cx)
        .remove(path_param::<FeedId>(cx))
        .await
        .map_err(internal_server_error)?
    {
        return Err(not_found().into());
    }

    Ok(see_other("/admin/feeds"))
}

/// Controls the field list under the editor.
///
/// The state rides in the URL rather than in browser state, so a reviewer
/// shares an exact view and keeps it across the reload that follows a save.
#[query_params(error = bad_request)]
struct MatchView {
    /// [`Diff::slug`] of the only state to list, or absent for every state.
    diff: Option<String>,

    /// Comma-separated stored item ids held at the top of the match list.
    ///
    /// The ids are the feed items the section matches against, so a pin
    /// survives an edit that changes which rules claim the title.
    pinned: Option<String>,

    /// Comma-separated [`ruleset::Field::name`] values flipped between
    /// inherited and replaced since the last save.
    replaced: Option<String>,
}

impl MatchView {
    fn filter(&self) -> Option<Diff> {
        Diff::from_slug(self.diff.as_deref()?)
    }

    /// The whole query state, ready to rebuild any link on the page.
    fn query(&self) -> EditorQuery<'_> {
        EditorQuery {
            diff: self.filter(),
            pinned: IdList::new(self.pinned.as_deref()),
            replaced: IdList::new(self.replaced.as_deref()),
        }
    }
}

/// Every key the editor carries in its URL.
///
/// A control rebuilds the whole struct with one field changed rather than
/// editing a single query key. Changing the filter therefore never drops the
/// pins, and replacing a field never drops the filter.
#[derive(Clone, Copy)]
struct EditorQuery<'a> {
    diff: Option<Diff>,
    pinned: IdList<'a>,
    replaced: IdList<'a>,
}

impl EditorQuery<'_> {
    fn url(&self, ruleset: &str, anchor: &str) -> String {
        self.build(
            ruleset,
            anchor,
            self.pinned.as_str(),
            self.replaced.as_str(),
        )
    }

    /// The same query with the pin list replaced.
    fn with_pins(&self, pins: &str, ruleset: &str, anchor: &str) -> String {
        self.build(ruleset, anchor, pins, self.replaced.as_str())
    }

    /// The same query with the replaced-field list replaced.
    fn with_replaced(&self, replaced: &str, ruleset: &str, anchor: &str) -> String {
        self.build(ruleset, anchor, self.pinned.as_str(), replaced)
    }

    fn build(&self, ruleset: &str, anchor: &str, pinned: &str, replaced: &str) -> String {
        query::url(
            &format!("/admin/rulesets/{ruleset}"),
            &[
                ("diff", self.diff.map_or("", Diff::slug)),
                ("pinned", pinned),
                ("replaced", replaced),
            ],
            anchor,
        )
    }
}

#[page("/admin/rulesets/{ruleset_id}")]
async fn ruleset_editor(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let ruleset = engine
        .ruleset(path_param::<RulesetId>(cx))
        .ok_or_not_found()?;
    let view = query_params::<MatchView>(cx)?;
    let q = view.query();

    let replacements = q.replaced.entries();
    let parent = engine.parent(ruleset);
    let fields = ruleset.resolved_fields(parent, &replacements);
    let inheriting = parent.is_some();
    let enabled = ruleset.enabled;

    let ruleset_id = ruleset.id.clone();
    let diff_slug = q
        .diff
        .map_or_else(String::new, |state| state.slug().to_owned());
    let pinned_raw = q.pinned.as_str().to_owned();

    // What the browser posts on the first keystroke: the ruleset's own rows,
    // because a disabled inherited input sends nothing.
    let initial_draft = RulesetForm {
        name: ruleset.name.clone(),
        inherits: ruleset.inherits.clone(),
        fields: ruleset.fields.clone(),
    }
    .encode();

    view! {
        signal draft = initial_draft;

        <nav class="text-sm text-slate-500">
            <a href="/admin" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">(&ruleset.name)</span>
        </nav>

        <div id="top" class="mt-3 flex scroll-mt-24 flex-wrap items-start justify-between gap-4">
            <div>
                <div class="flex flex-wrap items-center gap-3">
                    <h1 class="text-2xl font-semibold tracking-tight">(&ruleset.name)</h1>
                    components::status_badge(enabled: enabled)
                </div>
            </div>

            <div class="flex flex-wrap items-center gap-2">
                <button
                    type="button"
                    class="rounded-md border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:border-slate-600 hover:text-slate-100"
                >
                    "Add field"
                </button>
                <button
                    type="submit"
                    form="ruleset-fields"
                    class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                >
                    "Save ruleset"
                </button>
                components::status_toggle(
                    enabled: enabled,
                    action: switch_action(&ruleset.id, &q.url(&ruleset.id, "#top")),
                )
                components::action_button(
                    action: format!("/admin/rulesets/{}/remove", ruleset.id),
                    label: "Delete",
                )
            </div>
        </div>

        match parent {
            Some(parent) => <p class="mt-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2 text-xs text-slate-400">
                "Narrows "
                <a
                    href=(format!("/admin/rulesets/{}", parent.id))
                    class="text-slate-200 underline decoration-slate-700 underline-offset-2 hover:text-white"
                >(&parent.name)</a>
                ". A greyed field carries the parent's value. Replace one to give this ruleset its own."
            </p>,
            None => "",
        }

        <form
            id="ruleset-fields"
            method="post"
            action=(format!("/admin/rulesets/{}", ruleset.id))
            class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40"
            // FormData skips a disabled input, so an inherited row stays out
            // of the draft and the parent's field keeps applying.
            @input=$(|_e: Event| draft.set(raw!(
                "new URLSearchParams(new FormData(document.getElementById('ruleset-fields'))).toString()",
                String::new()
            )))
        >
            // The editor has no name or parent input of its own, so it posts
            // back the ones the ruleset already carries.
            <input type="hidden" name="name" value=(&ruleset.name)>
            <input type="hidden" name="inherits" value=(ruleset.inherits.as_deref().unwrap_or_default())>

            <div class="flex flex-wrap items-center justify-between gap-3 px-4 py-3">
                <h2 class="text-sm font-semibold text-slate-100">
                    "Fields " <span class="text-slate-500">"(" (fields.len()) ")"</span>
                </h2>
                <p class="text-xs text-slate-500">
                    if inheriting {
                        "A greyed field is inherited. Use the source column to replace it."
                    } else {
                        "Each field names a capture group and the type its value parses as."
                    }
                </p>
            </div>

            for (index, resolved) in fields.iter().enumerate() {
                components::field_row(
                    index: index,
                    resolved: *resolved,
                    inheriting: inheriting,
                    toggle_href: q.with_replaced(
                            &q.replaced.toggled(&resolved.field.name),
                            &ruleset.id,
                            &format!("#field-{}", resolved.field.part.slug()),
                        ),
                )
            }

        </form>

        live_matches(
            ruleset: $(ruleset_id),
            diff: $(diff_slug),
            pinned: $(pinned_raw),
            draft: $(draft.get()),
        )
    }
}

/// Runs the saved rules and the edited rules over every stored title.
///
/// `after` is the edited field list. The page passes the saved fields until
/// the form feeds its own rows in, so a title reads unchanged rather than
/// gained or lost on a page nobody has edited yet.
///
/// The items arrive from the caller rather than being read here, because a
/// [`Match`] borrows the title it describes and cannot outlive the read.
fn compute_matches<'a>(
    registry: &FeedRegistry,
    engine: &Engine,
    ruleset: &Ruleset,
    items: &'a [StoredItem],
    after: &[&Field],
) -> (Vec<Match<'a>>, Vec<PatternError>) {
    let saved = ruleset.resolved_fields(engine.parent(ruleset), &[]);
    let saved = saved.iter().map(|field| field.field).collect::<Vec<_>>();

    let (before, _) = matches::rules(&saved, &Edits::default());
    let (after, errors) = matches::rules(after, &Edits::default());

    let matched = items
        .iter()
        .map(|item| {
            let (diff, segments) = matches::diff(&before, &after, &item.item.title);

            Match {
                id: item.id,
                segments,
                diff,
                feed: feed_name(registry, item),
            }
        })
        .collect();

    (matched, errors)
}

/// The fields a draft describes, resolved against the parent it names.
///
/// A child posts only the rows it overrides, so the parent supplies the
/// rest. Matching by name rather than by position is what lets the reader
/// reorder or drop a row without the override landing on a different field.
fn draft_fields<'a>(parent: Option<&'a Ruleset>, own: &'a [Field]) -> Vec<&'a Field> {
    let Some(parent) = parent else {
        return own.iter().collect();
    };

    parent
        .fields
        .iter()
        .map(|inherited| {
            own.iter()
                .find(|field| field.name == inherited.name)
                .unwrap_or(inherited)
        })
        .collect()
}

/// Re-renders the Matches section against the draft the editor holds.
///
/// The draft is the form's own body, so what the reader typed reaches the
/// rules without a save. Every argument crosses the network and none of it
/// is trusted: the ruleset is looked up rather than taken, and a draft that
/// does not parse reports itself instead of matching anything.
#[shard]
async fn live_matches(
    cx: &Cx,
    ruleset: String,
    diff: String,
    pinned: String,
    draft: String,
) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();
    let saved = engine.ruleset(&ruleset).ok_or_not_found()?;

    let posted = match RulesetForm::parse(&draft) {
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

    let parent = posted.inherits.as_deref().and_then(|id| engine.ruleset(id));
    let after = draft_fields(parent, &posted.fields);

    let services = app_context::<Services>(cx);
    let items = store::items(&services.db, None).await?;
    let (matched, errors) = compute_matches(
        app_context::<Arc<FeedRegistry>>(cx),
        &engine,
        saved,
        &items,
        &after,
    );

    view! {
        match_section(
            ruleset: &ruleset,
            matched: &matched,
            errors: &errors,
            query: EditorQuery {
                diff: Diff::from_slug(&diff),
                pinned: IdList::new(Some(&pinned)),
                replaced: IdList::new(None),
            },
        )
    }
}

/// The stored titles the edited rules claim, and what the edit changed.
///
/// A pinned title stays visible under every filter. Pinning exists to keep
/// one name in sight while the patterns above it change.
#[component]
async fn match_section(
    ruleset: &str,
    matched: &[Match<'_>],
    errors: &[PatternError],
    query: EditorQuery<'_>,
) -> Result {
    let filter = query.diff;
    let count = |state: Diff| matched.iter().filter(|one| one.diff == state).count();

    let (pinned, rest): (Vec<_>, Vec<_>) = matched
        .iter()
        .partition(|one| query.pinned.contains(&one.id.to_string()));

    let listed: Vec<_> = rest
        .into_iter()
        .filter(|one| filter.is_none_or(|state| one.diff == state))
        .collect();

    view! {
        <section id="matches" class="mt-8 scroll-mt-24">
            <div class="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h2 class="text-lg font-semibold tracking-tight">"Matches"</h2>
                    <p class="mt-1 text-sm text-slate-400">
                        (format::count(matched.len(), "stored title", "stored titles"))
                        " against the edited rules."
                    </p>
                </div>
                <p class="text-xs text-slate-500">
                    (count(Diff::Added)) " gained, " (count(Diff::Removed)) " lost"
                </p>
            </div>

            for error in errors {
                <p class="mt-2 text-xs text-rose-300">(&error.field) ": " (&error.message)</p>
            }

            <nav class="mt-4 flex flex-wrap gap-2">
                components::diff_filter(
                    href: EditorQuery { diff: None, ..query }.url(ruleset, "#matches"),
                    label: "All",
                    count: matched.len(),
                    current: filter.is_none(),
                )
                for state in Diff::ALL {
                    components::diff_filter(
                        href: EditorQuery { diff: Some(*state), ..query }.url(ruleset, "#matches"),
                        label: state.label(),
                        count: count(*state),
                        current: filter == Some(*state),
                    )
                }
            </nav>

            if !pinned.is_empty() {
                <div class="mt-4 rounded-lg border border-amber-400/25 bg-amber-400/5 px-3 py-3">
                    <h3 class="text-xs font-medium tracking-wide text-amber-300/80 uppercase">
                        "Pinned"
                    </h3>
                    <ul class="mt-2 flex flex-col gap-2">
                        for one in pinned {
                            components::match_row(
                                matched: one,
                                ruleset: ruleset,
                                pin_href: query.with_pins(
                                        &query.pinned.toggled(&one.id.to_string()),
                                        ruleset,
                                        &format!("#match-{}", one.id),
                                        ),
                                pinned: true,
                            )
                        }
                    </ul>
                </div>
            }

            if listed.is_empty() {
                <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                    "No stored title sits in this state."
                </p>
            } else {
                <ul class="mt-4 flex flex-col gap-2">
                    for one in listed {
                        components::match_row(
                            matched: one,
                            ruleset: ruleset,
                            pin_href: query.with_pins(
                                    &query.pinned.toggled(&one.id.to_string()),
                                    ruleset,
                                    &format!("#match-{}", one.id),
                                    ),
                            pinned: false,
                        )
                    }
                </ul>
            }
        </section>
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
        SaveError::Engine { .. } | SaveError::HasChildren { .. } => {
            bad_request(error.to_string()).into()
        }
        SaveError::Store { .. } => internal_server_error(error).into(),
    }
}

/// Creates a ruleset from the new-ruleset form, then opens its editor.
///
/// The id comes from the name, counting up past a slug already taken. It
/// never changes after, so the library rows and the grab records that carry
/// it survive every later rename.
#[route(POST "/admin/rulesets")]
async fn create_ruleset(cx: &Cx, form: RawForm) -> Result<SeeOther> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let posted = posted(&form)?;

    let id = {
        let engine = rulesets.engine();

        form::unique_slug(&posted.name, |id| engine.ruleset(id).is_some())
            .ok_or_else(|| bad_request("the name has no letters or digits to build an id from"))?
    };

    rulesets
        .save(Ruleset {
            id: id.clone(),
            name: posted.name,
            enabled: false,
            inherits: posted.inherits,
            fields: posted.fields,
        })
        .await
        .map_err(write_failed)?;

    Ok(see_other(format!("/admin/rulesets/{id}")))
}

/// Saves an edited ruleset, then returns to its editor.
///
/// The id and the enabled flag stay as they were. The form carries neither,
/// because renaming a ruleset never moves it and saving an edit is not a
/// request to start or stop it.
#[route(POST "/admin/rulesets/{ruleset_id}")]
async fn save_ruleset(cx: &Cx, form: RawForm) -> Result<SeeOther> {
    let rulesets = app_context::<Arc<Rulesets>>(cx);
    let id = path_param::<RulesetId>(cx);
    let posted = posted(&form)?;

    let enabled = rulesets.engine().ruleset(id).ok_or_not_found()?.enabled;

    rulesets
        .save(Ruleset {
            id: id.to_owned(),
            name: posted.name,
            enabled,
            inherits: posted.inherits,
            fields: posted.fields,
        })
        .await
        .map_err(write_failed)?;

    Ok(see_other(format!("/admin/rulesets/{id}")))
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

    Ok(see_other("/admin"))
}

#[page("/admin/rulesets/new")]
async fn new_ruleset(cx: &Cx) -> Result {
    let engine = app_context::<Arc<Rulesets>>(cx).engine();

    view! {
        <nav class="text-sm text-slate-500">
            <a href="/admin" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">"New"</span>
        </nav>

        <h1 class="mt-3 text-2xl font-semibold tracking-tight">"New ruleset"</h1>
        <p class="mt-1 text-sm text-slate-400">
            "Start from nothing, or narrow an existing ruleset to one series."
        </p>

        <form
            method="post"
            action="/admin/rulesets"
            class="mt-6 flex flex-col gap-4 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4"
        >
            <div>
                <label for="name" class="block text-xs text-slate-500">"Name"</label>
                <input
                    id="name"
                    type="text"
                    name="name"
                    placeholder="Coastal Ecology"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
            </div>

            <div>
                <label for="inherits" class="block text-xs text-slate-500">"Inherit from"</label>
                <select
                    id="inherits"
                    name="inherits"
                    class="mt-1 w-full rounded-md border border-slate-800 bg-slate-950 px-2 py-1.5 text-sm text-slate-100 focus:border-slate-600 focus:outline-none"
                >
                    <option value="">"Nothing. Declare every field here."</option>
                    for base in engine.bases() {
                        <option value=(&base.id)>
                            (&base.name) " (" (base.fields.len()) " fields)"
                        </option>
                    }
                </select>
                <p class="mt-2 text-xs text-slate-500">
                    "An inherited ruleset starts with every field greyed out. Replace only the
                    field that names the series, and the other rules keep tracking the parent."
                </p>
            </div>

            <div class="flex flex-wrap items-center gap-3 border-t border-slate-800 pt-4">
                <button
                    type="submit"
                    class="rounded-md bg-slate-100 px-3 py-1.5 text-sm font-medium text-slate-900 hover:bg-white"
                >
                    "Create ruleset"
                </button>
                <a href="/admin" class="text-sm text-slate-400 hover:text-slate-200">"Cancel"</a>
            </div>
        </form>
    }
}
