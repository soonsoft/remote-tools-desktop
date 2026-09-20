use crate::envelope::ErrorCode;
use crate::rules::within_root;
use serde_json::{json, Value};
use std::path::PathBuf;

/// 词法规范化：消除 `..`/`.`（绝对路径与盘符前缀按 Component 原样保留）。
fn lexical_normalize(p: PathBuf) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 多根路径围栏在执行器侧二次强制——规则层放行后仍守门：对每个 allowRoot 做
/// `root.join(raw)` 的词法规范化，返回**第一个**规范化后仍落在该根内的
/// (规范化目标, 命中根)；没有任何根命中 → `path_denied`。
/// 绝对路径经 join 后替换基底，天然只可能命中其真正所属的根。
pub fn resolve_in_root(
    roots: &[PathBuf], raw: &str,
) -> Result<(PathBuf, PathBuf), (ErrorCode, String)> {
    for root in roots {
        let out = lexical_normalize(root.join(raw));
        if within_root(root, &out) { return Ok((out, root.clone())); }
    }
    Err((ErrorCode::PathDenied, format!("路径越出所有允许根：{raw}")))
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Result<&'a str, (ErrorCode, String)> {
    args.get(key).and_then(Value::as_str)
        .ok_or((ErrorCode::BadArgs, format!("缺少字符串参数 {key}")))
}

pub fn read_file(roots: &[PathBuf], args: &Value) -> Result<Value, (ErrorCode, String)> {
    let (path, _) = resolve_in_root(roots, str_arg(args, "path")?)?;
    let content = std::fs::read_to_string(&path)
        .map_err(|e| (ErrorCode::ExecError, format!("读取失败 {path:?}：{e}")))?;
    let lines: Vec<&str> = content.split('\n').collect();
    let offset = args.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize;
    let limit = args.get("limit").and_then(Value::as_u64).map(|l| l as usize);
    let slice: Vec<&str> = match limit {
        Some(l) => lines.iter().skip(offset).take(l).copied().collect(),
        None => lines.iter().skip(offset).copied().collect(),
    };
    Ok(json!({ "path": path.to_string_lossy(), "content": slice.join("\n"), "truncated": false }))
}

pub fn write_file(roots: &[PathBuf], args: &Value) -> Result<Value, (ErrorCode, String)> {
    let (path, _) = resolve_in_root(roots, str_arg(args, "path")?)?;
    let content = str_arg(args, "content")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| (ErrorCode::ExecError, format!("建目录失败：{e}")))?;
    }
    std::fs::write(&path, content).map_err(|e| (ErrorCode::ExecError, format!("写入失败 {path:?}：{e}")))?;
    Ok(json!({ "written": true, "bytes": content.len() }))
}

pub fn edit_file(roots: &[PathBuf], args: &Value) -> Result<Value, (ErrorCode, String)> {
    let (path, _) = resolve_in_root(roots, str_arg(args, "path")?)?;
    let old_text = str_arg(args, "old_text")?;
    let new_text = str_arg(args, "new_text")?;
    let src = std::fs::read_to_string(&path)
        .map_err(|e| (ErrorCode::ExecError, format!("读取失败 {path:?}：{e}")))?;
    let count = src.split(old_text).count() - 1;
    if count != 1 {
        return Err((ErrorCode::BadArgs, format!("old_text 命中 {count} 次，必须恰好 1 次")));
    }
    let out = src.replacen(old_text, new_text, 1);
    std::fs::write(&path, out).map_err(|e| (ErrorCode::ExecError, format!("写回失败：{e}")))?;
    Ok(json!({ "edited": true }))
}
