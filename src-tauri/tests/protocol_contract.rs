use plumelink::protocol::ClientMessage;
#[test]
fn same_cases_as_typescript() {
    let cases: serde_json::Value =
        serde_json::from_str(include_str!("../../shared/protocol-cases.json")).unwrap();
    for case in cases.as_array().unwrap() {
        assert_eq!(
            ClientMessage::parse(&case["message"].to_string()).is_ok(),
            case["valid"].as_bool().unwrap(),
            "{}",
            case["name"]
        );
    }
}

#[test]
fn keyboard_keys_use_windows_virtual_key_codes() {
    use plumelink::protocol::Key;
    for key in b'a'..=b'z' {
        let parsed: Key = serde_json::from_value(serde_json::json!((key as char).to_string())).unwrap();
        assert_eq!(parsed as u16, key.to_ascii_uppercase() as u16);
    }
    for key in 0..=9 {
        let parsed: Key = serde_json::from_value(serde_json::json!(key.to_string())).unwrap();
        assert_eq!(parsed as u16, 0x30 + key);
    }
    for key in 1..=12 {
        let parsed: Key = serde_json::from_value(serde_json::json!(format!("f{key}"))).unwrap();
        assert_eq!(parsed as u16, 0x6f + key);
    }
    assert_eq!(Key::Delete as u16, 0x2e);
    assert_eq!(Key::Enter as u16, 0x0d);
}
