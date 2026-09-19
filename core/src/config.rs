use crate::exec::bash::ShellKind;
use crate::rules::{dangerous_defaults, RulesConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;

pub const DEFAULT_MAX_OUTPUT_CHARS: usize = 100_000;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    pub server_url: String,
    pub token: String,
    pub shell: String, // "powershell"|"pwsh"|"cmd"|"sh"|自定义可执行路径；空 = 按平台默认
    pub allow_roots: Vec<String>,
    pub dangerous: Option<Vec<String>>,
    pub tool_overrides: Option<HashMap<String, String>>,
    pub max_output_chars: usize,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            server_url: String::new(),
            token: String::new(),
            shell: String::new(),
            allow_roots: vec![],
            dangerous: None,
            tool_overrides: None,
            max_output_chars: DEFAULT_MAX_OUTPUT_CHARS,
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() { std::fs::create_dir_all(dir)?; }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap())
    }
    pub fn rules(&self) -> RulesConfig {
        RulesConfig {
            allow_roots: self.allow_roots.iter().map(Into::into).collect(),
            dangerous: self.dangerous.clone().unwrap_or_else(dangerous_defaults),
            tool_overrides: self.tool_overrides.clone().unwrap_or_default(),
        }
    }
    pub fn shell_kind(&self) -> ShellKind {
        match self.shell.as_str() {
            "powershell" | "" if cfg!(windows) => ShellKind::Powershell,
            "pwsh" => ShellKind::Pwsh,
            "cmd" => ShellKind::Cmd,
            "sh" | "" => ShellKind::Sh,
            other => ShellKind::Custom(other.to_string()),
        }
    }
}

/// 超限截断 + 落盘（spec §6 修订：客户端执行截断；spill 目录 = 目标 allowRoot 下 .remote-tools/）。
pub fn truncate_result(result: &mut Value, max_chars: usize, spill_dir: &Path, request_id: &str) {
    for key in ["stdout", "stderr", "content"] {
        let Some(s) = result.get(key).and_then(Value::as_str) else { continue };
        if s.chars().count() <= max_chars { continue; }
        let spill_path = spill_dir.join(format!("{request_id}.txt"));
        if std::fs::create_dir_all(spill_dir).is_ok() && std::fs::write(&spill_path, s).is_ok() {
            let head: String = s.chars().take(max_chars).collect();
            if let Some(obj) = result.as_object_mut() {
                obj.insert(key.to_string(), Value::String(head));
                obj.insert("truncated".into(), Value::Bool(true));
                obj.insert("spillPath".into(), Value::String(spill_path.to_string_lossy().into_owned()));
            }
        }
        return; // 单字段截断即整结果标记
    }
}
