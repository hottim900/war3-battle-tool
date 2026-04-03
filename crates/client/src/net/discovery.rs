use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;
use tracing::{error, info, warn};
use war3_protocol::messages::{ClientMessage, ServerMessage};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);
const INITIAL_RECONNECT_DELAY: Duration = Duration::from_secs(1);
const MAX_RECONNECT_DELAY: Duration = Duration::from_secs(30);

/// 從網路層送回 UI 的事件
#[derive(Debug, Clone)]
pub enum NetEvent {
    Connected,
    Disconnected,
    Reconnecting { attempt: u32 },
    ServerMessage(ServerMessage),
}

/// 啟動 WebSocket 連線迴圈（在 tokio runtime 中呼叫）
pub async fn run_connection(
    server_url: String,
    mut cmd_rx: mpsc::UnboundedReceiver<ClientMessage>,
    event_tx: mpsc::UnboundedSender<NetEvent>,
) {
    let mut reconnect_delay = INITIAL_RECONNECT_DELAY;
    let mut attempt: u32 = 0;

    loop {
        attempt += 1;
        info!(%server_url, attempt, "連線發現伺服器");

        match connect_async(&server_url).await {
            Ok((ws_stream, _)) => {
                info!("WebSocket 連線成功");
                reconnect_delay = INITIAL_RECONNECT_DELAY;
                attempt = 0;

                let _ = event_tx.send(NetEvent::Connected);

                let disconnected = handle_session(ws_stream, &mut cmd_rx, &event_tx).await;
                let _ = event_tx.send(NetEvent::Disconnected);

                if !disconnected {
                    break;
                }
            }
            Err(e) => {
                warn!("連線失敗: {e}");
            }
        }

        let _ = event_tx.send(NetEvent::Reconnecting { attempt });
        info!(?reconnect_delay, "等待重連");
        tokio::time::sleep(reconnect_delay).await;
        reconnect_delay = (reconnect_delay * 2).min(MAX_RECONNECT_DELAY);
    }
}

/// 處理一次已建立的 WebSocket 連線
///
/// 回傳 true 表示需要重連，false 表示 UI 端已結束
async fn handle_session(
    ws_stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    cmd_rx: &mut mpsc::UnboundedReceiver<ClientMessage>,
    event_tx: &mpsc::UnboundedSender<NetEvent>,
) -> bool {
    let (mut ws_tx, mut ws_rx) = ws_stream.split();
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);

    loop {
        tokio::select! {
            cmd = cmd_rx.recv() => {
                match cmd {
                    Some(msg) => {
                        let json = serde_json::to_string(&msg).unwrap();
                        if ws_tx.send(Message::Text(json.into())).await.is_err() {
                            return true;
                        }
                    }
                    None => return false,
                }
            }

            ws_msg = ws_rx.next() => {
                match ws_msg {
                    Some(Ok(Message::Text(text))) => {
                        match serde_json::from_str::<ServerMessage>(&text) {
                            Ok(server_msg) => {
                                let _ = event_tx.send(NetEvent::ServerMessage(server_msg));
                            }
                            Err(e) => {
                                error!("解析伺服器訊息失敗: {e}");
                            }
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => return true,
                    Some(Err(e)) => {
                        warn!("WebSocket 錯誤: {e}");
                        return true;
                    }
                    _ => {}
                }
            }

            _ = heartbeat.tick() => {
                let json = serde_json::to_string(&ClientMessage::Heartbeat).unwrap();
                if ws_tx.send(Message::Text(json.into())).await.is_err() {
                    return true;
                }
            }
        }
    }
}
