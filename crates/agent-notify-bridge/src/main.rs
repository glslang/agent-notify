#[cfg(windows)]
mod icons;
mod settings;
#[cfg(windows)]
mod tray;
mod uhk;
mod url;
mod worker;

use clap::Parser;
use settings::{load_config, validate};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long)]
    config: Option<std::path::PathBuf>,
    #[arg(long)]
    server: Option<String>,
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    mock_display: bool,
}

fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    // Layer precedence: CLI flags override the file/env base, then the merged
    // result is validated so a token from any layer (including --token alone)
    // is accepted.
    let mut config = load_config(args.config.as_deref())?;
    if let Some(server) = args.server {
        config.server_url = server;
    }
    if let Some(token) = args.token {
        config.token = token;
    }
    if args.mock_display {
        config.mock_display = true;
    }
    validate(&config)?;

    #[cfg(windows)]
    {
        tray::run_windows_tray(config)
    }

    #[cfg(not(windows))]
    {
        worker::run_console(config)
    }
}
