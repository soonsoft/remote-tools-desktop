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
