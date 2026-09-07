//! Every torrent the client holds, kept between restarts.
//!
//! The feed page marks an item as owned, which it answers from here rather
//! than from the client. One query per request beats one client call per
//! request, and the page still renders when the client is down.
//!
//! Names are stored as the client reports them, and [`identities`] reads them
//! through the engine at query time. A parser edit therefore changes what
//! counts as owned with no rescan.
//!
//! A scan writes this table whole. It records a snapshot rather than a stream
//! of changes, so a torrent removed in the client is known only by its
//! absence from the next scan.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};

use crate::engine::Engine;
use crate::torrent::{Torrent, TorrentId, TorrentState};

/// Reads the whole index, newest addition first.
///
/// A torrent the client gave no date for sorts last rather than first, which
/// is the opposite of where a null lands on its own. The id makes the order
/// total, so two rows never swap places between reads.
const SELECT: &str = "
    SELECT id, name, state, error, size, progress, added_at
    FROM torrents
    ORDER BY added_at IS NULL, added_at DESC, id
";

/// Rewrites the whole index from one scan.
///
/// Runs as a single transaction, so a failure part way leaves the previous
/// snapshot rather than an empty table.
pub(crate) async fn replace(
    pool: &SqlitePool,
    scanned_at: DateTime<Utc>,
    torrents: &[Torrent],
) -> Result<(), sqlx::Error> {
    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM torrents")
        .execute(&mut *tx)
        .await?;

    for torrent in torrents {
        let (state, error) = state_label(&torrent.state);
        let size =
            i64::try_from(torrent.size).map_err(|error| sqlx::Error::Encode(Box::new(error)))?;

        sqlx::query(
            "INSERT OR REPLACE INTO torrents
                (id, name, state, error, size, progress, added_at, scanned_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&torrent.id.0)
        .bind(&torrent.name)
        .bind(state)
        .bind(error)
        .bind(size)
        .bind(torrent.progress)
        .bind(torrent.added_at)
        .bind(scanned_at)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await
}

/// Returns every indexed torrent.
///
/// # Errors
///
/// Returns a decode failure when a row carries a state label this build does
/// not know. Every stored label was one when it was written, so the row is
/// corrupt rather than merely unexpected.
pub(crate) async fn all(pool: &SqlitePool) -> Result<Vec<Torrent>, sqlx::Error> {
    let mut torrents = Vec::new();

    for row in sqlx::query(SELECT).fetch_all(pool).await? {
        let label: String = row.try_get("state")?;
        let size: i64 = row.try_get("size")?;

        torrents.push(Torrent {
            id: TorrentId(row.try_get("id")?),
            name: row.try_get("name")?,
            state: state_of(&label, row.try_get("error")?)?,
            size: u64::try_from(size).map_err(sqlx::Error::decode)?,
            progress: row.try_get("progress")?,
            added_at: row.try_get("added_at")?,
        });
    }

    Ok(torrents)
}

/// Returns the identity of every indexed name the engine reads.
///
/// The whole set comes back at once, because the feed page tests every listed
/// item against it. A lookup per item costs one pass of the engine per row.
///
/// A name no search claims contributes nothing. A client holds plenty this
/// application never grabbed, and such a name answers no question the feed
/// page asks.
pub(crate) fn identities(engine: &Engine, torrents: &[Torrent]) -> HashSet<String> {
    torrents
        .iter()
        .filter_map(|torrent| engine.parse(&torrent.name))
        .map(|parsed| parsed.identity.to_string())
        .collect()
}

/// Splits a state into the label and the message the table stores apart.
fn state_label(state: &TorrentState) -> (&'static str, Option<&str>) {
    match state {
        TorrentState::Queued => ("queued", None),
        TorrentState::Downloading => ("downloading", None),
        TorrentState::Seeding => ("seeding", None),
        TorrentState::Paused => ("paused", None),
        TorrentState::Error(message) => ("error", Some(message)),
    }
}

/// Rebuilds a state from the label and message a row carries.
///
/// An `error` row with no message reads as an empty one. The column is
/// nullable for the other four states, so a null here is a row written wrong
/// rather than a state of its own.
fn state_of(label: &str, error: Option<String>) -> Result<TorrentState, sqlx::Error> {
    match label {
        "queued" => Ok(TorrentState::Queued),
        "downloading" => Ok(TorrentState::Downloading),
        "seeding" => Ok(TorrentState::Seeding),
        "paused" => Ok(TorrentState::Paused),
        "error" => Ok(TorrentState::Error(error.unwrap_or_default())),
        other => Err(sqlx::Error::decode(format!(
            "unknown torrent state {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use chrono::{DateTime, TimeZone, Utc};
    use sqlx::SqlitePool;

    use super::{all, identities, replace};
    use crate::search::fixture::ENGINE;
    use crate::torrent::{Torrent, TorrentId, TorrentState};

    const HOLLOW: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const NEXT_EPISODE: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const UNCLAIMED: &str = "just some words with no structure at all";

    const HOLLOW_KEY: &str = "show+season+episodeNumber|the hollow meridian|4|6";

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, day, 12, 0, 0)
            .single()
            .expect("the test date is unambiguous")
    }

    fn torrent(id: &str, name: &str) -> Torrent {
        Torrent {
            id: TorrentId(id.to_owned()),
            name: name.to_owned(),
            state: TorrentState::Seeding,
            size: 734_003_200,
            progress: 1.0,
            added_at: Some(at(1)),
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
            &[torrent("t1", HOLLOW), torrent("t2", NEXT_EPISODE)],
        )
        .await
        .expect("first scan");

        replace(&pool, at(2), &[torrent("t2", NEXT_EPISODE)])
            .await
            .expect("second scan");

        assert_eq!(
            all(&pool)
                .await
                .expect("index")
                .iter()
                .map(|torrent| torrent.id.0.clone())
                .collect::<Vec<_>>(),
            ["t2"],
            "a torrent removed in the client drops out here"
        );
    }

    #[sqlx::test]
    async fn all_of_an_empty_index_is_empty(pool: SqlitePool) {
        assert_eq!(all(&pool).await.expect("index"), Vec::new());
    }

    #[sqlx::test]
    async fn a_state_round_trips_through_the_table(pool: SqlitePool) {
        let states = [
            TorrentState::Queued,
            TorrentState::Downloading,
            TorrentState::Seeding,
            TorrentState::Paused,
            TorrentState::Error("the tracker refused the announce".to_owned()),
        ];

        let stored: Vec<Torrent> = states
            .iter()
            .enumerate()
            .map(|(index, state)| Torrent {
                state: state.clone(),
                ..torrent(&format!("t{index}"), HOLLOW)
            })
            .collect();

        replace(&pool, at(1), &stored).await.expect("scan");

        assert_eq!(all(&pool).await.expect("index"), stored);
    }

    #[sqlx::test]
    async fn identities_reads_every_parsed_name(pool: SqlitePool) {
        replace(
            &pool,
            at(1),
            &[torrent("t1", HOLLOW), torrent("t2", UNCLAIMED)],
        )
        .await
        .expect("scan");

        let stored = all(&pool).await.expect("index");

        assert_eq!(
            identities(&ENGINE, &stored),
            set(&[HOLLOW_KEY]),
            "a name no parser reads contributes no identity"
        );
    }
}
