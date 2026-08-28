use std::{io, path::PathBuf};

use tokio::net::TcpListener;

use super::router;

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
}

/// Serves the web application until the process receives Ctrl+C or `SIGTERM`.
///
/// In-flight requests get the shutdown timeout of the router service to
/// finish.
///
/// # Errors
///
/// Returns an error if the asset bundle is missing or unreadable, if binding
/// the listener fails, or if accepting a connection fails.
pub async fn serve(config: &Config) -> io::Result<()> {
    let router = router::build(config.assets.as_deref())?;
    let listener = TcpListener::bind((config.host.as_str(), config.port)).await?;

    topcoat::serve(listener, router).await
}
