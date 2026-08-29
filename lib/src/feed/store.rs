//! The feeds this application watches, kept between restarts.
//!
//! The registry holds the same set in memory and empties on a restart. This
//! is what refills it, so a feed registered through the admin page outlives
//! the process that registered it.
//!
//! Keyed by URL. Two names for one URL are one feed, and the URL is what a
//! fetch actually goes to.

use sqlx::SqlitePool;
use url::Url;

use super::FeedAuth;

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
const SELECT: &str = "SELECT id, name, url, auth FROM feeds ORDER BY id";

/// One feed as the store holds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredFeed {
    pub id: i64,
    pub name: String,
    pub url: Url,
    pub auth: FeedAuth,
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
        sqlx::query_as::<_, (i64, String, String, Option<String>)>(SELECT)
            .fetch_all(&self.pool)
            .await?
            .into_iter()
            .map(stored_feed)
            .collect()
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

/// Rebuilds one feed from its row.
///
/// A URL or an auth value that no longer parses means the row is corrupt.
/// Both were valid when written, so each is a read failure rather than a
/// condition a caller handles apart.
fn stored_feed(row: (i64, String, String, Option<String>)) -> Result<StoredFeed, sqlx::Error> {
    let (id, name, url, auth) = row;

    Ok(StoredFeed {
        id,
        name,
        url: Url::parse(&url).map_err(sqlx::Error::decode)?,
        auth: match auth {
            Some(encoded) => serde_json::from_str(&encoded).map_err(sqlx::Error::decode)?,
            None => FeedAuth::default(),
        },
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use sqlx::SqlitePool;
    use url::Url;

    use super::{FeedStore, StoredFeed};
    use crate::feed::FeedAuth;

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
        }
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
}
