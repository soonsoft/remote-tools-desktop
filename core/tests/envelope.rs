use remote_tools_core::envelope::*;

#[test]
fn roundtrips_every_message_type() {
    let msgs = vec![
        r#"{"v":1,"type":"tool_request","id":"r1","tool":"client__bash","args":{"command":"npm test"},"meta":{"requestedAt":1694150400000}}"#,
        r#"{"v":1,"type":"tool_response","id":"r1","ok":true,"result":{"stdout":"","exitCode":0}}"#,
        r#"{"v":1,"type":"tool_response","id":"r2","ok":false,"error":{"code":"denied_by_user","message":"用户拒绝了此操作"}}"#,
        r#"{"v":1,"type":"hello","hostname":"pc","platform":"win32"}"#,
        r#"{"v":1,"type":"hello_ack","server":"remote-tools"}"#,
        r#"{"v":1,"type":"ping"}"#,
        r#"{"v":1,"type":"pong"}"#,
    ];
    for m in msgs {
        let env = parse_envelope(m).expect(m);
        let re = parse_envelope(&encode_envelope(&env)).expect("reparse");
        assert_eq!(encode_envelope(&env), encode_envelope(&re), "stable re-encode: {m}");
    }
}

#[test]
fn rejects_malformed() {
    for bad in [
        "not json", r#"{"v":1}"#, r#"{"v":2,"type":"ping"}"#,
        r#"{"v":1,"type":"nope"}"#,
        r#"{"v":1,"type":"tool_request","id":"x","tool":"bash","args":{}}"#,
        // result 必填（Plan A 修订镜像）
        r#"{"v":1,"type":"tool_response","id":"x","ok":true}"#,
    ] {
        assert!(parse_envelope(bad).is_none(), "should reject: {bad}");
    }
}

#[test]
fn tool_wire_names() {
    assert_eq!(ToolName::Bash.wire(), "client__bash");
    assert_eq!(ToolName::from_wire("client__glob"), Some(ToolName::Glob));
    assert_eq!(ToolName::from_wire("bash"), None);
}

#[test]
fn error_codes_verbatim() {
    let e = parse_envelope(
        r#"{"v":1,"type":"tool_response","id":"x","ok":false,"error":{"code":"path_denied","message":"m"}}"#,
    ).unwrap();
    match e { Envelope::ToolResponse(ToolResponse::Err{error,..}) =>
        assert_eq!(error.code, ErrorCode::PathDenied),
        _ => panic!("wrong variant") }
}

#[test]
fn request_ids_unique_and_long() {
    let a = new_request_id(); let b = new_request_id();
    assert_ne!(a, b); assert!(a.len() >= 26);
}
