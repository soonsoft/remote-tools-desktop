use remote_tools_core::envelope::ErrorCode;
use remote_tools_core::exec::file::*;
use remote_tools_core::exec::search::*;
use serde_json::json;
use std::path::PathBuf;

fn root() -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("rt-exec-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn read_write_edit_roundtrip() {
    let r = root();
    let roots = vec![r.clone()];
    write_file(&roots, &json!({"path":"a/b.txt","content":"line1\nline2\n"})).unwrap();
    let v = read_file(&roots, &json!({"path":"a/b.txt"})).unwrap();
    assert!(v["content"].as_str().unwrap().contains("line1"));
    let v = read_file(&roots, &json!({"path":"a/b.txt","offset":1,"limit":1})).unwrap();
    assert_eq!(v["content"].as_str().unwrap(), "line2");
    edit_file(&roots, &json!({"path":"a/b.txt","old_text":"line1","new_text":"LINE1"})).unwrap();
    let v = read_file(&roots, &json!({"path":"a/b.txt"})).unwrap();
    assert!(v["content"].as_str().unwrap().starts_with("LINE1"));
}

#[test]
fn edit_requires_unique_match() {
    let r = root();
    let roots = vec![r.clone()];
    write_file(&roots, &json!({"path":"u.txt","content":"x x x"})).unwrap();
    let err = edit_file(&roots, &json!({"path":"u.txt","old_text":"x","new_text":"y"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::BadArgs);
}

#[test]
fn path_fence_blocks_escape_at_executor_level() {
    let r = root();
    let roots = vec![r.clone()];
    let err = read_file(&roots, &json!({"path":"../../etc/passwd"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
    let err = write_file(&roots, &json!({"path":"/abs/evil.txt","content":"x"})).unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
}

/// 多根围栏：目标落在任一 allowRoot 内即放行，并返回命中根；全不中才拒绝。
#[test]
fn resolve_in_root_spans_roots_and_reports_matched() {
    let a = root();
    let b = root();
    let roots = vec![a.clone(), b.clone()];
    // 相对路径命中第一个根
    let (target, matched) = resolve_in_root(&roots, "f.txt").unwrap();
    assert_eq!(matched, a);
    assert_eq!(target, a.join("f.txt"));
    // 绝对路径命中其真正所属的第二个根
    let (target, matched) = resolve_in_root(&roots, b.join("x.txt").to_str().unwrap()).unwrap();
    assert_eq!(matched, b);
    assert_eq!(target, b.join("x.txt"));
    // 两个根都出不去 → path_denied
    let err = resolve_in_root(&roots, "../escape.txt").unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
    let err = resolve_in_root(&[], "f.txt").unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
}

#[test]
fn grep_and_glob_find_fixture_tree() {
    let r = root();
    let roots = vec![r.clone()];
    write_file(&roots, &json!({"path":"src/one.ts","content":"export const A = 1; // HIT\n"})).unwrap();
    write_file(&roots, &json!({"path":"src/two.ts","content":"export const B = 2;\n"})).unwrap();
    let v = grep(&roots, &json!({"pattern":"HIT"})).unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 1);
    assert!(v["matches"][0]["file"].as_str().unwrap().contains("one.ts"));
    let v = glob_search(&roots, &json!({"pattern":"**/*.ts"})).unwrap();
    let paths: Vec<&str> = v["paths"].as_array().unwrap().iter()
        .map(|p| p.as_str().unwrap()).collect();
    assert!(paths.iter().any(|p| p.ends_with("one.ts")), "{paths:?}");
    assert!(paths.iter().all(|p| !p.contains("node_modules")));
}

/// grep 的 glob 过滤针对**相对 base 的斜杠路径**（与 glob_search 同一约定）——
/// 此前拿过滤正则去匹配完整绝对路径，`src/*.ts` 永远匹配不到 `D:\root\src\one.ts`。
#[test]
fn grep_glob_filter_matches_relative_slash_paths() {
    let r = root();
    let roots: Vec<PathBuf> = vec![r.clone()];
    write_file(&roots, &json!({"path":"src/one.ts","content":"HIT\n"})).unwrap();
    write_file(&roots, &json!({"path":"other/two.ts","content":"HIT\n"})).unwrap();
    // 无 glob：两处都命中
    let v = grep(&roots, &json!({"pattern":"HIT"})).unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 2);
    // glob: src/*.ts 只命中 src/one.ts
    let v = grep(&roots, &json!({"pattern":"HIT","glob":"src/*.ts"})).unwrap();
    let ms = v["matches"].as_array().unwrap();
    assert_eq!(ms.len(), 1, "{ms:?}");
    assert!(ms[0]["file"].as_str().unwrap().contains("one.ts"));
    // glob 对子目录单层通配不越层
    let v = grep(&roots, &json!({"pattern":"HIT","glob":"*.ts"})).unwrap();
    assert_eq!(v["matches"].as_array().unwrap().len(), 0);
}
