use agent_notify_core::{
    AgentEvent, AgentEventInput, BridgeClientMessage, BridgeServerMessage, DismissResponse,
    HealthResponse, choose_latest,
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
use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{Mutex, broadcast};
use tower::limit::GlobalConcurrencyLimitLayer;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};
use tracing::{debug, error, info, warn};
use tracing_subscriber::{EnvFilter, fmt};

/// Maximum accepted JSON body for an event POST. Events are tiny; this just
/// bounds memory from a hostile client.
const MAX_BODY_BYTES: usize = 64 * 1024;
/// Cap on concurrent in-flight requests, which also bounds the number of
/// simultaneously held WebSocket upgrades.
const MAX_CONCURRENT_REQUESTS: usize = 1024;
/// Server-side WebSocket keepalive: ping this often, drop the peer if it has
/// been silent for longer than the idle timeout.
const WS_PING_INTERVAL: Duration = Duration::from_secs(10);
const WS_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

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
    bearer: Arc<String>,
    latest: Arc<Mutex<Option<AgentEvent>>>,
    seq: Arc<AtomicU64>,
    tx: broadcast::Sender<Arc<BridgeServerMessage>>,
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
    let bearer = format!("Bearer {}", args.token);
    let state = AppState {
        token: Arc::new(args.token),
        bearer: Arc::new(bearer),
        latest: Arc::new(Mutex::new(None)),
        seq: Arc::new(AtomicU64::new(0)),
        tx,
    };

    let app = Router::new()
        .route("/healthz", get(healthz))
        .route("/v1/events", post(post_event))
        .route("/v1/events/latest", get(get_latest).delete(delete_latest))
        .route("/v1/bridge/ws", get(bridge_ws))
        // Log only method + path; the WebSocket route carries the token in the
        // query string, which must never reach logs.
        .layer(TraceLayer::new_for_http().make_span_with(
            |request: &axum::http::Request<axum::body::Body>| {
                tracing::info_span!(
                    "request",
                    method = %request.method(),
                    path = %request.uri().path(),
                )
            },
        ))
        .layer(GlobalConcurrencyLimitLayer::new(MAX_CONCURRENT_REQUESTS))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_BYTES))
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
    require_auth(&headers, &state.bearer)?;
    let mut event = input.into_event().map_err(AppError::BadRequest)?;
    event.seq = state.seq.fetch_add(1, Ordering::Relaxed);
    let latest = {
        let mut guard = state.latest.lock().await;
        let chosen = choose_latest(guard.clone(), event);
        *guard = Some(chosen.clone());
        chosen
    };

    state
        .tx
        .send(Arc::new(BridgeServerMessage::Event {
            event: latest.clone(),
        }))
        .ok();

    Ok(Json(latest))
}

async fn get_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<Option<AgentEvent>>, AppError> {
    require_auth(&headers, &state.bearer)?;
    let latest = live_latest(&state).await;
    Ok(Json(latest))
}

async fn delete_latest(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<DismissResponse>, AppError> {
    require_auth(&headers, &state.bearer)?;
    let dismissed = dismiss_latest(&state, "api").await;
    Ok(Json(DismissResponse { dismissed }))
}

async fn bridge_ws(
    State(state): State<AppState>,
    Query(auth): Query<WsAuth>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, AppError> {
    let query_ok = auth
        .token
        .as_deref()
        .is_some_and(|token| constant_time_eq(token.as_bytes(), state.token.as_bytes()));
    if !query_ok {
        require_auth(&headers, &state.bearer)?;
    }

    Ok(ws.on_upgrade(move |socket| handle_bridge_socket(state, socket)))
}

async fn handle_bridge_socket(state: AppState, socket: WebSocket) {
    let mut rx = state.tx.subscribe();
    let (mut sender, mut receiver) = socket.split();
    let mut paused = false;
    let mut keepalive = tokio::time::interval(WS_PING_INTERVAL);
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_seen = tokio::time::Instant::now();

    if let Some(event) = live_latest(&state).await {
        send_server_message(&mut sender, &BridgeServerMessage::Event { event }).await;
    }

    loop {
        tokio::select! {
            _ = keepalive.tick() => {
                if last_seen.elapsed() > WS_IDLE_TIMEOUT {
                    warn!("bridge idle past timeout; dropping connection");
                    break;
                }
                if sender.send(Message::Ping(Vec::new().into())).await.is_err() {
                    break;
                }
            }
            received = receiver.next() => {
                let Some(received) = received else {
                    break;
                };
                last_seen = tokio::time::Instant::now();

                match received {
                    Ok(Message::Text(text)) => {
                        match serde_json::from_str::<BridgeClientMessage>(&text) {
                            Ok(BridgeClientMessage::Status { status }) => {
                                paused = status.paused;
                                debug!(
                                    host = %status.host,
                                    keyboard_present = status.keyboard_present,
                                    paused,
                                    "bridge status"
                                );
                            }
                            Ok(BridgeClientMessage::RequestLatest) => {
                                if let Some(event) = live_latest(&state).await {
                                    send_server_message(&mut sender, &BridgeServerMessage::Event { event }).await;
                                }
                            }
                            Ok(BridgeClientMessage::DismissLatest) => {
                                dismiss_latest(&state, "bridge").await;
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
                    Ok(message) if !paused => {
                        if !send_server_message(&mut sender, message.as_ref()).await {
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

async fn dismiss_latest(state: &AppState, reason: &str) -> bool {
    let dismissed = {
        let mut guard = state.latest.lock().await;
        guard.take().is_some()
    };

    if dismissed {
        state
            .tx
            .send(Arc::new(BridgeServerMessage::Clear {
                reason: reason.to_string(),
            }))
            .ok();
    }

    dismissed
}

fn require_auth(headers: &HeaderMap, bearer: &str) -> Result<(), AppError> {
    let Some(value) = headers.get(axum::http::header::AUTHORIZATION) else {
        return Err(AppError::Unauthorized);
    };
    if constant_time_eq(value.as_bytes(), bearer.as_bytes()) {
        Ok(())
    } else {
        Err(AppError::Unauthorized)
    }
}

/// Length-independent of the comparison but value-constant-time: avoids the
/// early-exit timing signal of a plain `==` on the secret.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[derive(Debug)]
enum AppError {
    BadRequest(agent_notify_core::EventError),
    Unauthorized,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            AppError::BadRequest(err) => (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "code": "invalid_event", "error": err.to_string() })),
            )
                .into_response(),
            AppError::Unauthorized => (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({ "code": "unauthorized", "error": "unauthorized" })),
            )
                .into_response(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_notify_core::AgentState;
    use axum::http::{HeaderValue, header::AUTHORIZATION};

    fn state() -> AppState {
        let (tx, _) = broadcast::channel(16);
        AppState {
            token: Arc::new("secret".to_string()),
            bearer: Arc::new("Bearer secret".to_string()),
            latest: Arc::new(Mutex::new(None)),
            seq: Arc::new(AtomicU64::new(0)),
            tx,
        }
    }

    fn auth_headers() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer secret"));
        headers
    }

    fn input(state: AgentState, host: &str, priority: Option<u8>) -> AgentEventInput {
        AgentEventInput {
            agent: "codex".to_string(),
            host: host.to_string(),
            repo: Some("agent-notify".to_string()),
            state,
            summary: None,
            priority,
            ttl_seconds: Some(60),
            run_id: None,
        }
    }

    #[tokio::test]
    async fn post_event_returns_chosen_latest_event() {
        let state = state();
        let Json(high) = post_event(
            State(state.clone()),
            auth_headers(),
            Json(input(AgentState::WaitingInput, "workstation", Some(90))),
        )
        .await
        .unwrap();

        let Json(response) = post_event(
            State(state),
            auth_headers(),
            Json(input(AgentState::Running, "other-host", Some(20))),
        )
        .await
        .unwrap();

        assert_eq!(response.id, high.id);
        assert_eq!(response.host, high.host);
        assert_eq!(response.state, AgentState::WaitingInput);
    }

    #[tokio::test]
    async fn get_latest_requires_auth() {
        let state = state();
        let result = get_latest(State(state), HeaderMap::new()).await;
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }

    #[tokio::test]
    async fn wrong_token_is_rejected() {
        let state = state();
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer nope"));
        let result = get_latest(State(state), headers).await;
        assert!(matches!(result, Err(AppError::Unauthorized)));
    }

    #[tokio::test]
    async fn get_latest_filters_expired_event() {
        let state = state();
        let mut expired = input(AgentState::Done, "workstation", None)
            .into_event()
            .unwrap();
        expired.expires_at_unix_ms = 1; // far in the past
        *state.latest.lock().await = Some(expired);

        let Json(latest) = get_latest(State(state.clone()), auth_headers())
            .await
            .unwrap();
        assert!(latest.is_none());
        // The expired event should have been cleared from storage.
        assert!(state.latest.lock().await.is_none());
    }

    #[tokio::test]
    async fn delete_latest_reports_whether_something_was_dismissed() {
        let state = state();
        let mut rx = state.tx.subscribe();

        // Nothing stored yet: no dismissal, no broadcast.
        let Json(empty) = delete_latest(State(state.clone()), auth_headers())
            .await
            .unwrap();
        assert!(!empty.dismissed);
        assert!(rx.try_recv().is_err());

        // Store an event, then dismiss it.
        let _ = post_event(
            State(state.clone()),
            auth_headers(),
            Json(input(AgentState::Done, "workstation", None)),
        )
        .await
        .unwrap();
        let _ = rx.try_recv(); // drain the Event broadcast

        let Json(dismissed) = delete_latest(State(state.clone()), auth_headers())
            .await
            .unwrap();
        assert!(dismissed.dismissed);
        assert!(matches!(
            rx.try_recv().as_deref(),
            Ok(BridgeServerMessage::Clear { .. })
        ));
        assert!(state.latest.lock().await.is_none());
    }

    #[test]
    fn constant_time_eq_matches_semantics() {
        assert!(constant_time_eq(b"Bearer secret", b"Bearer secret"));
        assert!(!constant_time_eq(b"Bearer secret", b"Bearer wrong!"));
        assert!(!constant_time_eq(b"short", b"longer-value"));
    }
}
