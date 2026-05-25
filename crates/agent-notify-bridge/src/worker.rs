use crate::settings::BridgeConfig;
use crate::uhk::DisplayAdapter;
use crate::url::{redacted_url, websocket_url};
use agent_notify_core::{
    AgentEventInput, AgentState, BridgeClientMessage, BridgeServerMessage, clear_macro_command,
    local_hostname as detect_local_hostname, macro_command_for_event,
};
use anyhow::Context;
use futures_util::{SinkExt, StreamExt};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, SystemTime},
};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tracing::{info, warn};

const RECONNECT_BASE: Duration = Duration::from_millis(500);
const RECONNECT_MAX: Duration = Duration::from_secs(30);
/// A session that stayed connected at least this long counts as healthy, so a
/// drop afterwards reconnects promptly. Shorter sessions are treated like a
/// failed attempt and back off, to avoid hammering a server that accepts the
/// handshake and then immediately closes.
const STABLE_SESSION: Duration = Duration::from_secs(10);

#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum BridgeCommand {
    Test,
    Dismiss,
    SetPaused(bool),
    Reconnect,
    Quit,
}

#[derive(Debug, Clone)]
pub struct BridgeRuntimeState {
    /// Source of truth for pause state, owned by the bridge. The server only
    /// mirrors this for the lifetime of a socket (via Status messages) so it
    /// can skip broadcasts; it never originates a pause.
    pub paused: Arc<AtomicBool>,
}

impl BridgeRuntimeState {
    pub fn new() -> Self {
        Self {
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum BridgeExit {
    Disconnected,
    Reconnect,
    Quit,
}

#[cfg(not(windows))]
pub fn run_console(config: BridgeConfig) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let (tx, rx) = mpsc::unbounded_channel();
    // Without a tray, Ctrl-C is the only way to ask for a clean shutdown.
    runtime.spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            let _ = tx.send(BridgeCommand::Quit);
        }
    });
    let state = BridgeRuntimeState::new();
    runtime.block_on(run_bridge_worker(config, state, rx))
}

pub async fn run_bridge_worker(
    config: BridgeConfig,
    state: BridgeRuntimeState,
    mut commands: mpsc::UnboundedReceiver<BridgeCommand>,
) -> anyhow::Result<()> {
    let mut failures: u32 = 0;
    loop {
        if failures > 0 {
            let delay = reconnect_delay(failures);
            warn!(?delay, attempt = failures, "waiting before reconnect");
            tokio::time::sleep(delay).await;
        }

        let started = tokio::time::Instant::now();
        match bridge_session(&config, &state, &mut commands).await {
            Ok(BridgeExit::Quit) => return Ok(()),
            // User asked to reconnect: retry now, regardless of session length.
            Ok(BridgeExit::Reconnect) => failures = 0,
            // A drop after a healthy session reconnects promptly; a drop right
            // after connecting backs off so a flapping server isn't hammered.
            Ok(BridgeExit::Disconnected) => {
                warn!("bridge websocket disconnected");
                failures = next_failures(failures, started.elapsed());
            }
            Err(err) => {
                warn!(?err, "bridge session failed");
                failures = failures.saturating_add(1);
            }
        }
    }
}

fn next_failures(failures: u32, session: Duration) -> u32 {
    if session >= STABLE_SESSION {
        0
    } else {
        failures.saturating_add(1)
    }
}

/// Exponential backoff with additive jitter, capped at `RECONNECT_MAX`.
fn reconnect_delay(failures: u32) -> Duration {
    let exp = failures.saturating_sub(1).min(16);
    let base = RECONNECT_BASE
        .checked_mul(1u32 << exp)
        .unwrap_or(RECONNECT_MAX)
        .min(RECONNECT_MAX);
    base + jitter(RECONNECT_BASE)
}

/// Cheap, dependency-free jitter derived from the wall clock. Good enough to
/// desynchronize a fleet of bridges reconnecting after a server restart.
fn jitter(span: Duration) -> Duration {
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.subsec_nanos())
        .unwrap_or(0);
    let span_nanos = span.as_nanos().max(1) as u64;
    Duration::from_nanos(u64::from(nanos) % span_nanos)
}

async fn bridge_session(
    config: &BridgeConfig,
    state: &BridgeRuntimeState,
    commands: &mut mpsc::UnboundedReceiver<BridgeCommand>,
) -> anyhow::Result<BridgeExit> {
    let url = websocket_url(&config.server_url, &config.token)?;
    let display = DisplayAdapter::new(config.mock_display);
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .context("failed to connect websocket")?;
    info!(url = %redacted_url(&url), "connected to agent-notify server");

    send_status(&mut ws, config, &display, state, None).await?;
    ws.send(Message::Text(
        serde_json::to_string(&BridgeClientMessage::RequestLatest)?.into(),
    ))
    .await?;

    let mut heartbeat = tokio::time::interval(Duration::from_secs(5));
    let mut last_display: Option<String> = None;

    loop {
        tokio::select! {
            _ = heartbeat.tick() => {
                send_status(&mut ws, config, &display, state, last_display.clone()).await?;
            }
            command = commands.recv() => {
                match command {
                    Some(BridgeCommand::Quit) => return Ok(BridgeExit::Quit),
                    Some(BridgeCommand::Reconnect) => return Ok(BridgeExit::Reconnect),
                    Some(BridgeCommand::SetPaused(paused)) => {
                        state.paused.store(paused, Ordering::Relaxed);
                        send_status(&mut ws, config, &display, state, last_display.clone()).await?;
                    }
                    Some(BridgeCommand::Test) => {
                        let command = test_macro_command(config)?;
                        display.display_macro_command(&command)?;
                        last_display = Some(command);
                        send_status(&mut ws, config, &display, state, last_display.clone()).await?;
                    }
                    Some(BridgeCommand::Dismiss) => {
                        ws.send(Message::Text(
                            serde_json::to_string(&BridgeClientMessage::DismissLatest)?.into(),
                        ))
                        .await?;
                        clear_display(&display, &mut last_display, "tray");
                        send_status(&mut ws, config, &display, state, last_display.clone()).await?;
                    }
                    None => return Ok(BridgeExit::Quit),
                }
            }
            message = ws.next() => {
                let Some(message) = message else {
                    return Ok(BridgeExit::Disconnected);
                };
                match message? {
                    Message::Text(text) => {
                        let message: BridgeServerMessage = serde_json::from_str(&text)?;
                        match message {
                            BridgeServerMessage::Event { event } => {
                                if state.paused.load(Ordering::Relaxed) {
                                    continue;
                                }
                                let command = macro_command_for_event(&event)?;
                                if let Err(err) = display.display_macro_command(&command) {
                                    warn!(?err, %command, "failed to update UHK display");
                                    continue;
                                }
                                last_display = Some(command);
                            }
                            BridgeServerMessage::Clear { reason } => {
                                info!(%reason, "clear requested");
                                clear_display(&display, &mut last_display, &reason);
                            }
                        }
                    }
                    Message::Ping(payload) => ws.send(Message::Pong(payload)).await?,
                    Message::Close(_) => return Ok(BridgeExit::Disconnected),
                    _ => {}
                }
            }
        }
    }
}

/// Build the test notification through the same path real events take, so any
/// macro-formatting regression surfaces from the tray Test action too.
fn test_macro_command(config: &BridgeConfig) -> anyhow::Result<String> {
    let event = AgentEventInput {
        agent: "agent-notify".to_string(),
        host: config.hostname.clone().unwrap_or_else(local_hostname),
        repo: None,
        state: AgentState::Done,
        summary: Some("test notification".to_string()),
        priority: None,
        ttl_seconds: Some(60),
        run_id: None,
    }
    .into_event()?;
    Ok(macro_command_for_event(&event)?)
}

fn clear_display(display: &DisplayAdapter, last_display: &mut Option<String>, reason: &str) {
    let command = clear_macro_command();
    if let Err(err) = display.display_macro_command(command) {
        warn!(?err, %command, %reason, "failed to clear UHK display");
        return;
    }

    *last_display = None;
}

async fn send_status(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    config: &BridgeConfig,
    display: &DisplayAdapter,
    state: &BridgeRuntimeState,
    last_display: Option<String>,
) -> anyhow::Result<()> {
    let status = agent_notify_core::BridgeStatus {
        host: config.hostname.clone().unwrap_or_else(local_hostname),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        keyboard_present: display.keyboard_present(),
        paused: state.paused.load(Ordering::Relaxed),
        last_display,
    };
    ws.send(Message::Text(
        serde_json::to_string(&BridgeClientMessage::Status { status })?.into(),
    ))
    .await?;
    Ok(())
}

pub fn local_hostname() -> String {
    detect_local_hostname().unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_failures_backs_off_on_quick_drop_and_resets_when_stable() {
        // Immediate close after the handshake escalates the backoff.
        assert_eq!(next_failures(0, Duration::from_millis(50)), 1);
        assert_eq!(next_failures(3, Duration::from_millis(50)), 4);
        // A session that stayed up long enough is treated as healthy.
        assert_eq!(next_failures(3, STABLE_SESSION), 0);
        assert_eq!(next_failures(3, Duration::from_secs(120)), 0);
    }

    #[test]
    fn reconnect_delay_grows_and_caps() {
        let first = reconnect_delay(1);
        let later = reconnect_delay(5);
        assert!(first < later || later >= RECONNECT_MAX);
        // Even with maximum jitter the cap is respected.
        assert!(reconnect_delay(100) <= RECONNECT_MAX + RECONNECT_BASE);
    }

    #[test]
    fn test_macro_command_fits_uhk_payload() {
        let config = BridgeConfig {
            server_url: "http://127.0.0.1:8787".to_string(),
            token: "change-me".to_string(),
            hostname: Some("workstation".to_string()),
            mock_display: true,
        };
        let command = test_macro_command(&config).unwrap();
        assert!(command.len() <= agent_notify_core::UHK_MAX_MACRO_COMMAND_BYTES);
    }
}
