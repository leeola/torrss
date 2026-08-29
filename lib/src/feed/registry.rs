//! The set of feeds the application currently watches.
//!
//! A registration lives in memory and a restart empties it. The items a feed
//! returned do not live here at all, only in the `feed_items` table, so what
//! a restart costs is the list of feeds rather than any history.
//!
//! This mirrors [`RulesetSwitches`](crate::server) in both respects: it sits
//! in the app context, and a handler reads it there rather than through an
//! argument.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::field::{Empty, display};
use tracing::{Span, info, instrument, warn};
use url::Url;

use crate::clock::Clock;
use crate::feed::{FeedSource, redacted};
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
#[derive(Debug, Default)]
pub struct FeedRegistry {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    feeds: Vec<FeedEntry>,

    /// Counts every id ever issued, so a removal never frees one for reuse.
    ///
    /// Numbering from the current length instead hands a removed feed's id to
    /// the next one added, and a check then records against the wrong feed.
    next_id: u32,
}

impl FeedRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `url` under `name` and returns the new id.
    pub fn add(&self, name: String, url: Url) -> String {
        let mut inner = self.lock();
        inner.next_id += 1;
        let id = format!("f{}", inner.next_id);

        inner.feeds.push(FeedEntry {
            id: id.clone(),
            name,
            url,
            check: None,
        });

        id
    }

    /// Removes the feed `id`, and reports whether one was there.
    pub fn remove(&self, id: &str) -> bool {
        let mut inner = self.lock();
        let before = inner.feeds.len();
        inner.feeds.retain(|feed| feed.id != id);

        inner.feeds.len() != before
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
    let outcome = match source.fetch(&entry.url).await {
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
    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;
    use url::Url;

    use super::{FeedCheck, FeedEntry, FeedRegistry, check, check_all};
    use crate::clock::Clock;
    use crate::feed::{FeedError, fake};
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

    fn entry(id: &str, name: &str, feed: &str, check: Option<FeedCheck>) -> FeedEntry {
        FeedEntry {
            id: id.to_owned(),
            name: name.to_owned(),
            url: url(feed),
            check,
        }
    }

    #[test]
    fn add_assigns_sequential_ids() {
        let registry = FeedRegistry::new();

        assert_eq!(registry.add("Tracker".to_owned(), url(FEED)), "f1");
        assert_eq!(registry.add("Other".to_owned(), url(OTHER)), "f2");
        assert_eq!(
            registry.entries(),
            vec![
                entry("f1", "Tracker", FEED, None),
                entry("f2", "Other", OTHER, None),
            ],
            "insertion order, with no check yet"
        );
        assert_eq!(
            registry.get("f2"),
            Some(entry("f2", "Other", OTHER, None)),
            "one feed by id"
        );
        assert_eq!(registry.get("f404"), None, "an unknown id finds nothing");
    }

    #[test]
    fn removed_id_is_never_reused() {
        let registry = FeedRegistry::new();
        registry.add("Tracker".to_owned(), url(FEED));

        assert!(registry.remove("f1"), "the feed was there");
        registry.add("Other".to_owned(), url(OTHER));

        assert_eq!(
            registry.entries(),
            vec![entry("f2", "Other", OTHER, None)],
            "the second feed gets a fresh id"
        );
    }

    #[test]
    fn remove_unknown_is_false() {
        let registry = FeedRegistry::new();
        registry.add("Tracker".to_owned(), url(FEED));

        assert!(!registry.remove("f404"));
        assert_eq!(
            registry.entries(),
            vec![entry("f1", "Tracker", FEED, None)],
            "an unknown id removes nothing"
        );
    }

    #[test]
    fn record_replaces_previous_check() {
        let registry = FeedRegistry::new();
        let id = registry.add("Tracker".to_owned(), url(FEED));

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
            vec![entry("f1", "Tracker", FEED, Some(succeeded))],
            "only the last check is kept"
        );
    }

    #[test]
    fn record_unknown_is_false() {
        let registry = FeedRegistry::new();

        assert!(!registry.record(
            "f404",
            FeedCheck {
                at: at(1),
                outcome: Ok(Ingest { items: 0, added: 0 }),
            }
        ));
        assert_eq!(registry.entries(), Vec::new());
    }

    #[test]
    fn name_of_unknown_url_is_none() {
        let registry = FeedRegistry::new();
        registry.add("Tracker".to_owned(), url(FEED));

        assert_eq!(registry.name_of(&url(FEED)), Some("Tracker".to_owned()));
        assert_eq!(registry.name_of(&url(OTHER)), None);
    }

    #[sqlx::test]
    async fn check_unknown_id_is_false(pool: SqlitePool) {
        let (services, _fakes) = Services::fake(pool);
        let registry = FeedRegistry::new();

        assert!(
            !check(
                &registry,
                &services.db,
                services.feeds.as_ref(),
                services.clock.as_ref(),
                "f404",
            )
            .await
        );
        assert_eq!(registry.entries(), Vec::new(), "nothing was recorded");
    }

    #[sqlx::test]
    async fn check_ingests_items_and_records_counts(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let registry = FeedRegistry::new();
        let id = registry.add("Tracker".to_owned(), url(FEED));
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
                "f1",
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
        let registry = FeedRegistry::new();
        let id = registry.add("Tracker".to_owned(), url(FEED));
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
                "f1",
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
        let registry = FeedRegistry::new();
        registry.add("Tracker".to_owned(), url(FEED));
        registry.add("Other".to_owned(), url(OTHER));
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
}
