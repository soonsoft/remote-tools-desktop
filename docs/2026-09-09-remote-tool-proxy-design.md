> 注：本文档从开发工作区归档入库（2026-09-21）。文内提及的 design-demos/ 原型与方向决策史存于开发工作区 docs/remote-tools/design-demos/，未随本仓库分发。

# remote-tools 远程工具代理 · 设计文档

- 日期：2026-09-09
- 状态：已定案（与用户逐节确认）
- 参考实现：cc-haha（Claude Code 架构镜像，GitHub `NanmiCoder/cc-haha`，工具系统全链路见 `docs/tools/cc-haha-tools-architecture.html`）

## 1. 背景与动机

服务端项目使用 Claude Agent SDK 运行 agent。需求的痛点：

- 需求清单、技术方案、文档和数据在**服务器**上；
- 代码在**用户本机**（客户端）上；
- agent 若在服务器上写代码，编码动作本身可行，但服务器**难以构建运行环境**，也**没有资源**运行/调试大型项目。

结论：让服务端 agent 通过代理把工具调用转发到用户本机执行——读文档在服务器，编码/构建/调试/执行在客户端。

网络环境：公司内网，客户端与服务器互访，服务器有内网域名，无公网。

## 2. 目标与非目标

**目标**

1. 服务端 agent 获得 6 个操作用户本机的 MCP 工具（bash / read / write / edit / grep / glob）；
2. 客户端为轻量桌面应用（macOS Intel+Apple Silicon、Windows 10/11，内存目标 ~40-70MB），托盘常驻、状态直观；
3. 混合权限模型：白名单内只读操作自动执行；写操作与危险命令本机弹窗确认；
4. 防混淆：结构上消除"在错误机器上写代码/执行"的可能；
5. 阶段化模式切换：文档阶段（纯服务器）→ 编码阶段（远程模式）。

**非目标（YAGNI）**

- 公网穿透 / 第三方中继（内网互访已满足）；
- 多客户端同时接入（协议留 session 概念，本期单客户端）；
- 二进制文件上传/下载、delete/list_dir 专用工具（bash 与 glob 覆盖）；
- 工具输出流式回传（先整段回传 + 超时）；
- TLS（内网 + 令牌已覆盖当前威胁模型；协议版本字段为升级 wss 留位）。

## 3. 架构总览

```
┌─────────────────────── 服务器（内网域名）───────────────────────┐
│  宿主应用 ── Claude Agent SDK (query，逐阶段换 options)          │
│                │  mcpServers 注册                               │
│                ▼                                                │
│  ① mcp-bridge（SDK MCP Server，进程内）                         │
│     client__bash / read / write / edit / grep / glob            │
│                │ tool.call() → 生成请求 id，挂起等响应           │
│                ▼                                               │
│  ② tunnel-server（WS 端点 ws://<内网域名>/client）               │
│     令牌鉴权 · 心跳 · 按 id 路由响应 · 超时兜底                  │
└──────────────┬─────────────────────────────────────────────────┘
               │ 客户端主动拨出的 WebSocket 长连接（唯一通道）
┌──────────────▼─────────────────────────────────────────────────┐
│  客户端桌面应用（Tauri v2 + Rust，托盘常驻）                      │
│  ③ 拨号连接 + 自动重连（tokio）                                  │
│  ④ 规则引擎（纯函数：路径白名单 → 分类 → 危险模式 → 允许/确认/拒绝）│
│  ⑤ 执行器（shell / fs 读写 / 内容搜索 / 文件名匹配）              │
│  ⑥ 确认 UI（原生弹窗：命令全文 / 目标路径 / 命中原因）             │
│  状态可视化：托盘图标（灰未连/绿已连/黄待确认）+ 主窗连接横幅、     │
│  指令流水、设置页（令牌 / shell 选择 / allowRoots / 规则覆盖）     │
└────────────────────────────────────────────────────────────────┘
```

组件职责与 cc-haha 对应物：

| # | 组件 | 职责 | cc-haha 对应物 |
|---|------|------|----------------|
| ① | mcp-bridge | 6 工具注册进 SDK；handler 把调用转隧道请求 | `MCPTool` + `assembleToolPool` |
| ② | tunnel-server | WS 端点：鉴权、保活、id 路由、超时 | `callMCPTool` 请求-响应骨架 |
| ③ | 拨号客户端 | 连接、心跳、指数退避重连 | `connectToServer` 连接管理 |
| ④ | 规则引擎 | 有序决策链，输出 allow/confirm/deny | `hasPermissionsToUseTool` |
| ⑤ | 执行器 | 真执行；统一结果信封 | `Tool.call()` + result 映射 |
| ⑥ | 确认 UI | 确认在客户端本地闭环，服务端只感知耗时 | PermissionRequest 队列 |

关键设计决定：

- **人工确认完全在客户端本地闭环**（⑥→④阻塞等待），协议层无"确认往返"消息；
- **连接方向：客户端拨出**。客户端零监听端口（不受防火墙/DHCP 影响），服务器域名是唯一需要稳定寻址的点；确认交互的异步性契合双向通道；
- **shell 抽象**：`client__bash` 的实际 shell 由客户端配置（Windows 默认 PowerShell，可配 Git Bash/cmd；macOS zsh）。工具描述声明执行环境，跨平台命令意识由系统提示引导。

## 4. 工具清单（6 个）

MCP 服务器名 `client`，全名形如 `mcp__client__bash`。

| 工具 | 参数 | 语义 |
|---|---|---|
| `client__bash` | `command`, `timeout_ms?` | 客户端执行 shell 命令，回 stdout/stderr/exitCode |
| `client__read` | `path`, `offset?`, `limit?` | 读客户端文件（文本，行区间可选） |
| `client__write` | `path`, `content` | 写/覆盖客户端文件（父目录自动创建） |
| `client__edit` | `path`, `old_text`, `new_text` | 精确串替换（old_text 必须唯一命中，str_replace 语义） |
| `client__grep` | `pattern`, `path?`, `glob?` | 内容正则搜索，回 文件:行号:内容 列表；`path` 缺省 = allowRoots 第一项 |
| `client__glob` | `pattern`, `path?` | 文件名模式匹配，回路径列表；`path` 缺省 = allowRoots 第一项 |

刻意不做：`delete`（bash 走确认）、`list_dir`（glob 覆盖）、二进制传输（YAGNI）。

## 5. 客户端桌面应用形态

- **技术栈**：Tauri v2 + Rust 后端 + 系统 WebView 前端。理由：内存 ~40-70MB（Electron 150-300MB）；macOS universal binary 同时覆盖 Intel/AS；Windows 10/11 走 WebView2；cc-haha desktop 已是 Tauri，团队熟悉。
- **常驻形态**：托盘图标（灰=未连接 / 绿=已连接 / 黄=有待确认），主窗可关不影响运行。
- **主窗口内容**：连接状态横幅、最近指令流水（命令、耗时、结果 ok/err、是否经过确认）、设置页（配对令牌、服务器地址、shell 选择、allowRoots、危险模式覆盖）。
- **确认弹窗**：原生窗口展示命令全文 / 目标路径 / 规则命中原因；三动作：**允许一次 / 本会话内允许同类 / 拒绝**。"会话内允许"写入临时规则（断线即清）。

## 6. 通信协议

WS 上的极简 JSON 信封。字段刻意保持少量，TS 与 Rust 双语言无共享类型、仅结构对齐。

下行（服务器 → 客户端）业务消息一种：

```json
{ "v": 1, "type": "tool_request",
  "id": "req_01HZ…",              // ULID；响应按此路由
  "tool": "client__bash",
  "args": { "command": "npm test" },
  "meta": { "requestedAt": 1694150400000 } }
```

上行两种：

```json
{ "v": 1, "type": "tool_response", "id": "req_01HZ…", "ok": true,
  "result": { "stdout": "…", "stderr": "", "exitCode": 0, "durationMs": 1834 } }

{ "v": 1, "type": "tool_response", "id": "req_01HZ…", "ok": false,
  "error": { "code": "denied_by_user", "message": "用户拒绝了此操作" } }
```

控制消息：`hello`（**仅主机信息交换**——鉴权已在 WS 握手 Authorization 头完成，`hello` 不再二次验令牌）/ `hello_ack` / `ping` / `pong`（30s 心跳，3 次失联判死）。

**错误码枚举**：`client_offline`（服务器侧生成）/ `timeout` / `denied_by_user` / `denied_by_rule`（仅由用户配置覆盖的显式 deny 规则产生，内置决策链不产生此码）/ `path_denied` / `bad_args` / `exec_error` / `output_too_large`。错误文本面向 agent 可行动——`denied_by_user` 附建议（"询问用户是否允许，或换一种方式"），引导 agent 合理续跑。

**输出上限**：单结果 `maxOutputChars` 默认 100KB，**由客户端配置并执行截断**（工具描述中声明该上限）；超出时完整输出落盘到目标 allowRoot 下的 `.remote-tools/` 子目录（该目录自动纳入可读白名单），结果附 `truncated: true` 与落盘完整路径，agent 可用 `client__read` 分段读取（仿 cc-haha `maxResultSizeChars` + 大结果落盘模式）。

**并发**：请求 id 路由天然支持并发下发；客户端执行器内部限制并行 bash 数（默认 2），文件工具可并行。

## 7. 配对与鉴权

- 服务端生成 32 字节随机 `pairing token` 存服务器配置；客户端设置页粘贴一次；
- WS 握手 `Authorization: Bearer <token>`，失败 401 + 客户端提示；
- 内网 + 令牌覆盖当前威胁模型；`v: 1` 为将来升级内部 CA / wss 留位，协议不变。

## 8. 客户端规则引擎（混合权限）

纯函数 `(request, config, sessionRules) → allow | confirm(reason) | deny(reason)`，与执行器解耦、可完整单测。有序决策链：

1. **路径白名单**（`allowRoots`）：文件工具与 bash cwd 的绝对路径规范化后必须落在某 root 内（前缀比对，防 `..` 逃逸、符号链接穿透）。白名单外 → `path_denied`，不弹窗。
2. **工具分类**（内置表 + 配置覆盖）：
   - 自动放行：`read` / `grep` / `glob`（只读且路径已受控）；
   - 需确认：`write` / `edit` / bash 写类命令；
   - bash 危险模式（正则黑名单：`rm -rf`、`del /s`、格式化、注册表写、`curl | sh` 类管道执行、`sudo`…）→ 强制确认且弹窗高亮，无论白名单。
3. **确认弹窗**（仅判"需确认"时）：三动作见 §5；"会话内允许"写临时规则（断线清空，刻意保守）。

## 9. 服务端工具围栏与防混淆（非对称模式 v2）

内置工具与远程工具的能力矩阵：

| 能力 | 服务器（内置） | 客户端（MCP） |
|---|---|---|
| 读 | ✅ Read / Grep / Glob（全服务器） | ✅ 只读三件套 |
| 写 | ⚠️ Write / Edit **仅限文档根**（如 `/data/docs/**`） | ✅ write / edit（限 allowRoots） |
| 执行 | ❌ Bash 禁用 | ✅ bash（构建/调试/运行） |

**实现**：Agent SDK `canUseTool` 回调做路径前缀规则（规范化后必须落在文档根内），语义照搬 cc-haha 权限引擎的 `Tool(pattern)` 路径规则与文件系统安全检查。

**拒绝必须带引导**——围栏外写操作的 tool_result 返回：

> "服务器上的写入仅限 docs/ 目录；代码文件在用户本机，请改用 client__write。"

agent 读到会自行换工具，不卡死不硬闯。

**防混淆分层**（危害面从结构上消除，而非仅降低概率）：

1. 结构层：服务器无 Bash、写限文档根 → "在服务器写代码/构建"不可能发生；
2. 命名与描述层：`client__` 前缀即语义；每工具描述首句标明目标机器与触发条件（"在**用户的本地电脑**执行……凡是提到用户项目/本地文件的操作必须用此工具"）；
3. 系统提示层（`appendSystemPrompt`）："服务器存放需求与技术文档（docs/ 内可读可写）；用户的所有代码、依赖与运行环境都在其本机——编码、构建、调试、执行一律使用 `client__*` 工具。"
4. Skill 不作为机制（懒加载、约束力弱），仅当需要长篇操作规范时再加。

残余混淆只剩"读方向"（用服务器 Read 读客户端路径）：危害小、自纠（文件不存在），由层 2/3 兜底。

## 10. 阶段化模式切换

**方式一（本期实现）：分阶段会话**。宿主应用逐次调用 `query()`，`mcpServers` 与工具白名单是每次入参——"切模式"即换一套 options：

```
阶段 1 · 文档会话                            阶段 2 · 编码会话
内置: Read/Write/Edit/Grep/Glob ✅          内置: 读 ✅ / 写限 docs 围栏 / Bash ❌
WebFetch/WebSearch ✅                        + mcpServers: client ✅
client-tools: 不注册                         systemPrompt: "代码在用户本机…"
(配置项: 可注册 client 只读三件套，            │
 便于技术方案参考客户端已有代码；默认关闭)      │
     │ 产出 Spec/Plan 落盘服务器 docs/ ──────┘ 阶段 2 用服务器 Read 读取
```

- 阶段 1 的 agent 眼里**不存在** `client__*`（非"被拒"）；阶段 2 反之无 WebFetch 干扰；工具集与阶段职责严格对齐；
- Spec/Plan 是两阶段的交接物——结构化上下文传递，阶段 2 不继承阶段 1 对话历史，省 token 且上下文干净；
- 切换触发在宿主应用：手动（UI 按钮）或自动（检测 Plan 产出）；
- 代价：换工具集使下轮提示缓存前缀失效（tools 在缓存渲染序最前）；每项目仅一两次，可接受。

**方式二（后续增量）**：单会话 + `canUseTool` 查可变 `mode` 标志，所有工具常驻、按模式放行/拒绝。缓存不失效、切换即时，但纪律性弱（看得见但被拒）。`canUseTool` 本来就要实现，加 mode 字段成本极低，本期不做。

**切换控制：agent 提议 + 宿主裁决**（仿 Claude Code plan mode 的 `ExitPlanMode` 模式）：

- 切换本身建模为宿主注册的**本地 MCP 工具 `submit_phase`**（宿主进程内执行，不走隧道）：
  - 参数：`{ planPath: string, summary: string }`；
  - 宿主裁决链：① 校验 `planPath` 在服务器文档根内、存在且非空（防空枪）→ ②（可配置，默认开）用户在宿主 UI 确认"进入编码阶段" → ③ 通过则结束阶段 1 会话，以阶段 2 options 发起新 `query()`；任一步拒绝则 `submit_phase` 返回拒绝原因文本，agent 继续完善方案；
- **agent 不能自行改工具集**（options 归宿主所有）——围栏的裁决权必须留在被围者之外，防止被提示注入的 agent 自我扩权；
- 宿主 UI 的手动切换按钮保留为旁路（agent 忘调工具时人可直接切）；
- 纯文件检测不做主触发（半成品 Plan 与完成 Plan 不可区分）。

## 11. 错误处理与重连

| 场景 | 行为 |
|---|---|
| 客户端断线 | 指数退避重连 1s→2s→…→60s 封顶，成功复位；托盘灰 + 横幅"正在重连" |
| 断线期间的调用 | mcp-bridge 立即返回 `client_offline`（不挂起等超时），agent 可等待重试或告知用户 |
| 断线时已在执行的调用 | 被断连杀掉的子进程对应请求同样返回 `client_offline`（非 timeout），agent 可在新连接后重发 |
| 调用超时 | 默认：bash 120s / 文件工具 10s / grep·glob 30s（schema 声明；bash 可传 `timeout_ms` 覆盖）；超时返回 `timeout`，客户端杀子进程 |
| 执行中进程清理 | bash 子进程挂在连接会话下；连接断开 → 已运行子进程 kill（防孤儿进程继续写文件） |
| 服务端重启 | 客户端重连即可；会话内确认规则清空，重新走确认（刻意保守） |
| 确认弹窗无人响应 | 等待计入该请求超时；超时按 `denied_by_user` 返回 |

## 12. 包结构（拆分两目录）

```
ai-agent-code/
├─ remote-tools/               # TS workspace 包：协议 + 服务端
│  ├─ package.json
│  ├─ src/
│  │  ├─ protocol/envelope.ts  # 信封类型 + 编解码（TS 侧唯一权威）
│  │  ├─ mcp-bridge/           # createClientToolsMcp(tunnel) → 供 mcpServers 注册
│  │  │  └─ tools/             # 每工具一文件：schema + handler
│  │  └─ tunnel/server.ts      # WS 端点
│  └─ test/                    # protocol / e2e 测试
└─ remote-tools-desktop/       # 独立 Tauri 项目（自己的 Cargo/构建链）
   ├─ src-tauri/src/
   │  ├─ main.rs               # 托盘 + 窗口生命周期
   │  ├─ tunnel.rs             # 拨号、心跳、重连（tokio）
   │  ├─ rules.rs              # 规则引擎（纯函数）
   │  ├─ exec/                 # bash / 文件 / grep / glob 执行器
   │  └─ confirm.rs            # 确认弹窗
   └─ src/                     # 状态 UI 前端
```

拆分理由：TS workspace 与 Rust/Tauri 工具链不同（Tauri 前端的 package.json 会被 npm workspace 误收编）；两者唯一耦合面是信封 JSON 结构，各自独立构建。

依赖边界：`protocol` 零依赖；`mcp-bridge` 仅依赖 `protocol + tunnel`；Rust 侧仅对齐信封字段名。

## 13. 测试策略

- **协议层**（TS）：信封往返、未知 type 容错、超时触发、id 路由；
- **规则引擎**（Rust + TS 用例镜像）：路径逃逸攻击（`..`、符号链接、盘符大小写）、危险命令模式、白名单边界、会话规则生命周期——同一组用例文件双语言各自跑；
- **e2e**（TS）：loopback 起 tunnel-server + 内存版客户端（非 Tauri），从 SDK MCP 注册入口发起真实 `query()` 调 6 工具，断言 tool_result；围栏用例（围栏外写 → 引导性错误文案）；
- **桌面端**：手动验收清单（托盘状态流转、确认三动作、断网重连横幅、macOS Intel/AS 双架构构建）。

## 14. 遗留与将来

- wss（内部 CA）升级：协议不变；
- 多客户端：tunnel-server 按 token 绑定客户端身份，mcp-bridge 按 agent 会话选路；
- bash 输出流式回传（长构建日志实时可见）；
- 方式二动态模式开关。
