use futures_util::{SinkExt, StreamExt};
use remote_tools_core::config::AppConfig;
use remote_tools_core::envelope::*;
use remote_tools_core::tunnel::*;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// 进程内测试服务器：校验 Bearer，hello→hello_ack，按脚本回放 tool_request 并收集响应。
// tungstenite 0.24 Callback 的 Err = http::Response<Option<String>>（136B），类型由 trait 固定。
#[allow(clippy::result_large_err)]
async fn test_server(token: &str) -> (String, mpsc::Receiver<Envelope>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel(16);
    let token = token.to_string();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(
            stream,
            // 适配（tungstenite 0.24 编译器纠偏）：回调签名 =
            // FnOnce(&Request, Response) -> Result<Response, ErrorResponse>，
            // 其中 ErrorResponse = http::Response<Option<String>>；
            // 拒答时把状态改为 401（语义不变：错误 token → 非 101 → 握手失败）。
            |req: &tungstenite::http::Request<()>, resp: tungstenite::http::Response<()>| {
                let ok = req.headers().get("authorization")
                    .and_then(|v| v.to_str().ok()) == Some(format!("Bearer {token}").as_str());
                if ok {
                    Ok(resp)
                } else {
                    let (mut parts, ()) = resp.into_parts();
                    parts.status = tungstenite::http::StatusCode::UNAUTHORIZED;
                    Err(tungstenite::http::Response::<Option<String>>::from_parts(parts, None))
                }
            },
        ).await.unwrap();
        // hello → hello_ack
        let hello = ws.next().await.unwrap().unwrap();
        let env = parse_envelope(hello.to_text().unwrap()).unwrap();
        assert!(matches!(env, Envelope::Hello(_)));
        ws.send(tungstenite::Message::text(encode_envelope(&Envelope::HelloAck))).await.unwrap();
        // 发一个 tool_request
        let req = Envelope::ToolRequest(serde_json::from_value(json!({
            "v":1,"type":"tool_request","id":"t1","tool":"client__bash",
            "args":{"command":"echo TUNNEL_OK"},"meta":{"requestedAt":0}})).unwrap());
        ws.send(tungstenite::Message::text(encode_envelope(&req))).await.unwrap();
        // ping → 期待 pong
        ws.send(tungstenite::Message::text(encode_envelope(&Envelope::Ping))).await.unwrap();
        for _ in 0..2 {
            let msg = tokio::time::timeout(Duration::from_secs(5), ws.next()).await.unwrap().unwrap().unwrap();
            if let Some(env) = parse_envelope(msg.to_text().unwrap()) {
                let _ = tx.send(env).await;
            }
        }
    });
    (format!("ws://{addr}"), rx)
}

#[tokio::test]
async fn connects_auths_executes_and_pongs() {
    let (url, mut rx) = test_server("tok123").await;
    let root = std::env::temp_dir().join(format!("rt-tunnel-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&root).unwrap();
    let cfg = AppConfig {
        server_url: url, token: "tok123".into(),
        allow_roots: vec![root.to_string_lossy().into_owned()],
        ..Default::default()
    };
    let exec = Arc::new(DirectExecutor::new(cfg.clone()));
    // 适配（语义必需）：DirectExecutor 对 Confirm 一律 DeniedByUser（无 UI 场景），
    // 而未预置会话规则时 bash 属于 Confirm —— 预放行 bash 才能走到“执行”分支。
    exec.session.lock().await.insert_allow(ToolName::Bash, false);
    let (etx, mut erx) = mpsc::channel(16);
    let (_sd, srx) = watch::channel(false);
    let handle = tokio::spawn(run_tunnel(cfg, exec, etx, srx));
    // 收到两条：tool_response(ok) + pong
    let first = rx.recv().await.unwrap();
    match first {
        Envelope::ToolResponse(ToolResponse::Ok { result, .. }) =>
            assert!(result["stdout"].as_str().unwrap().contains("TUNNEL_OK"), "{result}"),
        other => panic!("expected ok response, got {other:?}"),
    }
    assert!(matches!(rx.recv().await.unwrap(), Envelope::Pong));
    assert!(matches!(erx.recv().await.unwrap(), TunnelEvent::Connected));
    handle.abort();
}

#[tokio::test]
async fn bad_token_reconnects_with_backoff_and_reports_disconnect() {
    // 服务器要求错误 token；客户端应收到 Disconnected 事件并持续重试（不 panic）
    let (url, _rx) = test_server("right").await;
    let cfg = AppConfig { server_url: url, token: "wrong".into(), ..Default::default() };
    let exec = Arc::new(DirectExecutor::new(cfg.clone()));
    let (etx, mut erx) = mpsc::channel(16);
    let (_sd, srx) = watch::channel(false);
    let handle = tokio::spawn(run_tunnel(cfg, exec, etx, srx));
    // 401 后服务器关连接 → Disconnected 事件
    let ev = tokio::time::timeout(Duration::from_secs(5), erx.recv()).await.unwrap().unwrap();
    assert!(matches!(ev, TunnelEvent::Disconnected(_)));
    handle.abort();
}
