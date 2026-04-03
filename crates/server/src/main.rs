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

    // 背景任務：清理超時玩家
    spawn_cleanup_task(state.clone());

    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(CorsLayer::permissive())
        .with_state(state);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3000));
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
    info!(%addr, "WebSocket 連線");
    ws.on_upgrade(move |socket| ws::handle_socket(socket, addr, state))
}

fn spawn_cleanup_task(state: Arc<AppState>) {
    tokio::spawn(async move {
        let mut tick = interval(CLEANUP_INTERVAL);
        loop {
            tick.tick().await;

            let expired: Vec<String> = {
                let players = state.players.read().await;
                players
                    .iter()
                    .filter(|(_, p)| p.last_heartbeat.elapsed() > HEARTBEAT_TIMEOUT)
                    .map(|(id, _)| id.clone())
                    .collect()
            };

            for player_id in expired {
                warn!(%player_id, "心跳超時，移除玩家");
                state.remove_player(&player_id).await;
            }
        }
    });
}
