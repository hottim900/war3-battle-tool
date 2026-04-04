use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
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
        msgs.len() >= 3,
        "Expected >= 3 messages, got {}",
        msgs.len()
    );

    let welcome = find_msg(&msgs, "Welcome").expect("No Welcome");
    assert!(welcome["player_id"].is_string());

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
