use war3_protocol::messages::*;
use war3_protocol::war3::War3Version;

// ── validate: Register ──

#[test]
fn register_valid() {
    let msg = ClientMessage::Register {
        nickname: "TestPlayer".into(),
        war3_version: War3Version::V127,
        client_version: None,
    };
    assert!(msg.validate().is_ok());
}

#[test]
fn register_empty_nickname() {
    let msg = ClientMessage::Register {
        nickname: "".into(),
        war3_version: War3Version::V127,
        client_version: None,
    };
    assert!(msg.validate().is_err());
}

#[test]
fn register_whitespace_nickname() {
    let msg = ClientMessage::Register {
        nickname: "   ".into(),
        war3_version: War3Version::V127,
        client_version: None,
    };
    assert!(msg.validate().is_err());
}

#[test]
fn register_nickname_too_long() {
    let msg = ClientMessage::Register {
        nickname: "a".repeat(MAX_NICKNAME_LEN + 1),
        war3_version: War3Version::V129c,
        client_version: None,
    };
    assert!(msg.validate().is_err());
}

#[test]
fn register_nickname_at_limit() {
    let msg = ClientMessage::Register {
        nickname: "a".repeat(MAX_NICKNAME_LEN),
        war3_version: War3Version::V127,
        client_version: None,
    };
    assert!(msg.validate().is_ok());
}

#[test]
fn register_cjk_nickname() {
    let msg = ClientMessage::Register {
        nickname: "台灣玩家".into(),
        war3_version: War3Version::V127,
        client_version: None,
    };
    assert!(msg.validate().is_ok());
}

// ── validate: CreateRoom ──

#[test]
fn create_room_valid() {
    let msg = ClientMessage::CreateRoom {
        room_name: "來玩啊".into(),
        map_name: "LostTemple".into(),
        max_players: 4,
        gameinfo: vec![0xf7, 0x30],
    };
    assert!(msg.validate().is_ok());
}

#[test]
fn create_room_empty_name() {
    let msg = ClientMessage::CreateRoom {
        room_name: "".into(),
        map_name: "LT".into(),
        max_players: 4,
        gameinfo: Vec::new(),
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_name_too_long() {
    let msg = ClientMessage::CreateRoom {
        room_name: "x".repeat(MAX_ROOM_NAME_LEN + 1),
        map_name: "LT".into(),
        max_players: 4,
        gameinfo: Vec::new(),
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_map_name_too_long() {
    let msg = ClientMessage::CreateRoom {
        room_name: "Room".into(),
        map_name: "m".repeat(MAX_MAP_NAME_LEN + 1),
        max_players: 4,
        gameinfo: Vec::new(),
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_max_players_too_low() {
    let msg = ClientMessage::CreateRoom {
        room_name: "Room".into(),
        map_name: "LT".into(),
        max_players: 1,
        gameinfo: Vec::new(),
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_max_players_too_high() {
    let msg = ClientMessage::CreateRoom {
        room_name: "Room".into(),
        map_name: "LT".into(),
        max_players: 13,
        gameinfo: Vec::new(),
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_max_players_boundaries() {
    for n in [2, 12] {
        let msg = ClientMessage::CreateRoom {
            room_name: "Room".into(),
            map_name: "LT".into(),
            max_players: n,
            gameinfo: Vec::new(),
        };
        assert!(msg.validate().is_ok(), "max_players={n} should be valid");
    }
}

#[test]
fn create_room_gameinfo_too_large() {
    let msg = ClientMessage::CreateRoom {
        room_name: "Room".into(),
        map_name: "LT".into(),
        max_players: 4,
        gameinfo: vec![0u8; MAX_GAMEINFO_LEN + 1],
    };
    assert!(msg.validate().is_err());
}

#[test]
fn create_room_gameinfo_at_limit() {
    let msg = ClientMessage::CreateRoom {
        room_name: "Room".into(),
        map_name: "LT".into(),
        max_players: 4,
        gameinfo: vec![0u8; MAX_GAMEINFO_LEN],
    };
    assert!(msg.validate().is_ok());
}

// ── validate: passthrough ──

#[test]
fn heartbeat_always_valid() {
    assert!(ClientMessage::Heartbeat.validate().is_ok());
}

#[test]
fn close_room_always_valid() {
    assert!(ClientMessage::CloseRoom.validate().is_ok());
}

#[test]
fn join_room_always_valid() {
    let msg = ClientMessage::JoinRoom {
        room_id: "some-id".into(),
    };
    assert!(msg.validate().is_ok());
}

// ── Serialization round-trip ──

#[test]
fn client_message_serde_roundtrip() {
    let messages = vec![
        ClientMessage::Register {
            nickname: "Player".into(),
            war3_version: War3Version::V127,
            client_version: Some("0.2.0".into()),
        },
        ClientMessage::Heartbeat,
        ClientMessage::CreateRoom {
            room_name: "Room".into(),
            map_name: "Map".into(),
            max_players: 8,
            gameinfo: vec![0xf7, 0x30],
        },
        ClientMessage::CloseRoom,
        ClientMessage::JoinRoom {
            room_id: "abc-123".into(),
        },
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            json,
            "round-trip failed for: {json}"
        );
    }
}

#[test]
fn server_message_serde_roundtrip() {
    let messages: Vec<ServerMessage> = vec![
        ServerMessage::Welcome {
            player_id: "id-1".into(),
        },
        ServerMessage::PlayerUpdate {
            players: vec![PlayerInfo {
                player_id: "p1".into(),
                nickname: "Nick".into(),
                war3_version: War3Version::V129c,
                is_hosting: true,
            }],
        },
        ServerMessage::RoomUpdate {
            rooms: vec![RoomInfo {
                room_id: "r1".into(),
                host_nickname: "Host".into(),
                room_name: "Room".into(),
                map_name: "Map".into(),
                max_players: 4,
                current_players: 2,
                war3_version: War3Version::V127,
            }],
        },
        ServerMessage::JoinResult {
            success: true,
            room_id: Some("room-1".into()),
            tunnel_token: Some("token-abc".into()),
            gameinfo: Some(vec![0xf7, 0x30]),
        },
        ServerMessage::PlayerJoined {
            nickname: "Joiner".into(),
            tunnel_token: "token-xyz".into(),
        },
        ServerMessage::TunnelReady {
            tunnel_token: "token-ready".into(),
        },
        ServerMessage::YourObservedAddr {
            ip: "1.2.3.4".into(),
        },
        ServerMessage::PeerUPnPAddr {
            external_addr: "5.6.7.8:19870".into(),
        },
        ServerMessage::Error {
            message: "err".into(),
        },
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).unwrap();
        let parsed: ServerMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
    }
}

// ── T15: serde(other) catch-all ──

#[test]
fn unknown_server_message_deserializes_to_unknown() {
    // 模擬新版 server 送出舊版 client 不認識的 variant
    let json = r#"{"type":"SomeFutureVariant","data":"hello"}"#;
    let msg: ServerMessage = serde_json::from_str(json).unwrap();
    assert!(matches!(msg, ServerMessage::Unknown));
}

#[test]
fn client_message_tagged_format() {
    let msg = ClientMessage::Heartbeat;
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains(r#""type":"Heartbeat"#));
}

// ── War3Version ──

#[test]
fn war3_version_display() {
    assert_eq!(War3Version::V127.as_str(), "1.27");
    assert_eq!(War3Version::V129c.as_str(), "1.29c");
}

#[test]
fn war3_version_broadcast_packet_length() {
    assert_eq!(War3Version::V127.broadcast_packet().len(), 16);
    assert_eq!(War3Version::V129c.broadcast_packet().len(), 16);
}

#[test]
fn war3_version_broadcast_header() {
    // All broadcast packets start with 0xf7 0x2f
    for v in [War3Version::V127, War3Version::V129c] {
        let pkt = v.broadcast_packet();
        assert_eq!(pkt[0], 0xf7);
        assert_eq!(pkt[1], 0x2f);
    }
}

#[test]
fn war3_version_serde() {
    let json = serde_json::to_string(&War3Version::V127).unwrap();
    assert_eq!(json, r#""1.27""#);

    let parsed: War3Version = serde_json::from_str(r#""1.29c""#).unwrap();
    assert_eq!(parsed, War3Version::V129c);
}
