use crate::config::{truncate_result, AppConfig};
use crate::envelope::*;
use crate::exec::bash::run_bash;
use crate::exec::file;
use crate::exec::search;
use crate::rules::{decide, Decision, SessionRules};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch, Mutex};

#[derive(Debug)]
pub enum TunnelEvent { Connected, Disconnected(String), Log(String), ConfirmPending(String) }

#[async_trait::async_trait]
pub trait Executor: Send + Sync {
    async fn execute(&self, req: &ToolRequest) -> Result<Value, (ErrorCode, String)>;
}

fn tool_timeout(tool: ToolName, args: &Value) -> Duration {
    match tool {
        ToolName::Bash => Duration::from_millis(
            args.get("timeout_ms").and_then(Value::as_u64).unwrap_or(120_000).min(600_000)),
        ToolName::Read | ToolName::Write | ToolName::Edit => Duration::from_secs(10),
        ToolName::Grep | ToolName::Glob => Duration::from_secs(30),
    }
}

fn root_of(cfg: &AppConfig) -> Result<PathBuf, (ErrorCode, String)> {
    cfg.allow_roots.first().cloned().map(PathBuf::from)
        .ok_or((ErrorCode::PathDenied, "客户端未配置 allowRoots".into()))
}

/// 直连执行器：规则放行即执行；Confirm 视为 DeniedByUser（无 UI 场景）。
pub struct DirectExecutor { pub cfg: AppConfig, pub session: Mutex<SessionRules> }

impl DirectExecutor {
    pub fn new(cfg: AppConfig) -> Self {
        Self { cfg, session: Mutex::new(SessionRules::default()) }
    }
}

#[async_trait::async_trait]
impl Executor for DirectExecutor {
    async fn execute(&self, req: &ToolRequest) -> Result<Value, (ErrorCode, String)> {
        let rules = self.cfg.rules();
        let d = decide(req.tool, &req.args, &rules, &*self.session.lock().await);
        // (root, spill_root) 均为第一个 allowRoot（brief 原样转录）。
        let (root, spill_root) = match d {
            Decision::Allow => (root_of(&self.cfg)?, root_of(&self.cfg)?),
            Decision::Confirm { reason, .. } =>
                return Err((ErrorCode::DeniedByUser, format!("需要确认：{reason}"))),
            Decision::Deny { code, reason } => return Err((code, reason)),
        };
        // block_in_place：同步执行器跑在异步上下文里（tokio 多线程运行时下安全）。
        let outcome = match req.tool {
            ToolName::Bash => {
                let cmd = req.args.get("command").and_then(Value::as_str)
                    .ok_or((ErrorCode::BadArgs, "缺少 command".into()))?;
                let out = run_bash(cmd, tool_timeout(ToolName::Bash, &req.args), &root,
                    &self.cfg.shell_kind()).await;
                serde_json::to_value(&out).unwrap()
            }
            ToolName::Read => tokio::task::block_in_place(|| file::read_file(&root, &req.args))?,
            ToolName::Write => tokio::task::block_in_place(|| file::write_file(&root, &req.args))?,
            ToolName::Edit => tokio::task::block_in_place(|| file::edit_file(&root, &req.args))?,
            ToolName::Grep => tokio::task::block_in_place(|| search::grep(&root, &req.args))?,
            ToolName::Glob => tokio::task::block_in_place(|| search::glob_search(&root, &req.args))?,
        };
        let mut value = outcome;
        truncate_result(&mut value, self.cfg.max_output_chars,
            &spill_root.join(".remote-tools"), &req.id);
        Ok(value)
    }
}

pub async fn run_tunnel(
    cfg: AppConfig,
    executor: Arc<dyn Executor>,
    events: mpsc::Sender<TunnelEvent>,
    mut shutdown: watch::Receiver<bool>,
) {
    let mut backoff = Duration::from_secs(1);
    loop {
        if *shutdown.borrow() { return; }
        let mut established = false;
        match connect_and_serve(&cfg, executor.clone(), &events, &mut shutdown, &mut established).await {
            Ok(()) => return, // shutdown
            Err(e) => {
                let _ = events.send(TunnelEvent::Disconnected(e)).await;
                let _ = events.send(TunnelEvent::Log(
                    format!("{} 后重连（退避 {:?}）", backoff.as_secs(), backoff))).await;
                if tokio::time::timeout(backoff, shutdown.changed()).await.is_err() {
                    // 退避计时到，继续重连
                }
                // 适配（brief 全局约束「成功复位」）：会话曾建立过 → 退避复位 1s；
                // 从未建立（拨号/鉴权失败）→ 继续指数增长，60s 封顶。
                if established { backoff = Duration::from_secs(1); }
                else { backoff = (backoff * 2).min(Duration::from_secs(60)); }
            }
        }
    }
}

async fn connect_and_serve(
    cfg: &AppConfig,
    executor: Arc<dyn Executor>,
    events: &mpsc::Sender<TunnelEvent>,
    shutdown: &mut watch::Receiver<bool>,
    established: &mut bool,
) -> Result<(), String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut request = cfg.server_url.clone().into_client_request().map_err(|e| e.to_string())?;
    let auth: tokio_tungstenite::tungstenite::http::HeaderValue =
        format!("Bearer {}", cfg.token).parse()
            .map_err(|_| "配对令牌含非法字符，无法用于鉴权头".to_string())?;
    request.headers_mut().insert("authorization", auth);
    let (ws, _resp) = tokio_tungstenite::connect_async(request)
        .await.map_err(|e| format!("连接失败：{e}"))?;
    let (mut sink, mut stream) = ws.split();
    sink.send(tokio_tungstenite::tungstenite::Message::text(encode_envelope(&Envelope::Hello(Hello {
        hostname: hostname_string(), platform: std::env::consts::OS.to_string(),
    })))).await.map_err(|e| e.to_string())?;
    // 等 hello_ack（Connected 之前保持顺序执行）
    match stream.next().await {
        Some(Ok(msg)) => {
            match parse_envelope(msg.to_text().unwrap_or("")) {
                Some(Envelope::HelloAck) => {}
                other => return Err(format!("期待 hello_ack，收到 {other:?}")),
            }
        }
        _ => return Err("等待 hello_ack 时断开".into()),
    }
    let _ = events.send(TunnelEvent::Connected).await;
    *established = true;
    // 出站队列 + select 循环：tool_request 在独立任务里执行，
    // 执行期间 ping→pong 与其他出站消息不被阻塞（心跳不被长命令饿死）。
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(16);
    let reason = loop {
        if *shutdown.borrow() { return Ok(()); }
        tokio::select! {
            msg = stream.next() => {
                // 流结束（None）也要 break，否则重连循环会卡死在死套接字上
                let Some(msg) = msg else { break "连接关闭".to_string(); };
                let msg = match msg { Ok(m) => m, Err(e) => break e.to_string() };
                let Some(env) = parse_envelope(msg.to_text().unwrap_or("")) else { continue };
                match env {
                    Envelope::Ping => {
                        let _ = tx.send(Envelope::Pong).await;
                    }
                    Envelope::ToolRequest(req) => {
                        let exec = executor.clone();
                        let tx = tx.clone();
                        let events = events.clone();
                        let id = req.id.clone();
                        tokio::spawn(async move {
                            let out = exec.execute(&req).await;
                            // bash 超时（run_bash 杀掉子进程 → killed:true）→ 回 timeout 而非 ok 结果（spec §11）
                            let resp = match out {
                                Ok(result) if result.get("killed").and_then(Value::as_bool) == Some(true) =>
                                    Envelope::ToolResponse(ToolResponse::Err {
                                        id: id.clone(), error: ToolError {
                                            code: ErrorCode::Timeout,
                                            message: "命令执行超时被终止（killed）".into() } }),
                                Ok(result) => Envelope::ToolResponse(ToolResponse::Ok { id: id.clone(), result }),
                                Err((code, message)) => Envelope::ToolResponse(ToolResponse::Err {
                                    id: id.clone(), error: ToolError { code, message } }),
                            };
                            let _ = tx.send(resp).await;
                            let _ = events.send(TunnelEvent::Log(format!("{id} 完成"))).await;
                        });
                    }
                    _ => {}
                }
            }
            Some(env) = rx.recv() => {
                if let Err(e) = sink.send(tokio_tungstenite::tungstenite::Message::text(
                    encode_envelope(&env))).await {
                    break format!("发送失败：{e}");
                }
            }
            changed = shutdown.changed() => {
                // changed() 报错 = shutdown 发送端已消失，永远不会再来信号 → 一并视为退出
                if changed.is_err() || *shutdown.borrow_and_update() { return Ok(()); }
            }
        }
    };
    Err(reason)
}

fn hostname_string() -> String {
    std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "unknown".into())
}
