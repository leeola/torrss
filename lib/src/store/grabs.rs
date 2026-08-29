//! What was already grabbed, and how each attempt went.
//!
//! The feed page marks a row the application has acted on, so a reader tells
//! a release nobody has touched from one that failed to reach the client.
//!
//! One row per item. A grab is an attempt rather than an event, so a retry
//! overwrites the last result instead of growing a history nothing reads.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

/// The latest attempt to grab one stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grab {
    /// The [`StoredItem::id`](crate::store::StoredItem::id) this attempt was
    /// made for.
    pub(crate) item_id: i64,

    pub(crate) at: DateTime<Utc>,

    /// Why the attempt failed, or nothing when the client accepted it.
    pub(crate) error: Option<String>,
}

/// Records one attempt, replacing whatever the last one left.
///
/// The item has to exist in `feed_items`. Foreign keys are enforced, so a
/// grab against an id nothing stored is a caller bug and fails here rather
/// than leaving a row that names nothing.
pub(crate) async fn record(
    pool: &SqlitePool,
    item_id: i64,
    at: DateTime<Utc>,
    error: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT OR REPLACE INTO grabs (item_id, grabbed_at, error)
         VALUES (?1, ?2, ?3)",
    )
    .bind(item_id)
    .bind(at)
    .bind(error)
    .execute(pool)
    .await?;

    Ok(())
}

/// Returns every attempt, keyed by the item it was made for.
///
/// The whole table comes back at once, because the feed page tests every
/// listed row against it. A query per row costs one round trip each.
pub(crate) async fn all(pool: &SqlitePool) -> Result<HashMap<i64, Grab>, sqlx::Error> {
    sqlx::query("SELECT item_id, grabbed_at, error FROM grabs")
        .fetch_all(pool)
        .await?
        .iter()
        .map(|row| {
            let grab = Grab {
                item_id: row.try_get("item_id")?,
                at: row.try_get("grabbed_at")?,
                error: row.try_get("error")?,
            };

            Ok((grab.item_id, grab))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;

    use super::{Grab, all, record};
    use crate::feed::fake;
    use crate::store;

    const FEED: &str = "https://tracker.invalid/rss";

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    /// Stores one item per title and returns the ids, in the order given.
    ///
    /// Foreign keys are enforced, so a grab needs a real item behind it.
    async fn ingested(pool: &SqlitePool, titles: &[&str]) -> Vec<i64> {
        let items: Vec<_> = titles.iter().map(|title| fake::item(title)).collect();
        let feed = FEED.parse().expect("the test URL parses");

        store::ingest(pool, &feed, at(1), &items)
            .await
            .expect("ingest");

        let by_title: HashMap<_, _> = store::items(pool, None)
            .await
            .expect("items")
            .into_iter()
            .map(|stored| (stored.item.title, stored.id))
            .collect();

        titles.iter().map(|title| by_title[*title]).collect()
    }

    #[sqlx::test]
    async fn record_replaces_previous_grab(pool: SqlitePool) {
        let ids = ingested(&pool, &["A.Release", "B.Release"]).await;

        record(
            &pool,
            ids[0],
            at(2),
            Some("the download answered with status 403"),
        )
        .await
        .expect("first attempt");
        record(&pool, ids[0], at(3), None).await.expect("retry");
        record(&pool, ids[1], at(4), None)
            .await
            .expect("other item");

        assert_eq!(
            all(&pool).await.expect("all"),
            HashMap::from([
                (
                    ids[0],
                    Grab {
                        item_id: ids[0],
                        at: at(3),
                        error: None,
                    }
                ),
                (
                    ids[1],
                    Grab {
                        item_id: ids[1],
                        at: at(4),
                        error: None,
                    }
                ),
            ]),
            "a retry replaces the failure it followed"
        );
    }

    #[sqlx::test]
    async fn record_keeps_the_failure_text(pool: SqlitePool) {
        let ids = ingested(&pool, &["A.Release"]).await;
        record(
            &pool,
            ids[0],
            at(2),
            Some("the torrent client rejected the request"),
        )
        .await
        .expect("record");

        assert_eq!(
            all(&pool).await.expect("all"),
            HashMap::from([(
                ids[0],
                Grab {
                    item_id: ids[0],
                    at: at(2),
                    error: Some("the torrent client rejected the request".to_owned()),
                }
            )])
        );
    }

    #[sqlx::test]
    async fn all_of_empty_table_is_empty(pool: SqlitePool) {
        assert_eq!(all(&pool).await.expect("all"), HashMap::new());
    }

    #[sqlx::test]
    async fn record_against_unknown_item_fails(pool: SqlitePool) {
        assert!(
            record(&pool, 404, at(2), None).await.is_err(),
            "foreign keys keep a grab from naming an item nothing stored"
        );
    }
}
