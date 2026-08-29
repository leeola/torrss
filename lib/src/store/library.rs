//! What the torrent client already holds.
//!
//! The feed page marks an item as owned, which it answers from here rather
//! than from the client. One query per request beats one client call per
//! request, and the page still renders when the client is down.
//!
//! A sync writes this table whole. It records a snapshot rather than a
//! stream of changes, so a torrent removed in the client is known only by
//! its absence from the next snapshot.

// FIXME: This module belongs to the crate rather than to its API. It is
// public only because the library sync that fills it does not exist yet, and
// a `pub(crate)` item no caller reaches reads as dead code. Narrow it
// alongside `rules`, once the sync lands.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::torrent::TorrentId;

/// One release the client holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Owned {
    /// The rendered [`Identity`](crate::rules::Identity), which is the key.
    pub identity: String,

    /// The root ruleset the identity came from, kept so a reader sees which
    /// rules claimed the torrent without parsing the key apart.
    pub ruleset: String,

    pub torrent_id: TorrentId,

    /// What the client calls the torrent, for a listing that names it.
    pub name: String,
}

/// Rewrites the whole library from one sync.
///
/// Runs as a single transaction, so a failure part way leaves the previous
/// snapshot rather than an empty table.
///
/// Two torrents sometimes parse to one identity, such as the same episode
/// grabbed twice in different qualities. The later one wins rather than
/// failing the sync, because one odd pair is no reason to lose the rest.
pub async fn replace(
    pool: &SqlitePool,
    synced_at: DateTime<Utc>,
    owned: &[Owned],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM library").execute(&mut *tx).await?;

    for entry in owned {
        sqlx::query(
            "INSERT OR REPLACE INTO library
                (identity, ruleset, torrent_id, name, synced_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .bind(&entry.identity)
        .bind(&entry.ruleset)
        .bind(&entry.torrent_id.0)
        .bind(&entry.name)
        .bind(synced_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await
}

/// Returns every identity the client holds.
///
/// The whole set comes back at once, because the feed page tests every
/// listed item against it. A query per item costs one round trip per row.
pub async fn identities(pool: &SqlitePool) -> Result<HashSet<String>, sqlx::Error> {
    let rows: Vec<String> = sqlx::query_scalar("SELECT identity FROM library")
        .fetch_all(pool)
        .await?;

    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;

    use super::{Owned, identities, replace};
    use crate::torrent::TorrentId;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    fn owned(identity: &str, torrent: &str, name: &str) -> Owned {
        Owned {
            identity: identity.to_owned(),
            ruleset: "series-episodes".to_owned(),
            torrent_id: TorrentId(torrent.to_owned()),
            name: name.to_owned(),
        }
    }

    fn set(identities: &[&str]) -> HashSet<String> {
        identities.iter().map(|id| (*id).to_owned()).collect()
    }

    #[sqlx::test]
    async fn replace_drops_rows_absent_from_the_new_set(pool: SqlitePool) {
        replace(
            &pool,
            at(1),
            &[
                owned("series|show|4|6", "t1", "Show.S04E06"),
                owned("series|show|4|7", "t2", "Show.S04E07"),
            ],
        )
        .await
        .expect("first sync");

        replace(
            &pool,
            at(2),
            &[owned("series|show|4|7", "t2", "Show.S04E07")],
        )
        .await
        .expect("second sync");

        assert_eq!(
            identities(&pool).await.expect("identities"),
            set(&["series|show|4|7"]),
            "a torrent removed in the client drops out here"
        );
    }

    #[sqlx::test]
    async fn replace_keeps_one_row_per_identity(pool: SqlitePool) {
        replace(
            &pool,
            at(1),
            &[
                owned("series|show|4|6", "t1", "Show.S04E06.720p"),
                owned("series|show|4|6", "t2", "Show.S04E06.1080p"),
            ],
        )
        .await
        .expect("a duplicate identity does not fail the sync");

        assert_eq!(
            identities(&pool).await.expect("identities"),
            set(&["series|show|4|6"])
        );
    }

    #[sqlx::test]
    async fn identities_of_empty_library_is_empty(pool: SqlitePool) {
        assert_eq!(identities(&pool).await.expect("identities"), HashSet::new());
    }
}
