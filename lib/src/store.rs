//! What the application keeps between restarts.
//!
//! The feed registry lives in memory and empties on a restart, but the items
//! it fetched do not. Every item any feed returned is written here raw, so
//! the release parser reads the whole history rather than only what the last
//! poll happened to catch.

pub mod library;

use chrono::{DateTime, Utc};
use sqlx::migrate::MigrateError;
use sqlx::sqlite::SqliteRow;
use sqlx::{Row, SqlitePool};
use url::Url;

use crate::feed::FeedItem;

/// Adds every item, and updates the row for one already stored.
///
/// `first_seen` keeps the value it was written with, because it records when
/// a release reached this application rather than when it was last
/// confirmed. A value the fetch omits keeps whatever an earlier fetch
/// stored, so a tracker that drops a size between fetches does not erase it.
const UPSERT: &str = "
    INSERT INTO feed_items
        (feed_url, guid, title, link, published, size, seeders, first_seen, last_seen)
    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
    ON CONFLICT (feed_url, guid) DO UPDATE SET
        title = excluded.title,
        link = excluded.link,
        published = coalesce(excluded.published, published),
        size = coalesce(excluded.size, size),
        seeders = coalesce(excluded.seeders, seeders),
        last_seen = excluded.last_seen
";

/// Reads every stored item, or one feed's, newest first.
///
/// A null bind matches every feed, which keeps one statement for both reads
/// rather than two that drift apart as columns change. The cost is that the
/// filtered read scans rather than using the index behind the unique key,
/// which a feed reader's row counts never make noticeable.
///
/// Undated items sort last rather than first, which is the opposite of where
/// a null lands on its own. The trailing terms make the order total, so two
/// rows never swap places between reads.
const SELECT: &str = "
    SELECT id, feed_url, guid, title, link, published, size, seeders,
           first_seen, last_seen
    FROM feed_items
    WHERE ?1 IS NULL OR feed_url = ?1
    ORDER BY published IS NULL, published DESC, first_seen DESC, id DESC
";

/// What one fetch put into the store.
///
/// The two counts differ whenever a tracker republishes what it already
/// announced, which is most of the time. A caller reports a fetch as
/// interesting only when `added` is above zero.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ingest {
    /// How many items the feed carried.
    pub items: usize,

    /// How many of them the store had never seen.
    pub added: usize,
}

/// One stored item, with the times it was first and last seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredItem {
    pub id: i64,
    pub feed_url: Url,
    pub item: FeedItem,

    /// When this application first saw the release announced.
    pub first_seen: DateTime<Utc>,

    /// When a fetch last found it still listed.
    pub last_seen: DateTime<Utc>,
}

/// Writes one fetch of `feed_url` into the store.
///
/// Runs as one transaction, so a failure part way through leaves the store
/// as it was rather than half updated.
pub async fn ingest(
    pool: &SqlitePool,
    feed_url: &Url,
    seen_at: DateTime<Utc>,
    items: &[FeedItem],
) -> Result<Ingest, sqlx::Error> {
    let mut tx = pool.begin().await?;
    let before = count(&mut tx, feed_url).await?;

    for item in items {
        sqlx::query(UPSERT)
            .bind(feed_url.as_str())
            .bind(&item.guid)
            .bind(&item.title)
            .bind(item.link.as_str())
            .bind(item.published)
            // A size beyond an i64 is a tracker typo rather than a real
            // release, and clamping keeps one bad row from failing the fetch.
            .bind(
                item.size
                    .map(|size| i64::try_from(size).unwrap_or(i64::MAX)),
            )
            .bind(item.seeders.map(i64::from))
            .bind(seen_at)
            .execute(&mut *tx)
            .await?;
    }

    let after = count(&mut tx, feed_url).await?;
    tx.commit().await?;

    Ok(Ingest {
        items: items.len(),
        added: usize::try_from(after - before).unwrap_or(0),
    })
}

/// Returns stored items newest first, for one feed or for every feed.
pub async fn items(
    pool: &SqlitePool,
    feed_url: Option<&Url>,
) -> Result<Vec<StoredItem>, sqlx::Error> {
    sqlx::query(SELECT)
        .bind(feed_url.map(Url::as_str))
        .fetch_all(pool)
        .await?
        .into_iter()
        .map(stored)
        .collect()
}

/// Brings `pool` up to the current schema.
///
/// Applies only the migrations a database lacks, so a call on every start is
/// correct and cheap. The SQL is embedded at compile time from the library's
/// `migrations` directory, so a deployment ships the binary alone.
pub async fn migrate(pool: &SqlitePool) -> Result<(), MigrateError> {
    sqlx::migrate!().run(pool).await
}

async fn count(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    feed_url: &Url,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT count(*) FROM feed_items WHERE feed_url = ?1")
        .bind(feed_url.as_str())
        .fetch_one(&mut **tx)
        .await
}

/// Rebuilds one item from its row.
///
/// A URL that no longer parses, or a size too large for the domain type,
/// means the row is corrupt. That is a read failure rather than a panic, so
/// each maps to a decode error naming the column.
fn stored(row: SqliteRow) -> Result<StoredItem, sqlx::Error> {
    let feed_url: String = row.try_get("feed_url")?;
    let link: String = row.try_get("link")?;
    let size: Option<i64> = row.try_get("size")?;
    let seeders: Option<i64> = row.try_get("seeders")?;

    Ok(StoredItem {
        id: row.try_get("id")?,
        feed_url: url(&feed_url)?,
        item: FeedItem {
            guid: row.try_get("guid")?,
            title: row.try_get("title")?,
            link: url(&link)?,
            published: row.try_get("published")?,
            size: size.map(u64::try_from).transpose().map_err(decode)?,
            seeders: seeders.map(u32::try_from).transpose().map_err(decode)?,
        },
        first_seen: row.try_get("first_seen")?,
        last_seen: row.try_get("last_seen")?,
    })
}

fn url(raw: &str) -> Result<Url, sqlx::Error> {
    Url::parse(raw).map_err(decode)
}

fn decode(error: impl std::error::Error + Send + Sync + 'static) -> sqlx::Error {
    sqlx::Error::decode(error)
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;
    use url::Url;

    use super::{Ingest, StoredItem, ingest, items, migrate};
    use crate::feed::FeedItem;
    use crate::feed::fake;

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

    fn stored(id: i64, feed: &str, item: FeedItem, first: u32, last: u32) -> StoredItem {
        StoredItem {
            id,
            feed_url: url(feed),
            item,
            first_seen: at(first),
            last_seen: at(last),
        }
    }

    #[sqlx::test]
    async fn ingest_inserts_new_rows(pool: SqlitePool) {
        let report = ingest(
            &pool,
            &url(FEED),
            at(1),
            &[
                fake::item("First.Release").published(at(2)),
                fake::item("Second.Release").published(at(3)),
            ],
        )
        .await
        .expect("ingest");

        assert_eq!(report, Ingest { items: 2, added: 2 });
        assert_eq!(
            items(&pool, None).await.expect("items"),
            vec![
                stored(2, FEED, fake::item("Second.Release").published(at(3)), 1, 1),
                stored(1, FEED, fake::item("First.Release").published(at(2)), 1, 1),
            ],
            "newest published first"
        );
    }

    #[sqlx::test]
    async fn ingest_same_guid_updates_row_and_keeps_first_seen(pool: SqlitePool) {
        let feed = url(FEED);
        ingest(&pool, &feed, at(1), &[fake::item("A.Release")])
            .await
            .expect("first ingest");

        let mut renamed = fake::item("A.Release");
        renamed.title = "A.Release.PROPER".to_owned();

        let report = ingest(&pool, &feed, at(4), &[renamed.clone()])
            .await
            .expect("second ingest");

        assert_eq!(
            report,
            Ingest { items: 1, added: 0 },
            "the same guid adds no row"
        );
        assert_eq!(
            items(&pool, None).await.expect("items"),
            vec![stored(1, FEED, renamed, 1, 4)],
            "the title and last seen move, the first seen does not"
        );
    }

    #[sqlx::test]
    async fn ingest_keeps_size_when_refetch_omits_it(pool: SqlitePool) {
        let feed = url(FEED);
        ingest(
            &pool,
            &feed,
            at(1),
            &[fake::item("A.Release").size(2048).seeders(9)],
        )
        .await
        .expect("first ingest");

        ingest(&pool, &feed, at(2), &[fake::item("A.Release")])
            .await
            .expect("second ingest");

        assert_eq!(
            items(&pool, None).await.expect("items"),
            vec![stored(
                1,
                FEED,
                fake::item("A.Release").size(2048).seeders(9),
                1,
                2
            )],
            "a fetch that omits a value keeps the stored one"
        );
    }

    #[sqlx::test]
    async fn items_filter_by_feed_url(pool: SqlitePool) {
        ingest(&pool, &url(FEED), at(1), &[fake::item("Mine")])
            .await
            .expect("first feed");
        ingest(&pool, &url(OTHER), at(1), &[fake::item("Theirs")])
            .await
            .expect("second feed");

        assert_eq!(
            items(&pool, Some(&url(OTHER))).await.expect("items"),
            vec![stored(2, OTHER, fake::item("Theirs"), 1, 1)],
            "only the named feed"
        );
        assert_eq!(
            items(&pool, None).await.expect("items").len(),
            2,
            "no filter returns every feed"
        );
    }

    #[sqlx::test]
    async fn items_sort_newest_first_undated_last(pool: SqlitePool) {
        ingest(
            &pool,
            &url(FEED),
            at(1),
            &[
                fake::item("Undated"),
                fake::item("Older").published(at(2)),
                fake::item("Newer").published(at(5)),
            ],
        )
        .await
        .expect("ingest");

        assert_eq!(
            items(&pool, None)
                .await
                .expect("items")
                .into_iter()
                .map(|stored| stored.item.title)
                .collect::<Vec<_>>(),
            vec!["Newer", "Older", "Undated"],
            "an undated item sorts last, not first"
        );
    }

    /// `#[sqlx::test]` has already applied the same migrations to `pool`, so
    /// this runs them a second time. Reading the table afterwards proves both
    /// that it exists and that the repeat run changed nothing.
    #[sqlx::test]
    async fn migrate_is_idempotent(pool: SqlitePool) {
        migrate(&pool)
            .await
            .expect("a repeat migration is harmless");

        assert_eq!(
            items(&pool, None).await.expect("items"),
            Vec::new(),
            "a migration seeds no rows"
        );
    }
}
