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

/// 多根围栏端到端（修复：此前执行器只认第一个根，落在第二个根下的目标
/// 全部 path_denied）：两个 allowRoot A/B，B 下写+读成功；从 B `../` 逃出
/// 两个根 → path_denied。
#[tokio::test]
async fn multi_root_write_read_under_second_root_and_escape_denied() {
    let tmp = std::env::temp_dir();
    let a = tmp.join(format!("rt-multi-a-{}", ulid::Ulid::new()));
    let b = tmp.join(format!("rt-multi-b-{}", ulid::Ulid::new()));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let cfg = AppConfig {
        allow_roots: vec![a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()],
        ..Default::default()
    };
    let exec = Arc::new(DirectExecutor::new(cfg));
    // Write 分类属 Confirm → 直连层视为 DeniedByUser；预放行 (Write, danger=false)
    exec.session.lock().await.insert_allow(ToolName::Write, false);
    let mk = |tool: ToolName, args: Value| ToolRequest {
        id: "r1".into(), tool, args, meta: Meta { requested_at: 0 },
    };
    let target = b.join("in_b.txt");
    let target_str = target.to_string_lossy().into_owned();
    // 写入 + 读回 B（第二个根）
    exec.execute(&mk(ToolName::Write,
        json!({"path": target_str, "content": "hello"}))).await.unwrap();
    let v = exec.execute(&mk(ToolName::Read, json!({"path": target_str}))).await.unwrap();
    assert_eq!(v["content"], json!("hello"));
    // `../` 从 B 逃出所有根（temp 父目录）→ 规则/执行器双层围栏 path_denied
    let escape = b.join("..").join("escaped-outside-both-roots.txt");
    let err = exec.execute(&mk(ToolName::Write,
        json!({"path": escape.to_string_lossy(), "content": "x"}))).await.unwrap_err();
    assert_eq!(err.0, ErrorCode::PathDenied);
}

// ---- 断线取消在途执行（spec §11） ----

struct CancelFlag(Arc<std::sync::atomic::AtomicBool>);
impl Drop for CancelFlag {
    fn drop(&mut self) { self.0.store(true, std::sync::atomic::Ordering::SeqCst); }
}

/// 在途探测执行器：execute 先置 started，睡 30s；future 被 abort/drop 时
/// CancelFlag 置 cancelled（模拟 run_bash 的 kill_on_drop 连带终止）。
struct AbortProbeExecutor {
    started: Arc<std::sync::atomic::AtomicBool>,
    cancelled: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl Executor for AbortProbeExecutor {
    async fn execute(&self, _req: &ToolRequest) -> Result<Value, (ErrorCode, String)> {
        let _guard = CancelFlag(self.cancelled.clone());
        self.started.store(true, std::sync::atomic::Ordering::SeqCst);
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(json!({}))
    }
}

/// 握手完成后发 tool_request，200ms 后直接掐断连接（不收响应）。
// tungstenite 0.24 Callback 的 Err = http::Response<Option<String>>（136B），类型由 trait 固定。
#[allow(clippy::result_large_err)]
async fn aborting_server(token: &str) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let token = token.to_string();
    tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(stream, auth_callback(token)).await.unwrap();
        let hello = ws.next().await.unwrap().unwrap();
        assert!(matches!(parse_envelope(hello.to_text().unwrap()).unwrap(), Envelope::Hello(_)));
        ws.send(tungstenite::Message::text(encode_envelope(&Envelope::HelloAck))).await.unwrap();
        let req = Envelope::ToolRequest(serde_json::from_value(json!({
            "v":1,"type":"tool_request","id":"abort1","tool":"client__read",
            "args":{"path":"x.txt"},"meta":{"requestedAt":0}})).unwrap());
        ws.send(tungstenite::Message::text(encode_envelope(&req))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(200)).await;
        drop(ws); // 断开
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn disconnect_aborts_in_flight_execution() {
    let url = aborting_server("tok123").await;
    let cfg = AppConfig { server_url: url, token: "tok123".into(), ..Default::default() };
    let exec = Arc::new(AbortProbeExecutor {
        started: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    let (etx, mut erx) = mpsc::channel(16);
    let (_sd, srx) = watch::channel(false);
    let handle = tokio::spawn(run_tunnel(cfg, exec.clone(), etx, srx));
    // 等在途执行真的开始
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while !exec.started.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "executor 未被调用");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Disconnected 应在 ~200ms 到达，远早于 30s 的在途执行结束（teardown 不被
    // 在途执行阻塞；排在前面的 Connected 等事件直接略过）
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let ev = tokio::time::timeout_at(deadline, erx.recv()).await
            .expect("1s 内未收到 Disconnected").unwrap();
        if let TunnelEvent::Disconnected(_) = ev { break; }
    }
    // 在途 future 已被 abort → 调度器下一轮 drop → CancelFlag 置位
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while !exec.cancelled.load(std::sync::atomic::Ordering::SeqCst) {
        assert!(std::time::Instant::now() < deadline, "在途 future 未被 abort/drop");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    handle.abort();
}

/// 退避复位在入睡**之前**生效（修复：复位原先写在 sleep 之后，带失败史的
/// 健康会话闪断后会错误等待上一轮累积的退避）。时序：轮1 拨号失败（退避
/// 累积 1→2s）→ 轮2 完整会话建立后闪断 → 轮3 连接应在 ~1s 到来（<1.9s）。
#[tokio::test]
async fn healthy_session_reconnect_resets_backoff_before_sleep() {
    let listener = Arc::new(tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap());
    let addr = listener.local_addr().unwrap();
    let token = "tok123".to_string();
    let srv = listener.clone();
    let (round2_done, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        // 轮 1：接受 TCP 即断 → ws 握手失败 → established=false，退避累积到 2s
        let (s, _) = srv.accept().await.unwrap();
        drop(s);
        // 轮 2：完整握手 + hello_ack → established=true，随后闪断
        let (s, _) = srv.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_hdr_async(s, auth_callback(token)).await.unwrap();
        let hello = ws.next().await.unwrap().unwrap();
        assert!(matches!(parse_envelope(hello.to_text().unwrap()).unwrap(), Envelope::Hello(_)));
        ws.send(tungstenite::Message::text(encode_envelope(&Envelope::HelloAck))).await.unwrap();
        drop(ws);
        let _ = round2_done.send(());
    });
    let cfg = AppConfig {
        server_url: format!("ws://{addr}"), token: "tok123".into(), ..Default::default()
    };
    let exec = Arc::new(DirectExecutor::new(cfg.clone()));
    let (etx, _erx) = mpsc::channel(16);
    let (_sd, srx) = watch::channel(false);
    let handle = tokio::spawn(run_tunnel(cfg, exec, etx, srx));
    // 等轮 2 闪断（断言主体内完成，让轮 3 的计时窗从这里起算）
    tokio::time::timeout(Duration::from_secs(10), rx).await.unwrap().unwrap();
    // 轮 3：退避必须已复位为 1s → 1.9s 内必须发起第三次连接（旧行为为 2s+）
    let round3 = tokio::time::timeout(Duration::from_millis(1900), listener.accept()).await;
    assert!(round3.is_ok(), "健康会话断开后退避应复位为 1s（~1s 内重连）");
    handle.abort();
}
