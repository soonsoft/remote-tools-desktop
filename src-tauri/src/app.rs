//! 桌面壳装配：托盘三态、隧道后台任务、事件泵、UI 层执行器与确认桥。
//!
//! 事件（后端→UI，名称逐字）：`tunnel-status` / `tool-log` /
//! `confirm-request` / `confirm-resolved`。

use crate::confirm::{ConfirmAction, ConfirmBridge};
use crate::icons::icon::{amber_icon, green_icon, grey_icon};
use crate::state::{AppState, LogEntry};
use remote_tools_core::config::AppConfig;
use remote_tools_core::envelope::{ErrorCode, ToolRequest};
use remote_tools_core::rules::{decide, Decision, RulesConfig, SessionRules};
use remote_tools_core::tunnel::{run_tunnel, DirectExecutor, Executor, TunnelEvent};
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent};
use tokio::sync::{mpsc, oneshot, watch};

/// 活动日志上限（超出丢弃最旧）。
pub const MAX_LOGS: usize = 500;

/// UI 层执行器：先跑完整规则链（路径围栏 → 分类 → 确认桥），再交
/// [`DirectExecutor`] 实际执行。UI 裁决是唯一权威，直连层只负责干活。
struct UiExecutor {
    app: AppHandle,
    state: Arc<AppState>,
    bridge: Arc<ConfirmBridge>,
}

/// 摘要：bash 取 command，文件类取 path，截断到 200 字符。
fn request_summary(req: &ToolRequest) -> String {
    let raw = req
        .args
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| req.args.get("path").and_then(Value::as_str))
        .unwrap_or("");
    let mut s: String = raw.chars().take(200).collect();
    if raw.chars().count() > 200 {
        s.push('…');
    }
    s
}

/// 该请求在「无会话规则」时的危险标记。`Decision::Allow` 不携带 danger，
/// 而 DirectExecutor 重新 decide 时需要精确的 (tool, danger) 对才命中注入
/// 的会话放行，故用空会话重算一次（与直连层的 is_dangerous 结果一致）。
fn fresh_danger(req: &ToolRequest, rules: &RulesConfig) -> bool {
    match decide(req.tool, &req.args, rules, &SessionRules::default()) {
        Decision::Confirm { danger, .. } => danger,
        _ => false,
    }
}

async fn push_log(app: &AppHandle, state: &AppState, entry: LogEntry) {
    {
        let mut logs = state.logs.lock().unwrap();
        logs.push(entry.clone());
        let len = logs.len();
        if len > MAX_LOGS {
            logs.drain(..len - MAX_LOGS);
        }
    }
    let _ = app.emit("tool-log", entry);
}

#[async_trait::async_trait]
impl Executor for UiExecutor {
    async fn execute(&self, req: &ToolRequest) -> Result<Value, (ErrorCode, String)> {
        let cfg = self.state.config.read().unwrap().clone();
        let rules = cfg.rules();
        // 锁只护住同步的 decide，不跨 await。
        let decision = {
            let session = self.state.session_rules.lock().await;
            decide(req.tool, &req.args, &rules, &session)
        };

        let mut confirmed = false;
        match decision {
            Decision::Allow => {}
            Decision::Confirm { reason, danger } => {
                // 先登记再广播：confirm-request 若先于登记送达（UI 极快裁决/
                // 自动化点击），resolve 会扑空。登记 → 广播 → 刷托盘。
                let (tx, rx) = oneshot::channel();
                self.bridge.register(req.id.clone(), tx);
                let _ = self.app.emit(
                    "confirm-request",
                    serde_json::json!({
                        "id": req.id, "tool": req.tool.wire(),
                        "summary": request_summary(req),
                        "danger": danger, "reason": reason,
                    }),
                );
                refresh_tray(&self.app, &self.state, &self.bridge); // 琥珀：待确认
                match rx.await {
                    Ok(ConfirmAction::Deny) => {
                        push_log(&self.app, &self.state, LogEntry {
                            id: req.id.clone(), tool: req.tool.wire().into(),
                            summary: request_summary(req), ok: false,
                            duration_ms: 0, confirmed: true,
                        }).await;
                        return Err((ErrorCode::DeniedByUser, "用户拒绝了此操作".into()));
                    }
                    Ok(ConfirmAction::Session) => {
                        // 本会话内允许同类（同工具 + 同危险级别）
                        self.state.session_rules.lock().await.insert_allow(req.tool, danger);
                    }
                    Ok(ConfirmAction::Once) => {}
                    Err(_) => return Err((ErrorCode::DeniedByUser, "确认已取消".into())),
                }
                confirmed = true;
            }
            Decision::Deny { code, reason } => {
                push_log(&self.app, &self.state, LogEntry {
                    id: req.id.clone(), tool: req.tool.wire().into(),
                    summary: request_summary(req), ok: false,
                    duration_ms: 0, confirmed: false,
                }).await;
                return Err((code, reason));
            }
        }

        // 两级裁决的衔接点：DirectExecutor 会重新 decide 并把 Confirm 判为
        // denied_by_user。把本次 (tool, danger) 注入它的一次性会话规则使其
        // 直接放行——完整规则链（围栏/override/分类/确认）已在 UI 层跑过，
        // 注入不绕过任何检查，只消除重复裁决（brief「两级 decide 幂等」的
        // 实际落地形态）。
        let started = Instant::now();
        let direct = DirectExecutor::new(cfg);
        direct.session.lock().await.insert_allow(req.tool, fresh_danger(req, &rules));
        let result = direct.execute(req).await;
        push_log(&self.app, &self.state, LogEntry {
            id: req.id.clone(), tool: req.tool.wire().into(),
            summary: request_summary(req), ok: result.is_ok(),
            duration_ms: started.elapsed().as_millis() as u64, confirmed,
        }).await;
        result
    }
}

/// 托盘三态刷新：黄（有待确认）> 绿/灰（连接状态）。
pub(crate) fn refresh_tray(app: &AppHandle, state: &AppState, bridge: &ConfirmBridge) {
    let Some(tray) = app.tray_by_id("main") else { return };
    let icon = if !bridge.is_empty() {
        amber_icon()
    } else if state.connected.load(Ordering::Relaxed) {
        green_icon()
    } else {
        grey_icon()
    };
    let _ = tray.set_icon(Some(icon));
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_clipboard_manager::init())
        .invoke_handler(tauri::generate_handler![
            crate::commands::get_config,
            crate::commands::save_config,
            crate::commands::resolve_confirm,
            crate::commands::recent_logs,
            crate::commands::get_status,
            crate::commands::get_platform_chrome,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let cfg_path = app.path().app_config_dir()?.join("config.json");
            let cfg = AppConfig::load(&cfg_path);
            let (shutdown_tx, shutdown_rx) = watch::channel(false);
            // 平台材质（2026-09-23 设计）：Win11=Mica / Win10=Acrylic /
            // macOS=UnderWindowBackground；conf 里的 windowEffects 已移除，
            // 运行时按探测结果单点上效果。
            {
                use crate::platform::PlatformChrome;
                use tauri::window::{Effect, EffectState, EffectsBuilder};
                if let Some(w) = app.get_webview_window("main") {
                    let chrome = crate::platform::detect();
                    let builder = match chrome {
                        PlatformChrome::WindowsMica => EffectsBuilder::new()
                            .effects([Effect::Mica]),
                        PlatformChrome::WindowsAcrylic => EffectsBuilder::new()
                            .effects([Effect::Acrylic]),
                        PlatformChrome::MacOS { .. } => EffectsBuilder::new()
                            .effects([Effect::UnderWindowBackground])
                            .state(EffectState::FollowsWindowActiveState),
                        PlatformChrome::Other { .. } => EffectsBuilder::new()
                            .effects([Effect::WindowBackground]),
                    };
                    let _ = w.set_effects(builder.build());
                }
            }

            let state = Arc::new(AppState {
                config_path: cfg_path,
                config: std::sync::RwLock::new(cfg.clone()),
                logs: std::sync::Mutex::new(Vec::new()),
                session_rules: tokio::sync::Mutex::new(SessionRules::default()),
                connected: std::sync::atomic::AtomicBool::new(false),
                last_disconnect_reason: std::sync::Mutex::new(None),
                tunnel_shutdown: shutdown_tx,
            });
            let bridge = Arc::new(ConfirmBridge::default());
            app.manage(state.clone());
            app.manage(bridge.clone());

            // 托盘：代码注册（tauri.conf 不声明 trayIcon，避免引用二进制
            // 资产与双托盘）。左键唤起主窗，右键菜单：显示窗口 / 退出。
            let show = MenuItemBuilder::with_id("show", "显示窗口").build(app)?;
            let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app).item(&show).item(&quit).build()?;
            TrayIconBuilder::with_id("main")
                .icon(grey_icon())
                .tooltip("remote-tools")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(w) = tray.app_handle().get_webview_window("main") {
                            let _ = w.show();
                            let _ = w.unminimize();
                            let _ = w.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 隧道后台任务。setup 在主线程执行、不在 reactor 里，
            // 必须用 tauri::async_runtime::spawn（tokio::spawn 会 panic）。
            let (etx, mut erx) = mpsc::channel::<TunnelEvent>(64);
            {
                let exec: Arc<dyn Executor> = Arc::new(UiExecutor {
                    app: handle.clone(),
                    state: state.clone(),
                    bridge: bridge.clone(),
                });
                tauri::async_runtime::spawn(run_tunnel(cfg.clone(), exec, etx, shutdown_rx));
            }

            // 事件泵：隧道事件 → UI 事件 + 托盘三态。
            // Connected 与 Disconnected 均清空会话规则（spec：重连/断线从严）。
            {
                let handle = handle.clone();
                let state = state.clone();
                let bridge = bridge.clone();
                tauri::async_runtime::spawn(async move {
                    while let Some(ev) = erx.recv().await {
                        match ev {
                            TunnelEvent::Connected => {
                                state.session_rules.lock().await.clear();
                                state.connected.store(true, Ordering::Relaxed);
                                *state.last_disconnect_reason.lock().unwrap() = None;
                                let _ = handle.emit(
                                    "tunnel-status",
                                    serde_json::json!({ "connected": true }),
                                );
                            }
                            TunnelEvent::Disconnected(reason) => {
                                state.session_rules.lock().await.clear();
                                // 断线清扫（spec §11）：在途待确认一律拒绝，
                                // 请求方收到 denied_by_user，托盘琥珀态解除。
                                bridge.deny_all();
                                state.connected.store(false, Ordering::Relaxed);
                                *state.last_disconnect_reason.lock().unwrap() = Some(reason.clone());
                                let _ = handle.emit(
                                    "tunnel-status",
                                    serde_json::json!({ "connected": false, "reason": reason }),
                                );
                            }
                            // 隧道诊断（退避重连等）并入同一 tool-log 流，
                            // tool 标记为 "tunnel"，保持事件载荷形状统一。
                            TunnelEvent::Log(msg) => {
                                push_log(&handle, &state, LogEntry {
                                    id: String::new(), tool: "tunnel".into(),
                                    summary: msg, ok: true, duration_ms: 0, confirmed: false,
                                }).await;
                            }
                            TunnelEvent::ConfirmPending(_) => {}
                        }
                        refresh_tray(&handle, &state, &bridge);
                    }
                });
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // v2 中最后一扇窗被关闭即销毁（此后 get_webview_window 恒为
            // None，托盘两条唤起路径全部失效）。因此拦截关闭请求：
            // 只隐藏窗口，进程驻留托盘，窗口可随时经托盘唤回。
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.unminimize();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(tauri::generate_context!())
        .expect("tauri 构建失败")
        .run(|_app, event| match event {
            // 主窗可关不影响运行。主防线是 on_window_event 的 prevent_close
            // + hide（窗口保持存活、可随时唤回）；此处 prevent_exit 为
            // 保险带：即便窗口被程序性销毁（code=None 的退出请求），进程
            // 仍驻留托盘而非退出。托盘菜单「退出」（app.exit(0)，code=Some）
            // 不受影响，正常走硬退出。
            RunEvent::ExitRequested { code, api, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                }
            }
            // v1：退出即硬结束进程。tokio 运行时 drop 可能等待被孙进程持有
            // 的管线读端（秒级挂起）；硬退出彻底规避，配置已同步落盘。
            RunEvent::Exit => std::process::exit(0),
            _ => {}
        });
}
