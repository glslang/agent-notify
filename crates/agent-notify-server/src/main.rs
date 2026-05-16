use agent_notify_core::{
    AgentEvent, AgentEventInput, BridgeClientMessage, BridgeServerMessage, HealthResponse,
    choose_latest,
};
use anyhow::Context;
use axum::{
    Json, Router,
    extract::{
        Query, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::{net::SocketAddr, sync::Arc};
use tokio::sync::{Mutex, broadcast};
use tower_http::trace::TraceLayer;
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Debug, Parser)]
struct Args {
    #[arg(long, env = "AGENT_NOTIFY_BIND", default_value = "0.0.0.0:8787")]
    bind: SocketAddr,
    #[arg(long, env = "AGENT_NOTIFY_TOKEN")]
    token: String,
}

#[derive(Clone)]
struct AppState {
    token: Arc<String>,
    latest: Arc<Mutex<Option<AgentEvent>>>,
    tx: broadcast::Sender<BridgeServerMessage>,
}

#[derive(Debug, Deserialize)]
struct WsAuth {
    token: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let args = Args::parse();
    let (tx, _) = broadcast::channel(128);
    let state = AppState {
        token: Arc::new(args.token),
        latest: Arc::new(Mutex::new(None)),
        tx,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/events", post(post_event))
        .route("/v1/events/latest", get(get_latest))
        .route("/v1/bridge/ws", get(bridge_ws))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.bind)
        .await
        .with_context(|| format!("failed to bind {}", args.bind))?;
    info!("agent-notify-server listening on {}", args.bind);
    axum::serve(listener, app).await?;
    Ok(())
}

async fn healthz() -> Json<HealthResponse> {
    Json(HealthResponse { status: "ok" })
}

async fn post_event(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(input): Json<AgentEventInput>,
) -> Result<Json<AgentEvent>, AppError> {
    require_auth(&headers, &state.token)?;
    let event = input.into_event().map_err(AppError::BadRequest)?;
    let latest = {
        let mut guard = state.latest.lock().await;
        let chosen = choose_latest(guard.clone(), event.clone());
        *guard = Some(chosen.clone());
        chosen
    };

    state
        .tx
        .send(BridgeServerMessage::Event {
            event: latest.clone(),
        })
        .ok();

    Ok(Json(event))
}

async fn get_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<AgentEvent>>, AppError> {
    require_auth(&headers, &state.token)?;
    let latest = live_latest(&state).await;
    Ok(Json(latest))
}

async fn bridge_ws(
    State(state): State<AppState>,
    Query(auth): Query<WsAuth>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    if auth.token.as_deref() != Some(state.token.as_str()) {
        require_auth(&headers, &state.token)?;
    }

    Ok(ws.on_upgrade(move |socket| handle_bridge_socket(state, socket)))
}

async fn handle_bridge_socket(state: AppState, socket: WebSocket) {
    let mut rx = state.tx.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let mut keyboard_present = false;
    let mut paused = false;

    if let Some(event) = live_latest(&state).await {
        send_server_message(&mut sender, &BridgeServerMessage::Event { event }).await;
    }

    loop {
        tokio::select! {
            received = receiver.next() => {
                let Some(received) = received else {
                    break;
                };

                match received {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<BridgeClientMessage>(&text) {
                            Ok(BridgeClientMessage::Status { status }) => {
                                keyboard_present = status.keyboard_present;
                                paused = status.paused;
                                debug!(host = %status.host, keyboard_present, paused, "bridge status");
                            }
                            Ok(BridgeClientMessage::RequestLatest) => {
                                if let Some(event) = live_latest(&state).await {
                                    send_server_message(&mut sender, &BridgeServerMessage::Event { event }).await;
                                }
                            }
                            Err(err) => warn!(?err, "invalid bridge message"),
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Ok(Message::Ping(payload)) => {
                        if sender.send(Message::Pong(payload)).await.is_err() {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(err) => {
                        warn!(?err, "websocket receive error");
                        break;
                    }
                }
            }
            broadcast = rx.recv() => {
                match broadcast {
                    Ok(message) if keyboard_present && !paused => {
                        if !send_server_message(&mut sender, &message).await {
                            break;
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        warn!(skipped, "bridge lagged behind events");
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        }
    }
}

async fn send_server_message(
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    message: &BridgeServerMessage,
) -> bool {
    match serde_json::to_string(message) {
        Ok(text) => sender.send(Message::Text(text.into())).await.is_ok(),
        Err(err) => {
            error!(?err, "failed to serialize server message");
            true
        }
    }
}

async fn live_latest(state: &AppState) -> Option<AgentEvent> {
    let mut guard = state.latest.lock().await;
    if guard.as_ref().is_some_and(AgentEvent::is_live) {
        guard.clone()
    } else {
        *guard = None;
        None
    }
}

fn require_auth(headers: &HeaderMap, token: &str) -> Result<(), AppError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(AppError::Unauthorized);
    };
    let Ok(value) = value.to_str() else {
        return Err(AppError::Unauthorized);
    };
    if value == format!("Bearer {token}") {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

enum AppError {
    BadRequest(agent_notify_core::EventError),
    Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::BadRequest(err) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response(),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "error": "unauthorized" })),
            )
                .into_response(),
        }
    }
}
