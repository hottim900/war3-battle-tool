use war3_protocol::messages::ServerMessage;
use war3_protocol::war3::War3Version;

#[test]
fn server_message_clone() {
    let msg = ServerMessage::PlayerUpdate {
        players: vec![war3_protocol::messages::PlayerInfo {
            player_id: "p1".into(),
            nickname: "Test".into(),
            war3_version: War3Version::V127,
            is_hosting: false,
        }],
    };
    let cloned = msg.clone();
    let json1 = serde_json::to_string(&msg).unwrap();
    let json2 = serde_json::to_string(&cloned).unwrap();
    assert_eq!(json1, json2);
}

#[test]
fn server_message_room_update_serialization() {
    let msg = ServerMessage::RoomUpdate {
        rooms: vec![war3_protocol::messages::RoomInfo {
            room_id: "r1".into(),
            host_nickname: "Host".into(),
            room_name: "來玩啊".into(),
            map_name: "LostTemple".into(),
            max_players: 8,
            current_players: 3,
            war3_version: War3Version::V129c,
        }],
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("來玩啊"));
    assert!(json.contains("1.29c"));
}

#[test]
fn join_result_with_tunnel_token() {
    let msg = ServerMessage::JoinResult {
        success: true,
        room_id: Some("room-1".into()),
        tunnel_token: Some("token-abc".into()),
        gameinfo: Some(vec![0xf7, 0x30]),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("token-abc"));
    assert!(json.contains("room-1"));
}

#[test]
fn join_result_failure() {
    let msg = ServerMessage::join_failure();
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::JoinResult {
            success,
            tunnel_token,
            ..
        } => {
            assert!(!success);
            assert!(tunnel_token.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn player_joined_contains_tunnel_token() {
    let msg = ServerMessage::PlayerJoined {
        nickname: "Joiner".into(),
        tunnel_token: "token-xyz".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("token-xyz"));
    assert!(json.contains("Joiner"));
}

#[test]
fn tunnel_ready_serialization() {
    let msg = ServerMessage::TunnelReady {
        tunnel_token: "token-ready".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("TunnelReady"));
    assert!(json.contains("token-ready"));
}
