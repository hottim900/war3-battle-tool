use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
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

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

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
    assert!(jr["host_ip"].is_string());

    let msgs = drain_messages(&mut host).await;
    let pj = find_msg(&msgs, "PlayerJoined").expect("No PlayerJoined");
    assert_eq!(pj["nickname"], "Joiner");
    assert!(pj["player_ip"].is_string());
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
