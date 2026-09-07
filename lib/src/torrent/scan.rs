//! Recording what the torrent client holds.
//!
//! The client lists its torrents and the index takes the whole list, names
//! and all. The feed page then answers "do I have this" from one query
//! rather than one client call per request.
//!
//! A name no search claims is indexed with the rest. The engine reads the
//! names at query time, so a parser edit changes what counts as owned with
//! no rescan, and a name that reads as nothing today may read as something
//! after the next edit.
//!
//! The index and the status of the scan that wrote it both persist, so a
//! restart reads what the last process found rather than listing the whole
//! client again at once.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::{info, instrument, warn};

use crate::clock::{self, Clock};
use crate::engine::Engine;
use crate::search::registry::Searches;
use crate::torrent::TorrentClient;
use crate::torrent::index;

/// Replaces the recorded scan, or writes the first one.
///
/// The row is upserted rather than inserted, because the table holds one row
/// by construction and every scan after the first replaces it.
const RECORD: &str = "
    INSERT INTO scan_status (id, scanned_at, torrents, matched, error)
    VALUES (1, ?1, ?2, ?3, ?4)
    ON CONFLICT (id) DO UPDATE SET
        scanned_at = excluded.scanned_at,
        torrents = excluded.torrents,
        matched = excluded.matched,
        error = excluded.error
";

/// Reads the recorded scan, which is absent until one runs.
const LAST: &str = "SELECT scanned_at, torrents, matched, error FROM scan_status WHERE id = 1";

/// The result of the last scan.
///
/// The status persists, so the client page reads it after a restart and the
/// scan loop knows how old it is rather than listing the whole client at
/// once.
///
/// This mirrors the feed registry. It lives in the app context, and a handler
/// reads it there rather than through an argument.
#[derive(Debug)]
pub(crate) struct ScanState {
    last: Mutex<Option<ScanStatus>>,
}

/// What one scan produced.
///
/// A client failure and a store failure both end a scan the same way, and the
/// pages show only the text, so nothing is gained by keeping the two error
/// types apart this far out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanStatus {
    pub(crate) at: DateTime<Utc>,
    pub(crate) outcome: Result<ScanReport, String>,
}

/// How much of the client's queue the searches claimed.
///
/// The gap between the two counts is what a user reads to judge the rules. A
/// client full of torrents with nothing matched means the searches are wrong,
/// not that the client is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanReport {
    /// How many torrents the client holds.
    pub(crate) torrents: usize,

    /// How many of them a search claimed. Two torrents sometimes share one
    /// identity, so this counts torrents rather than rows written.
    pub(crate) matched: usize,
}

impl ScanState {
    /// Reads the recorded scan into a state, or an empty one until a scan
    /// has run.
    ///
    /// # Errors
    ///
    /// Returns the store's error when the read fails. A count too large for
    /// the machine's index type reads as a decode failure, because it was
    /// written from one and only a corrupt row holds another.
    pub(crate) async fn load(pool: &SqlitePool) -> Result<Self, sqlx::Error> {
        let row = sqlx::query_as::<_, StatusRow>(LAST)
            .fetch_optional(pool)
            .await?;

        let last = match row {
            None => None,
            Some((at, torrents, matched, error)) => Some(ScanStatus {
                at,
                outcome: match error {
                    Some(error) => Err(error),
                    None => Ok(ScanReport {
                        torrents: narrow(torrents)?,
                        matched: narrow(matched)?,
                    }),
                },
            }),
        };

        Ok(Self {
            last: Mutex::new(last),
        })
    }

    /// Returns the last scan's status, or nothing until one runs.
    pub(crate) fn last(&self) -> Option<ScanStatus> {
        self.lock().clone()
    }

    fn lock(&self) -> MutexGuard<'_, Option<ScanStatus>> {
        // Nothing panics while the guard is held, so the lock never poisons.
        self.last
            .lock()
            .expect("the scan state lock is never poisoned")
    }
}

/// Rebuilds the index from what the client holds, and records the outcome.
///
/// Returns the status it stored, so a handler that asked for the scan renders
/// the result without reading the state back.
///
/// A client that fails to answer leaves the previous index alone. Stale rows
/// are the better wrong answer, because an empty index marks every release as
/// missing and invites grabbing the lot a second time.
///
/// The clock is read once, at the start. The same instant stamps the written
/// rows and the recorded status, so a page never shows the two disagreeing by
/// the length of a scan.
#[instrument(name = "scan_torrents", skip_all)]
pub(crate) async fn scan(
    state: &ScanState,
    pool: &SqlitePool,
    client: &dyn TorrentClient,
    clock: &dyn Clock,
    engine: &Engine,
) -> ScanStatus {
    let at = clock.now();

    let outcome = match client.list().await {
        Ok(torrents) => {
            let report = ScanReport {
                torrents: torrents.len(),
                matched: torrents
                    .iter()
                    .filter(|torrent| engine.parse(&torrent.name).is_some())
                    .count(),
            };

            index::replace(pool, at, &torrents)
                .await
                .map(|()| report)
                .map_err(|error| error.to_string())
        }
        Err(error) => Err(error.to_string()),
    };

    // Logged by reference, so the line and the stored status carry one
    // rendering of the error rather than two.
    match &outcome {
        Ok(report) => info!(
            torrents = report.torrents,
            matched = report.matched,
            "scanned"
        ),
        Err(error) => warn!(error = %error, "scan failed"),
    }

    let status = ScanStatus { at, outcome };
    *state.lock() = Some(status.clone());

    // Memory first, as the feed check does. A loop that reads an unscanned
    // index scans again at once, so a refused write costs less than a state
    // that forgot the scan it just ran. The scan itself succeeded either way,
    // and only the next restart reads the gap.
    if let Err(error) = record(pool, &status).await {
        warn!(error = %error, "scan status not stored");
    }

    status
}

/// One `scan_status` row as sqlx hands it back.
type StatusRow = (DateTime<Utc>, Option<i64>, Option<i64>, Option<String>);

/// Replaces the recorded scan with `status`.
async fn record(pool: &SqlitePool, status: &ScanStatus) -> Result<(), sqlx::Error> {
    let (torrents, matched, error) = match &status.outcome {
        Ok(report) => (
            Some(count(report.torrents)?),
            Some(count(report.matched)?),
            None,
        ),
        Err(error) => (None, None, Some(error.as_str())),
    };

    sqlx::query(RECORD)
        .bind(status.at)
        .bind(torrents)
        .bind(matched)
        .bind(error)
        .execute(pool)
        .await?;

    Ok(())
}

/// Narrows a stored count to the machine's index type, treating a missing
/// one as zero.
fn narrow(count: Option<i64>) -> Result<usize, sqlx::Error> {
    usize::try_from(count.unwrap_or(0)).map_err(sqlx::Error::decode)
}

/// Widens a count for storage.
fn count(value: usize) -> Result<i64, sqlx::Error> {
    i64::try_from(value).map_err(|error| sqlx::Error::Encode(Box::new(error)))
}

/// Scans the client when the last scan is older than `interval`, and returns
/// how long to wait before the next one falls due.
///
/// A client never scanned is due at once. The status persists, so this reads
/// what the last process already did, which is what makes a restart cheap.
///
/// The wait is measured after the pass, over the status it just wrote, so a
/// pass that took minutes shortens the wait by what it spent. It never drops
/// below [`clock::MIN_PAUSE`]. A status that fails to store leaves the scan
/// due forever, and the floor is what keeps that from spinning the loop.
pub(crate) async fn scan_due(
    state: &ScanState,
    pool: &SqlitePool,
    client: &dyn TorrentClient,
    clock: &dyn Clock,
    engine: &Engine,
    interval: Duration,
) -> Duration {
    let now = clock.now();
    let due = state
        .last()
        .is_none_or(|last| clock::remaining(interval, last.at, now).is_zero());

    if due {
        scan(state, pool, client, clock, engine).await;
    }

    state
        .last()
        .map_or(interval, |last| {
            clock::remaining(interval, last.at, clock.now())
        })
        .max(clock::MIN_PAUSE)
}

/// Scans the client forever, waiting until the next scan falls due.
///
/// The first turn skips a scan the last process ran within `interval`, and
/// the wait ends when that scan reaches it. A restart therefore neither lists
/// the whole client at once nor waits a whole interval.
///
/// The wait runs after a pass rather than on a fixed schedule, so a slow
/// client delays the next pass instead of stacking passes on top of each
/// other.
///
/// This runs as its own task rather than beside the feed poll. The two have
/// no reason to share a rate, and one slow client would otherwise hold up
/// every feed check behind it.
#[instrument(name = "scan_poll", skip_all, fields(interval_secs = interval.as_secs()))]
pub(crate) async fn poll(
    state: Arc<ScanState>,
    searches: Arc<Searches>,
    pool: SqlitePool,
    client: Arc<dyn TorrentClient>,
    clock: Arc<dyn Clock>,
    interval: Duration,
) {
    loop {
        let pause = scan_due(
            &state,
            &pool,
            client.as_ref(),
            clock.as_ref(),
            &searches.engine(),
            interval,
        )
        .await;

        clock.sleep(pause).await;
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::Duration;

    use sqlx::SqlitePool;

    use super::{ScanReport, ScanState, ScanStatus, scan, scan_due};
    use crate::clock::Clock;
    use crate::search::fixture::ENGINE;
    use crate::services::Services;
    use crate::torrent::TorrentError;
    use crate::torrent::index;

    const HOLLOW: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const NEXT_EPISODE: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const UNCLAIMED: &str = "just some words with no structure at all";
    /// A whole season the client holds as one torrent.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S04.1080p.Broadcast";

    const HOLLOW_KEY: &str = "show+season+episodeNumber|the hollow meridian|4|6";
    const NEXT_KEY: &str = "show+season+episodeNumber|the hollow meridian|4|7";
    const FILM_KEY: &str = "title+year|coastal drift|2024";
    const PACK_KEY: &str = "show+season+episodeNumber|the hollow meridian|4|";

    fn set(identities: &[&str]) -> HashSet<String> {
        identities.iter().map(|id| (*id).to_owned()).collect()
    }

    #[sqlx::test]
    async fn scan_stores_one_identity_per_parsed_name(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);
        fakes.torrents.seed(FILM);

        let status = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            status,
            ScanStatus {
                at: fakes.clock.now(),
                outcome: Ok(ScanReport {
                    torrents: 2,
                    matched: 2
                }),
            }
        );
        assert_eq!(
            state.last(),
            Some(status),
            "the returned status is the recorded one"
        );
        assert_eq!(
            index::identities(&ENGINE, &index::all(&services.db).await.expect("torrents")),
            set(&[HOLLOW_KEY, FILM_KEY]),
            "each name reaches the index under its own identity"
        );
    }

    #[sqlx::test]
    async fn load_restores_the_last_scan(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);
        fakes.torrents.seed(FILM);

        let scanned = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            ScanState::load(&services.db).await.expect("load").last(),
            Some(scanned),
            "a restart reads back the counts the last scan recorded"
        );

        fakes.torrents.fail_next(TorrentError::Unauthorized);
        let failed = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert!(failed.outcome.is_err(), "the client refused the listing");
        assert_eq!(
            ScanState::load(&services.db).await.expect("load").last(),
            Some(failed),
            "a failure replaces the counts, error text and all"
        );
    }

    const INTERVAL: Duration = Duration::from_secs(900);

    async fn due(services: &Services, state: &ScanState) -> Duration {
        scan_due(
            state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
            INTERVAL,
        )
        .await
    }

    #[sqlx::test]
    async fn scan_due_skips_a_recent_scan(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);

        let first = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        fakes.clock.advance(Duration::from_secs(300));

        assert_eq!(
            due(&services, &state).await,
            Duration::from_secs(600),
            "ten of the fifteen minutes are left to run"
        );
        assert_eq!(
            state.last().map(|last| last.at),
            Some(first.at),
            "the scan five minutes ago still stands, so none ran"
        );
    }

    #[sqlx::test]
    async fn scan_due_scans_when_nothing_is_recorded(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);

        assert_eq!(
            due(&services, &state).await,
            INTERVAL,
            "the scan it just ran starts the interval over"
        );
        assert_eq!(
            state.last().map(|last| last.at),
            Some(fakes.clock.now()),
            "a client never scanned is due at once"
        );
    }

    #[sqlx::test]
    async fn scan_stores_a_season_pack_as_a_span(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW_PACK);

        scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            index::identities(&ENGINE, &index::all(&services.db).await.expect("torrents")),
            set(&[PACK_KEY]),
            "the empty episode part is what makes the stored key a span"
        );
    }

    #[sqlx::test]
    async fn scan_indexes_a_name_no_search_claims(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);
        fakes.torrents.seed(UNCLAIMED);

        let status = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            status.outcome,
            Ok(ScanReport {
                torrents: 2,
                matched: 1
            }),
            "an unclaimed torrent counts against the total, not the match"
        );

        let indexed = index::all(&services.db).await.expect("torrents");

        assert_eq!(
            indexed
                .iter()
                .map(|torrent| torrent.name.as_str())
                .collect::<HashSet<_>>(),
            HashSet::from([HOLLOW, UNCLAIMED]),
            "the index holds every name the client reported"
        );
        assert_eq!(
            index::identities(&ENGINE, &indexed),
            set(&[HOLLOW_KEY]),
            "and an unclaimed name contributes no identity"
        );
    }

    #[sqlx::test]
    async fn scan_records_client_error_and_keeps_the_index(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        fakes.torrents.seed(HOLLOW);

        scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        fakes.torrents.fail_next(TorrentError::Unauthorized);
        let status = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            status.outcome,
            Err("the torrent client rejected the credentials".to_owned())
        );
        assert_eq!(
            index::identities(&ENGINE, &index::all(&services.db).await.expect("torrents")),
            set(&[HOLLOW_KEY]),
            "a client that cannot answer leaves the last snapshot standing"
        );
    }

    #[sqlx::test]
    async fn scan_replaces_previous_snapshot(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::load(&services.db).await.expect("load");
        let removed = fakes.torrents.seed(HOLLOW);

        scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        fakes.torrents.forget(&removed);
        fakes.torrents.seed(NEXT_EPISODE);

        let status = scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        assert_eq!(
            status.outcome,
            Ok(ScanReport {
                torrents: 1,
                matched: 1
            })
        );
        assert_eq!(
            index::identities(&ENGINE, &index::all(&services.db).await.expect("torrents")),
            set(&[NEXT_KEY]),
            "a torrent gone from the client drops out of the index"
        );
    }
}
