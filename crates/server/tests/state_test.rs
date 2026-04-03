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
fn join_result_with_ip() {
    let msg = ServerMessage::JoinResult {
        success: true,
        host_ip: Some("192.168.1.100".into()),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("192.168.1.100"));
}

#[test]
fn join_result_failure() {
    let msg = ServerMessage::JoinResult {
        success: false,
        host_ip: None,
    };
    let json = serde_json::to_string(&msg).unwrap();
    let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
    match parsed {
        ServerMessage::JoinResult { success, host_ip } => {
            assert!(!success);
            assert!(host_ip.is_none());
        }
        _ => panic!("wrong variant"),
    }
}

#[test]
fn player_joined_contains_ip() {
    let msg = ServerMessage::PlayerJoined {
        nickname: "Joiner".into(),
        player_ip: "10.0.0.5".into(),
    };
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("10.0.0.5"));
    assert!(json.contains("Joiner"));
}
