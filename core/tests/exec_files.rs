use remote_tools_core::envelope::ErrorCode;
use remote_tools_core::exec::file::*;
use remote_tools_core::exec::search::*;
use serde_json::json;

fn root() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rt-exec-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn read_write_edit_roundtrip() {
    let r = root();
    write_file(&r, &json!({"path":"a/b.txt","content":"line1\nline2\n"})).unwrap();
    let v = read_file(&r, &json!({"path":"a/b.txt"})).unwrap();
    assert!(v["content"].as_str().unwrap().contains("line1"));
    let v = read_file(&r, &json!({"path":"a/b.txt","offset":1,"limit":1})).unwrap();
    assert_eq!(v["content"].as_str().unwrap(), "line2");
    edit_file(&r, &json!({"path":"a/b.txt","old_text":"line1","new_text":"LINE1"})).unwrap();
    let v = read_file(&r, &json!({"path":"a/b.txt"})).unwrap();
    assert!(v["content"].as_str().unwrap().starts_with("LINE1"));
}

#[test]
fn edit_requires_unique_match() {
    let r = root();
    write_file(&r, &json!({"path":"u.txt","content":"x x x"})).unwrap();
    let err = edit_file(&r, &json!({"path":"u.txt","old_text":"x","new_text":"y"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::BadArgs);
}

#[test]
fn path_fence_blocks_escape_at_executor_level() {
    let r = root();
    let err = read_file(&r, &json!({"path":"../../etc/passwd"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
    let err = write_file(&r, &json!({"path":"/abs/evil.txt","content":"x"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
}

#[test]
fn grep_and_glob_find_fixture_tree() {
    let r = root();
    write_file(&r, &json!({"path":"src/one.ts","content":"export const A = 1; // HIT\n"})).unwrap();
    write_file(&r, &json!({"path":"src/two.ts","content":"export const B = 2;\n"})).unwrap();
    let v = grep(&r, &json!({"pattern":"HIT"})).unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 1);
    assert!(v["matches"][0]["file"].as_str().unwrap().contains("one.ts"));
    let v = glob_search(&r, &json!({"pattern":"**/*.ts"})).unwrap();
    let paths: Vec<&str> = v["paths"].as_array().unwrap().iter()
        .map(|p| p.as_str().unwrap()).collect();
    assert!(paths.iter().any(|p| p.ends_with("one.ts")), "{paths:?}");
    assert!(paths.iter().all(|p| !p.contains("node_modules")));
}
