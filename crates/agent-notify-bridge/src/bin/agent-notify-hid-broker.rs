use agent_notify_bridge::hid_broker::{MockHidBackend, RealHidBackend, run_stdio};
use clap::Parser;
use std::io::{BufReader, BufWriter};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Parser)]
struct Args {
    /// Run the broker over stdin/stdout. This is the default transport used by
    /// agent-notify-bridge; stdout is reserved for IPC responses.
    #[arg(long, default_value_t = true)]
    stdio: bool,
    /// Log generated UHK commands instead of touching HID hardware.
    #[arg(long)]
    mock_hid: bool,
}

fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .with_writer(std::io::stderr)
        .init();

    let args = Args::parse();
    if !args.stdio {
        anyhow::bail!("only --stdio transport is supported");
    }

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut reader = BufReader::new(stdin.lock());
    let mut writer = BufWriter::new(stdout.lock());

    if args.mock_hid {
        let mut backend = MockHidBackend::default();
        run_stdio(&mut backend, &mut reader, &mut writer)
    } else {
        let mut backend = RealHidBackend;
        run_stdio(&mut backend, &mut reader, &mut writer)
    }
}
