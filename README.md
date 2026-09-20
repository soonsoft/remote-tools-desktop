# remote-tools-desktop

让服务端 agent 安全地操作你的本机：一个 Tauri 2 桌面端，通过 WebSocket 隧道接收
服务端下发的 6 个 `client__*` 工具调用（bash / read / write / edit / grep / glob），
在本机执行并把结果回传。所有操作先过完整规则链——**路径白名单围栏 → 工具分类
（读自动放行；写/编辑/命令需确认；危险命令强制确认并红色高亮）→ 确认三动作
（允许一次 / 本会话内允许同类 / 拒绝）**——你可以在玻璃暗色主窗里看到每一条
指令流水，也可以随时关窗让应用驻留托盘（灰=未连接 / 绿=已连接 / 黄=有待确认）。

## 与 Plan A（`../remote-tools`）的关系

- [remote-tools](../remote-tools)（Plan A）是**服务端**：Claude Agent SDK 的
  MCP 桥把 6 个 `client__*` 工具暴露给模型，`TunnelServer` 经 WebSocket 隧道把
  `tool_request` 转发给桌面端。
- 本仓库是**用户桌面执行端**（Plan B）：直连 `TunnelServer`，按 JSON 信封协议
  （下行 `tool_request` / 上行 `tool_response`，TS 与 Rust 仅结构对齐）执行并回传。
- 协议逐字对齐：工具名六枚、错误码八枚（`client_offline` `timeout`
  `denied_by_user` `denied_by_rule` `path_denied` `bad_args` `exec_error`
  `output_too_large`）、`Authorization: Bearer <token>` 握手、心跳 pong、
  指数退避重连（1s→2s→…→60s 封顶，重连后会话规则清空）。
- 设计文档：`../docs/remote-tools/2026-09-09-remote-tool-proxy-design.md`；
  UI 视觉规范：`../docs/remote-tools/ui-visual-spec.md`。

仓库结构：`core/`（协议信封、规则引擎、执行器、配置、隧道客户端——纯 Rust 库，
不依赖 Tauri）；`src-tauri/`（Tauri 壳：托盘三态、确认桥、配置命令、UI 层执行器）；
`ui/`（无构建步骤的静态 HTML/CSS/JS，暗色默认玻璃风格）。

## 开发

```bash
cargo test --workspace                                   # 全部单元/集成测试
cargo check --workspace                                  # 快速类型检查
cargo clippy --workspace --all-targets -- -D warnings    # lint，零警告
cargo tauri dev                                          # 跑桌面应用（需 tauri-cli）
cargo build -p remote-tools-desktop                      # 只编译应用二进制，不启动 GUI
```

- `cargo tauri dev` 需要 tauri-cli：`cargo install tauri-cli --version "^2"`。
- 工具链：Rust ≥ 1.94（`x86_64-pc-windows-msvc` 已验证）；Windows 依赖 WebView2
  （Edge 通道，Win10/11 默认在场）。`ui/` 是纯静态资源，无 npm 安装与构建步骤。

## 配置

配置存于系统配置目录（Windows：`%APPDATA%\com.remotetools.desktop\config.json`），
通过应用内「设置」页编辑；UI 暴露以下字段：

| 字段 | 类型 | 默认 | 说明 |
|------|------|------|------|
| `serverUrl` | string | 空 | Plan A `TunnelServer` 的 ws 地址，如 `ws://127.0.0.1:8787` |
| `token` | string | 空 | 隧道令牌，32 字节十六进制；生成：`node -e "console.log(require('crypto').randomBytes(32).toString('hex'))"` |
| `shell` | string | 空（按平台） | `powershell`（Windows 默认）/ `pwsh` / `cmd` / `sh` / 自定义可执行路径 |
| `allowRoots` | string[] | `[]` | 路径白名单，可配多个根（前缀围栏，`..` 与绝对逃逸直接 `path_denied` 不弹窗）；截断落盘目录取**命中根**下的 `.remote-tools/`（bash / 无 path 的搜索用第一项） |
| `maxOutputChars` | number | `100000` | 单结果截断阈值；超出部分完整落盘 `spillPath` 并附 `truncated:true` |

高级字段 `dangerous`（危险命令正则数组，默认九枚：`rm\s+(-[a-z]*r[a-z]*f|-[a-z]*f[a-z]*r)`、
`rm\s+-r`、`Remove-Item\s+.*-Recurse`、`rd\s+/s`、`del\s+/[sq]`、`format\s+[a-z]:`、
`reg\s+(add|delete)`、`sudo\b`、`(curl|wget)[^|]*\|\s*(sh|bash|iex|pwsh)`）与
`toolOverrides`（按工具覆写分类：`"auto"` 自动放行 / `"deny"` 直接拒绝；
围栏仍优先，auto 也出不了 allowRoots；注意 `"auto"` 连**危险命令确认**一并
跳过——属 power-user 设置，请只对完全信任的工具启用）未在设置页暴露，需直接
编辑 `config.json`。保存后：`allowRoots` / `shell` / `maxOutputChars` /
`dangerous` / `toolOverrides` 均自**下一次请求**起生效；仅 `serverUrl` /
`token` 需重启应用（隧道任务持有启动时的连接配置，见已知偏差 b）。

## 验收

跨语言联调（桌面端 × Plan A 真实服务端）的手动验收清单与两终端操作步骤见
[docs/ACCEPTANCE.md](docs/ACCEPTANCE.md)：brief 验收 11 项 + 窗口生命周期回归 3 项，
逐项含步骤、预期与空白结果行。

## 平台

- **Windows 10/11**：即时可用（`cargo tauri dev` 或 `cargo build`；打包 target
  含 msi）。
- **macOS（Intel + Apple Silicon universal）**：在 Mac 上执行

  ```bash
  rustup target add aarch64-apple-darwin x86_64-apple-darwin
  cargo tauri build --target universal-apple-darwin
  ```

  `tauri.conf.json` 已开启 `macOSPrivateApi`（WKWebView 透明玻璃材质）。
  universal 打包时建议在真机确认托盘行为（Windows 允许非主线程 `set_icon`，
  macOS 惯常要求主线程，tauri v2 有代理解除但未在 Mac 实测）。

## 已知偏差（如实记录）

- **a · 托盘菜单为系统原生样式**：spec §9 设想自绘浮窗菜单；当前是 tauri 原生
  右键菜单（显示窗口 / 退出）。自绘浮窗留作后续任务。
- **b · 仅 serverUrl/token 需重启生效**：`save_config` 已落盘并更新共享状态，
  allowRoots / shell / maxOutputChars / dangerous / toolOverrides 均按请求实时
  读取、下一次请求即生效；但隧道后台任务持有启动时的 `serverUrl`/`token` 快照，
  改连接配置仍需重启应用。
- **c · 退出不清理工具子进程**：托盘「退出」走 `exit(0)` 硬结束（规避 tokio 运行时
  因被孙进程持有的管线读端而挂起的问题）；工具启动的长驻子进程不会被显式终止。
