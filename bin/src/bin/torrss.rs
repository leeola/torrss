use std::panic;
use std::time::Duration;
use std::{fs, io, io::ErrorKind, path::Path, path::PathBuf, sync::Arc};

use clap::{ArgAction, Parser, ValueEnum};
use sqlx::SqlitePool;
use torrss::clock::SystemClock;
use torrss::config::ConfigFile;
use torrss::download::HttpDownloader;
use torrss::feed::HttpFeedSource;
use torrss::server::{self, Config};
use torrss::services::Services;
use torrss::store;
use torrss::torrent::Qbit;
use torrss::torrent::store::ClientStore;
use tracing::error;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::fmt::format::FmtSpan;

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
    #[arg(short, long, env = "PORT", default_value_t = server::DEFAULT_PORT)]
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

    /// TOML file declaring feeds and the qBittorrent connection.
    ///
    /// Repeatable, and later files win field by field, so a file outside the
    /// Nix store supplies a password to a connection a store-side file
    /// names. The environment variable names one file.
    #[arg(
        long,
        env = "TORRSS_CONFIG",
        value_name = "FILE",
        action = ArgAction::Append
    )]
    config: Vec<PathBuf>,

    /// Seconds to wait between two passes of the feed poll and the library scan.
    #[arg(
        long,
        env = "TORRSS_POLL_INTERVAL",
        value_name = "SECONDS",
        default_value_t = 900
    )]
    poll_interval: u64,

    /// Log filter directives, such as `info,torrss=debug,sqlx=debug`.
    #[arg(
        long,
        env = "TORRSS_LOG",
        value_name = "DIRECTIVES",
        default_value = "info"
    )]
    log: String,

    /// Shape of each log line.
    #[arg(
        long,
        env = "TORRSS_LOG_FORMAT",
        value_name = "FORMAT",
        value_enum,
        default_value = "text"
    )]
    log_format: LogFormat,
}

/// How each log line is written.
#[derive(Clone, Copy, ValueEnum)]
enum LogFormat {
    /// One line per event, for a person reading a terminal.
    Text,

    /// One JSON object per event, for a collector to parse.
    ///
    /// The object carries the stack of open spans, so a line written
    /// inside a feed check names the check it belongs to.
    Json,
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

/// Installs the subscriber every log line goes through.
///
/// Both formats write when a span closes rather than when it opens. A closed
/// span carries its fields and the time it was busy, so one request reads as
/// one line instead of as a pair to match up.
///
/// # Errors
///
/// Returns [`ErrorKind::InvalidInput`] when `filter` is not a valid set of
/// directives, because that is a mistake in the command line rather than a
/// condition the program recovers from.
///
/// # Panics
///
/// Panics when a subscriber is already installed. The subscriber is global
/// to the process, so this runs once, from `main`.
fn init_tracing(filter: &str, format: LogFormat) -> io::Result<()> {
    let filter = EnvFilter::try_new(filter)
        .map_err(|error| io::Error::new(ErrorKind::InvalidInput, error))?;

    // `init` also installs the `log` bridge, so the lines reqwest and hyper
    // write through `log` arrive at this subscriber too.
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_span_events(FmtSpan::CLOSE);

    match format {
        LogFormat::Text => builder.init(),
        LogFormat::Json => builder
            .json()
            .with_span_list(true)
            .with_current_span(true)
            .init(),
    }

    Ok(())
}

/// Sends every panic to the log instead of to stderr.
///
/// topcoat turns a handler panic into a 500 and drops the payload, so without
/// this the message reaches only stderr, outside the one stream an operator
/// reads.
///
/// The hook runs on the panicking thread while its spans are still entered,
/// so a panic inside a request carries that request's fields.
///
/// This replaces the default hook, and with it the backtrace that
/// `RUST_BACKTRACE` prints. The location takes its place, which names the
/// file and line without the frames.
fn log_panics() {
    panic::set_hook(Box::new(|info| {
        let location = info.location().map(ToString::to_string).unwrap_or_default();

        error!(
            panic.message = info.payload_as_str().unwrap_or("non-string payload"),
            panic.location = %location,
            "panic"
        );
    }));
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

    for path in &cli.config {
        // The message names the file. A complaint about an unknown key or a
        // bad URL says nothing when several files are in play.
        apply_config(&db, path)
            .await
            .map_err(|error| io::Error::other(format!("{}: {error}", path.display())))?;
    }

    let client = ClientStore::new(db.clone())
        .load()
        .await
        .map_err(io::Error::other)?;

    Ok(Services {
        feeds: Arc::new(HttpFeedSource::new()),
        downloads: Arc::new(HttpDownloader::new()),
        torrents: Arc::new(Qbit::new(client.url, &client.username, &client.password)),
        clock: Arc::new(SystemClock),
        db,
    })
}

/// Reads one configuration file and writes its declarations into the stores.
async fn apply_config(pool: &SqlitePool, path: &Path) -> io::Result<()> {
    let text = fs::read_to_string(path)?;
    let config = ConfigFile::parse(&text).map_err(io::Error::other)?;

    config.apply(pool).await.map_err(io::Error::other)
}

#[tokio::main]
async fn main() -> io::Result<()> {
    let cli = Cli::parse();
    init_tracing(&cli.log, cli.log_format)?;
    log_panics();

    let services = services(&cli).await?;

    server::serve(&cli.into(), services).await
}
