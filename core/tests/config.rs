use remote_tools_core::config::*;
use remote_tools_core::exec::bash::ShellKind;
use serde_json::json;
use std::path::Path;

#[test]
fn load_missing_file_returns_defaults() {
    let c = AppConfig::load(Path::new("Z:/definitely/missing/config.json"));
    assert_eq!(c.max_output_chars, DEFAULT_MAX_OUTPUT_CHARS);
    assert!(c.server_url.contains("ws://") || c.server_url.is_empty());
    if cfg!(windows) {
        assert!(matches!(AppConfig::default().shell_kind(), ShellKind::Powershell));
    }
}

#[test]
fn save_then_load_roundtrips() {
    let dir = std::env::temp_dir().join(format!("rt-cfg-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.json");
    let mut c = AppConfig::load(&p);
    c.server_url = "ws://srv.internal/client".into();
    c.token = "abc123".into();
    c.allow_roots = vec!["D:\\Code".into()];
    c.shell = "pwsh".into();
    c.save(&p).unwrap();
    let c2 = AppConfig::load(&p);
    assert_eq!(c2.server_url, "ws://srv.internal/client");
    assert_eq!(c2.token, "abc123");
    assert_eq!(c2.allow_roots, vec!["D:\\Code".to_string()]);
    assert!(matches!(c2.shell_kind(), ShellKind::Pwsh));
    assert_eq!(c2.rules().allow_roots.len(), 1);
}

#[test]
fn corrupt_file_falls_back_to_defaults() {
    let dir = std::env::temp_dir().join(format!("rt-cfg-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let p = dir.join("config.json");
    std::fs::write(&p, "{ not json !!!").unwrap();
    let c = AppConfig::load(&p);
    assert_eq!(c.token, "");
}

#[test]
fn truncate_spills_oversized_output() {
    let dir = std::env::temp_dir().join(format!("rt-spill-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut v = json!({ "stdout": "x".repeat(150), "exitCode": 0 });
    truncate_result(&mut v, 100, &dir, "req_TEST");
    assert_eq!(v["truncated"], json!(true));
    let spill = v["spillPath"].as_str().unwrap();
    let content = std::fs::read_to_string(spill).unwrap();
    assert_eq!(content.len(), 150);
    assert!(v["stdout"].as_str().unwrap().len() < 150);
    // 小结果不动
    let mut ok = json!({ "stdout": "short", "exitCode": 0 });
    truncate_result(&mut ok, 100, &dir, "req_OK");
    assert!(ok.get("truncated").is_none());
}

/// 落盘失败（spill 目录位置被一个文件占据 → create_dir_all 必败）不得关闭
/// 上限：仍截断头部 + truncated:true，无 spillPath，附 spillFailed:true。
#[test]
fn spill_failure_still_truncates_and_flags() {
    let blocker = std::env::temp_dir().join(format!("rt-spill-fail-{}", ulid::Ulid::new()));
    std::fs::write(&blocker, "我是文件，不是目录").unwrap();
    let mut v = json!({ "stdout": "y".repeat(150), "exitCode": 0 });
    truncate_result(&mut v, 100, &blocker, "req_FAIL");
    assert_eq!(v["truncated"], json!(true));
    assert_eq!(v["spillFailed"], json!(true));
    assert!(v.get("spillPath").is_none());
    assert!(v["stdout"].as_str().unwrap().len() <= 100);
}
