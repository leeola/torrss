use std::time::Duration;
use std::{io, path::PathBuf, sync::Arc};

use clap::Parser;
use sqlx::SqlitePool;
use torrss::clock::SystemClock;
use torrss::feed::HttpFeedSource;
use torrss::server::{self, Config};
use torrss::services::Services;
use torrss::store;
use torrss::torrent::Qbit;
use url::Url;

// `topcoat dev` spawns the built executable with no arguments and offers no
// way to supply any, so serving is what a bare invocation does. The options
// stand alone rather than sitting under a subcommand that could never run.
#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Host to bind the listener to.
    #[arg(long, env = "HOST", default_value = server::DEFAULT_HOST)]
    host: String,

    /// Port to bind the listener to.
    #[arg(long, env = "PORT", default_value_t = server::DEFAULT_PORT)]
    port: u16,

    /// Asset bundle directory, defaulting to `assets` beside the executable.
    #[arg(long, env = "TORRSS_ASSETS", value_name = "DIR")]
    assets: Option<PathBuf>,

    /// SQLite database file, created if it does not exist.
    #[arg(
        long,
        env = "TORRSS_DB",
        value_name = "FILE",
        default_value = "torrss.db"
    )]
    db: PathBuf,

    /// qBittorrent web interface address.
    #[arg(
        long,
        env = "QBIT_URL",
        value_name = "URL",
        default_value = "http://127.0.0.1:8080"
    )]
    qbit_url: Url,

    /// qBittorrent account name.
    #[arg(
        long,
        env = "QBIT_USERNAME",
        value_name = "NAME",
        default_value = "admin"
    )]
    qbit_username: String,

    /// qBittorrent account password.
    #[arg(
        long,
        env = "QBIT_PASSWORD",
        value_name = "PASSWORD",
        default_value = ""
    )]
    qbit_password: String,

    /// Seconds to wait between two passes over every feed.
    #[arg(
        long,
        env = "TORRSS_POLL_INTERVAL",
        value_name = "SECONDS",
        default_value_t = 900
    )]
    poll_interval: u64,
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Self {
            host: cli.host,
            port: cli.port,
            assets: cli.assets,
            poll_interval: Duration::from_secs(cli.poll_interval),
        }
    }
}

/// Opens everything the application talks to the outside world through.
///
/// `mode=rwc` creates the database file and the migration builds its schema,
/// so a first run needs no setup step.
///
/// Only the database is contacted here. A wrong qBittorrent address or
/// password therefore starts the server anyway, and shows up on the admin
/// page as a failed connection check.
async fn services(cli: &Cli) -> io::Result<Services> {
    let db = SqlitePool::connect(&format!("sqlite://{}?mode=rwc", cli.db.display()))
        .await
        .map_err(io::Error::other)?;

    store::migrate(&db).await.map_err(io::Error::other)?;

    Ok(Services {
        feeds: Arc::new(HttpFeedSource::new()),
        torrents: Arc::new(Qbit::new(
            cli.qbit_url.clone(),
            &cli.qbit_username,
            &cli.qbit_password,
        )),
        clock: Arc::new(SystemClock),
        db,
    })
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let services = services(&cli).await?;

    server::serve(&cli.into(), services).await
}
