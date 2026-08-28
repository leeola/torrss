use std::{io, path::PathBuf, sync::Arc};

use clap::Parser;
use sqlx::SqlitePool;
use torrss::clock::SystemClock;
use torrss::feed::HttpFeedSource;
use torrss::server::{self, Config};
use torrss::services::Services;

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
}

impl From<Cli> for Config {
    fn from(cli: Cli) -> Self {
        Self {
            host: cli.host,
            port: cli.port,
            assets: cli.assets,
        }
    }
}

/// Opens everything the application talks to the outside world through.
///
/// `mode=rwc` creates the database file, so a first run needs no setup step.
async fn services(cli: &Cli) -> io::Result<Services> {
    let db = SqlitePool::connect(&format!("sqlite://{}?mode=rwc", cli.db.display()))
        .await
        .map_err(io::Error::other)?;

    Ok(Services {
        feeds: Arc::new(HttpFeedSource::new()),
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
