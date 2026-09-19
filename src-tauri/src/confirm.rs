//! 确认桥：隧道任务里的待确认请求 ↔ UI 命令之间的单次握手。
//!
//! 落地说明（brief 示意代码与测试相互矛盾处的裁定）：`register`/`resolve`
//! 均为同步方法，`pending` 用 `std::sync::Mutex`——
//! 1. brief 的测试同步调用 `register(...)`，要求其为非 async；
//! 2. `blocking_lock` 在 `#[tokio::test]` / tauri 异步命令所在的 tokio
//!    运行时内会 panic，而 std 锁在「持锁不跨 await」前提下完全安全
//!    （临界区只有 map 增删，无 await）。

use std::collections::HashMap;
use std::sync::Mutex;
use tokio::sync::oneshot;

/// 用户对一次确认请求的裁决。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    /// 允许一次（仅本请求）。
    Once,
    /// 本会话内允许同类（同工具 + 同危险级别；断线/重连清空）。
    Session,
    /// 拒绝。
    Deny,
}

/// 请求 id → 待送达裁决的 oneshot 发送端。
#[derive(Default)]
pub struct ConfirmBridge {
    pub pending: Mutex<HashMap<String, oneshot::Sender<ConfirmAction>>>,
}

impl ConfirmBridge {
    /// 注册一个待确认请求（UiExecutor 在发出 confirm-request 事件后调用）。
    pub fn register(&self, id: String, tx: oneshot::Sender<ConfirmAction>) {
        self.pending.lock().unwrap().insert(id, tx);
    }

    /// 送达裁决；id 不存在（已处理/已取消）或接收端已取消时返回 Err。
    pub fn resolve(&self, id: &str, action: ConfirmAction) -> Result<(), String> {
        let mut map = self.pending.lock().unwrap();
        let tx = map.remove(id).ok_or_else(|| format!("无待确认项 {id}"))?;
        tx.send(action).map_err(|_| "接收端已取消".to_string())
    }

    /// 是否还有待确认项（托盘琥珀态的判据）。
    pub fn is_empty(&self) -> bool {
        self.pending.lock().unwrap().is_empty()
    }
}
