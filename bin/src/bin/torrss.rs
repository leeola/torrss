use std::{io, path::PathBuf};

use clap::Parser;
use torrss::server::{self, Config};

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

#[tokio::main]
async fn main() -> io::Result<()> {
    server::serve(&Cli::parse().into()).await
}
