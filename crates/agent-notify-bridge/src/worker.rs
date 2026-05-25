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

/// The tray icon the bridge should currently show. Derived from the latest
/// event's `AgentState` plus the bridge-level pause flag; not part of the wire
/// protocol, so it lives here rather than in `agent-notify-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum IconState {
    Idle,
    Running,
    Waiting,
    Done,
    Failed,
    Paused,
}

/// Callback the worker uses to report icon-state changes to the UI. The Windows
/// tray forwards these to the winit event loop; the console build ignores them.
pub type IconSink = Box<dyn Fn(IconState) + Send>;

fn icon_for_state(state: AgentState) -> IconState {
    match state {
        AgentState::Running => IconState::Running,
        AgentState::WaitingInput => IconState::Waiting,
        AgentState::Done => IconState::Done,
        AgentState::Failed => IconState::Failed,
    }
}

/// Pause takes precedence over any event; otherwise the latest event's icon, or
/// `Idle` when there is no active event.
fn effective_icon(current: Option<AgentState>, paused: bool) -> IconState {
    if paused {
        IconState::Paused
    } else {
        current.map(icon_for_state).unwrap_or(IconState::Idle)
    }
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
    // No tray on this path, so icon-state updates have nowhere to go.
    let icon_sink: IconSink = Box::new(|_| {});
    runtime.block_on(run_bridge_worker(config, state, rx, icon_sink))
}

pub async fn run_bridge_worker(
    config: BridgeConfig,
    state: BridgeRuntimeState,
    mut commands: mpsc::UnboundedReceiver<BridgeCommand>,
    icon_sink: IconSink,
) -> anyhow::Result<()> {
    let mut failures: u32 = 0;
    loop {
        if failures > 0 {
            let delay = reconnect_delay(failures);
            warn!(?delay, attempt = failures, "waiting before reconnect");
            tokio::time::sleep(delay).await;
        }

        let started = tokio::time::Instant::now();
        match bridge_session(&config, &state, &mut commands, &icon_sink).await {
            Ok(BridgeExit::Quit) => return Ok(()),
            // User asked to reconnect: retry now, regardless of session length.
            Ok(BridgeExit::Reconnect) => failures = 0,
            // A drop after a healthy session reconnects promptly; a drop right
            // after connecting backs off so a flapping server isn't hammered.
            Ok(BridgeExit::Disconnected) => {
                warn!("bridge websocket disconnected");
                icon_sink(IconState::Idle);
                failures = next_failures(failures, started.elapsed());
            }
            Err(err) => {
                warn!(?err, "bridge session failed");
                icon_sink(IconState::Idle);
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
    icon_sink: &IconSink,
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
    // Latest event state we are showing; drives the tray icon together with the
    // pause flag. The server re-sends the latest event right after connecting,
    // so starting from `None` (Idle) self-corrects within the first round-trip.
    let mut current_state: Option<AgentState> = None;
    icon_sink(effective_icon(
        current_state,
        state.paused.load(Ordering::Relaxed),
    ));

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
                        icon_sink(effective_icon(current_state, paused));
                        send_status(&mut ws, config, &display, state, last_display.clone()).await?;
                    }
                    Some(BridgeCommand::Test) => {
                        let command = test_macro_command(config)?;
                        display.display_macro_command(&command)?;
                        last_display = Some(command);
                        // The test notification is a synthetic Done event.
                        current_state = Some(AgentState::Done);
                        icon_sink(effective_icon(current_state, state.paused.load(Ordering::Relaxed)));
                        send_status(&mut ws, config, &display, state, last_display.clone()).await?;
                    }
                    Some(BridgeCommand::Dismiss) => {
                        ws.send(Message::Text(
                            serde_json::to_string(&BridgeClientMessage::DismissLatest)?.into(),
                        ))
                        .await?;
                        clear_display(&display, &mut last_display, "tray");
                        current_state = None;
                        icon_sink(effective_icon(current_state, state.paused.load(Ordering::Relaxed)));
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
                                current_state = Some(event.state);
                                icon_sink(effective_icon(current_state, false));
                            }
                            BridgeServerMessage::Clear { reason } => {
                                info!(%reason, "clear requested");
                                clear_display(&display, &mut last_display, &reason);
                                current_state = None;
                                icon_sink(effective_icon(
                                    current_state,
                                    state.paused.load(Ordering::Relaxed),
                                ));
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
    fn effective_icon_maps_states_and_pause_wins() {
        // Each agent state maps to its icon.
        assert_eq!(
            effective_icon(Some(AgentState::Running), false),
            IconState::Running
        );
        assert_eq!(
            effective_icon(Some(AgentState::WaitingInput), false),
            IconState::Waiting
        );
        assert_eq!(
            effective_icon(Some(AgentState::Done), false),
            IconState::Done
        );
        assert_eq!(
            effective_icon(Some(AgentState::Failed), false),
            IconState::Failed
        );
        // No active event is idle.
        assert_eq!(effective_icon(None, false), IconState::Idle);
        // Pause takes precedence over any event, and over idle.
        assert_eq!(
            effective_icon(Some(AgentState::Failed), true),
            IconState::Paused
        );
        assert_eq!(effective_icon(None, true), IconState::Paused);
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
