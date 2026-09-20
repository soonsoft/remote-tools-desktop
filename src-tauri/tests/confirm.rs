use remote_tools_desktop::confirm::{ConfirmAction, ConfirmBridge};

#[tokio::test]
async fn resolve_delivers_action_and_drops_missing() {
    let bridge = ConfirmBridge::default();
    let (tx, rx) = tokio::sync::oneshot::channel();
    bridge.register("c1".into(), tx);
    assert!(bridge.resolve("c1", ConfirmAction::Deny).is_ok());
    assert_eq!(rx.await.unwrap(), ConfirmAction::Deny);
    // 重复 resolve 同 id：已不在表 → Err
    assert!(bridge.resolve("c1", ConfirmAction::Once).is_err());
}

// 断线清扫（spec §11）：deny_all 把所有待确认请求一律按 Deny 送达并清空，
// 托盘琥珀态（is_empty）随之解除。
#[tokio::test]
async fn deny_all_resolves_every_pending_with_deny() {
    let bridge = ConfirmBridge::default();
    let (tx1, rx1) = tokio::sync::oneshot::channel();
    let (tx2, rx2) = tokio::sync::oneshot::channel();
    bridge.register("c1".into(), tx1);
    bridge.register("c2".into(), tx2);
    bridge.deny_all();
    assert!(bridge.is_empty());
    assert_eq!(rx1.await.unwrap(), ConfirmAction::Deny);
    assert_eq!(rx2.await.unwrap(), ConfirmAction::Deny);
    // 空表上再 deny_all：无害
    bridge.deny_all();
    assert!(bridge.is_empty());
}
