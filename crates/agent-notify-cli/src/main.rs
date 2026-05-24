use agent_notify_core::{AgentEventInput, AgentState, local_hostname as detect_local_hostname};
use anyhow::{Context, bail};
use clap::{Parser, ValueEnum};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "AGENT_NOTIFY_SERVER")]
    server: String,
    #[arg(long, env = "AGENT_NOTIFY_TOKEN")]
    token: String,
    #[arg(long, env = "AGENT_NOTIFY_AGENT", default_value = "codex")]
    agent: String,
    #[arg(long, env = "AGENT_NOTIFY_HOST")]
    host: Option<String>,
    #[arg(long, env = "AGENT_NOTIFY_REPO")]
    repo: Option<String>,
    #[arg(long, value_enum)]
    state: Option<StateArg>,
    #[arg(long)]
    dismiss: bool,
    #[arg(long)]
    summary: Option<String>,
    #[arg(long)]
    priority: Option<u8>,
    #[arg(long)]
    ttl_seconds: Option<u64>,
    #[arg(long)]
    run_id: Option<String>,
}

#[derive(Debug, Clone, ValueEnum)]
enum StateArg {
    Running,
    WaitingInput,
    Done,
    Failed,
}

impl From<StateArg> for AgentState {
    fn from(value: StateArg) -> Self {
        match value {
            StateArg::Running => AgentState::Running,
            StateArg::WaitingInput => AgentState::WaitingInput,
            StateArg::Done => AgentState::Done,
            StateArg::Failed => AgentState::Failed,
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    if args.dismiss {
        dismiss_latest(&args.server, &args.token).await?;
        return Ok(());
    }

    let host = match args.host {
        Some(host) => host,
        None => local_hostname()?,
    };
    let state = args
        .state
        .context("--state is required unless --dismiss is supplied")?;

    let input = AgentEventInput {
        agent: args.agent,
        host,
        repo: args.repo,
        state: state.into(),
        summary: args.summary,
        priority: args.priority,
        ttl_seconds: args.ttl_seconds,
        run_id: args.run_id,
    };

    let url = format!("{}/v1/events", args.server.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .post(url)
        .bearer_auth(args.token)
        .json(&input)
        .send()
        .await
        .context("failed to post event")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("server returned {status}: {body}");
    }

    Ok(())
}

async fn dismiss_latest(server: &str, token: &str) -> anyhow::Result<()> {
    let url = format!("{}/v1/events/latest", server.trim_end_matches('/'));
    let response = reqwest::Client::new()
        .delete(url)
        .bearer_auth(token)
        .send()
        .await
        .context("failed to dismiss latest event")?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        bail!("server returned {status}: {body}");
    }

    Ok(())
}

fn local_hostname() -> anyhow::Result<String> {
    detect_local_hostname().context(
        "host was not supplied and hostname could not be inferred from environment or system",
    )
}
