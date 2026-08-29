use std::sync::Arc;
use std::time::Duration;
use std::{io, path::PathBuf};

use tokio::net::TcpListener;

use super::router;
use crate::feed::registry;
use crate::feed::registry::FeedRegistry;
use crate::services::Services;

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

    /// Pause between two passes over every registered feed.
    ///
    /// Measured from the end of one pass to the start of the next, so a slow
    /// tracker delays the next pass rather than stacking passes on top of
    /// each other.
    pub poll_interval: Duration,
}

/// Serves the web application until the process receives Ctrl+C or `SIGTERM`.
///
/// Polls every registered feed in the background for as long as the listener
/// runs, and stops polling before returning. In-flight requests get the
/// shutdown timeout of the router service to finish.
///
/// # Errors
///
/// Returns an error if the asset bundle is missing or unreadable, if binding
/// the listener fails, or if accepting a connection fails.
pub async fn serve(config: &Config, services: Services) -> io::Result<()> {
    let feed_registry = Arc::new(FeedRegistry::new());

    // Both fallible steps run before the poll task starts. Dropping a join
    // handle detaches the task rather than stopping it, so an early return
    // between the spawn and the abort strands a task that then polls forever.
    let router = router::build(
        config.assets.as_deref(),
        services.clone(),
        Arc::clone(&feed_registry),
    )?;
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;

    let polling = tokio::spawn(registry::poll(
        feed_registry,
        services.db,
        services.feeds,
        services.clock,
        config.poll_interval,
    ));

    let served = topcoat::serve(listener, router).await;
    polling.abort();

    served
}
