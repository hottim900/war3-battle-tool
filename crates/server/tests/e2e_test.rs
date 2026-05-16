use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

struct ServerHandle {
    _child: tokio::process::Child,
    port: u16,
}

async fn start_server() -> ServerHandle {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let child = tokio::process::Command::new(env!("CARGO_BIN_EXE_war3-server"))
        .env("PORT", port.to_string())
        .env("RUST_LOG", "warn")
        // 清掉外部環境可能設的 WAR3_ALLOWED_ORIGINS，確保 tests 走預設 allowlist。
        // 若 CI runner 或 dev shell 不小心設了非預設值，整批 origin 測試會神祕失敗。
        .env_remove("WAR3_ALLOWED_ORIGINS")
        .kill_on_drop(true)
        .spawn()
        .expect("Failed to start server");

    // Wait for server to be ready
    for _ in 0..50 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        if tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .is_ok()
        {
            return ServerHandle {
                _child: child,
                port,
            };
        }
    }
    panic!("Server didn't start within 5 seconds");
}

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn connect(port: u16) -> WsStream {
    let url = format!("ws://127.0.0.1:{port}/ws");
    let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws
}

async fn send_json(ws: &mut WsStream, msg: Value) {
    ws.send(Message::Text(serde_json::to_string(&msg).unwrap().into()))
        .await
        .unwrap();
}

async fn drain_messages(ws: &mut WsStream) -> Vec<Value> {
    let mut msgs = Vec::new();
    loop {
        match tokio::time::timeout(Duration::from_millis(500), ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                msgs.push(serde_json::from_str(&t).unwrap());
            }
            _ => break,
        }
    }
    msgs
}

fn find_msg<'a>(msgs: &'a [Value], msg_type: &str) -> Option<&'a Value> {
    msgs.iter().find(|m| m["type"] == msg_type)
}

// ── Tests ──

#[tokio::test]
async fn register_and_welcome() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "Alice", "war3_version": "1.27"}),
    )
    .await;

    let msgs = drain_messages(&mut ws).await;
    assert!(
        msgs.len() >= 4,
        "Expected >= 4 messages (Welcome, YourObservedAddr, PlayerUpdate, RoomUpdate), got {}",
        msgs.len()
    );

    let welcome = find_msg(&msgs, "Welcome").expect("No Welcome");
    assert!(welcome["player_id"].is_string());

    // T9: YourObservedAddr 在 Register 後立即送出
    let observed = find_msg(&msgs, "YourObservedAddr").expect("No YourObservedAddr");
    assert!(observed["ip"].is_string());
    // Server 本地測試，IP 是 127.0.0.1
    assert_eq!(observed["ip"].as_str().unwrap(), "127.0.0.1");

    // YourObservedAddr 必須在 Welcome 之後、broadcast_state 之前
    let welcome_idx = msgs.iter().position(|m| m["type"] == "Welcome").unwrap();
    let observed_idx = msgs
        .iter()
        .position(|m| m["type"] == "YourObservedAddr")
        .unwrap();
    let pu_idx = msgs
        .iter()
        .position(|m| m["type"] == "PlayerUpdate")
        .unwrap();
    assert!(
        welcome_idx < observed_idx,
        "YourObservedAddr must come after Welcome"
    );
    assert!(
        observed_idx < pu_idx,
        "YourObservedAddr must come before PlayerUpdate"
    );

    let pu = find_msg(&msgs, "PlayerUpdate").expect("No PlayerUpdate");
    let players = pu["players"].as_array().unwrap();
    assert_eq!(players.len(), 1);
    assert_eq!(players[0]["nickname"], "Alice");

    let ru = find_msg(&msgs, "RoomUpdate").expect("No RoomUpdate");
    assert_eq!(ru["rooms"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn create_and_close_room() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut ws).await;

    send_json(
        &mut ws,
        json!({"type": "CreateRoom", "room_name": "TestRoom", "map_name": "LT", "max_players": 4}),
    )
    .await;

    let msgs = drain_messages(&mut ws).await;
    let ru = find_msg(&msgs, "RoomUpdate").expect("No RoomUpdate after CreateRoom");
    let rooms = ru["rooms"].as_array().unwrap();
    assert_eq!(rooms.len(), 1);
    assert_eq!(rooms[0]["room_name"], "TestRoom");
    assert_eq!(rooms[0]["host_nickname"], "Host");

    send_json(&mut ws, json!({"type": "CloseRoom"})).await;
    let msgs = drain_messages(&mut ws).await;
    let ru = find_msg(&msgs, "RoomUpdate").expect("No RoomUpdate after CloseRoom");
    assert_eq!(ru["rooms"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn join_room_exchanges_ips() {
    let srv = start_server().await;

    let mut host = connect(srv.port).await;
    send_json(
        &mut host,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut host).await;

    send_json(
        &mut host,
        json!({"type": "CreateRoom", "room_name": "R", "map_name": "M", "max_players": 4}),
    )
    .await;
    let msgs = drain_messages(&mut host).await;
    let room_id = msgs
        .iter()
        .find_map(|m| {
            m["rooms"]
                .as_array()?
                .first()?
                .get("room_id")?
                .as_str()
                .map(String::from)
        })
        .expect("No room_id");

    let mut joiner = connect(srv.port).await;
    send_json(
        &mut joiner,
        json!({"type": "Register", "nickname": "Joiner", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut joiner).await;

    send_json(&mut joiner, json!({"type": "JoinRoom", "room_id": room_id})).await;

    let msgs = drain_messages(&mut joiner).await;
    let jr = find_msg(&msgs, "JoinResult").expect("No JoinResult");
    assert_eq!(jr["success"], true);
    assert!(jr["tunnel_token"].is_string());
    assert!(jr["room_id"].is_string());

    let msgs = drain_messages(&mut host).await;
    let pj = find_msg(&msgs, "PlayerJoined").expect("No PlayerJoined");
    assert_eq!(pj["nickname"], "Joiner");
    assert!(pj["tunnel_token"].is_string());
}

#[tokio::test]
async fn join_nonexistent_room() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "P", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut ws).await;

    send_json(&mut ws, json!({"type": "JoinRoom", "room_id": "nope"})).await;

    let msgs = drain_messages(&mut ws).await;
    let jr = find_msg(&msgs, "JoinResult").expect("No JoinResult");
    assert_eq!(jr["success"], false);
}

#[tokio::test]
async fn duplicate_register_rejected() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "P", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut ws).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "P2", "war3_version": "1.27"}),
    )
    .await;

    let msgs = drain_messages(&mut ws).await;
    let err = find_msg(&msgs, "Error").expect("No Error");
    assert!(err["message"].as_str().unwrap().contains("已經註冊"));
}

#[tokio::test]
async fn invalid_json_returns_error() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    ws.send(Message::Text("not json".into())).await.unwrap();

    let msgs = drain_messages(&mut ws).await;
    let err = find_msg(&msgs, "Error").expect("No Error");
    assert!(err["message"].as_str().unwrap().contains("無法解析"));
}

#[tokio::test]
async fn empty_nickname_rejected() {
    let srv = start_server().await;
    let mut ws = connect(srv.port).await;

    send_json(
        &mut ws,
        json!({"type": "Register", "nickname": "  ", "war3_version": "1.27"}),
    )
    .await;

    let msgs = drain_messages(&mut ws).await;
    let err = find_msg(&msgs, "Error").expect("No Error");
    assert!(err["message"].as_str().unwrap().contains("暱稱"));
}

#[tokio::test]
async fn two_players_see_each_other() {
    let srv = start_server().await;

    let mut p1 = connect(srv.port).await;
    send_json(
        &mut p1,
        json!({"type": "Register", "nickname": "P1", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut p1).await;

    let mut p2 = connect(srv.port).await;
    send_json(
        &mut p2,
        json!({"type": "Register", "nickname": "P2", "war3_version": "1.29c"}),
    )
    .await;
    drain_messages(&mut p2).await;

    // P1 should have received broadcast
    let msgs = drain_messages(&mut p1).await;
    let pu = find_msg(&msgs, "PlayerUpdate").expect("P1 should see update");
    let players = pu["players"].as_array().unwrap();
    assert_eq!(players.len(), 2);

    let nicks: Vec<&str> = players
        .iter()
        .map(|p| p["nickname"].as_str().unwrap())
        .collect();
    assert!(nicks.contains(&"P1"));
    assert!(nicks.contains(&"P2"));
}

// ── Tunnel Tests ──

async fn connect_tunnel(port: u16, token: &str, role: &str) -> WsStream {
    let url = format!("ws://127.0.0.1:{port}/tunnel?token={token}&role={role}");
    let (ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
    ws
}

/// 完整 tunnel relay：host + joiner 配對，雙向 binary 轉發
#[tokio::test]
async fn tunnel_relay_roundtrip() {
    let srv = start_server().await;

    // 1. 註冊 host 和 joiner，建房加入取得 tunnel_token
    let mut host_ws = connect(srv.port).await;
    send_json(
        &mut host_ws,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut host_ws).await;

    send_json(
        &mut host_ws,
        json!({"type": "CreateRoom", "room_name": "R", "map_name": "M", "max_players": 4, "gameinfo": [0xf7, 0x30]}),
    )
    .await;
    let msgs = drain_messages(&mut host_ws).await;
    let room_id = msgs
        .iter()
        .find_map(|m| {
            m["rooms"]
                .as_array()?
                .first()?
                .get("room_id")?
                .as_str()
                .map(String::from)
        })
        .expect("No room_id");

    let mut joiner_ws = connect(srv.port).await;
    send_json(
        &mut joiner_ws,
        json!({"type": "Register", "nickname": "Joiner", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut joiner_ws).await;

    send_json(
        &mut joiner_ws,
        json!({"type": "JoinRoom", "room_id": room_id}),
    )
    .await;

    let msgs = drain_messages(&mut joiner_ws).await;
    let jr = find_msg(&msgs, "JoinResult").expect("No JoinResult");
    let joiner_token = jr["tunnel_token"]
        .as_str()
        .expect("No tunnel_token")
        .to_string();

    let msgs = drain_messages(&mut host_ws).await;
    let pj = find_msg(&msgs, "PlayerJoined").expect("No PlayerJoined");
    let host_token = pj["tunnel_token"]
        .as_str()
        .expect("No tunnel_token")
        .to_string();

    // host 和 joiner 應該拿到同一個 token
    assert_eq!(host_token, joiner_token);

    // 2. 開 tunnel WS
    let mut host_tunnel = connect_tunnel(srv.port, &host_token, "host").await;
    // 給 host 一點時間註冊
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut joiner_tunnel = connect_tunnel(srv.port, &joiner_token, "join").await;
    // 給配對一點時間
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 3. joiner → host binary relay
    let payload = vec![0x01, 0x02, 0x03, 0x04];
    joiner_tunnel
        .send(Message::Binary(payload.clone().into()))
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(2), host_tunnel.next())
        .await
        .expect("Timeout waiting for host to receive")
        .expect("Stream ended")
        .expect("WS error");
    match msg {
        Message::Binary(data) => assert_eq!(data.as_ref(), &payload),
        other => panic!("Expected Binary, got {other:?}"),
    }

    // 4. host → joiner binary relay
    let payload2 = vec![0xaa, 0xbb, 0xcc];
    host_tunnel
        .send(Message::Binary(payload2.clone().into()))
        .await
        .unwrap();

    let msg = tokio::time::timeout(Duration::from_secs(2), joiner_tunnel.next())
        .await
        .expect("Timeout waiting for joiner to receive")
        .expect("Stream ended")
        .expect("WS error");
    match msg {
        Message::Binary(data) => assert_eq!(data.as_ref(), &payload2),
        other => panic!("Expected Binary, got {other:?}"),
    }
}

/// Tunnel half-close：一端斷開，另一端應收到 close
#[tokio::test]
async fn tunnel_half_close() {
    let srv = start_server().await;

    // Setup: host + joiner 建房加入
    let mut host_ws = connect(srv.port).await;
    send_json(
        &mut host_ws,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut host_ws).await;

    send_json(
        &mut host_ws,
        json!({"type": "CreateRoom", "room_name": "R", "map_name": "M", "max_players": 4}),
    )
    .await;
    let msgs = drain_messages(&mut host_ws).await;
    let room_id = msgs
        .iter()
        .find_map(|m| {
            m["rooms"]
                .as_array()?
                .first()?
                .get("room_id")?
                .as_str()
                .map(String::from)
        })
        .expect("No room_id");

    let mut joiner_ws = connect(srv.port).await;
    send_json(
        &mut joiner_ws,
        json!({"type": "Register", "nickname": "Joiner", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut joiner_ws).await;

    send_json(
        &mut joiner_ws,
        json!({"type": "JoinRoom", "room_id": room_id}),
    )
    .await;

    let msgs = drain_messages(&mut joiner_ws).await;
    let jr = find_msg(&msgs, "JoinResult").expect("No JoinResult");
    let token = jr["tunnel_token"]
        .as_str()
        .expect("No tunnel_token")
        .to_string();
    drain_messages(&mut host_ws).await; // consume PlayerJoined

    let mut host_tunnel = connect_tunnel(srv.port, &token, "host").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let mut joiner_tunnel = connect_tunnel(srv.port, &token, "join").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // joiner 斷開
    joiner_tunnel.close(None).await.unwrap();

    // host 應該收到 close 或 stream 結束
    let result = tokio::time::timeout(Duration::from_secs(2), host_tunnel.next()).await;
    match result {
        Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {
            // 預期行為：收到 close frame 或 stream 結束
        }
        Ok(Some(Err(_))) => {
            // WS error 也算斷線
        }
        other => {
            panic!("Expected close/end after joiner disconnect, got {other:?}");
        }
    }
}

/// Invalid tunnel token 應被拒絕
#[tokio::test]
async fn tunnel_invalid_token() {
    let srv = start_server().await;

    // 嘗試用不存在的 token 連 tunnel
    let result = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/tunnel?token=nonexistent&role=join",
        srv.port
    ))
    .await;

    // joiner 連上後 server 應該關閉連線（token 不存在）
    match result {
        Ok((mut ws, _)) => {
            // 連線成功但 server 應該很快關閉
            let msg = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;
            match msg {
                Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {}
                Ok(Some(Err(_))) => {}
                other => panic!("Expected close for invalid token, got {other:?}"),
            }
        }
        Err(_) => {
            // 連線被拒也是合理的
        }
    }
}

/// Token 重複使用應被拒絕
#[tokio::test]
async fn tunnel_token_reuse_rejected() {
    let srv = start_server().await;

    // Setup
    let mut host_ws = connect(srv.port).await;
    send_json(
        &mut host_ws,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut host_ws).await;

    send_json(
        &mut host_ws,
        json!({"type": "CreateRoom", "room_name": "R", "map_name": "M", "max_players": 4}),
    )
    .await;
    let msgs = drain_messages(&mut host_ws).await;
    let room_id = msgs
        .iter()
        .find_map(|m| {
            m["rooms"]
                .as_array()?
                .first()?
                .get("room_id")?
                .as_str()
                .map(String::from)
        })
        .expect("No room_id");

    let mut joiner_ws = connect(srv.port).await;
    send_json(
        &mut joiner_ws,
        json!({"type": "Register", "nickname": "Joiner", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut joiner_ws).await;

    send_json(
        &mut joiner_ws,
        json!({"type": "JoinRoom", "room_id": room_id}),
    )
    .await;

    let msgs = drain_messages(&mut joiner_ws).await;
    let jr = find_msg(&msgs, "JoinResult").expect("No JoinResult");
    let token = jr["tunnel_token"]
        .as_str()
        .expect("No tunnel_token")
        .to_string();
    drain_messages(&mut host_ws).await;

    // 第一次使用
    let mut host_tunnel = connect_tunnel(srv.port, &token, "host").await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    let _joiner_tunnel = connect_tunnel(srv.port, &token, "join").await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 第二次用同 token 當 joiner，應該被拒
    let result = tokio_tungstenite::connect_async(format!(
        "ws://127.0.0.1:{}/tunnel?token={token}&role=join",
        srv.port
    ))
    .await;

    match result {
        Ok((mut ws, _)) => {
            let msg = tokio::time::timeout(Duration::from_secs(2), ws.next()).await;
            match msg {
                Ok(Some(Ok(Message::Close(_)))) | Ok(None) => {}
                Ok(Some(Err(_))) => {}
                other => panic!("Expected close for reused token, got {other:?}"),
            }
        }
        Err(_) => {}
    }

    // 確認第一組 tunnel 仍然正常
    let payload = vec![0xde, 0xad];
    host_tunnel
        .send(Message::Binary(payload.into()))
        .await
        .unwrap();
}

#[tokio::test]
async fn stun_info_only_unicast_to_participants() {
    let srv = start_server().await;

    // 三個玩家：host, joiner, bystander
    let mut host = connect(srv.port).await;
    send_json(
        &mut host,
        json!({"type": "Register", "nickname": "Host", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut host).await;

    let mut joiner = connect(srv.port).await;
    send_json(
        &mut joiner,
        json!({"type": "Register", "nickname": "Joiner", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut joiner).await;

    let mut bystander = connect(srv.port).await;
    send_json(
        &mut bystander,
        json!({"type": "Register", "nickname": "Bystander", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut bystander).await;

    // Host 建房
    send_json(
        &mut host,
        json!({"type": "CreateRoom", "room_name": "R", "map_name": "M", "max_players": 4}),
    )
    .await;
    let msgs = drain_messages(&mut host).await;
    let room_id = msgs
        .iter()
        .find_map(|m| {
            m["rooms"]
                .as_array()?
                .first()?
                .get("room_id")?
                .as_str()
                .map(String::from)
        })
        .expect("No room_id");
    drain_messages(&mut bystander).await;

    // Joiner 加入
    send_json(&mut joiner, json!({"type": "JoinRoom", "room_id": room_id})).await;

    // Joiner 應收到 StunInfo
    let joiner_msgs = drain_messages(&mut joiner).await;
    assert!(
        find_msg(&joiner_msgs, "StunInfo").is_some(),
        "Joiner should receive StunInfo"
    );

    // Host 應收到 StunInfo
    let host_msgs = drain_messages(&mut host).await;
    assert!(
        find_msg(&host_msgs, "StunInfo").is_some(),
        "Host should receive StunInfo"
    );

    // Bystander 不應收到 StunInfo
    let bystander_msgs = drain_messages(&mut bystander).await;
    assert!(
        find_msg(&bystander_msgs, "StunInfo").is_none(),
        "Bystander should NOT receive StunInfo"
    );
}

// ── T13: Web-viewer sentinel ──

#[tokio::test]
async fn web_viewer_excluded_from_player_update() {
    let srv = start_server().await;

    // 普通玩家
    let mut player = connect(srv.port).await;
    send_json(
        &mut player,
        json!({"type": "Register", "nickname": "RealPlayer", "war3_version": "1.27"}),
    )
    .await;
    drain_messages(&mut player).await;

    // Web viewer 註冊，收集初始訊息
    let mut viewer = connect(srv.port).await;
    send_json(
        &mut viewer,
        json!({"type": "Register", "nickname": "__web-viewer-test123__", "war3_version": "1.27"}),
    )
    .await;
    let viewer_msgs = drain_messages(&mut viewer).await;

    // Web viewer 應收到 Welcome + RoomUpdate（但不收 YourObservedAddr）
    assert!(
        find_msg(&viewer_msgs, "Welcome").is_some(),
        "Web viewer should receive Welcome"
    );
    assert!(
        find_msg(&viewer_msgs, "YourObservedAddr").is_none(),
        "Web viewer should NOT receive YourObservedAddr"
    );
    assert!(
        find_msg(&viewer_msgs, "RoomUpdate").is_some(),
        "Web viewer should receive RoomUpdate"
    );

    // 普通玩家收到的 PlayerUpdate 不包含 web viewer
    let msgs = drain_messages(&mut player).await;
    let pu = find_msg(&msgs, "PlayerUpdate").expect("No PlayerUpdate");
    let players = pu["players"].as_array().unwrap();
    let nicks: Vec<&str> = players
        .iter()
        .map(|p| p["nickname"].as_str().unwrap())
        .collect();
    assert!(nicks.contains(&"RealPlayer"), "Should see real player");
    assert!(
        !nicks.iter().any(|n| n.starts_with("__web-viewer-")),
        "Should NOT see web viewer in PlayerUpdate"
    );
}

// ── Origin verification tests (PR-A #34) ──
//
// 這些測試送 raw HTTP/1.1 取得 status code，不依賴 tokio-tungstenite client
// 自動 handshake，因為要驗證 server 在 WS upgrade 完成 *之前* 就拒絕。
// 101 success path 仍走完整 WS upgrade（server 端會回 101 Switching Protocols）。

async fn send_request_get_status(port: u16, request: &str) -> u16 {
    let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
        .await
        .unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut buf = [0u8; 256];
    let n = stream.read(&mut buf).await.unwrap();
    let resp = std::str::from_utf8(&buf[..n]).expect("non-utf8 response");
    let status_line = resp.lines().next().expect("empty response");
    status_line
        .split_whitespace()
        .nth(1)
        .expect("malformed status line")
        .parse()
        .expect("non-numeric status")
}

async fn raw_ws_request(port: u16, path: &str, origin: Option<&str>) -> u16 {
    let mut req = format!(
        "GET {path} HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Upgrade: websocket\r\n\
         Connection: Upgrade\r\n\
         Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
         Sec-WebSocket-Version: 13\r\n"
    );
    if let Some(o) = origin {
        req.push_str(&format!("Origin: {o}\r\n"));
    }
    req.push_str("\r\n");
    send_request_get_status(port, &req).await
}

async fn raw_http_request(port: u16, path: &str, origin: Option<&str>) -> u16 {
    let mut req = format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n");
    if let Some(o) = origin {
        req.push_str(&format!("Origin: {o}\r\n"));
    }
    req.push_str("Connection: close\r\n\r\n");
    send_request_get_status(port, &req).await
}

#[tokio::test]
async fn ws_rejects_bad_origin_403() {
    let srv = start_server().await;
    let status = raw_ws_request(srv.port, "/ws", Some("https://evil.com")).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn ws_accepts_localhost_origin_101() {
    let srv = start_server().await;
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let status = raw_ws_request(srv.port, "/ws", Some(&origin)).await;
    assert_eq!(status, 101, "expected WS upgrade for allowlisted localhost");
}

#[tokio::test]
async fn ws_accepts_no_origin_101() {
    // Native War3 client 不送 Origin header — 必須允許
    let srv = start_server().await;
    let status = raw_ws_request(srv.port, "/ws", None).await;
    assert_eq!(status, 101);
}

#[tokio::test]
async fn ws_rejects_suffix_attack_403() {
    let srv = start_server().await;
    let status = raw_ws_request(srv.port, "/ws", Some("https://war3.kalthor.cc.evil.com")).await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn tunnel_rejects_bad_origin_403() {
    let srv = start_server().await;
    let status = raw_ws_request(
        srv.port,
        "/tunnel?token=test123&role=host",
        Some("https://evil.com"),
    )
    .await;
    assert_eq!(status, 403);
}

#[tokio::test]
async fn health_unaffected_by_bad_origin() {
    // /health 不套 Origin 驗證 — canary / monitoring 從任何 origin 都應 200
    let srv = start_server().await;
    let status = raw_http_request(srv.port, "/health", Some("https://evil.com")).await;
    assert_eq!(status, 200);
}

#[tokio::test]
async fn bad_origin_does_not_leak_connection_slot() {
    // E3 regression test: 15 個 bad-Origin requests 必須全 403。
    // 若 validate_origin 在 try_acquire_connection 之後，第 11+ 個會 hit per-IP cap
    // 變成 429（slot 被 leak 佔滿）。Validate-before-acquire 順序保證全 403。
    let srv = start_server().await;
    for i in 0..15 {
        let status = raw_ws_request(srv.port, "/ws", Some("https://evil.com")).await;
        assert_eq!(
            status, 403,
            "request {i}: expected 403 (validate-before-acquire), got {status}"
        );
    }

    // 接著用合法 Origin 連線，slot 應仍可用
    let origin = format!("http://127.0.0.1:{}", srv.port);
    let status = raw_ws_request(srv.port, "/ws", Some(&origin)).await;
    assert_eq!(status, 101, "good Origin should succeed after 15 rejected");
}
