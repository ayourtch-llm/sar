use sar_core::Message;

#[test]
fn test_message_new_with_string() {
    let msg = Message::new("sar:test", "test-source", "hello world");
    assert_eq!(msg.topic, "sar:test");
    assert_eq!(msg.source, "test-source");
    assert_eq!(msg.payload, serde_json::json!("hello world"));
}

#[test]
fn test_message_new_with_json_value() {
    let json = serde_json::json!({"key": "value", "number": 42});
    let msg = Message::new("sar:test", "source", json.clone());
    assert_eq!(msg.topic, "sar:test");
    assert_eq!(msg.source, "source");
    assert_eq!(msg.payload, json);
}

#[test]
fn test_message_new_with_integer() {
    let msg = Message::new("sar:test", "source", 123);
    assert_eq!(msg.payload, serde_json::json!(123));
}

#[test]
fn test_message_text() {
    let msg = Message::text("sar:input", "sar-tui", "hello");
    assert_eq!(msg.topic, "sar:input");
    assert_eq!(msg.source, "sar-tui");
    assert_eq!(msg.payload, serde_json::json!("hello"));
}

#[test]
fn test_message_display() {
    let msg = Message::text("sar:log", "sar-echo", "echo test");
    let display = format!("{}", msg);
    assert_eq!(display, "[sar:log] sar-echo -> \"echo test\"");
}

#[test]
fn test_message_clone() {
    let msg1 = Message::text("sar:test", "source", "data");
    let msg2 = msg1.clone();
    assert_eq!(msg1.topic, msg2.topic);
    assert_eq!(msg1.source, msg2.source);
    assert_eq!(msg1.payload, msg2.payload);
}

#[test]
fn test_message_serialization() {
    let msg = Message::text("sar:test", "source", "data");
    let json = serde_json::to_string(&msg).unwrap();
    assert!(json.contains("\"topic\":\"sar:test\""));
    assert!(json.contains("\"source\":\"source\""));
    assert!(json.contains("\"payload\":\"data\""));
}

#[test]
fn test_message_deserialization() {
    let json = r#"{"topic":"sar:test","source":"src","payload":"msg"}"#;
    let msg: Message = serde_json::from_str(json).unwrap();
    assert_eq!(msg.topic, "sar:test");
    assert_eq!(msg.source, "src");
    assert_eq!(msg.payload, serde_json::json!("msg"));
}

#[test]
fn test_message_with_array_payload() {
    let arr = serde_json::json!([1, 2, 3]);
    let msg = Message::new("sar:test", "source", arr);
    assert_eq!(msg.payload, serde_json::json!([1, 2, 3]));
}

#[test]
fn test_message_with_object_payload() {
    let obj = serde_json::json!({"nested": {"key": "value"}});
    let msg = Message::new("sar:test", "source", obj);
    assert_eq!(msg.payload["nested"]["key"], "value");
}