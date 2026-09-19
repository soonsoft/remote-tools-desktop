use crate::envelope::{ErrorCode, ToolName};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Confirm { reason: String, danger: bool },
    Deny { code: ErrorCode, reason: String },
}

#[derive(Debug, Clone)]
pub struct RulesConfig {
    pub allow_roots: Vec<PathBuf>,
    pub dangerous: Vec<String>,
    pub tool_overrides: HashMap<String, String>,
}

#[derive(Debug, Default)]
pub struct SessionRules {
    allowed: Vec<(ToolName, bool)>,
}

impl SessionRules {
    pub fn insert_allow(&mut self, tool: ToolName, danger: bool) {
        self.allowed.push((tool, danger));
    }
    pub fn allows(&self, tool: ToolName, danger: bool) -> bool {
        self.allowed.contains(&(tool, danger))
    }
    pub fn clear(&mut self) { self.allowed.clear(); }
    pub fn is_empty(&self) -> bool { self.allowed.is_empty() }
}

pub fn dangerous_defaults() -> Vec<String> {
    [
        r"rm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r)", // rm -rf / rm -fr
        r"rm\s+-r", r"Remove-Item\s+.*-Recurse", r"rd\s+/s", r"del\s+/[sq]",
        r"format\s+[a-z]:", r"reg\s+(add|delete)", r"sudo\b",
        r"(curl|wget)[^|]*\|\s*(sh|bash|iex|pwsh)",
    ].into_iter().map(String::from).collect()
}

pub fn within_root(root: &Path, target: &Path) -> bool {
    let rel = target.strip_prefix(root);
    match rel {
        Ok(r) => !r.to_string_lossy().split(['/', '\\']).any(|seg| seg == ".."),
        Err(_) => false,
    }
}

pub fn args_path(args: &Value) -> Option<String> {
    args.get("path").and_then(Value::as_str).map(String::from)
}

/// 决策链：会话规则 → 工具 override（deny）→ 路径围栏 → 分类（含危险模式）。
pub fn decide(tool: ToolName, args: &Value, cfg: &RulesConfig, session: &SessionRules) -> Decision {
    let danger = is_dangerous(args, cfg);
    if session.allows(tool, danger) {
        return Decision::Allow;
    }
    match cfg.tool_overrides.get(tool.wire()).map(String::as_str) {
        Some("deny") => return Decision::Deny {
            code: ErrorCode::DeniedByRule,
            reason: format!("{} 被客户端规则拒绝；建议换一种方式或询问用户调整规则", tool.wire()),
        },
        Some("auto") => return Decision::Allow,
        _ => {}
    }
    // 路径围栏：文件类工具（read/write/edit/grep/glob 的 path 参数）
    if let Some(p) = args_path(args) {
        let target = canonical_against_roots(&p, cfg);
        match target {
            Some(t) if cfg.allow_roots.iter().any(|r| within_root(r, &t)) => {}
            _ => return Decision::Deny {
                code: ErrorCode::PathDenied,
                reason: format!("目标路径不在白名单内：{p}"),
            },
        }
    }
    match tool {
        ToolName::Read | ToolName::Grep | ToolName::Glob => Decision::Allow,
        ToolName::Write | ToolName::Edit => Decision::Confirm {
            reason: confirm_reason(tool, args), danger: false,
        },
        ToolName::Bash => Decision::Confirm { reason: bash_summary(args), danger },
    }
}

fn is_dangerous(args: &Value, cfg: &RulesConfig) -> bool {
    let Some(cmd) = args.get("command").and_then(Value::as_str) else { return false };
    cfg.dangerous.iter().any(|p| regex::Regex::new(p).map(|r| r.is_match(cmd)).unwrap_or(false))
}

fn confirm_reason(tool: ToolName, args: &Value) -> String {
    match args_path(args) {
        Some(p) => format!("{} 修改文件：{p}", tool.wire()),
        None => format!("{} 修改文件", tool.wire()),
    }
}

fn bash_summary(args: &Value) -> String {
    let cmd = args.get("command").and_then(Value::as_str).unwrap_or("");
    let mut s: String = cmd.chars().take(200).collect();
    if cmd.chars().count() > 200 { s.push('…'); }
    s
}

/// 相对路径锚定到第一个 allowRoot 后规范化（词法层面；符号链接在威胁模型外）。
fn canonical_against_roots(raw: &str, cfg: &RulesConfig) -> Option<PathBuf> {
    let p = Path::new(raw);
    let anchored = if p.is_absolute() { p.to_path_buf() }
        else { cfg.allow_roots.first()?.join(p) };
    // 词法规范化消除 ".."
    let mut out = PathBuf::new();
    for c in anchored.components() {
        match c {
            std::path::Component::ParentDir => { out.pop(); }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}
