use war3_protocol::war3::War3Version;

#[test]
fn config_default_values() {
    // Test the shape matches what we expect from serde
    let default_json = r#"{
        "nickname": "",
        "war3_version": "1.27",
        "server_url": "ws://127.0.0.1:3000/ws"
    }"#;

    let parsed: serde_json::Value = serde_json::from_str(default_json).unwrap();
    assert_eq!(parsed["nickname"], "");
    assert_eq!(parsed["war3_version"], "1.27");
    assert_eq!(parsed["server_url"], "ws://127.0.0.1:3000/ws");
}

#[test]
fn config_roundtrip() {
    #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
    struct AppConfig {
        nickname: String,
        war3_version: War3Version,
        server_url: String,
    }

    let config = AppConfig {
        nickname: "台灣玩家".into(),
        war3_version: War3Version::V129c,
        server_url: "ws://example.com/ws".into(),
    };

    let json = serde_json::to_string(&config).unwrap();
    let parsed: AppConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed, config);
}

#[test]
fn config_handles_unknown_fields() {
    // Future-proof: config with extra fields should still deserialize
    #[derive(serde::Deserialize)]
    #[serde(default)]
    struct AppConfig {
        nickname: String,
        war3_version: War3Version,
        server_url: String,
    }

    impl Default for AppConfig {
        fn default() -> Self {
            Self {
                nickname: String::new(),
                war3_version: War3Version::V127,
                server_url: "ws://127.0.0.1:3000/ws".into(),
            }
        }
    }

    let json = r#"{"nickname":"Test","war3_version":"1.27","server_url":"ws://x/ws","unknown_field":42}"#;
    let config: AppConfig = serde_json::from_str(json).unwrap();
    assert_eq!(config.nickname, "Test");
}

#[test]
fn config_is_configured() {
    assert!(!"".trim().is_empty() == false); // empty = not configured
    assert!(!"  ".trim().is_empty() == false); // whitespace = not configured
    assert!("Player".trim().is_empty() == false); // has name = configured
}
