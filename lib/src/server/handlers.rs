use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use topcoat::{
    Result,
    context::Cx,
    context::app_context,
    router::{
        content::Form,
        error::{
            RouterErrorExt, SeeOther, bad_request, internal_server_error, not_found, see_other,
        },
        page, path_param, query_params, route,
    },
    view::{class, view},
};
use url::Url;

use crate::{
    feed::registry::{self, FeedRegistry},
    grab,
    mock::{self, Diff, RULESETS},
    rules::ENGINE,
    server::{
        components::{self, Grabbed, ItemDetails},
        format,
        query::{self, IdList},
        state::RulesetSwitches,
    },
    services::Services,
    store::grabs::{self, Grab},
    store::{self, StoredItem, library},
    torrent::sync::{self, SyncState},
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
}

impl FeedView {
    fn active(&self) -> Option<&str> {
        self.feed.as_deref().filter(|id| !id.is_empty())
    }

    fn selection(&self) -> IdList<'_> {
        IdList::new(self.selected.as_deref())
    }
}

/// Builds a feed URL carrying the feed filter and the selection.
///
/// The parameter is `filter` rather than `feed`, because the `#[page]`
/// attribute below puts a unit struct named `feed` in this scope.
fn feed_url(filter: Option<&str>, selected: &str, anchor: &str) -> String {
    query::url(
        "/",
        &[("feed", filter.unwrap_or_default()), ("selected", selected)],
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
/// The claimants and the identity come from two passes over the same
/// rulesets. A listing runs to tens of rows, so the second pass costs less
/// than threading one result through two shapes.
fn item_details(
    registry: &FeedRegistry,
    owned: &HashSet<String>,
    grabs: &HashMap<i64, Grab>,
    now: DateTime<Utc>,
    item: &StoredItem,
) -> ItemDetails {
    let title = &item.item.title;

    ItemDetails {
        rulesets: ENGINE
            .claimants(title)
            .into_iter()
            .filter_map(mock::ruleset)
            .collect(),
        have: ENGINE
            .parse(title)
            .is_some_and(|parsed| owned.contains(&parsed.identity.to_string())),
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

    let ids: Vec<String> = items.iter().map(|item| item.id.to_string()).collect();

    let owned = library::identities(&services.db).await?;
    let grabbed = grabs::all(&services.db).await?;
    let details: Vec<ItemDetails> = items
        .iter()
        .map(|item| item_details(registry, &owned, &grabbed, now, item))
        .collect();
    let have_count = details.iter().filter(|entry| entry.have).count();

    // Selecting every listed item is one link, so the target is the whole
    // listed set rather than a toggle of what is already selected.
    let all_listed = ids.join(",");
    let selected_here = ids.iter().filter(|id| selection.contains(id)).count();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Feed results"</h1>
        <p class="mt-1 text-sm text-slate-400">
            (format::count(items.len(), "item", "items")) " from "
            (format::count(registered.len(), "feed", "feeds"))
            if have_count > 0 {
                ", " (have_count) " already in the library"
            }
            "."
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
                href: feed_url(None, selection.as_str(), "#results"),
                label: "All",
                current: active.is_none(),
            )
            for entry in &registered {
                components::filter_chip(
                    href: feed_url(Some(&entry.id), selection.as_str(), "#results"),
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
                        if selected_here == items.len() { "" } else { all_listed.as_str() },
                        "#results",
                    ),
                    checked: selected_here == items.len() && !items.is_empty(),
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
                        href=(feed_url(active, "", "#results"))
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
                        &[("back", feed_url(active, selection.as_str(), "#results").as_str())],
                        "",
                    ),
                    label: "Fetch now",
                )

                <form method="post" action="/grab" class="contents">
                    <input type="hidden" name="selected" value=(selection.as_str())>
                    <input type="hidden" name="feed" value=(active.unwrap_or_default())>
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

        if items.is_empty() {
            <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                "No item in this feed yet."
            </p>
        } else {
            <ul class="mt-4 flex flex-col gap-2">
                for ((item, id), shown) in items.iter().zip(&ids).zip(&details) {
                    components::item_row(
                        item: item,
                        details: shown,
                        toggle_href: feed_url(
                            active,
                            &selection.toggled(id),
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
}

/// Grabs every selected item, then returns to the listing with the
/// selection cleared.
///
/// One failure never stops the rest. Each grab records its own outcome, so
/// the loop discards the result and the reader learns which release failed
/// from its badge rather than from a page that refuses to render.
///
/// The library is resynced once at the end rather than once per item. A sync
/// lists the client's whole queue, so a call per release would repeat that
/// listing for the same answer. Doing it before the redirect means the
/// listing already shows what was just grabbed.
#[route(POST "/grab")]
async fn grab_selected(cx: &Cx, Form(input): Form<GrabForm>) -> Result<SeeOther> {
    let services = app_context::<Services>(cx);

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
            &services.db,
            services.downloads.as_ref(),
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &item,
            &auth,
        )
        .await;
    }

    sync::sync(
        app_context::<Arc<SyncState>>(cx),
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &ENGINE,
    )
    .await;

    Ok(see_other(feed_url(
        input.feed.as_deref().filter(|id| !id.is_empty()),
        "",
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
/// This posts rather than links because it writes shared state. It redirects
/// instead of rendering, so a reload never repeats the flip.
#[route(POST "/admin/rulesets/{ruleset_id}/enabled")]
async fn set_enabled(cx: &Cx) -> Result<SeeOther> {
    let ruleset = mock::ruleset(path_param::<RulesetId>(cx)).ok_or_not_found()?;
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin".to_owned());

    app_context::<RulesetSwitches>(cx).toggle(ruleset.id);

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
    let switches = app_context::<RulesetSwitches>(cx);

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

        <ul id="rulesets" class="mt-6 flex scroll-mt-24 flex-col gap-3">
            for base in RULESETS.iter().filter(|ruleset| ruleset.inherits.is_none()) {
                components::ruleset_card(
                    ruleset: base,
                    nested: false,
                    enabled: switches.is_enabled(base.id),
                )

                for child in base.children() {
                    components::ruleset_card(
                        ruleset: child,
                        nested: true,
                        enabled: switches.is_enabled(child.id),
                    )
                }
            }
        </ul>
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

                        components::action_button(
                            action: format!("/admin/feeds/{}/remove", entry.id),
                            label: "Remove",
                        )
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

#[page("/admin/clients")]
async fn clients(cx: &Cx) -> Result {
    let entries = app_context::<Arc<FeedRegistry>>(cx).entries();
    let services = app_context::<Services>(cx);
    let now = services.clock.now();
    let client = services.torrents.check().await;
    let synced = app_context::<Arc<SyncState>>(cx).last();

    view! {
        <h1 class="text-2xl font-semibold tracking-tight">"Clients"</h1>
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
                        "bg-emerald-500/15 text-emerald-300" if client.is_ok()
                            else "bg-rose-500/15 text-rose-300",
                    ))>
                        if client.is_ok() { "ok" } else { "failed" }
                    </span>
                </div>

                <p class="mt-0.5 font-mono text-xs break-all text-slate-500">
                    match &client {
                        Ok(info) => (&info.version),
                        Err(error) => (error.to_string()),
                    }
                </p>

                <p class="mt-1 text-xs text-slate-500">
                    match &synced {
                        None => "never synced",
                        Some(status) => match &status.outcome {
                            Ok(report) => {
                                (report.matched) " of "
                                (format::count(report.torrents, "torrent", "torrents"))
                                " matched a ruleset, synced "
                                (format::age(now, Some(status.at)))
                            },
                            Err(error) => {
                                "sync failed: " (error) ", "
                                (format::age(now, Some(status.at)))
                            },
                        },
                    }
                </p>
            </div>

            components::action_button(
                action: "/admin/torrents/sync?back=/admin/clients",
                label: "Sync now",
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
                                "/admin/feeds/{}/check?back=/admin/clients",
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

/// Syncs the library from the torrent client now, then returns to `back`.
///
/// This runs the same pass the sync task runs, so a reader who just added a
/// torrent by hand sees it counted without waiting out the interval.
///
/// A pass that overlaps the sync task is harmless. Each pass rewrites the
/// whole table in one transaction, so two passes converge on the snapshot
/// the client reported last.
#[route(POST "/admin/torrents/sync")]
async fn sync_library(cx: &Cx) -> Result<SeeOther> {
    let back = query_params::<SwitchReturn>(cx)?
        .back
        .clone()
        .unwrap_or_else(|| "/admin/clients".to_owned());

    let services = app_context::<Services>(cx);

    sync::sync(
        app_context::<Arc<SyncState>>(cx),
        &services.db,
        services.torrents.as_ref(),
        services.clock.as_ref(),
        &ENGINE,
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
        .unwrap_or_else(|| "/admin/clients".to_owned());

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

/// Controls the candidate list under the editor.
///
/// Every key rides in the URL rather than in browser state, so a reviewer
/// shares an exact view and keeps it across the reload that follows a save.
#[query_params(error = bad_request)]
struct MatchView {
    /// [`Diff::slug`] of the only state to list, or absent for every state.
    diff: Option<String>,

    /// Comma-separated [`mock::Candidate::id`] values held at the top of the
    /// list.
    pinned: Option<String>,

    /// Comma-separated [`mock::Field::name`] values flipped between inherited
    /// and replaced since the last save.
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
    let ruleset = mock::ruleset(path_param::<RulesetId>(cx)).ok_or_not_found()?;
    let view = query_params::<MatchView>(cx)?;
    let q = view.query();
    let filter = q.diff;

    // A pinned candidate stays visible under every filter. Pinning exists to
    // keep one filename in sight while the patterns above it change.
    let (pinned, rest): (Vec<_>, Vec<_>) = ruleset
        .candidates
        .iter()
        .partition(|candidate| q.pinned.contains(candidate.id));

    let listed: Vec<_> = rest
        .into_iter()
        .filter(|candidate| filter.is_none_or(|state| candidate.diff == state))
        .collect();

    let replacements = q.replaced.entries();
    let fields = ruleset.resolved_fields(&replacements);
    let inheriting = ruleset.parent().is_some();
    let enabled = app_context::<RulesetSwitches>(cx).is_enabled(ruleset.id);

    view! {
        <nav class="text-sm text-slate-500">
            <a href="/admin" class="hover:text-slate-300">"Rulesets"</a>
            " / "
            <span class="text-slate-300">(ruleset.name)</span>
        </nav>

        <div id="top" class="mt-3 flex scroll-mt-24 flex-wrap items-start justify-between gap-4">
            <div>
                <div class="flex flex-wrap items-center gap-3">
                    <h1 class="text-2xl font-semibold tracking-tight">(ruleset.name)</h1>
                    components::status_badge(enabled: enabled)
                </div>
                <p class="mt-1 text-sm text-slate-400">(ruleset.summary)</p>
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
                    action: switch_action(ruleset.id, &q.url(ruleset.id, "#top")),
                )
            </div>
        </div>

        match ruleset.parent() {
            Some(parent) => <p class="mt-3 rounded-md border border-slate-800 bg-slate-900/40 px-3 py-2 text-xs text-slate-400">
                "Narrows "
                <a
                    href=(format!("/admin/rulesets/{}", parent.id))
                    class="text-slate-200 underline decoration-slate-700 underline-offset-2 hover:text-white"
                >(parent.name)</a>
                ". A greyed field carries the parent's value. Replace one to give this ruleset its own."
            </p>,
            None => "",
        }

        <section class="mt-6 rounded-lg border border-slate-800 bg-slate-900/30 px-4 py-4">
            <h2 class="text-xs font-medium tracking-wide text-slate-500 uppercase">"Preview"</h2>
            <div class="mt-2">
                components::filename(segments: ruleset.sample, ruleset: ruleset.id)
            </div>
        </section>

        <form id="ruleset-fields" class="mt-6 rounded-lg border border-slate-800 bg-slate-900/40">
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

            for resolved in fields {
                components::field_row(
                    resolved: resolved,
                    inheriting: inheriting,
                    toggle_href: q.with_replaced(
                            &q.replaced.toggled(resolved.field.name),
                            ruleset.id,
                            &format!("#field-{}", resolved.field.part.slug()),
                        ),
                )
            }

        </form>

        <section id="matches" class="mt-8 scroll-mt-24">
            <div class="flex flex-wrap items-end justify-between gap-3">
                <div>
                    <h2 class="text-lg font-semibold tracking-tight">"Matches"</h2>
                    <p class="mt-1 text-sm text-slate-400">
                        (format::count(ruleset.candidates.len(), "candidate", "candidates"))
                        " from " (ruleset.feeds.join(", ")) ", against the edited rules."
                    </p>
                </div>
                <p class="text-xs text-slate-500">
                    (ruleset.diff_count(Diff::Added)) " gained, "
                    (ruleset.diff_count(Diff::Removed)) " lost"
                </p>
            </div>

            <nav class="mt-4 flex flex-wrap gap-2">
                components::diff_filter(
                    href: EditorQuery { diff: None, ..q }.url(ruleset.id, "#matches"),
                    label: "All",
                    count: ruleset.candidates.len(),
                    current: filter.is_none(),
                )
                for state in Diff::ALL {
                    components::diff_filter(
                        href: EditorQuery { diff: Some(*state), ..q }.url(ruleset.id, "#matches"),
                        label: state.label(),
                        count: ruleset.diff_count(*state),
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
                        for candidate in pinned {
                            components::candidate_row(
                                candidate: candidate,
                                ruleset: ruleset.id,
                                pin_href: q.with_pins(
                                        &q.pinned.toggled(candidate.id),
                                        ruleset.id,
                                        &format!("#match-{}", candidate.id),
                                        ),
                                pinned: true,
                            )
                        }
                    </ul>
                </div>
            }

            if listed.is_empty() {
                <p class="mt-4 rounded-lg border border-slate-800 px-4 py-8 text-center text-sm text-slate-500">
                    "No candidate sits in this state."
                </p>
            } else {
                <ul class="mt-4 flex flex-col gap-2">
                    for candidate in listed {
                        components::candidate_row(
                            candidate: candidate,
                            ruleset: ruleset.id,
                            pin_href: q.with_pins(
                                    &q.pinned.toggled(candidate.id),
                                    ruleset.id,
                                    &format!("#match-{}", candidate.id),
                                    ),
                            pinned: false,
                        )
                    }
                </ul>
            }
        </section>
    }
}

#[page("/admin/rulesets/new")]
async fn new_ruleset() -> Result {
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

        <form class="mt-6 flex flex-col gap-4 rounded-lg border border-slate-800 bg-slate-900/40 px-4 py-4">
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
                <label for="summary" class="block text-xs text-slate-500">"Summary"</label>
                <input
                    id="summary"
                    type="text"
                    name="summary"
                    placeholder="Narrows the episode rules to one series."
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
                    for base in RULESETS.iter().filter(|ruleset| ruleset.inherits.is_none()) {
                        <option value=(base.id)>
                            (base.name) " (" (base.fields.len()) " fields)"
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
