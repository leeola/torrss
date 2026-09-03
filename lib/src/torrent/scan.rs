//! Recording which releases the torrent client already holds.
//!
//! A scan is where the three halves of the question meet. The client lists
//! what it holds, the rulesets turn each name into an identity, and the
//! library table takes the result. The feed page then answers "do I have
//! this" from one query.
//!
//! A name no ruleset claims is skipped rather than stored. A client holds
//! plenty this application never grabbed, and a row with no identity answers
//! no question the feed page asks.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use tracing::{info, instrument, warn};

use crate::clock::Clock;
use crate::rules::{ENGINE, Engine};
use crate::store::library;
use crate::store::library::Owned;
use crate::torrent::{Torrent, TorrentClient};

/// The result of the last library scan.
///
/// This mirrors the feed registry: it lives in the app context, and a handler
/// reads it there rather than through an argument.
#[derive(Debug, Default)]
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

/// How much of the client's queue the rulesets claimed.
///
/// The gap between the two counts is what a user reads to judge the rules. A
/// client full of torrents with nothing matched means the rulesets are wrong,
/// not that the client is empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ScanReport {
    /// How many torrents the client holds.
    pub(crate) torrents: usize,

    /// How many of them a ruleset claimed. Two torrents sometimes share one
    /// identity, so this counts torrents rather than rows written.
    pub(crate) matched: usize,
}

impl ScanState {
    pub(crate) fn new() -> Self {
        Self::default()
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

/// Rebuilds the library from what the client holds, and records the outcome.
///
/// Returns the status it stored, so a handler that asked for the scan renders
/// the result without reading the state back.
///
/// A client that fails to answer leaves the previous library alone. Stale
/// rows are the better wrong answer, because an empty library marks every
/// release as missing and invites grabbing the lot a second time.
///
/// The clock is read once, at the start. The same instant stamps the written
/// rows and the recorded status, so a page never shows the two disagreeing by
/// the length of a scan.
#[instrument(name = "scan_library", skip_all)]
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
            let owned = torrents
                .iter()
                .filter_map(|torrent| identify(torrent, engine))
                .collect::<Vec<_>>();

            let report = ScanReport {
                torrents: torrents.len(),
                matched: owned.len(),
            };

            library::replace(pool, at, &owned)
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

    status
}

/// Scans the library forever, pausing `interval` between passes.
///
/// The pause runs after a pass rather than on a fixed schedule, so a slow
/// client delays the next pass instead of stacking passes on top of each
/// other.
///
/// This runs as its own task rather than beside the feed poll. The two have
/// no reason to share a rate, and one slow client would otherwise hold up
/// every feed check behind it.
#[instrument(name = "scan_poll", skip_all, fields(interval_secs = interval.as_secs()))]
pub(crate) async fn poll(
    state: Arc<ScanState>,
    pool: SqlitePool,
    client: Arc<dyn TorrentClient>,
    clock: Arc<dyn Clock>,
    interval: Duration,
) {
    loop {
        scan(&state, &pool, client.as_ref(), clock.as_ref(), &ENGINE).await;
        clock.sleep(interval).await;
    }
}

/// Returns what the library stores for `torrent`, or nothing when no ruleset
/// claims its name.
fn identify(torrent: &Torrent, engine: &Engine) -> Option<Owned> {
    let parsed = engine.parse(&torrent.name)?;

    Some(Owned {
        identity: parsed.identity.to_string(),
        ruleset: parsed.identity.ruleset,
        torrent_id: torrent.id.clone(),
        name: torrent.name.clone(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use sqlx::SqlitePool;

    use super::{ScanReport, ScanState, ScanStatus, scan};
    use crate::clock::Clock;
    use crate::mock::fixture::ENGINE;
    use crate::services::Services;
    use crate::store::library;
    use crate::torrent::{TorrentClient, TorrentError};

    const HOLLOW: &str =
        "The.Hollow.Meridian.S04E06.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const NEXT_EPISODE: &str =
        "The.Hollow.Meridian.S04E07.1080p.Broadcast.AAC.Stereo.H.264-PublicWave.mkv";
    const FILM: &str = "Coastal.Drift.2024.1080p.Remaster.AAC.Stereo.H.264-MeridianPress.mkv";
    const UNCLAIMED: &str = "just some words with no structure at all";
    /// A whole season the client holds as one torrent.
    const HOLLOW_PACK: &str = "The.Hollow.Meridian.S04.1080p.Broadcast";

    const HOLLOW_KEY: &str = "series-episodes|the hollow meridian|4|6";
    const NEXT_KEY: &str = "series-episodes|the hollow meridian|4|7";
    const FILM_KEY: &str = "feature-films|coastal drift|2024";
    const PACK_KEY: &str = "series-episodes|the hollow meridian|4|";

    fn set(identities: &[&str]) -> HashSet<String> {
        identities.iter().map(|id| (*id).to_owned()).collect()
    }

    #[sqlx::test]
    async fn scan_stores_one_identity_per_parsed_name(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::new();
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
            library::identities(&services.db).await.expect("identities"),
            set(&[HOLLOW_KEY, FILM_KEY]),
            "each name reaches the library under its own identity"
        );
    }

    #[sqlx::test]
    async fn scan_stores_a_season_pack_as_a_span(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::new();
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
            library::identities(&services.db).await.expect("identities"),
            set(&[PACK_KEY]),
            "the empty episode part is what makes the stored key a span"
        );
    }

    #[sqlx::test]
    async fn scan_skips_names_no_ruleset_claims(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::new();
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
        assert_eq!(
            library::identities(&services.db).await.expect("identities"),
            set(&[HOLLOW_KEY]),
            "an unclaimed name stores no row"
        );
    }

    #[sqlx::test]
    async fn scan_records_client_error_and_keeps_library(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::new();
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
            library::identities(&services.db).await.expect("identities"),
            set(&[HOLLOW_KEY]),
            "a client that cannot answer leaves the last snapshot standing"
        );
    }

    #[sqlx::test]
    async fn scan_replaces_previous_snapshot(pool: SqlitePool) {
        let (services, fakes) = Services::fake(pool);
        let state = ScanState::new();
        let removed = fakes.torrents.seed(HOLLOW);

        scan(
            &state,
            &services.db,
            services.torrents.as_ref(),
            services.clock.as_ref(),
            &ENGINE,
        )
        .await;

        fakes
            .torrents
            .remove(&removed, false)
            .await
            .expect("remove the seeded torrent");
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
            library::identities(&services.db).await.expect("identities"),
            set(&[NEXT_KEY]),
            "a torrent gone from the client drops out of the library"
        );
    }
}
