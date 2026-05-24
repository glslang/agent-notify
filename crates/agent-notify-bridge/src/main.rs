mod settings;
mod uhk;

use agent_notify_core::{
    BridgeClientMessage, BridgeServerMessage, BridgeStatus, clear_macro_command,
    local_hostname as detect_local_hostname, macro_command_for_event,
};
use anyhow::Context;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use settings::{BridgeConfig, load_config};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_tungstenite::{connect_async, tungstenite::Message};
#[cfg(windows)]
use tracing::error;
use tracing::{info, warn};
use tracing_subscriber::{EnvFilter, fmt};
use uhk::DisplayAdapter;

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

#[derive(Debug)]
#[cfg_attr(not(windows), allow(dead_code))]
enum BridgeCommand {
    Test,
    Dismiss,
    SetPaused(bool),
    Reconnect,
    Quit,
}

#[derive(Debug, Clone)]
struct BridgeRuntimeState {
    paused: Arc<AtomicBool>,
}

fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
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

    #[cfg(windows)]
    {
        run_windows_tray(config)
    }

    #[cfg(not(windows))]
    {
        run_console(config)
    }
}

#[cfg(not(windows))]
fn run_console(config: BridgeConfig) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Runtime::new()?;
    let (_tx, rx) = mpsc::unbounded_channel();
    let state = BridgeRuntimeState {
        paused: Arc::new(AtomicBool::new(false)),
    };
    runtime.block_on(run_bridge_worker(config, state, rx))
}

#[cfg(windows)]
fn run_windows_tray(config: BridgeConfig) -> anyhow::Result<()> {
    use tao::event::Event;
    use tao::event_loop::{ControlFlow, EventLoopBuilder};
    use tray_icon::{
        Icon, TrayIconBuilder,
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    };

    let event_loop = EventLoopBuilder::new().build();
    let menu = Menu::new();
    let pause_item = CheckMenuItem::new("Pause notifications", true, false, None);
    let test_item = MenuItem::new("Send test notification", true, None);
    let dismiss_item = MenuItem::new("Dismiss notification", true, None);
    let reconnect_item = MenuItem::new("Reconnect", true, None);
    let quit_item = MenuItem::new("Quit", true, None);
    menu.append(&pause_item)?;
    menu.append(&test_item)?;
    menu.append(&dismiss_item)?;
    menu.append(&reconnect_item)?;
    menu.append(&PredefinedMenuItem::separator())?;
    menu.append(&quit_item)?;

    let icon = Icon::from_rgba(make_icon_rgba(), 16, 16)?;
    let _tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_tooltip("Agent Notify")
        .with_icon(icon)
        .build()?;

    let (command_tx, command_rx) = mpsc::unbounded_channel();
    let state = BridgeRuntimeState {
        paused: Arc::new(AtomicBool::new(false)),
    };
    let worker_state = state.clone();
    std::thread::spawn(move || {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(runtime) => runtime,
            Err(err) => {
                error!(?err, "failed to create tokio runtime");
                return;
            }
        };
        if let Err(err) = runtime.block_on(run_bridge_worker(config, worker_state, command_rx)) {
            error!(?err, "bridge worker stopped");
        }
    });

    let menu_rx = MenuEvent::receiver();
    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        while let Ok(event) = menu_rx.try_recv() {
            if event.id == quit_item.id() {
                let _ = command_tx.send(BridgeCommand::Quit);
                *control_flow = ControlFlow::Exit;
            } else if event.id == test_item.id() {
                let _ = command_tx.send(BridgeCommand::Test);
            } else if event.id == dismiss_item.id() {
                let _ = command_tx.send(BridgeCommand::Dismiss);
            } else if event.id == reconnect_item.id() {
                let _ = command_tx.send(BridgeCommand::Reconnect);
            } else if event.id == pause_item.id() {
                let paused = pause_item.is_checked();
                state.paused.store(paused, Ordering::Relaxed);
                let _ = command_tx.send(BridgeCommand::SetPaused(paused));
            }
        }

        if let Event::LoopDestroyed = event {
            let _ = command_tx.send(BridgeCommand::Quit);
        }
    });
}

#[cfg(windows)]
fn make_icon_rgba() -> Vec<u8> {
    let mut data = Vec::with_capacity(16 * 16 * 4);
    for y in 0..16 {
        for x in 0..16 {
            let active = (3..=12).contains(&x) && (3..=12).contains(&y);
            if active {
                data.extend_from_slice(&[0x27, 0xae, 0x60, 0xff]);
            } else {
                data.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    data
}

async fn run_bridge_worker(
    config: BridgeConfig,
    state: BridgeRuntimeState,
    mut commands: mpsc::UnboundedReceiver<BridgeCommand>,
) -> anyhow::Result<()> {
    let mut reconnect_now = true;
    loop {
        if reconnect_now {
            reconnect_now = false;
        } else {
            tokio::time::sleep(Duration::from_secs(3)).await;
        }

        let result = bridge_session(&config, &state, &mut commands).await;
        match result {
            Ok(BridgeExit::Quit) => return Ok(()),
            Ok(BridgeExit::Reconnect) => reconnect_now = true,
            Ok(BridgeExit::Disconnected) => warn!("bridge websocket disconnected"),
            Err(err) => warn!(?err, "bridge session failed"),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
enum BridgeExit {
    Disconnected,
    Reconnect,
    Quit,
}

async fn bridge_session(
    config: &BridgeConfig,
    state: &BridgeRuntimeState,
    commands: &mut mpsc::UnboundedReceiver<BridgeCommand>,
) -> anyhow::Result<BridgeExit> {
    let url = websocket_url(&config.server_url, &config.token)?;
    let display = DisplayAdapter::new(config.mock_display);
    let (mut ws, _) = connect_async(&url)
        .await
        .context("failed to connect websocket")?;
    info!(%url, "connected to agent-notify server");

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
                        let command = "notify \"agent notify test\"";
                        display.display_macro_command(command)?;
                        last_display = Some(command.to_string());
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
    let status = BridgeStatus {
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

fn websocket_url(server_url: &str, token: &str) -> anyhow::Result<String> {
    let mut base = server_url.trim_end_matches('/').to_string();
    if let Some(rest) = base.strip_prefix("https://") {
        base = format!("wss://{rest}");
    } else if let Some(rest) = base.strip_prefix("http://") {
        base = format!("ws://{rest}");
    }
    Ok(format!(
        "{base}/v1/bridge/ws?token={}",
        encode_query_component(token)
    ))
}

fn encode_query_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

fn local_hostname() -> String {
    detect_local_hostname().unwrap_or_else(|| "unknown".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn websocket_url_converts_http_to_ws() {
        let url = websocket_url("http://127.0.0.1:8787/", "change-me").unwrap();

        assert_eq!(url, "ws://127.0.0.1:8787/v1/bridge/ws?token=change-me");
    }

    #[test]
    fn websocket_url_encodes_token_query_value() {
        let url = websocket_url("https://agent.example", "a token&with=query").unwrap();

        assert_eq!(
            url,
            "wss://agent.example/v1/bridge/ws?token=a%20token%26with%3Dquery"
        );
    }
}
