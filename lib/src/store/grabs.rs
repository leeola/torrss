//! What was already grabbed, how each attempt went, and which rulesets
//! claimed it.
//!
//! The feed page marks a row the application has acted on, so a reader tells
//! a release nobody has touched from one that failed to reach the client.
//!
//! One row per item. A grab is an attempt rather than an event, so a retry
//! overwrites the last result instead of growing a history nothing reads.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

/// Records the attempt, keeping the row a retry lands on.
///
/// An update rather than a replace. A replace deletes the row first, and a
/// cascading reference loses its own rows to that delete. `grab_rulesets` is
/// rewritten immediately after this either way, so the two forms agree today.
/// The update form is what keeps them agreeing once a second table references
/// a grab.
const UPSERT: &str = "
    INSERT INTO grabs (item_id, grabbed_at, error)
    VALUES (?1, ?2, ?3)
    ON CONFLICT (item_id) DO UPDATE SET
        grabbed_at = excluded.grabbed_at,
        error = excluded.error
";

/// Reads every attempt with the rulesets that claimed it.
///
/// A left join, because a grab with no recorded ruleset is still a grab and
/// still belongs in the map.
///
/// The order is what carries the engine's ranking through to the page, so it
/// sorts by `position` rather than leaving the join to answer in its own
/// order.
const SELECT: &str = "
    SELECT g.item_id, g.grabbed_at, g.error, r.ruleset
    FROM grabs g
    LEFT JOIN grab_rulesets r ON r.item_id = g.item_id
    ORDER BY g.item_id, r.position
";

/// The latest attempt to grab one stored item.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Grab {
    /// The [`StoredItem::id`](crate::store::StoredItem::id) this attempt was
    /// made for.
    pub(crate) item_id: i64,

    pub(crate) at: DateTime<Utc>,

    /// Why the attempt failed, or nothing when the client accepted it.
    pub(crate) error: Option<String>,

    /// Every ruleset that claimed the release, most specific first.
    pub(crate) rulesets: Vec<String>,
}

/// Records one attempt and the rulesets it passed, replacing the last one.
///
/// Runs as one transaction, so a failure part way leaves the previous attempt
/// rather than a grab with half its rulesets.
///
/// The item has to exist in `feed_items`. Foreign keys are enforced, so a
/// grab against an id nothing stored is a caller bug and fails here rather
/// than leaving a row that names nothing.
pub(crate) async fn record(
    pool: &SqlitePool,
    item_id: i64,
    at: DateTime<Utc>,
    error: Option<&str>,
    rulesets: &[&str],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query(UPSERT)
        .bind(item_id)
        .bind(at)
        .bind(error)
        .execute(&mut *tx)
        .await?;

    sqlx::query("DELETE FROM grab_rulesets WHERE item_id = ?1")
        .bind(item_id)
        .execute(&mut *tx)
        .await?;

    for (position, ruleset) in rulesets.iter().enumerate() {
        sqlx::query(
            "INSERT INTO grab_rulesets (item_id, ruleset, position)
             VALUES (?1, ?2, ?3)",
        )
        .bind(item_id)
        .bind(ruleset)
        .bind(position as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await
}

/// Returns every attempt, keyed by the item it was made for.
///
/// The whole table comes back at once, because the feed page tests every
/// listed row against it. A query per row costs one round trip each.
///
/// One grab spans as many rows as it has rulesets, so the rows fold on the
/// item id rather than mapping one to one.
pub(crate) async fn all(pool: &SqlitePool) -> Result<HashMap<i64, Grab>, sqlx::Error> {
    let rows = sqlx::query(SELECT).fetch_all(pool).await?;
    let mut grabs: HashMap<i64, Grab> = HashMap::new();

    for row in &rows {
        let item_id: i64 = row.try_get("item_id")?;

        let grab = match grabs.entry(item_id) {
            Entry::Occupied(entry) => entry.into_mut(),
            Entry::Vacant(entry) => entry.insert(Grab {
                item_id,
                at: row.try_get("grabbed_at")?,
                error: row.try_get("error")?,
                rulesets: Vec::new(),
            }),
        };

        if let Some(ruleset) = row.try_get::<Option<String>, _>("ruleset")? {
            grab.rulesets.push(ruleset);
        }
    }

    Ok(grabs)
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
            &[],
        )
        .await
        .expect("first attempt");
        record(&pool, ids[0], at(3), None, &[])
            .await
            .expect("retry");
        record(&pool, ids[1], at(4), None, &[])
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
                        rulesets: Vec::new(),
                    }
                ),
                (
                    ids[1],
                    Grab {
                        item_id: ids[1],
                        at: at(4),
                        error: None,
                        rulesets: Vec::new(),
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
            &[],
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
                    rulesets: Vec::new(),
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
            record(&pool, 404, at(2), None, &[]).await.is_err(),
            "foreign keys keep a grab from naming an item nothing stored"
        );
    }

    #[sqlx::test]
    async fn record_stores_rulesets_in_order(pool: SqlitePool) {
        let ids = ingested(&pool, &["A.Release"]).await;
        record(
            &pool,
            ids[0],
            at(2),
            None,
            &["series-hollow-meridian", "series-episodes"],
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
                    error: None,
                    rulesets: vec![
                        "series-hollow-meridian".to_owned(),
                        "series-episodes".to_owned(),
                    ],
                }
            )]),
            "the engine ranked the child first, and the order survives the join"
        );
    }

    #[sqlx::test]
    async fn record_retry_replaces_rulesets(pool: SqlitePool) {
        let ids = ingested(&pool, &["A.Release"]).await;
        record(
            &pool,
            ids[0],
            at(2),
            None,
            &["series-hollow-meridian", "series-episodes"],
        )
        .await
        .expect("first attempt");

        record(&pool, ids[0], at(3), None, &["feature-films"])
            .await
            .expect("retry");

        assert_eq!(
            all(&pool).await.expect("all"),
            HashMap::from([(
                ids[0],
                Grab {
                    item_id: ids[0],
                    at: at(3),
                    error: None,
                    rulesets: vec!["feature-films".to_owned()],
                }
            )]),
            "a retry leaves none of the rulesets the first attempt recorded"
        );
    }

    #[sqlx::test]
    async fn grab_with_no_rulesets_still_returns(pool: SqlitePool) {
        let ids = ingested(&pool, &["A.Release"]).await;
        record(&pool, ids[0], at(2), None, &[])
            .await
            .expect("record");

        assert_eq!(
            all(&pool).await.expect("all"),
            HashMap::from([(
                ids[0],
                Grab {
                    item_id: ids[0],
                    at: at(2),
                    error: None,
                    rulesets: Vec::new(),
                }
            )]),
            "the left join keeps a grab that claimed nothing"
        );
    }
}
