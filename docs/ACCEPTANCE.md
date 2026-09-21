# remote-tools-desktop · 跨语言联调验收记录

本清单由实现代理按任务 9 brief（Step 1-2）整理，**由人工执行并逐项填写**。
覆盖范围：brief 验收清单 11 项 + 任务 7 修复的三项窗口生命周期行为（B1-B3），共 14 项。

- 被验收版本：`master` @ `f5e9e4f`（commits `bceca04..f5e9e4f`）
- 对端（Plan A）：`D:\Code\ai\ai-agent-code\remote-tools\`
- 执行日期：＿＿＿＿＿＿　执行人：＿＿＿＿＿＿

---

## 0 · 准备（两终端）

> 需要网络（`npx` 首次拉取 tsx）；需要 tauri-cli（`cargo install tauri-cli --version "^2"`）。
> 服务端脚本已用 `npx -y tsx` 实测可启动（2026-09-20），Node ≥ 24。

### 终端 1 —— Plan A 侧：验收驱动脚本

把下面的脚本存为 `/d/Code/ai/ai-agent-code/remote-tools/accept-driver.ts`
（它通过该仓库的 package.json exports 解析 `remote-tools`，必须放在仓库目录内）：

```ts
// accept-driver.ts —— remote-tools-desktop 手动验收驱动（Plan A 侧）
// 用法：在 remote-tools 仓库目录内  npx -y tsx accept-driver.ts [token]
import { TunnelServer } from "remote-tools";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import readline from "node:readline/promises";

const token = process.argv[2] ?? "smoke-token-please-rotate";
const root = path.resolve("smoke-root");        // ← 填进桌面端 allowRoots
const outside = path.resolve("smoke-outside");  // ← allowRoots 之外，测 path_denied
mkdirSync(root, { recursive: true });
mkdirSync(outside, { recursive: true });
writeFileSync(path.join(root, "hello.txt"), "SMOKE_FILE_MARKER_7f3a\n", "utf8");
writeFileSync(path.join(outside, "secret.txt"), "OUTSIDE_MARKER\n", "utf8");

const server = new TunnelServer({ token });
const url = await server.listen(8787);
console.log(`[driver] ws 地址   : ${url}`);
console.log(`[driver] token    : ${token}`);
console.log(`[driver] allowRoots 填：${root}`);
console.log(`[driver] 围栏外路径：${path.join(outside, "secret.txt")}`);

const rl = readline.createInterface({ input: process.stdin, output: process.stdout });
let lastSpill = "";
while (true) {
  const pick = (await rl.question(`
 1 bash echo        2 write 写文件     3 read hello.txt   4 edit hello.txt
 5 grep MARKER      6 glob *.txt      7 read 白名单外     8 bash rm -rf(危险)
 9 bash sleep 超时  f bash 130KB输出  r read 上次落盘     q 退出
> `)).trim();
  if (pick === "q") break;
  const t0 = Date.now();
  let out;
  switch (pick) {
    case "1": out = await server.execute("client__bash", { command: "echo SMOKE_ECHO_9c2e" }, 30_000); break;
    case "2": out = await server.execute("client__write", { path: path.join(root, "out.txt"), content: "written by acceptance\n" }, 30_000); break;
    case "3": out = await server.execute("client__read", { path: path.join(root, "hello.txt") }, 30_000); break;
    case "4": out = await server.execute("client__edit", { path: path.join(root, "hello.txt"), old_text: "SMOKE_FILE_MARKER_7f3a", new_text: "EDITED_MARKER_4b8d" }, 30_000); break;
    case "5": out = await server.execute("client__grep", { pattern: "MARKER", path: root }, 30_000); break;
    case "6": out = await server.execute("client__glob", { pattern: "*.txt", path: root }, 30_000); break;
    case "7": out = await server.execute("client__read", { path: path.join(outside, "secret.txt") }, 30_000); break;
    case "8": out = await server.execute("client__bash", { command: `rm -rf ${path.join(root, "junk")}` }, 30_000); break;
    case "9": out = await server.execute("client__bash", { command: "sleep 30", timeout_ms: 2000 }, 60_000); break;
    case "f": out = await server.execute("client__bash", { command: `powershell -NoProfile -Command "('x' * 130000)"` }, 60_000); break;
    case "r": if (lastSpill) out = await server.execute("client__read", { path: lastSpill }, 30_000); break;
    default: continue;
  }
  const spill = (out as { result?: { spillPath?: string } }).result?.spillPath;
  if (spill) lastSpill = spill;
  console.log(`[driver] (${Date.now() - t0}ms)`, JSON.stringify(out, null, 2));
}
rl.close();
await server.close();
```

运行：

```bash
cd /d/Code/ai/ai-agent-code/remote-tools
npx -y tsx accept-driver.ts
# 记下打印的 ws 地址（ws://127.0.0.1:8787）、token 与 allowRoots 路径
```

> 端口 8787 被占用时，改脚本里的 `listen(8787)` 与桌面端 `serverUrl` 保持一致。
> `edit`（菜单 4）每次 driver 重启会重建 hello.txt，可重复执行。

### 终端 2 —— 桌面应用

```bash
cd /d/Code/ai/ai-agent-code/remote-tools-desktop
cargo tauri dev        # 首次编译较久；仅验证亦可 cargo build -p remote-tools-desktop
```

### 桌面端设置页（一次）

1. 托盘左键（或启动窗）打开主窗 → 右侧「设置」。
2. 填入：服务器地址 = 终端 1 打印的 ws 地址；令牌 = 终端 1 的 token（可点复制）；
   allowRoots 添加终端 1 打印的 `smoke-root` 路径；shell 保持默认（Windows = PowerShell）；
   输出上限保持默认 100000。
3. 「保存」→ **重启应用**（已知偏差：配置保存后需重启生效）。
4. 重启后等横幅转「已连接」、托盘变绿，再开始清单。

---

## 1 · 验收清单（brief 11 项）

约定：每项完成后勾选 `[ ]` → `[x]`，并在「结果」行填写实际观察（可附截图路径）。

### 1. 托盘三态：灰 → 绿 → 黄

- 步骤：① 应用启动、终端 1 未运行 → 看托盘；② 启动终端 1，等连接 → 看托盘；
  ③ 选单 `2`（write，弹确认框期间）→ 看托盘；④ 弹窗点「允许一次」→ 再看托盘。
- 预期：灰（未连接）→ 绿（已连接）→ 琥珀/黄（有待确认）→ 回绿。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 2. 连接横幅：断开显示原因 + 自动重连

- 步骤：① 终端 1 Ctrl+C 停掉 → 看主窗横幅与指令流水（应有隧道诊断行：退避重连）；
  ② 重新 `npx -y tsx accept-driver.ts` 启动 → 等待。
- 预期：断开后横幅「未连接」并显示断开原因；客户端按 1s→2s→…→60s 退避重连；
  服务端恢复后横幅自动转「已连接」。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 3. 确认弹窗三动作

- 3a 允许一次：选单 `1`（echo）→ 弹窗（黄顶带）点「允许一次」→ 终端 1 应打印 ok，
  stdout 含 `SMOKE_ECHO_9c2e`；再选单 `1` → **仍弹窗**（允许一次不记忆）。
  - [ X ] 结果：通过
- 3b 本会话内允许同类：再选单 `1` → 点「本会话内允许同类」→ 终端 1 ok；再选单 `1`
  → **不再弹窗**、立即回传（流水徽标为「自动放行」）。
  - [ X ] 结果：通过
- 3c 拒绝：选单 `2`（write）→ 点「拒绝」→ 终端 1 打印 `ok:false`，
  `error.code = "denied_by_user"`。
  - [ X ] 结果：通过

### 4. 会话规则断线清空

- 步骤：接 3b（bash 已本会话放行）→ 终端 1 Ctrl+C → 重启终端 1 → 等重连 → 选单 `1`。
- 预期：重连后同样的非危险 bash **重新弹确认框**（会话规则已随断线清空）。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 5. 六工具冒烟

依次选单 `3` → `2` → `4` → `5` → `6`（bash 已在 3a 验过）。每项终端 1 均应打印
`ok:true`，且：

- `3` read：result.content 含 `SMOKE_FILE_MARKER_7f3a`
- `2` write：result.path 指向 smoke-root/out.txt，文件确实生成
- `4` edit：ok，且随后 `5` grep 能搜到 `EDITED_MARKER_4b8d`
- `5` grep：结果含 `hello.txt:<行号>:…MARKER…`
- `6` glob：结果含 `hello.txt` 与 `out.txt`
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 6. 路径围栏：allowRoots 外 → path_denied，无弹窗

- 步骤：选单 `7`（read 白名单外的 secret.txt）。
- 预期：**桌面端全程无弹窗**；终端 1 打印 `ok:false`，`error.code = "path_denied"`；
  流水徽标「未通过」。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 7. 危险命令高亮：rm -rf → 红色边框弹窗

- 步骤：选单 `8`（bash `rm -rf <root>/junk`）。
- 预期：弹窗为红色顶带 + 命令块红色描边 +「危险」徽标；默认焦点落在「拒绝」。
- 说明：若点「允许」，PowerShell 可能因 `rm -rf` 参数不识别而报执行错误——本项只判定
  弹窗高亮，不影响。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 8. 超时：sleep 30 + timeout_ms=2000 → timeout

- 步骤：选单 `9`。注意：若 bash 非危险已获本会话放行（3b/4 之后），本项直接执行不弹窗，
  属预期；想同时看弹窗可先重启应用再单做本项。
- 预期：约 2 秒后终端 1 打印 `ok:false`，`error.code = "timeout"`；流水行耗时 ≈2000ms。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 9. 截断落盘：>100KB 输出 → truncated + spillPath 可读

- 步骤：选单 `f`（输出 130,000 字符）→ 看终端 1 的 result；随后选单 `r`
  （read 上次 spillPath）。
- 预期：`f` 返回 stdout 被截到 100,000 字符，`truncated:true`，
  `spillPath = <allowRoots 第一项>\.remote-tools\<id>.txt`；`r` 能读回完整内容
  （长度 130,000）——落盘目录自动纳入可读白名单，不触发 path_denied。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### 10. 内存：稳态 RSS ≤ 100MB（目标带 40-70MB）

- 步骤：完成若干工具调用后静置 1 分钟，任务管理器 → 详细信息 → `remote-tools.exe`
  → 「内存（活动的专用工作集）」。
- 预期：≤ 100MB；记录实测值。
- [ X ] 结果：＿15.7MB＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿

### 11. 主题：暗色默认；亮色切换 + 记忆；tokens 与 spec 一致

- 步骤：① 重启应用（清浏览器态不必要，首次未切换过即默认）→ 看整体配色；
  ② 设置页切到「亮色」→ 全窗生效；③ 重启应用 → 仍是亮色；④ 切回暗色；
  ⑤ tokens 抽查 `--glass-strong`（确认弹窗背景即该 token）：暗色应为
  `rgba(41, 49, 64, .9)`，亮色应为 `rgba(247, 250, 254, .92)`
  （= ui-visual-spec.md §3.1 / §3.2 逐字值；两种主题下各弹一次确认框比对观感）。
- 预期：默认暗色；亮色即时生效且跨重启记忆（localStorage `rt-theme`）；
  玻璃弹窗在暗色下呈深灰蓝、亮色下呈微蓝白，与上值一致。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

---

## 2 · 窗口生命周期（任务 7 修复回归）

### B1. 关闭主窗 → 托盘唤回 → 窗口恢复

- 步骤：点自绘标题栏「×」（或 Alt+F4）→ 观察托盘仍在、终端 2 的 `cargo tauri dev`
  仍在运行；托盘左键（或右键菜单「显示窗口」）。
- 预期：窗口只是隐藏不销毁；唤回后窗口重现并置前，此前流水/设置内容保留。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### B2. 窗口隐藏期间隧道保持连接

- 步骤：先关窗（隐藏）→ 终端 1 选单 `3`（read，自动放行）→ 看终端 1；
  加测：隐藏时选单 `2`（write）→ 托盘应变黄 → 托盘左键唤回 → 确认弹窗在场。
- 预期：隐藏不影响收发——`3` 正常回传；`2` 触发的确认使托盘变黄，唤回窗口后弹窗
  完整可操作。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

### B3. 托盘「退出」→ 进程退出无挂起

- 步骤：托盘右键 → 「退出」→ 看终端 2 回到 shell 提示符；任务管理器确认无
  `remote-tools.exe` 残留。
- 预期：进程即时硬退出，不悬挂（已知偏差：仍在运行的工具子进程不会被显式清理，
  见 README「已知偏差」c 条）。
- [ X ] 结果：通过（2026-09-21 全量验收，符合预期）

---

## 3 · 汇总

| # | 项目 | 通过 | 备注 |
|---|------|------|------|
| 1 | 托盘三态 | ✅ | |
| 2 | 断线横幅 + 自动重连 | ✅ | |
| 3 | 确认三动作 | ✅ | |
| 4 | 会话规则断线清空 | ✅ | |
| 5 | 六工具冒烟 | ✅ | |
| 6 | 路径围栏 path_denied | ✅ | |
| 7 | 危险命令红色高亮 | ✅ | |
| 8 | 超时回 timeout | ✅ | |
| 9 | 截断落盘 spillPath | ✅ | |
| 10 | 内存 ≤ 100MB | ✅ | 达标（具体值未记录） |
| 11 | 主题默认暗色/切换记忆/tokens | ✅ | |
| B1 | 关窗 → 托盘唤回 | ✅ | |
| B2 | 隐藏期间隧道在线 | ✅ | |
| B3 | 托盘退出无挂起 | ✅ | |

验收结论：**14/14 全部通过，无异常，符合预期**。期间修复 4 项真机缺陷（图标 BFINAL、窗体圆角对齐、滚动底距、状态事件竞态，commits 2d00ef9/1682212/9b5e515/9142956）。　签字/日期：用户口头确认 · 2026-09-21

### 异常与备注

- ＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿＿
