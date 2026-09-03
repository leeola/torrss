use std::sync::Arc;
use std::time::Duration;
use std::{io, path::PathBuf};

use tokio::net::TcpListener;
use tracing::info;

use super::router;
use crate::feed::registry;
use crate::feed::registry::FeedRegistry;
use crate::feed::store::FeedStore;
use crate::ruleset::registry::Rulesets;
use crate::ruleset::store::RulesetStore;
use crate::services::Services;
use crate::torrent::scan;
use crate::torrent::scan::ScanState;

/// Host the listener binds to when the caller names none.
pub const DEFAULT_HOST: &str = "127.0.0.1";

/// Port the listener binds to when the caller names none.
pub const DEFAULT_PORT: u16 = 3000;

/// Where [`serve`] binds its listener and where it reads its assets from.
pub struct Config {
    pub host: String,
    pub port: u16,

    /// Directory holding the asset bundle.
    ///
    /// [`None`] reads the `assets` directory beside the executable, which is
    /// where `topcoat dev` and `topcoat asset bundle` write it. Name a
    /// directory here to serve a bundle from a deployment-specific location.
    pub assets: Option<PathBuf>,

    /// How old a feed's last check must be before the poll fetches it again,
    /// and the same for the library scan.
    ///
    /// The checks persist, so a restart resumes the schedule rather than
    /// starting it over. A feed checked recently is skipped, and the wait
    /// ends when the oldest one falls due. A slow tracker or client delays
    /// its own next pass rather than stacking passes on top of each other.
    /// The two loops run independently, so neither waits on the other.
    pub poll_interval: Duration,
}

/// Serves the web application until the process receives Ctrl+C or `SIGTERM`.
///
/// Polls every registered feed and scans the torrent client's library in the
/// background for as long as the listener runs, and stops both before
/// returning. In-flight requests get the shutdown timeout of the router
/// service to finish.
///
/// # Errors
///
/// Returns an error if the asset bundle is missing or unreadable, if binding
/// the listener fails, or if accepting a connection fails.
pub async fn serve(config: &Config, services: Services) -> io::Result<()> {
    let feed_registry = Arc::new(
        FeedRegistry::load(FeedStore::new(services.db.clone()))
            .await
            .map_err(io::Error::other)?,
    );

    let rulesets = Arc::new(
        Rulesets::load(RulesetStore::new(services.db.clone()))
            .await
            .map_err(io::Error::other)?,
    );

    // Named `scan_state` because the module `scan` has to stay in scope for
    // the poll below.
    let scan_state = Arc::new(ScanState::new());

    // Every fallible step runs before either background task starts. Dropping
    // a join handle detaches the task rather than stopping it, so an early
    // return after a spawn strands a task that then runs forever.
    let router = router::build(
        config.assets.as_deref(),
        services.clone(),
        Arc::clone(&feed_registry),
        Arc::clone(&rulesets),
        Arc::clone(&scan_state),
    )?;
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;

    // The bound address rather than the configured one, so a port of 0 reads
    // as the port the operating system chose.
    info!(address = %listener.local_addr()?, "listening");

    let polling = tokio::spawn(registry::poll(
        feed_registry,
        services.db.clone(),
        Arc::clone(&services.feeds),
        Arc::clone(&services.clock),
        config.poll_interval,
    ));

    let scanning = tokio::spawn(scan::poll(
        scan_state,
        rulesets,
        services.db,
        services.torrents,
        services.clock,
        config.poll_interval,
    ));

    let served = topcoat::serve(listener, router).await;
    polling.abort();
    scanning.abort();

    served
}
