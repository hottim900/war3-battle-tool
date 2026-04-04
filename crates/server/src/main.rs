mod state;
mod tunnel;
mod ws;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::http::HeaderMap;
use axum::response::IntoResponse;
use axum::routing::get;
use tokio::time::interval;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use state::AppState;
use tunnel::TunnelState;

struct SharedState {
    app: Arc<AppState>,
    tunnel: Arc<TunnelState>,
}

const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
const GRACE_PERIOD: Duration = Duration::from_secs(60);
const CLEANUP_INTERVAL: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "war3_server=info".parse().unwrap()),
        )
        .init();

    let app_state = AppState::new();
    let tunnel_state = TunnelState::new();

    spawn_cleanup_task(app_state.clone(), tunnel_state.clone());

    let shared = Arc::new(SharedState {
        app: app_state,
        tunnel: tunnel_state,
    });

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/tunnel", get(tunnel_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(shared);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let bind_addr: std::net::IpAddr = std::env::var("BIND")
        .ok()
        .and_then(|b| b.parse().ok())
        .unwrap_or_else(|| [127, 0, 0, 1].into());
    let addr = SocketAddr::from((bind_addr, port));
    info!("War3 發現伺服器啟動於 {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

/// 從 X-Real-IP header 取得真實 IP，僅信任來自 loopback 的連線（nginx）
fn real_ip(headers: &HeaderMap, fallback: SocketAddr) -> IpAddr {
    if fallback.ip().is_loopback() {
        headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<IpAddr>().ok())
            .unwrap_or_else(|| fallback.ip())
    } else {
        fallback.ip()
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(shared): State<Arc<SharedState>>,
) -> impl IntoResponse {
    let client_ip = real_ip(&headers, addr);
    let state = shared.app.clone();

    if !state.try_acquire_connection(client_ip).await {
        warn!(%client_ip, "連線數超過上限，拒絕連線");
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many connections from this IP",
        )
            .into_response();
    }

    info!(%client_ip, "WebSocket 連線");
    ws.on_upgrade(move |socket| async move {
        ws::handle_socket(socket, client_ip, state.clone()).await;
        state.release_connection(client_ip).await;
    })
    .into_response()
}

async fn tunnel_handler(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::extract::Query(params): axum::extract::Query<HashMap<String, String>>,
    State(shared): State<Arc<SharedState>>,
) -> impl IntoResponse {
    let client_ip = real_ip(&headers, addr);
    let tunnel_state = shared.tunnel.clone();

    let token = match params.get("token") {
        Some(t) if !t.is_empty() => t.clone(),
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Missing token parameter",
            )
                .into_response();
        }
    };

    let role = match params.get("role") {
        Some(r) if r == "host" || r == "join" => r.clone(),
        _ => {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                "Invalid role parameter (host|join)",
            )
                .into_response();
        }
    };

    if !tunnel_state.try_acquire_tunnel_connection(client_ip).await {
        warn!(%client_ip, "Tunnel 連線數超過上限");
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many tunnel connections from this IP",
        )
            .into_response();
    }

    let token_short = token.get(..8).unwrap_or(&token).to_string();
    info!(%client_ip, %token_short, %role, "Tunnel WebSocket 連線");

    ws.on_upgrade(move |socket| async move {
        tunnel::handle_tunnel(socket, client_ip, token, role, tunnel_state.clone()).await;
        tunnel_state.release_tunnel_connection(client_ip).await;
    })
    .into_response()
}

fn spawn_cleanup_task(state: Arc<AppState>, tunnel_state: Arc<TunnelState>) {
    tokio::spawn(async move {
        let mut tick = interval(CLEANUP_INTERVAL);
        loop {
            tick.tick().await;

            // 清理過期 tunnel sessions
            tunnel_state.cleanup_expired().await;

            let expired = state.find_expired(HEARTBEAT_TIMEOUT, GRACE_PERIOD).await;

            if expired.is_empty() {
                continue;
            }

            for player_id in &expired {
                warn!(%player_id, "玩家超時（心跳或 grace period），移除");
                state.remove_player(player_id).await;
            }
            state.broadcast_state().await;
        }
    });
}
