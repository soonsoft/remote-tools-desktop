//! 壳层共享状态。
//!
//! 落地指令对齐：`config` 用 `std::sync::RwLock<AppConfig>`、
//! `session_rules` 用 tokio `Mutex<SessionRules>`（`.lock().await`），
//! 无 parking_lot。`logs` 亦用 std 锁（临界区极短、不跨 await），
//! 使 `recent_logs` 命令可以是同步命令。`tunnel_shutdown` 持有隧道
//! 停机信号的发送端——若在 setup 里丢弃，断线退避等待会立即返回，
//! 重连循环将空转，故必须随 AppState 存活到进程结束。

use remote_tools_core::config::AppConfig;
use remote_tools_core::rules::SessionRules;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Mutex, RwLock};
use tokio::sync::watch;

pub struct AppState {
    pub config_path: PathBuf,
    pub config: RwLock<AppConfig>,
    /// 近端活动日志（工具执行 + 隧道诊断），上限 [`crate::app::MAX_LOGS`] 条。
    pub logs: Mutex<Vec<LogEntry>>,
    /// 会话内「允许同类」规则；Connected/Disconnected 时在事件泵清空。
    pub session_rules: tokio::sync::Mutex<SessionRules>,
    /// 隧道当前是否已连接（托盘绿/灰判据）。
    pub connected: AtomicBool,
    /// 最近一次断开原因（Connected 时清空）；`get_status` 供 webview 启动时
    /// 补拉当前状态——`tunnel-status` 事件可能在 listener 就绪前发出（竞态实锤 2026-09-21）。
    pub last_disconnect_reason: std::sync::Mutex<Option<String>>,
    /// 隧道停机信号发送端（v1 退出为硬退出，信号保留以备优雅停机）。
    pub tunnel_shutdown: watch::Sender<bool>,
}

/// 一条活动日志；`tool-log` 事件与 `recent_logs()` 命令共用此形状。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub id: String,
    pub tool: String,
    pub summary: String,
    pub ok: bool,
    pub duration_ms: u64,
    pub confirmed: bool,
}
