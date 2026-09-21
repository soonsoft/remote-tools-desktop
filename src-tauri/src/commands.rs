//! UI → 后端命令（Tauri v2 invoke 契约，名称逐字）：
//! `get_config` / `save_config` / `resolve_confirm` / `recent_logs`。

use crate::confirm::{ConfirmAction, ConfirmBridge};
use crate::state::{AppState, LogEntry};
use remote_tools_core::config::AppConfig;
use std::sync::Arc;
use tauri::{AppHandle, Emitter, State};

#[tauri::command]
pub fn get_config(state: State<'_, Arc<AppState>>) -> AppConfig {
    state.config.read().unwrap().clone()
}

#[tauri::command]
pub fn save_config(state: State<'_, Arc<AppState>>, config: AppConfig) -> Result<(), String> {
    config.save(&state.config_path).map_err(|e| e.to_string())?;
    *state.config.write().unwrap() = config;
    // 简化（brief 原样）：保存后需重启应用生效——隧道任务持有旧 cfg，热重载留后续。
    Ok(())
}

/// 确认三动作的送达入口；成功后广播 confirm-resolved 并刷新托盘三态。
#[tauri::command]
pub fn resolve_confirm(
    app: AppHandle,
    state: State<'_, Arc<AppState>>,
    bridge: State<'_, Arc<ConfirmBridge>>,
    id: String,
    action: String,
) -> Result<(), String> {
    let action = match action.as_str() {
        "once" => ConfirmAction::Once,
        "session" => ConfirmAction::Session,
        "deny" => ConfirmAction::Deny,
        other => return Err(format!("未知动作 {other}")),
    };
    bridge.resolve(&id, action)?;
    let _ = app.emit("confirm-resolved", serde_json::json!({ "id": id }));
    crate::app::refresh_tray(&app, &state, &bridge);
    Ok(())
}

/// 最近 100 条活动日志（时间倒序）。
#[tauri::command]
pub fn recent_logs(state: State<'_, Arc<AppState>>) -> Vec<LogEntry> {
    let logs = state.logs.lock().unwrap();
    logs.iter().rev().take(100).cloned().collect()
}

/// 当前隧道状态（webview 启动补拉——`tunnel-status` 事件可能在 listener
/// 就绪前发出，2026-09-21 竞态实锤：应用已连接而横幅停在初始灰态）。
#[tauri::command]
pub fn get_status(state: State<'_, Arc<AppState>>) -> serde_json::Value {
    serde_json::json!({
        "connected": state.connected.load(std::sync::atomic::Ordering::Relaxed),
        "reason": state.last_disconnect_reason.lock().unwrap().clone(),
    })
}
