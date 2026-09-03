//! The set of feeds the application currently watches.
//!
//! A registration persists in the `feeds` table, so a feed outlives the
//! process that registered it. What each entry knows about its last check
//! lives in memory alone, and a restart clears it: a check is a fact about a
//! run rather than about a feed.
//!
//! The items a feed returned do not live here at all, only in the
//! `feed_items` table, so nothing a restart drops is lost.
//!
//! This mirrors [`RulesetSwitches`](crate::server) in one respect: it sits in
//! the app context, and a handler reads it there rather than through an
//! argument.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::field::{Empty, display};
use tracing::{Span, info, instrument, warn};
use url::Url;

use crate::clock::Clock;
use crate::feed::store::FeedStore;
use crate::feed::{Feed, FeedAuth, FeedError, FeedSource, redacted};
use crate::store;
use crate::store::Ingest;

/// One registered feed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedEntry {
    /// The registration's own id, used in a URL and in a form action.
    pub id: String,

    /// What the admin page calls this feed. A blank registration falls back
    /// to the host, so this is never empty.
    pub name: String,

    pub url: Url,

    /// The credentials a fetch sends with this feed.
    ///
    /// A registration made through the admin page carries none. A feed that
    /// needs them arrives from the configuration file or the store.
    pub auth: FeedAuth,

    /// The result of the last check, or nothing until one runs.
    pub check: Option<FeedCheck>,
}

/// What one check of a feed produced.
///
/// The outcome carries the ingest counts on success and the error text on
/// failure. A fetch failure and a store failure both end a check the same
/// way, and the pages show only the text, so nothing is gained by keeping
/// the two error types apart this far out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FeedCheck {
    pub at: DateTime<Utc>,
    pub outcome: Result<Ingest, String>,
}

/// The registered feeds, in the order they were added.
pub struct FeedRegistry {
    store: FeedStore,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    feeds: Vec<FeedEntry>,
}

impl FeedRegistry {
    /// Reads every stored feed into a registry.
    ///
    /// Each entry starts with no check. A check records what one run of this
    /// process saw, so it belongs to the run rather than to the feed.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the feeds cannot be read.
    pub async fn load(store: FeedStore) -> Result<Self, sqlx::Error> {
        let feeds = store
            .list()
            .await?
            .into_iter()
            .map(|feed| FeedEntry {
                id: feed.id.to_string(),
                name: feed.name,
                url: feed.url,
                auth: feed.auth,
                check: None,
            })
            .collect();

        Ok(Self {
            store,
            inner: Mutex::new(Inner { feeds }),
        })
    }

    /// Registers `url` under `name` and returns the feed's id.
    ///
    /// A URL already registered keeps its id and takes the new name. Its
    /// credentials change only when `auth` carries some, which mirrors what
    /// the table does and is what lets the admin form re-register a feed
    /// without clearing what a configuration file gave it.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the write fails. Nothing is added to
    /// the registry in that case.
    pub async fn add(
        &self,
        name: String,
        url: Url,
        auth: Option<FeedAuth>,
    ) -> Result<String, sqlx::Error> {
        // The store answers before the lock is taken. A `MutexGuard` is not
        // `Send`, so one held across an await makes the whole handler future
        // not `Send`, which the router will not take.
        let id = self
            .store
            .upsert(&name, &url, auth.as_ref())
            .await?
            .to_string();

        let mut inner = self.lock();

        match inner.feeds.iter_mut().find(|feed| feed.id == id) {
            Some(feed) => {
                feed.name = name;
                feed.url = url;

                if let Some(auth) = auth {
                    feed.auth = auth;
                }
            }
            None => inner.feeds.push(FeedEntry {
                id: id.clone(),
                name,
                url,
                auth: auth.unwrap_or_default(),
                check: None,
            }),
        }

        Ok(id)
    }

    /// Removes the feed `id`, and reports whether one was there.
    ///
    /// An id the table never issued names no feed, so a value that is not a
    /// row id reports the same absence a missing row does.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the delete fails. The entry stays in
    /// the registry in that case, so the two never disagree.
    pub async fn remove(&self, id: &str) -> Result<bool, sqlx::Error> {
        let Ok(row) = id.parse::<i64>() else {
            return Ok(false);
        };

        if !self.store.remove(row).await? {
            return Ok(false);
        }

        self.lock().feeds.retain(|feed| feed.id != id);

        Ok(true)
    }

    pub fn get(&self, id: &str) -> Option<FeedEntry> {
        self.lock().feeds.iter().find(|feed| feed.id == id).cloned()
    }

    /// Returns every registered feed, in the order it was added.
    pub fn entries(&self) -> Vec<FeedEntry> {
        self.lock().feeds.clone()
    }

    /// Returns the name registered for `url`.
    ///
    /// The listing shows a feed name beside an item, and a stored item
    /// carries its feed's URL rather than an id, so this is the lookup that
    /// turns one into the other.
    pub fn name_of(&self, url: &Url) -> Option<String> {
        self.lock()
            .feeds
            .iter()
            .find(|feed| &feed.url == url)
            .map(|feed| feed.name.clone())
    }

    /// Returns the credentials registered for `url`.
    ///
    /// A stored item carries its feed's URL rather than an id, so this is what
    /// turns one into the credentials a download for it has to send.
    pub fn auth_of(&self, url: &Url) -> Option<FeedAuth> {
        self.lock()
            .feeds
            .iter()
            .find(|feed| &feed.url == url)
            .map(|feed| feed.auth.clone())
    }

    /// Stores `check` as the feed's latest, and reports whether `id` exists.
    ///
    /// Only the last check is kept. A history grows without bound, and the
    /// pages show one result per feed.
    pub fn record(&self, id: &str, check: FeedCheck) -> bool {
        let mut inner = self.lock();

        match inner.feeds.iter_mut().find(|feed| feed.id == id) {
            Some(feed) => {
                feed.check = Some(check);
                true
            }
            None => false,
        }
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.inner
            .lock()
            .expect("the feed registry lock is never poisoned")
    }
}

/// Fetches one feed, stores what it returned, and records the outcome.
///
/// Returns whether `id` names a registered feed. A feed that fails to fetch
/// is still a feed, so a failure here reports `true` with the error text
/// recorded against it.
///
/// The clock is read once, at the start. The same instant stamps the stored
/// rows and the recorded check, so a listing's age and the admin page's
/// last-checked time never disagree by the length of a fetch.
#[instrument(name = "check_feed", skip_all, fields(feed.id = %id, feed.url = Empty))]
pub async fn check(
    registry: &FeedRegistry,
    pool: &SqlitePool,
    source: &dyn FeedSource,
    clock: &dyn Clock,
    id: &str,
) -> bool {
    // Reading the entry out clones and releases the lock, so no page render
    // waits behind the fetch and the writes that follow.
    let Some(entry) = registry.get(id) else {
        return false;
    };

    // Recorded through `display` so `feed.url` reads the same here as it does
    // in the fetch span nested below, which sets it with `%`.
    Span::current().record("feed.url", display(redacted(&entry.url)));

    let at = clock.now();
    let outcome = match source.fetch(&entry.url, &entry.auth).await {
        Ok(feed) => store::ingest(pool, &entry.url, at, &feed.items)
            .await
            .map_err(|error| error.to_string()),
        Err(error) => Err(error.to_string()),
    };

    // Logged by reference, so the line and the stored check carry one
    // rendering of the error rather than two.
    match &outcome {
        Ok(ingest) => info!(items = ingest.items, added = ingest.added, "checked"),
        Err(error) => warn!(error = %error, "check failed"),
    }

    registry.record(id, FeedCheck { at, outcome })
}

/// Fetches one feed and returns what it carries, storing and recording
/// nothing.
///
/// This backs the Test button on the feeds page, so a reader sees what a
/// tracker hands back before a poll has run, or without running one.
///
/// Returns `None` when `id` names no feed, which the handler answers as a
/// 404. A fetch that fails comes back as the `Err`, never as a recorded
/// check, so the client page keeps showing the last real poll.
#[instrument(name = "preview_feed", skip_all, fields(feed.id = %id, feed.url = Empty))]
pub async fn preview(
    registry: &FeedRegistry,
    source: &dyn FeedSource,
    id: &str,
) -> Option<Result<Feed, FeedError>> {
    // Reading the entry out clones and releases the lock, so no page render
    // waits behind the fetch.
    let entry = registry.get(id)?;

    Span::current().record("feed.url", display(redacted(&entry.url)));

    let outcome = source.fetch(&entry.url, &entry.auth).await;

    // Both outcomes are `info`, because nothing retries a preview. The
    // reader sees a failure on the page they asked from.
    match &outcome {
        Ok(feed) => info!(items = feed.items.len(), "previewed"),
        Err(error) => info!(error = %error, "preview failed"),
    }

    Some(outcome)
}

/// Checks every registered feed, in the order they were added.
///
/// One feed's failure never stops the pass, because each check records its
/// own outcome and returns.
pub async fn check_all(
    registry: &FeedRegistry,
    pool: &SqlitePool,
    source: &dyn FeedSource,
    clock: &dyn Clock,
) {
    for entry in registry.entries() {
        check(registry, pool, source, clock, &entry.id).await;
    }
}

/// Checks every feed forever, pausing `interval` between passes.
///
/// The pause runs after a pass rather than on a fixed schedule, so a slow
/// tracker delays the next pass instead of stacking passes on top of each
/// other.
#[instrument(name = "poll", skip_all, fields(interval_secs = interval.as_secs()))]
pub async fn poll(
    registry: Arc<FeedRegistry>,
    pool: SqlitePool,
    source: Arc<dyn FeedSource>,
    clock: Arc<dyn Clock>,
    interval: Duration,
) {
    loop {
        check_all(&registry, &pool, source.as_ref(), clock.as_ref()).await;
        clock.sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;
    use url::Url;

    use super::{FeedCheck, FeedEntry, FeedRegistry, check, check_all, preview};
    use crate::clock::Clock;
    use crate::feed::store::FeedStore;
    use crate::feed::{Feed, FeedAuth, FeedError, fake};
    use crate::services::Services;
    use crate::store;
    use crate::store::Ingest;

    const FEED: &str = "https://tracker.invalid/rss";
    const OTHER: &str = "https://other.invalid/rss";

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    /// Builds a registry over `pool`, which `#[sqlx::test]` has migrated.
    async fn registry(pool: &SqlitePool) -> FeedRegistry {
        FeedRegistry::load(FeedStore::new(pool.clone()))
            .await
            .expect("an empty feeds table loads")
    }

    fn entry(id: &str, name: &str, feed: &str, check: Option<FeedCheck>) -> FeedEntry {
        FeedEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            url: url(feed),
            auth: FeedAuth::default(),
            check,
        }
    }

    #[sqlx::test]
    async fn add_assigns_sequential_ids(pool: SqlitePool) {
        let registry = registry(&pool).await;

        assert_eq!(
            registry
                .add("Tracker".to_owned(), url(FEED), None)
                .await
                .expect("add"),
            "1"
        );
        assert_eq!(
            registry
                .add("Other".to_owned(), url(OTHER), None)
                .await
                .expect("add"),
            "2"
        );
        assert_eq!(
            registry.entries(),
            vec![
                entry("1", "Tracker", FEED, None),
                entry("2", "Other", OTHER, None),
            ],
            "insertion order, with no check yet"
        );
        assert_eq!(
            registry.get("2"),
            Some(entry("2", "Other", OTHER, None)),
            "one feed by id"
        );
        assert_eq!(registry.get("404"), None, "an unknown id finds nothing");
    }

    #[sqlx::test]
    async fn removed_id_is_never_reused(pool: SqlitePool) {
        let registry = registry(&pool).await;
        registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");

        assert!(
            registry.remove("1").await.expect("remove"),
            "the feed was there"
        );
        registry
            .add("Other".to_owned(), url(OTHER), None)
            .await
            .expect("add");

        assert_eq!(
            registry.entries(),
            vec![entry("2", "Other", OTHER, None)],
            "the second feed gets a fresh id"
        );
    }

    #[sqlx::test]
    async fn remove_unknown_is_false(pool: SqlitePool) {
        let registry = registry(&pool).await;
        registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");

        assert!(!registry.remove("404").await.expect("remove"));
        assert_eq!(
            registry.entries(),
            vec![entry("1", "Tracker", FEED, None)],
            "an unknown id removes nothing"
        );
    }

    #[sqlx::test]
    async fn record_replaces_previous_check(pool: SqlitePool) {
        let registry = registry(&pool).await;
        let id = registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");

        let failed = FeedCheck {
            at: at(1),
            outcome: Err("the feed is unreachable".to_owned()),
        };
        let succeeded = FeedCheck {
            at: at(2),
            outcome: Ok(Ingest { items: 3, added: 1 }),
        };

        assert!(registry.record(&id, failed));
        assert!(registry.record(&id, succeeded.clone()));
        assert_eq!(
            registry.entries(),
            vec![entry("1", "Tracker", FEED, Some(succeeded))],
            "only the last check is kept"
        );
    }

    #[sqlx::test]
    async fn record_unknown_is_false(pool: SqlitePool) {
        let registry = registry(&pool).await;

        assert!(!registry.record(
            "404",
            FeedCheck {
                at: at(1),
                outcome: Ok(Ingest { items: 0, added: 0 }),
            }
        ));
        assert_eq!(registry.entries(), Vec::new());
    }

    #[sqlx::test]
    async fn auth_of_unknown_url_is_none(pool: SqlitePool) {
        let registry = registry(&pool).await;
        let auth = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("Cookie".to_owned(), "session=abc".to_owned())]),
        };

        registry
            .add("Tracker".to_owned(), url(FEED), Some(auth.clone()))
            .await
            .expect("add");

        assert_eq!(registry.auth_of(&url(FEED)), Some(auth));
        assert_eq!(
            registry.auth_of(&url(OTHER)),
            None,
            "a URL nothing registered has no credentials to find"
        );
    }

    #[sqlx::test]
    async fn name_of_unknown_url_is_none(pool: SqlitePool) {
        let registry = registry(&pool).await;
        registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");

        assert_eq!(registry.name_of(&url(FEED)), Some("Tracker".to_owned()));
        assert_eq!(registry.name_of(&url(OTHER)), None);
    }

    #[sqlx::test]
    async fn check_unknown_id_is_false(pool: SqlitePool) {
        let (services, _fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;

        assert!(
            !check(
                &registry,
                &services.db,
                services.feeds.as_ref(),
                services.clock.as_ref(),
                "404",
            )
            .await
        );
        assert_eq!(registry.entries(), Vec::new(), "nothing was recorded");
    }

    #[sqlx::test]
    async fn check_ingests_items_and_records_counts(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        let id = registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        let at = fakes.clock.now();
        fakes
            .feeds
            .feed(FEED, vec![fake::item("A.Release"), fake::item("B.Release")]);

        assert!(
            check(
                &registry,
                &services.db,
                services.feeds.as_ref(),
                services.clock.as_ref(),
                &id,
            )
            .await
        );

        assert_eq!(
            registry.entries(),
            vec![entry(
                "1",
                "Tracker",
                FEED,
                Some(FeedCheck {
                    at,
                    outcome: Ok(Ingest { items: 2, added: 2 }),
                })
            )],
            "the counts and the check time are recorded"
        );
        assert_eq!(
            store::items(&services.db, None)
                .await
                .expect("items")
                .into_iter()
                .map(|stored| stored.item.title)
                .collect::<Vec<_>>(),
            vec!["B.Release", "A.Release"],
            "both items reached the store, the later row first because \
             undated items of one fetch tie until the id breaks it"
        );
    }

    #[sqlx::test]
    async fn check_records_fetch_error_text(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        let id = registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        let at = fakes.clock.now();
        fakes.feeds.failing(FEED, FeedError::Status { code: 503 });

        assert!(
            check(
                &registry,
                &services.db,
                services.feeds.as_ref(),
                services.clock.as_ref(),
                &id,
            )
            .await,
            "a failed fetch is still a registered feed"
        );

        assert_eq!(
            registry.entries(),
            vec![entry(
                "1",
                "Tracker",
                FEED,
                Some(FeedCheck {
                    at,
                    outcome: Err("the feed answered with status 503".to_owned()),
                })
            )]
        );
        assert_eq!(
            store::items(&services.db, None).await.expect("items"),
            Vec::new(),
            "a failed fetch stores nothing"
        );
    }

    #[sqlx::test]
    async fn check_all_fetches_every_feed_in_order(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        registry
            .add("Other".to_owned(), url(OTHER), None)
            .await
            .expect("add");
        fakes.feeds.feed(FEED, vec![fake::item("A.Release")]);
        fakes.feeds.feed(OTHER, vec![fake::item("B.Release")]);

        check_all(
            &registry,
            &services.db,
            services.feeds.as_ref(),
            services.clock.as_ref(),
        )
        .await;

        assert_eq!(
            fakes.feeds.fetched(),
            vec![url(FEED), url(OTHER)],
            "registration order"
        );
        assert!(
            registry.entries().iter().all(|entry| entry.check.is_some()),
            "every feed was checked"
        );
    }

    #[sqlx::test]
    async fn add_same_url_updates_name_in_place(pool: SqlitePool) {
        let registry = registry(&pool).await;
        let first = registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        let again = registry
            .add("Renamed".to_owned(), url(FEED), None)
            .await
            .expect("re-register");

        assert_eq!(first, again, "one URL is one feed");
        assert_eq!(
            registry.entries(),
            vec![entry("1", "Renamed", FEED, None)],
            "the entry is renamed rather than duplicated"
        );
    }

    #[sqlx::test]
    async fn add_without_auth_keeps_existing_auth(pool: SqlitePool) {
        let registry = registry(&pool).await;
        let declared = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "declared".to_owned())]),
        };

        registry
            .add("Tracker".to_owned(), url(FEED), Some(declared.clone()))
            .await
            .expect("add with auth");
        registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("re-register from the form");

        assert_eq!(
            registry.get("1").map(|feed| feed.auth),
            Some(declared),
            "the form carries no credentials and erases none"
        );
    }

    #[sqlx::test]
    async fn check_sends_the_entry_auth(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        let auth = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]),
        };

        let id = registry
            .add("Tracker".to_owned(), url(FEED), Some(auth.clone()))
            .await
            .expect("add");
        fakes.feeds.feed(FEED, vec![fake::item("A.Release")]);

        check(
            &registry,
            &services.db,
            services.feeds.as_ref(),
            services.clock.as_ref(),
            &id,
        )
        .await;

        assert_eq!(
            fakes.feeds.fetched_auth(),
            vec![(url(FEED), auth)],
            "the credentials on the entry reach the fetch"
        );
    }

    #[sqlx::test]
    async fn load_restores_persisted_feeds(pool: SqlitePool) {
        let first = registry(&pool).await;
        first
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        first
            .add("Other".to_owned(), url(OTHER), None)
            .await
            .expect("add");

        let restarted = registry(&pool).await;

        assert_eq!(
            restarted.entries(),
            first.entries(),
            "a second registry over one database sees the same feeds"
        );
        assert_eq!(
            restarted.entries(),
            vec![
                entry("1", "Tracker", FEED, None),
                entry("2", "Other", OTHER, None),
            ],
            "and each keeps its id, its name, and no check"
        );
    }
    #[sqlx::test]
    async fn preview_unknown_id_is_none(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;

        assert_eq!(
            preview(&registry, services.feeds.as_ref(), "404").await,
            None
        );
        assert_eq!(fakes.feeds.fetched(), Vec::new(), "nothing was fetched");
    }

    #[sqlx::test]
    async fn preview_returns_items_and_stores_nothing(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        let auth = FeedAuth {
            basic: None,
            headers: BTreeMap::from([("X-Api-Key".to_owned(), "secret".to_owned())]),
        };

        let id = registry
            .add("Tracker".to_owned(), url(FEED), Some(auth.clone()))
            .await
            .expect("add");
        fakes
            .feeds
            .feed(FEED, vec![fake::item("A.Release"), fake::item("B.Release")]);

        assert_eq!(
            preview(&registry, services.feeds.as_ref(), &id).await,
            Some(Ok(Feed {
                items: vec![fake::item("A.Release"), fake::item("B.Release")],
            })),
        );
        assert_eq!(
            fakes.feeds.fetched_auth(),
            vec![(url(FEED), auth)],
            "the credentials on the entry reach the fetch"
        );
        assert_eq!(
            store::items(&services.db, None).await.expect("items"),
            Vec::new(),
            "a preview stores nothing"
        );
        assert_eq!(
            registry.get(&id).map(|feed| feed.check),
            Some(None),
            "a preview records no check"
        );
    }

    #[sqlx::test]
    async fn preview_passes_the_fetch_error_through(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = registry(&services.db).await;
        let id = registry
            .add("Tracker".to_owned(), url(FEED), None)
            .await
            .expect("add");
        fakes.feeds.failing(FEED, FeedError::Status { code: 503 });

        assert_eq!(
            preview(&registry, services.feeds.as_ref(), &id).await,
            Some(Err(FeedError::Status { code: 503 })),
        );
        assert_eq!(
            registry.get(&id).map(|feed| feed.check),
            Some(None),
            "a failed preview records no check either"
        );
    }
}
