//! The feeds this application watches, kept between restarts.
//!
//! The registry holds the same set in memory and empties on a restart. This
//! is what refills it, so a feed registered through the admin page outlives
//! the process that registered it.
//!
//! Keyed by URL. Two names for one URL are one feed, and the URL is what a
//! fetch actually goes to.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use url::Url;

use super::FeedAuth;
use crate::store::Ingest;

/// Adds a feed, or updates the one already registered for the URL.
///
/// The name always takes the new value. The auth takes the new value only
/// when the caller supplies one, because a re-registration through the admin
/// form carries no credentials and must not erase the ones a configuration
/// file gave the feed. That also makes applying one declaration twice change
/// nothing.
const UPSERT: &str = "
    INSERT INTO feeds (name, url, auth)
    VALUES (?1, ?2, ?3)
    ON CONFLICT (url) DO UPDATE SET
        name = excluded.name,
        auth = coalesce(excluded.auth, auth)
    RETURNING id
";

/// Reads every feed, oldest registration first.
///
/// The id orders them rather than the name, so a listing keeps the order the
/// feeds were added in and a rename never moves a row.
const SELECT: &str = "
    SELECT id, name, url, auth, checked_at, check_items, check_added, check_error
    FROM feeds
    ORDER BY id
";

/// Records the feed's latest check, replacing whatever it held.
const RECORD_CHECK: &str = "
    UPDATE feeds
    SET checked_at = ?2, check_items = ?3, check_added = ?4, check_error = ?5
    WHERE id = ?1
";

/// One feed as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFeed {
    pub id: i64,
    pub name: String,
    pub url: Url,
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

/// The stored feeds, read and written through one pool.
pub struct FeedStore {
    pool: SqlitePool,
}

impl FeedStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Registers `url` under `name` and returns the feed's id.
    ///
    /// A URL already registered keeps its id, so anything holding one stays
    /// pointed at the same feed.
    ///
    /// `auth` of [`None`] leaves whatever credentials the feed already has.
    pub async fn upsert(
        &self,
        name: &str,
        url: &Url,
        auth: Option<&FeedAuth>,
    ) -> Result<i64, sqlx::Error> {
        let encoded = auth
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| sqlx::Error::Encode(Box::new(error)))?;

        sqlx::query_scalar::<_, i64>(UPSERT)
            .bind(name)
            .bind(url.as_str())
            .bind(encoded)
            .fetch_one(&self.pool)
            .await
    }

    /// Returns every stored feed, in the order they were registered.
    pub async fn list(&self) -> Result<Vec<StoredFeed>, sqlx::Error> {
        sqlx::query_as::<_, FeedRow>(SELECT)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(stored_feed)
            .collect()
    }

    /// Replaces the feed's recorded check, and reports whether `id` exists.
    ///
    /// Only the last check is kept. A history grows without bound, and the
    /// pages show one result per feed.
    pub async fn record_check(&self, id: i64, check: &FeedCheck) -> Result<bool, sqlx::Error> {
        let (items, added, error) = match &check.outcome {
            Ok(ingest) => (Some(count(ingest.items)?), Some(count(ingest.added)?), None),
            Err(error) => (None, None, Some(error.as_str())),
        };

        let result = sqlx::query(RECORD_CHECK)
            .bind(id)
            .bind(check.at)
            .bind(items)
            .bind(added)
            .bind(error)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }

    /// Removes the feed `id`, and reports whether one was there.
    pub async fn remove(&self, id: i64) -> Result<bool, sqlx::Error> {
        let result = sqlx::query("DELETE FROM feeds WHERE id = ?1")
            .bind(id)
            .execute(&self.pool)
            .await?;

        Ok(result.rows_affected() == 1)
    }
}

/// One `feeds` row as sqlx hands it back, before it becomes a [`StoredFeed`].
type FeedRow = (
    i64,
    String,
    String,
    Option<String>,
    Option<DateTime<Utc>>,
    Option<i64>,
    Option<i64>,
    Option<String>,
);

/// Rebuilds one feed from its row.
///
/// A URL or an auth value that no longer parses means the row is corrupt.
/// Both were valid when written, so each is a read failure rather than a
/// condition a caller handles apart. A count too large for the machine's
/// index type reads the same way.
///
/// The check's timestamp is what says a check ran. The counts and the error
/// text are each absent in the outcome the other describes.
fn stored_feed(row: FeedRow) -> Result<StoredFeed, sqlx::Error> {
    let (id, name, url, auth, checked_at, items, added, error) = row;

    let check = match checked_at {
        None => None,
        Some(at) => Some(FeedCheck {
            at,
            outcome: match error {
                Some(error) => Err(error),
                None => Ok(Ingest {
                    items: index(items)?,
                    added: index(added)?,
                }),
            },
        }),
    };

    Ok(StoredFeed {
        id,
        name,
        url: Url::parse(&url).map_err(sqlx::Error::decode)?,
        auth: match auth {
            Some(encoded) => serde_json::from_str(&encoded).map_err(sqlx::Error::decode)?,
            None => FeedAuth::default(),
        },
        check,
    })
}

/// Narrows a stored count to the machine's index type, treating a missing
/// one as zero.
fn index(count: Option<i64>) -> Result<usize, sqlx::Error> {
    usize::try_from(count.unwrap_or(0)).map_err(sqlx::Error::decode)
}

/// Widens a count for storage.
fn count(value: usize) -> Result<i64, sqlx::Error> {
    i64::try_from(value).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;
    use url::Url;

    use super::{FeedCheck, FeedStore, StoredFeed};
    use crate::feed::FeedAuth;
    use crate::store::Ingest;

    const FEED: &str = "https://tracker.invalid/rss";
    const OTHER: &str = "https://other.invalid/rss";

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("the test URL parses")
    }

    fn auth(key: &str) -> FeedAuth {
        FeedAuth {
            basic: None,
            headers: BTreeMap::from([("X-Api-Key".to_owned(), key.to_owned())]),
        }
    }

    fn stored(id: i64, name: &str, feed: &str, auth: FeedAuth) -> StoredFeed {
        StoredFeed {
            id,
            name: name.to_owned(),
            url: url(feed),
            auth,
            check: None,
        }
    }

    fn at(hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2024, 1, 1, hour, 0, 0)
            .single()
            .expect("the test timestamp is unambiguous")
    }

    #[sqlx::test]
    async fn upsert_same_url_keeps_id_and_replaces_name(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let first = feeds
            .upsert("Tracker", &url(FEED), None)
            .await
            .expect("add");
        let again = feeds
            .upsert("Renamed", &url(FEED), None)
            .await
            .expect("re-register");

        assert_eq!(first, again, "one URL is one feed, so the id holds");
        assert_eq!(
            feeds.list().await.expect("list"),
            vec![stored(first, "Renamed", FEED, FeedAuth::default())],
            "the name takes the new value, and no second row appears"
        );
    }

    #[sqlx::test]
    async fn upsert_without_auth_keeps_stored_auth(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let id = feeds
            .upsert("Tracker", &url(FEED), Some(&auth("declared")))
            .await
            .expect("add with auth");

        feeds
            .upsert("Tracker", &url(FEED), None)
            .await
            .expect("re-register from the admin form");

        assert_eq!(
            feeds.list().await.expect("list"),
            vec![stored(id, "Tracker", FEED, auth("declared"))],
            "a form that carries no credentials never erases the ones on file"
        );
    }

    #[sqlx::test]
    async fn upsert_with_auth_replaces_it(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let id = feeds
            .upsert("Tracker", &url(FEED), Some(&auth("old")))
            .await
            .expect("add");

        feeds
            .upsert("Tracker", &url(FEED), Some(&auth("new")))
            .await
            .expect("re-declare");

        assert_eq!(
            feeds.list().await.expect("list"),
            vec![stored(id, "Tracker", FEED, auth("new"))]
        );
    }

    #[sqlx::test]
    async fn list_orders_by_id(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let first = feeds.upsert("Zed", &url(FEED), None).await.expect("add");
        let second = feeds.upsert("Alpha", &url(OTHER), None).await.expect("add");

        assert_eq!(
            feeds.list().await.expect("list"),
            vec![
                stored(first, "Zed", FEED, FeedAuth::default()),
                stored(second, "Alpha", OTHER, FeedAuth::default()),
            ],
            "registration order, not alphabetical"
        );
    }

    #[sqlx::test]
    async fn remove_unknown_is_false(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let id = feeds
            .upsert("Tracker", &url(FEED), None)
            .await
            .expect("add");

        assert!(!feeds.remove(id + 404).await.expect("remove"));
        assert_eq!(
            feeds.list().await.expect("list"),
            vec![stored(id, "Tracker", FEED, FeedAuth::default())],
            "an unknown id removes nothing"
        );
    }

    #[sqlx::test]
    async fn removed_id_is_never_reused(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let first = feeds
            .upsert("Tracker", &url(FEED), None)
            .await
            .expect("add");

        assert!(
            feeds.remove(first).await.expect("remove"),
            "the feed was there"
        );

        let second = feeds.upsert("Other", &url(OTHER), None).await.expect("add");

        assert_ne!(second, first, "a removed id never comes back");
        assert_eq!(
            feeds.list().await.expect("list"),
            vec![stored(second, "Other", OTHER, FeedAuth::default())]
        );
    }

    #[sqlx::test]
    async fn record_check_keeps_the_last_outcome(pool: SqlitePool) {
        let feeds = FeedStore::new(pool);
        let id = feeds
            .upsert("Tracker", &url(FEED), None)
            .await
            .expect("add");

        let succeeded = FeedCheck {
            at: at(2),
            outcome: Ok(Ingest { items: 3, added: 1 }),
        };

        for check in [
            FeedCheck {
                at: at(1),
                outcome: Err("the feed is unreachable".to_owned()),
            },
            succeeded.clone(),
        ] {
            assert!(
                feeds.record_check(id, &check).await.expect("record"),
                "the feed was there"
            );
        }

        assert_eq!(
            feeds.list().await.expect("list"),
            vec![StoredFeed {
                check: Some(succeeded.clone()),
                ..stored(id, "Tracker", FEED, FeedAuth::default())
            }],
            "the later check replaces the earlier one, clearing its error text"
        );
        assert!(
            !feeds
                .record_check(id + 404, &succeeded)
                .await
                .expect("record"),
            "an id naming no feed records nothing"
        );
    }
}
