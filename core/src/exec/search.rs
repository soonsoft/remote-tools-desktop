use crate::envelope::ErrorCode;
use crate::exec::file::resolve_in_root;
use crate::rules::within_root;
use serde_json::{json, Value};
use std::path::Path;

/// 深度 6、条目 2000 双上限的词法遍历；跳过 node_modules/.git。
fn walk_files(root: &Path, depth: usize, out: &mut Vec<std::path::PathBuf>) {
    if depth > 6 || out.len() > 2_000 { return; }
    let Ok(entries) = std::fs::read_dir(root) else { return };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if name == "node_modules" || name == ".git" { continue; }
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() { walk_files(&e.path(), depth + 1, out); }
        else { out.push(e.path()); }
    }
}

/// 极简 glob → 正则：`**` 跨段、`*` 单段内、`?` 单字符；其余按字面转义。
fn glob_to_regex(g: &str) -> regex::Regex {
    let mut re = String::from("^");
    let mut chars = g.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    if chars.peek() == Some(&'/') { chars.next(); }
                    re.push_str(".*");
                } else { re.push_str("[^/]*"); }
            }
            '?' => re.push_str("[^/]"),
            other => re.push_str(&regex::escape(&other.to_string())),
        }
    }
    re.push('$');
    regex::Regex::new(&re).expect("glob regex")
}

pub fn grep(root: &Path, args: &Value) -> Result<Value, (ErrorCode, String)> {
    let base = match args.get("path").and_then(Value::as_str) {
        Some(p) => resolve_in_root(root, p)?,
        None => root.to_path_buf(),
    };
    if !within_root(root, &base) {
        return Err((ErrorCode::PathDenied, "搜索根越界".into()));
    }
    let pattern = args.get("pattern").and_then(Value::as_str)
        .ok_or((ErrorCode::BadArgs, "缺少 pattern".into()))?;
    let re = regex::Regex::new(pattern).map_err(|e| (ErrorCode::BadArgs, format!("正则不合法：{e}")))?;
    let filter = args.get("glob").and_then(Value::as_str).map(glob_to_regex);
    let mut files = Vec::new();
    walk_files(&base, 0, &mut files);
    let mut matches = Vec::new();
    'outer: for f in files {
        if let Some(re_f) = &filter {
            if !re_f.is_match(&f.to_string_lossy()) { continue; }
        }
        let Ok(meta) = std::fs::metadata(&f) else { continue };
        if meta.len() > 2_000_000 { continue; }
        let Ok(content) = std::fs::read_to_string(&f) else { continue };
        for (i, line) in content.split('\n').enumerate() {
            if re.is_match(line) {
                matches.push(json!({ "file": f.to_string_lossy(), "line": i + 1, "text": line }));
                if matches.len() >= 200 { break 'outer; }
            }
        }
    }
    let truncated = matches.len() >= 200;
    Ok(json!({ "matches": matches, "truncated": truncated }))
}

pub fn glob_search(root: &Path, args: &Value) -> Result<Value, (ErrorCode, String)> {
    let base = match args.get("path").and_then(Value::as_str) {
        Some(p) => resolve_in_root(root, p)?,
        None => root.to_path_buf(),
    };
    let pattern = args.get("pattern").and_then(Value::as_str)
        .ok_or((ErrorCode::BadArgs, "缺少 pattern".into()))?;
    let re = glob_to_regex(pattern);
    let mut files = Vec::new();
    walk_files(&base, 0, &mut files);
    let mut paths: Vec<String> = files.iter()
        .map(|f| f.strip_prefix(&base).unwrap_or(f).to_string_lossy().replace('\\', "/"))
        .filter(|rel| re.is_match(rel))
        .collect();
    paths.truncate(500);
    let truncated = paths.len() >= 500;
    Ok(json!({ "paths": paths, "truncated": truncated }))
}
