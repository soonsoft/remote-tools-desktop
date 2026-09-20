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
use tokio::sync::{mpsc, watch, Mutex, Semaphore};

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

/// 五个文件类工具共享的同步签名（file::read_file / write_file / edit_file、
/// search::grep / glob_search）。
type FileOp = fn(&[PathBuf], &Value) -> Result<Value, (ErrorCode, String)>;

/// 文件类工具（Read/Write/Edit/Grep/Glob）的统一超时包装：同步执行体放到
/// blocking 池（spawn_blocking——`block_in_place` 与 timeout 无法组合：同步体
/// 阻塞轮询线程期间 timer 永远轮不到，超时档形同虚设），外面套 `tool_timeout`
/// 的 10s/30s 档；超时 → `timeout` 错误码（孤儿 blocking 任务随后自然结束并被
/// 丢弃）。此前这几档从未生效。时长作参数以便测试注入；档位语义在 `mod tests`
/// 里于 helper 层锁死。
async fn timed_file_op(
    timeout: Duration,
    roots: Vec<PathBuf>,
    args: Value,
    op: FileOp,
) -> Result<Value, (ErrorCode, String)> {
    let fut = tokio::task::spawn_blocking(move || op(&roots, &args));
    match tokio::time::timeout(timeout, fut).await {
        Ok(Ok(outcome)) => outcome,
        Ok(Err(e)) => Err((ErrorCode::ExecError, format!("执行任务崩溃：{e}"))),
        Err(_) => Err((ErrorCode::Timeout, "工具执行超时（10s/30s 档）".into())),
    }
}

/// spec §6：bash 并发上限默认 2，防风扇起飞（其余文件类工具不受限）。
const BASH_CONCURRENCY: usize = 2;

/// 直连执行器：规则放行即执行；Confirm 视为 DeniedByUser（无 UI 场景）。
pub struct DirectExecutor {
    pub cfg: AppConfig,
    pub session: Mutex<SessionRules>,
    bash_gate: Arc<Semaphore>,
}

impl DirectExecutor {
    pub fn new(cfg: AppConfig) -> Self {
        Self { cfg, session: Mutex::new(SessionRules::default()),
            bash_gate: Arc::new(Semaphore::new(BASH_CONCURRENCY)) }
    }
}

#[async_trait::async_trait]
impl Executor for DirectExecutor {
    async fn execute(&self, req: &ToolRequest) -> Result<Value, (ErrorCode, String)> {
        let rules = self.cfg.rules();
        let d = decide(req.tool, &req.args, &rules, &*self.session.lock().await);
        // 全部 allowRoot 构成多根围栏；bash 的 cwd 与兜底 spill 根取第一项。
        let roots: Vec<PathBuf> = match d {
            Decision::Allow => self.cfg.allow_roots.iter().map(PathBuf::from).collect(),
            Decision::Confirm { reason, .. } =>
                return Err((ErrorCode::DeniedByUser, format!("需要确认：{reason}"))),
            Decision::Deny { code, reason } => return Err((code, reason)),
        };
        if roots.is_empty() {
            return Err((ErrorCode::PathDenied, "客户端未配置 allowRoots".into()));
        }
        // spill 目录取**命中根**下的 .remote-tools/（无 path 参数的工具退回第一个根）。
        let spill_root = match req.tool {
            ToolName::Bash => roots[0].clone(),
            _ => match req.args.get("path").and_then(Value::as_str) {
                Some(p) => file::resolve_in_root(&roots, p)?.1,
                None => roots[0].clone(),
            },
        };
        // 文件类工具在 blocking 池执行并统一套 10s/30s 超时档（timed_file_op）。
        let outcome = match req.tool {
            ToolName::Bash => {
                // 并发闸：最多 BASH_CONCURRENCY 个 bash 同时在跑（spec §6）。
                let _permit = self.bash_gate.acquire().await
                    .map_err(|e| (ErrorCode::ExecError, format!("bash 并发闸不可用：{e}")))?;
                let cmd = req.args.get("command").and_then(Value::as_str)
                    .ok_or((ErrorCode::BadArgs, "缺少 command".into()))?;
                let out = run_bash(cmd, tool_timeout(ToolName::Bash, &req.args), &roots[0],
                    &self.cfg.shell_kind()).await;
                serde_json::to_value(&out).unwrap()
            }
            ToolName::Read =>
                timed_file_op(tool_timeout(req.tool, &req.args), roots.clone(),
                    req.args.clone(), file::read_file).await?,
            ToolName::Write =>
                timed_file_op(tool_timeout(req.tool, &req.args), roots.clone(),
                    req.args.clone(), file::write_file).await?,
            ToolName::Edit =>
                timed_file_op(tool_timeout(req.tool, &req.args), roots.clone(),
                    req.args.clone(), file::edit_file).await?,
            ToolName::Grep =>
                timed_file_op(tool_timeout(req.tool, &req.args), roots.clone(),
                    req.args.clone(), search::grep).await?,
            ToolName::Glob =>
                timed_file_op(tool_timeout(req.tool, &req.args), roots.clone(),
                    req.args.clone(), search::glob_search).await?,
        };
        let mut value = outcome;
        truncate_result(&mut value, self.cfg.max_output_chars,
            &spill_root.join(".remote-tools"), &req.id);
        Ok(value)
    }
}

/// 连接内 spawned 请求任务登记表：本函数任何退出路径（断线/关停）触发 Drop →
/// 统一 abort 全部在途任务。被弃 future 连带 drop → run_bash 的子进程因
/// `kill_on_drop(true)` 随之终止（spec §11：断开 = 在途执行一并取消）。
struct TaskGuard(Vec<tokio::task::JoinHandle<()>>);

impl Drop for TaskGuard {
    fn drop(&mut self) {
        for t in self.0.drain(..) { t.abort(); }
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
                // 适配（brief 全局约束「成功复位」）：会话曾建立过 → 退避复位 1s
                // **先于**入睡生效，健康的会话断开后 1s 内即重连；从未建立
                // （拨号/鉴权失败）→ 继续指数增长，60s 封顶。
                if established { backoff = Duration::from_secs(1); }
                else { backoff = (backoff * 2).min(Duration::from_secs(60)); }
                let _ = events.send(TunnelEvent::Log(
                    format!("{} 后重连（退避 {:?}）", backoff.as_secs(), backoff))).await;
                if tokio::time::timeout(backoff, shutdown.changed()).await.is_err() {
                    // 退避计时到，继续重连
                }
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
    // 在途请求登记：断线时 Drop 统一 abort（见 TaskGuard 文档）。
    let mut tasks = TaskGuard(Vec::new());
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
                        let handle = tokio::spawn(async move {
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
                        tasks.0.retain(|t| !t.is_finished()); // 防登记表无限增长
                        tasks.0.push(handle);
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // 文件类工具的 10s/30s 超时档为固定值、端到端造"慢文件系统"不现实 →
    // 在 timed_file_op 层锁死语义（时长可注入）：超时 → timeout 错误码；
    // 及时完成 → 结果原样透传。execute() 内全部文件分支均经此 helper 包装，
    // 同步体跑在 spawn_blocking 上、超时真正可打断等待（而非常规误区的
    // block_in_place——线程被同步体占死时 timer 永远轮不到）。
    #[tokio::test]
    async fn timed_file_op_maps_elapsed_to_timeout() {
        let out = timed_file_op(Duration::from_millis(50), Vec::new(), json!({}), |_, _| {
            std::thread::sleep(Duration::from_millis(500));
            Ok(json!({ "slow": true }))
        }).await;
        assert_eq!(out.unwrap_err(),
            (ErrorCode::Timeout, "工具执行超时（10s/30s 档）".to_string()));
    }

    #[tokio::test]
    async fn timed_file_op_passes_through_fast_result() {
        let out = timed_file_op(Duration::from_secs(10), Vec::new(), json!({}), |_, _| {
            Ok(json!({ "ok": 1 }))
        }).await;
        assert_eq!(out.unwrap(), json!({ "ok": 1 }));
    }
}
