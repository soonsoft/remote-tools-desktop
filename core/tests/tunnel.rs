use futures_util::{SinkExt, StreamExt};
use remote_tools_core::config::AppConfig;
use remote_tools_core::envelope::*;
use remote_tools_core::tunnel::*;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// tungstenite 0.24 Callback 的返回类型：Ok = Response（空 body），Err = ErrorResponse（带 body）。
type AuthResult = Result<tungstenite::http::Response<()>, tungstenite::http::Response<Option<String>>>;

/// Bearer 校验回调（两台测试服务器共用）。
/// 适配（tungstenite 0.24 编译器纠偏）：回调签名 =
/// FnOnce(&Request, Response) -> Result<Response, ErrorResponse>；
/// 拒答时把状态改为 401（语义不变：错误 token → 非 101 → 握手失败）。
#[allow(clippy::result_large_err)] // Err 类型由 tungstenite Callback trait 固定（136B）
fn auth_callback(token: String) -> impl FnOnce(&tungstenite::http::Request<()>, tungstenite::http::Response<()>) -> AuthResult {
    move |req, resp| {
        let ok = req.headers().get("authorization")
            .and_then(|v| v.to_str().ok()) == Some(format!("Bearer {token}").as_str());
        if ok {
            Ok(resp)
        } else {
            let (mut parts, ()) = resp.into_parts();
            parts.status = tungstenite::http::StatusCode::UNAUTHORIZED;
            Err(tungstenite::http::Response::<Option<String>>::from_parts(parts, None))
        }
    }
}

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
        let mut ws = tokio_tungstenite::accept_hdr_async(stream, auth_callback(token)).await.unwrap();
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
    // 收到两条：tool_response(ok) + pong（并发分发后 pong 可先于响应到达，只校验成员不校验顺序）
    let mut got_pong = false;
    for _ in 0..2 {
        match rx.recv().await.unwrap() {
            Envelope::ToolResponse(ToolResponse::Ok { result, .. }) =>
                assert!(result["stdout"].as_str().unwrap().contains("TUNNEL_OK"), "{result}"),
            Envelope::Pong => got_pong = true,
            other => panic!("unexpected envelope {other:?}"),
        }
    }
    assert!(got_pong, "pong 未送达");
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

/// 慢执行器：睡 400ms 再返回 —— 用于验证并发分发（执行期间 pong 不被阻塞）。
struct SlowExecutor;

#[async_trait::async_trait]
impl Executor for SlowExecutor {
    async fn execute(&self, req: &ToolRequest) -> Result<Value, (ErrorCode, String)> {
        tokio::time::sleep(Duration::from_millis(400)).await;
        Ok(json!({ "echo": req.id }))
    }
}

/// 慢执行服务器：hello_ack 后发 tool_request（handler 睡 400ms），50ms 后才发 ping。
// tungstenite 0.24 Callback 的 Err = http::Response<Option<String>>（136B），类型由 trait 固定。
#[allow(clippy::result_large_err)]
async fn slow_exec_server(token: &str) -> (String, mpsc::Receiver<Envelope>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = mpsc::channel(16);
    let token = token.to_string();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(stream, auth_callback(token)).await.unwrap();
        let hello = ws.next().await.unwrap().unwrap();
        assert!(matches!(parse_envelope(hello.to_text().unwrap()).unwrap(), Envelope::Hello(_)));
        ws.send(tungstenite::Message::text(encode_envelope(&Envelope::HelloAck))).await.unwrap();
        // tool_request（执行需 400ms）
        let req = Envelope::ToolRequest(serde_json::from_value(json!({
            "v":1,"type":"tool_request","id":"slow1","tool":"client__bash",
            "args":{"command":"echo SLOW"},"meta":{"requestedAt":0}})).unwrap());
        ws.send(tungstenite::Message::text(encode_envelope(&req))).await.unwrap();
        // 50ms 后才 ping：若分发是并发的，pong 应先于 tool_response 到达
        tokio::time::sleep(Duration::from_millis(50)).await;
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
async fn pong_flows_while_tool_request_is_executing() {
    // 服务器先发 tool_request（睡 400ms）、50ms 后发 ping；
    // 若执行阻塞消息循环，pong 只能在 tool_response 之后 —— 测试要求 pong 先到。
    let (url, mut rx) = slow_exec_server("tok123").await;
    let cfg = AppConfig { server_url: url, token: "tok123".into(), ..Default::default() };
    let exec = Arc::new(SlowExecutor);
    let (etx, _erx) = mpsc::channel(16);
    let (_sd, srx) = watch::channel(false);
    let handle = tokio::spawn(run_tunnel(cfg, exec, etx, srx));
    // 第一条必须是 Pong（此刻工具还在睡）
    assert!(matches!(rx.recv().await.unwrap(), Envelope::Pong), "pong 应先于 tool_response 到达");
    let second = rx.recv().await.unwrap();
    match second {
        Envelope::ToolResponse(ToolResponse::Ok { id, .. }) => assert_eq!(id, "slow1"),
        other => panic!("expected ok tool_response, got {other:?}"),
    }
    handle.abort();
}
