mod state;
mod ws;

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{ConnectInfo, State, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::time::interval;
use tower_http::cors::CorsLayer;
use tracing::{info, warn};

use state::AppState;

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

    let state = AppState::new();

    spawn_cleanup_task(state.clone());

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    info!("War3 發現伺服器啟動於 {addr}");

    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
    .unwrap();
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let ip = addr.ip();

    // 連線數限制
    if !state.try_acquire_connection(ip).await {
        warn!(%addr, "連線數超過上限，拒絕連線");
        return (
            axum::http::StatusCode::TOO_MANY_REQUESTS,
            "Too many connections from this IP",
        )
            .into_response();
    }

    info!(%addr, "WebSocket 連線");
    ws.on_upgrade(move |socket| async move {
        ws::handle_socket(socket, addr, state.clone()).await;
        state.release_connection(ip).await;
    })
    .into_response()
}

fn spawn_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = interval(CLEANUP_INTERVAL);
        loop {
            tick.tick().await;

            let expired = state
                .cleanup_expired(HEARTBEAT_TIMEOUT, GRACE_PERIOD)
                .await;

            for player_id in expired {
                warn!(%player_id, "玩家超時（心跳或 grace period），移除");
                state.remove_player(&player_id).await;
            }
        }
    });
}
