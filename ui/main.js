/* ======================================================================
   remote-tools · ui/main.js
   T7 契约（名称逐字）：
   listen  tunnel-status {connected, reason?} / tool-log (string | LogEntry)
           confirm-request {id, tool, summary, danger, reason} / confirm-resolved {id}
   invoke  get_config() / save_config({config}) / resolve_confirm({id, action}) / recent_logs()
   主题约定：documentElement.dataset.theme + localStorage["rt-theme"]，暗色默认（无 attr）。
   ====================================================================== */

const T = window.__TAURI__ || {};
const invoke = T.core && typeof T.core.invoke === "function" ? T.core.invoke.bind(T.core) : null;
const listen = T.event && typeof T.event.listen === "function" ? T.event.listen.bind(T.event) : null;
const clipboard = T.clipboardManager || T["clipboard-manager"] || null;

const $ = (id) => document.getElementById(id);

/* ---------- 内联图标（全部 stroke/fill currentColor，不引用布景 token、不由 SVG 画 imagery） ---------- */
const ICONS = {
  bash: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><polyline points="4 5 9 10 4 15"/><line x1="11" y1="15" x2="16" y2="15"/></svg>',
  read: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M6 3h5.5L16 7.5V17H6z"/><path d="M11.5 3v4.5H16"/><line x1="8.5" y1="10.5" x2="13.5" y2="10.5"/><line x1="8.5" y1="13.5" x2="13.5" y2="13.5"/></svg>',
  write: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M4 16l1.2-4.2L14.5 2.5l3 3L8.2 14.8 4 16z"/><line x1="12.5" y1="4.5" x2="15.5" y2="7.5"/></svg>',
  edit: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M16 10.5V16H4V4h5.5"/><path d="M8.5 11.5 15.5 4.5l1 1-7 7-2.5.5z"/></svg>',
  grep: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="8.5" cy="8.5" r="5"/><line x1="12.5" y1="12.5" x2="16.5" y2="16.5"/></svg>',
  glob: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><circle cx="10" cy="10" r="6.5"/><path d="M10 6.6v6.8M7.2 8.3l5.6 3.4M12.8 8.3l-5.6 3.4"/></svg>',
  tunnel: '<svg width="13" height="13" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><polyline points="3 10 6.5 10 9 5 11.5 15 13.5 10 17 10"/></svg>',
  check: '<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.4" stroke-linecap="round" stroke-linejoin="round"><polyline points="3.5 8.5 6.5 11.5 12.5 4.5"/></svg>',
  clock: '<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round"><circle cx="8" cy="8" r="6"/><polyline points="8 4.5 8 8 10.5 9.5"/></svg>',
  x: '<svg width="12" height="12" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="2.6" stroke-linecap="round"><path d="M4 4l8 8M12 4l-8 8"/></svg>',
  copy: '<svg width="15" height="15" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><rect x="7" y="7" width="10" height="10" rx="2.5"/><path d="M5 13H4.5A1.5 1.5 0 0 1 3 11.5v-7A1.5 1.5 0 0 1 4.5 3h7A1.5 1.5 0 0 1 13 4.5V5"/></svg>',
  warn: '<svg width="15" height="15" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M10 3.5 18 17H2z"/><line x1="10" y1="9" x2="10" y2="12.5"/><circle cx="10" cy="14.8" r=".4" fill="currentColor"/></svg>',
  folder: '<svg width="15" height="15" viewBox="0 0 20 20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M3 6.5a2 2 0 0 1 2-2h3.4l2 2.3H15a2 2 0 0 1 2 2v6.7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>',
  lock: '<svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"><rect x="4.5" y="10.5" width="15" height="9.5" rx="3"/><path d="M8 10.5V7.8a4 4 0 0 1 8 0v2.7"/></svg>',
  pencil: '<svg width="21" height="21" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 19l1.4-5L15.5 4.9l3.6 3.6L10 17.6 5 19z"/><line x1="13.6" y1="6.8" x2="17.2" y2="10.4"/></svg>',
};
function icoSpan(svg) {
  const s = document.createElement("span");
  s.className = "ico";
  s.innerHTML = svg; /* 静态字面量，非用户数据 */
  return s;
}
function toolIcon(tool) {
  if (ICONS[tool]) return ICONS[tool];
  const tail = String(tool || "").split("__").pop();
  return ICONS[tail] || "";
}

/* ---------- 全局状态 ---------- */
const state = {
  connected: false,
  reason: "",
  entries: [],      /* 与 loglist DOM 同步（新→旧），上限 MAX_ROWS */
  cfg: null,        /* get_config() 原样缓存（保存时展开，保留 dangerous/toolOverrides 等未编辑字段） */
  roots: [],
  shellKind: "",    /* "" 默认 | powershell | pwsh | cmd | sh | __custom */
};

/* ---------- 主题（暗色默认：无 data-theme 即暗色） ---------- */
function applyTheme(t) {
  const light = t === "light";
  if (light) document.documentElement.dataset.theme = "light";
  else delete document.documentElement.dataset.theme;
  try { localStorage.setItem("rt-theme", light ? "light" : "dark"); } catch (_) {}
  syncSeg($("theme-seg"), "themeOpt", light ? "light" : "dark");
}
function syncSeg(seg, attr, value) {
  for (const b of seg.querySelectorAll(".seg")) b.classList.toggle("on", b.dataset[attr] === value);
}

/* ---------- 连接横幅 + LED ---------- */
function setStatus(connected, reason) {
  state.connected = connected;
  state.reason = reason || "";
  const st = $("conn-st");
  st.textContent = connected ? "已连接" : "未连接";
  st.classList.toggle("ok", connected);
  const rsn = $("conn-rsn");
  rsn.textContent = !connected && state.reason ? state.reason : "";
  rsn.hidden = !rsn.textContent;
  refreshLeds();
}
function refreshLeds() {
  const pending = Boolean(currentConfirm || confirmQueue.length);
  for (const id of ["banner-led", "rail-led"]) {
    const el = $(id);
    el.classList.toggle("wait", pending);                       /* 黄脉冲：有待确认 */
    el.classList.toggle("off", !pending && !state.connected);   /* 灰：未连接 */
  }
}

/* ---------- 指令流水 ---------- */
const MAX_ROWS = 200;

/* tool-log 双形状：纯字符串（隧道诊断旧形状）或 camelCase LogEntry */
function normalize(raw) {
  if (typeof raw === "string") {
    return { id: "", tool: "tunnel", summary: raw, ok: true, durationMs: 0, confirmed: false };
  }
  const l = raw || {};
  return {
    id: l.id != null ? String(l.id) : "",
    tool: l.tool || "",
    summary: l.summary != null ? String(l.summary) : (l.log != null ? String(l.log) : ""),
    ok: l.ok !== false,
    durationMs: Number(l.durationMs ?? l.duration_ms ?? 0) || 0,
    confirmed: Boolean(l.confirmed),
  };
}
/* §8 判定徽标。LogEntry 无错误码：ok:false 且 confirmed:true = 用户拒绝（确证）；
   其余 ok:false（规则拒绝/执行失败）无以区分，统一中性灰「未通过」，不滥用红色。 */
function judgment(e) {
  if (e.tool === "tunnel") return null;                          /* 诊断行不打判定章 */
  if (e.ok && !e.confirmed) return { cls: "pass", label: "自动放行", icon: ICONS.check };
  if (e.ok && e.confirmed) return { cls: "man", label: "人工确认后放行", icon: ICONS.clock };
  if (e.confirmed) return { cls: "denyfill", label: "已拒绝", icon: ICONS.x, deny: true };
  return { cls: "fail", label: "未通过", icon: ICONS.x };
}
function fmtDur(e) {
  if (!e.ok && e.confirmed) return "—";                          /* 拒绝发生在执行前 */
  const ms = e.durationMs;
  return ms >= 1000 ? (ms / 1000).toFixed(1) + "s" : ms + "ms";
}
function rowEl(e) {
  const row = document.createElement("div");
  row.className = "row";

  const chip = document.createElement("span");
  chip.className = "chip";
  const svg = toolIcon(e.tool);
  if (svg) chip.appendChild(icoSpan(svg));
  chip.appendChild(document.createTextNode(e.tool || "—"));

  const wrap = document.createElement("span");
  wrap.className = "cmdwrap";
  const cmd = document.createElement("span");
  cmd.className = "cmd";
  cmd.textContent = e.summary || "（无摘要）";
  cmd.title = e.summary || "";
  wrap.appendChild(cmd);

  const dur = document.createElement("span");
  dur.className = "dur";
  dur.textContent = fmtDur(e);

  row.appendChild(chip);
  row.appendChild(wrap);
  row.appendChild(dur);

  const j = judgment(e);
  if (j) {
    const b = document.createElement("span");
    b.className = "badge " + j.cls;
    if (j.icon) b.appendChild(icoSpan(j.icon));
    b.appendChild(document.createTextNode(j.label));
    row.appendChild(b);
    if (j.deny) row.classList.add("deny");
  } else {
    row.appendChild(document.createElement("span"));             /* 诊断行补齐第 4 列 */
  }
  return row;
}
function renderLog(raw) {
  const e = normalize(raw);
  const list = $("loglist");
  const empty = list.querySelector(".empty");
  if (empty) empty.remove();
  list.prepend(rowEl(e));
  state.entries.unshift(e);
  while (list.children.length > MAX_ROWS) {
    list.lastElementChild.remove();
    state.entries.pop();
  }
  updateStats();
}
/* 横幅计数与构成条：只统计真实工具执行（tunnel 诊断不计），三值互斥不重复 */
function updateStats() {
  let pass = 0, man = 0, fail = 0;
  for (const e of state.entries) {
    if (e.tool === "tunnel") continue;
    if (e.ok) { if (e.confirmed) man += 1; else pass += 1; }
    else fail += 1;
  }
  $("t-pass").textContent = String(pass);
  $("t-man").textContent = String(man);
  $("t-fail").textContent = String(fail);
  const total = pass + man + fail;
  const seg = $("segbar");
  seg.hidden = total === 0;
  if (total) {
    seg.querySelector(".s1").style.width = (pass / total) * 100 + "%";
    seg.querySelector(".s2").style.width = (man / total) * 100 + "%";
    seg.querySelector(".s3").style.width = (fail / total) * 100 + "%";
  }
  $("log-cnt").textContent = state.entries.length ? "最近 " + state.entries.length + " 条" : "";
}

/* ---------- 确认弹窗（三动作逐字：once / session / deny） ---------- */
let confirmQueue = [];
let currentConfirm = null;

function onConfirmRequest(payload) {
  if (!payload || typeof payload !== "object") return;
  confirmQueue.push(payload);
  flushConfirm();
}
function flushConfirm() {
  if (currentConfirm || !confirmQueue.length) return;
  currentConfirm = confirmQueue.shift();
  paintConfirm(currentConfirm);
  $("confirm-overlay").classList.add("open");
  /* 危险操作默认焦点落在「拒绝」，误触回车不会放行 */
  (currentConfirm.danger ? $("c-deny") : $("c-once")).focus();
  refreshLeds();
}
function closeConfirm() {
  currentConfirm = null;
  $("confirm-overlay").classList.remove("open");
  refreshLeds();
  flushConfirm();
}
async function resolveConfirm(action) {
  if (!currentConfirm || !invoke) return;
  const id = currentConfirm.id;
  closeConfirm();
  try {
    await invoke("resolve_confirm", { id, action });
  } catch (_) { /* 已处理/已取消（后端裁决即真相） */ }
}
function onConfirmResolved(payload) {
  const id = payload && payload.id;
  if (!id) return;
  if (currentConfirm && currentConfirm.id === id) { closeConfirm(); return; }
  const i = confirmQueue.findIndex((c) => c && c.id === id);
  if (i >= 0) confirmQueue.splice(i, 1);
  refreshLeds();
}
function paintConfirm(c) {
  const danger = Boolean(c.danger);
  $("c-head").className = "m-head " + (danger ? "h-red" : "h-yel");
  $("c-ico").innerHTML = danger ? ICONS.lock : ICONS.pencil;
  $("c-title").textContent = danger ? "危险命令确认" : "需要确认";
  $("c-sub").textContent = danger ? "放行前请确认命令全文" : "操作等待放行";
  $("c-rule").textContent = danger ? "危险" : (c.tool || "");
  $("c-tool").textContent = c.tool || "—";
  $("c-block").className = "cblock" + (danger ? " danger" : "");

  const line = $("c-summary");
  line.textContent = "";
  if (String(c.tool || "").endsWith("bash")) {
    const ps = document.createElement("span");
    ps.className = "ps";
    ps.textContent = "$";
    line.appendChild(ps);
  }
  line.appendChild(document.createTextNode(c.summary || "（无预览）"));

  const reason = String(c.reason || "").trim();
  const note = $("c-note");
  if (reason && reason !== String(c.summary || "").trim()) {
    note.hidden = false;
    note.className = "mnote " + (danger ? "danger" : "warn");
    $("c-note-ico").innerHTML = ICONS.warn;
    $("c-note-txt").textContent = reason;
  } else {
    note.hidden = true;
  }
}

/* ---------- 设置页 ---------- */
const SHELL_PRESETS = ["powershell", "pwsh", "cmd", "sh"];
function applyShell(shell) {
  if (!shell) { state.shellKind = ""; state.shellCustom = ""; }
  else if (SHELL_PRESETS.includes(shell)) { state.shellKind = shell; state.shellCustom = ""; }
  else { state.shellKind = "__custom"; state.shellCustom = shell; }
  const custom = $("f-shell-custom");
  custom.hidden = state.shellKind !== "__custom";
  custom.value = state.shellCustom;
  syncSeg($("shell-seg"), "shell", state.shellKind);
}
function shellValue() {
  return state.shellKind === "__custom" ? $("f-shell-custom").value.trim() : state.shellKind;
}
function setRoots(list) {
  state.roots = (list || []).slice();
  renderRoots();
}
function renderRoots() {
  const wrapEl = $("roots");
  wrapEl.textContent = "";
  for (const path of state.roots) {
    const chip = document.createElement("span");
    chip.className = "root";
    chip.appendChild(icoSpan(ICONS.folder));
    const p = document.createElement("span");
    p.className = "p";
    p.textContent = path;
    const rm = document.createElement("button");
    rm.type = "button";
    rm.className = "rm";
    rm.setAttribute("aria-label", "移除 " + path);
    rm.innerHTML = '<svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round"><path d="M1.5 1.5l7 7M8.5 1.5l-7 7"/></svg>';
    rm.addEventListener("click", () => {
      state.roots = state.roots.filter((x) => x !== path);
      renderRoots();
    });
    chip.appendChild(p);
    chip.appendChild(rm);
    wrapEl.appendChild(chip);
  }
}
function addRoot() {
  const input = $("root-input");
  const v = input.value.trim();
  if (!v) return;
  if (!state.roots.includes(v)) state.roots.push(v);
  input.value = "";
  renderRoots();
  input.focus();
}
let noteTimer = 0;
function saveNote(kind, text) {
  const el = $("save-note");
  el.className = "note" + (kind === "err" ? " err" : "");
  $("save-note-txt").textContent = text;
  clearTimeout(noteTimer);
  if (kind !== "err") {
    noteTimer = setTimeout(() => {
      el.className = "note";
      $("save-note-txt").textContent = "重启应用生效";
    }, 3000);
  }
}
async function saveConfig(ev) {
  ev.preventDefault();
  if (!invoke) return;
  const base = state.cfg || {};
  const config = Object.assign({}, base, {
    serverUrl: $("f-url").value.trim(),
    token: $("f-token").value,
    shell: shellValue(),
    allowRoots: state.roots.slice(),
    maxOutputChars: Math.max(1, Math.floor(Number($("f-max").value) || 100000)),
  });
  try {
    await invoke("save_config", { config });
    state.cfg = config;
    $("conn-url").textContent = config.serverUrl || "尚未配置服务器";
    saveNote("ok", "已保存 · 重启应用生效");
  } catch (err) {
    saveNote("err", "保存失败：" + (typeof err === "string" ? err : (err && err.message) || "未知错误"));
  }
}
async function loadConfig() {
  if (!invoke) return;
  try {
    const c = await invoke("get_config");
    state.cfg = c || {};
    $("f-url").value = state.cfg.serverUrl || "";
    $("f-token").value = state.cfg.token || "";
    applyShell(state.cfg.shell || "");
    $("f-max").value = String(state.cfg.maxOutputChars != null ? state.cfg.maxOutputChars : 100000);
    setRoots(Array.isArray(state.cfg.allowRoots) ? state.cfg.allowRoots : []);
    $("conn-url").textContent = state.cfg.serverUrl || "尚未配置服务器";
  } catch (err) {
    console.error("get_config 失败", err);
  }
}

/* ---------- 复制令牌 ---------- */
async function copyToken() {
  const value = $("f-token").value;
  try {
    if (clipboard && typeof clipboard.writeText === "function") await clipboard.writeText(value);
    else if (navigator.clipboard && navigator.clipboard.writeText) await navigator.clipboard.writeText(value);
    else return;
    const btn = $("copy-token");
    btn.innerHTML = ICONS.check;
    btn.classList.add("done");
    setTimeout(() => {
      btn.innerHTML = ICONS.copy;
      btn.classList.remove("done");
    }, 1200);
  } catch (_) {}
}

/* ---------- 窗口控制（decorations:false 自绘标题栏） ---------- */
function winApi() {
  try { return (T.window && T.window.getCurrentWindow && T.window.getCurrentWindow()) || null; }
  catch (_) { return null; }
}
function winCall(fn) {
  const w = winApi();
  if (!w) return;
  try {
    const r = fn(w);
    if (r && typeof r.catch === "function") r.catch(() => {});
  } catch (_) {}
}
async function toggleMaximize() {
  const w = winApi();
  if (!w) return;
  try {
    const maxed = w.isMaximized ? await w.isMaximized() : false;
    if (maxed && w.unmaximize) await w.unmaximize();
    else if (w.maximize) await w.maximize();
    else if (w.toggleMaximize) await w.toggleMaximize();
  } catch (_) {}
}

/* ---------- 事件接线 ---------- */
function wire() {
  $("win-min").addEventListener("click", () => winCall((w) => w.minimize()));
  $("win-max").addEventListener("click", toggleMaximize);
  $("win-hide").addEventListener("click", () => winCall((w) => w.hide()));   /* 主窗可关不影响运行：隐藏驻托盘 */
  document.querySelector(".titlebar").addEventListener("dblclick", (ev) => {
    if (ev.target.closest(".tb-btn")) return;
    toggleMaximize();
  });

  $("nav-log").addEventListener("click", () => { document.body.dataset.state = "log"; });
  $("nav-settings").addEventListener("click", () => { document.body.dataset.state = "settings"; });

  $("theme-seg").addEventListener("click", (ev) => {
    const b = ev.target.closest(".seg");
    if (b) applyTheme(b.dataset.themeOpt);
  });
  $("shell-seg").addEventListener("click", (ev) => {
    const b = ev.target.closest(".seg");
    if (!b) return;
    state.shellKind = b.dataset.shell;
    $("f-shell-custom").hidden = state.shellKind !== "__custom";
    syncSeg($("shell-seg"), "shell", state.shellKind);
  });

  $("root-add").addEventListener("click", addRoot);
  $("root-input").addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") { ev.preventDefault(); addRoot(); }
  });
  $("settings-form").addEventListener("submit", saveConfig);
  $("copy-token").addEventListener("click", copyToken);

  $("c-once").addEventListener("click", () => resolveConfirm("once"));
  $("c-session").addEventListener("click", () => resolveConfirm("session"));
  $("c-deny").addEventListener("click", () => resolveConfirm("deny"));

  if (listen) {
    listen("tunnel-status", (e) => {
      const p = e.payload || {};
      setStatus(Boolean(p.connected), p.reason || "");
    });
    listen("tool-log", (e) => renderLog(e.payload));
    listen("confirm-request", (e) => onConfirmRequest(e.payload));
    listen("confirm-resolved", (e) => onConfirmResolved(e.payload));
  } else {
    console.error("window.__TAURI__ 事件 API 不可用：检查 tauri.conf withGlobalTauri / capabilities");
  }
}

/* ---------- 启动 ---------- */
wire();

$("copy-token").innerHTML = ICONS.copy;
$("save-note-ico").innerHTML = ICONS.warn;

let savedTheme = "dark";
try { savedTheme = localStorage.getItem("rt-theme") || "dark"; } catch (_) {}
applyTheme(savedTheme === "light" ? "light" : "dark");

setStatus(false, "");

if (invoke) {
  /* recent_logs() 已是时间倒序（新→旧）；renderLog 是头部插入，故需倒着回放才能在页面上保持新→旧 */
  invoke("recent_logs").then((ls) => { for (let i = (ls || []).length - 1; i >= 0; i--) renderLog(ls[i]); }).catch(() => {});
  // 启动补拉当前连接状态——tunnel-status 事件可能在 listener 就绪前发出（竞态实锤 2026-09-21）
  invoke("get_status").then((s) => { if (s && typeof s.connected === "boolean") setStatus(s.connected, s.reason); }).catch(() => {});
  loadConfig();
} else {
  console.error("window.__TAURI__ 命令 API 不可用：检查 tauri.conf withGlobalTauri / capabilities");
}
