// 远程控制 P1 端到端：像浏览器一样调用 Rust 远程桥的 HTTP / WebSocket 接口，
// 在隔离的 Codex 桌面上走完「看会话 → 新建 / 打开 → 发消息 → 实时输出 → 审批 → 插话 → 中断 → 回答提问」。
//
// 前提：隔离桌面与假上游已按 README 启动；远程桥用 example 单独启动：
//   P1_HOME=… P1_APP=… P1_CDP_PORT=9333 P1_PORT=57399 P1_TOKEN=… \
//     cargo run -p codex-plus-core --example remote_control_serve
// 然后：P1_PORT=57399 P1_TOKEN=… P1_WORK=… node run-p1.mjs
import fs from "node:fs";
import http from "node:http";
import path from "node:path";

const PORT = Number(process.env.P1_PORT || 57399);
const TOKEN = process.env.P1_TOKEN;
const WORK = process.env.P1_WORK;
if (!TOKEN || !WORK) {
  console.error("需要 P1_TOKEN、P1_WORK");
  process.exit(2);
}
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = [];
function record(name, ok, detail = "") {
  results.push({ name, ok });
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
}

async function api(method, route, body, { token = TOKEN, headers = {} } = {}) {
  const response = await fetch(`${BASE}/api${route}`, {
    method,
    headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(body ? { "Content-Type": "application/json" } : {}), ...headers },
    body: body ? JSON.stringify(body) : undefined,
  });
  const payload = await response.json().catch(() => ({}));
  return { status: response.status, ...payload };
}

/** 订阅会话推送，记下最新视图。 */
function stream(threadId) {
  const socket = new WebSocket(`ws://127.0.0.1:${PORT}/api/threads/${threadId}/stream?token=${TOKEN}`);
  const box = { view: null, messages: 0, closed: null, socket };
  const waiters = new Set();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    box.messages += 1;
    if (message.type === "view") box.view = message.view;
    else box.closed = message;
    for (const waiter of [...waiters]) waiter();
  });
  box.until = (predicate, ms = 15000) => new Promise((resolve) => {
    const check = () => {
      const value = box.view && predicate(box.view);
      if (value) {
        clearTimeout(timer);
        waiters.delete(check);
        resolve(value);
      }
    };
    const timer = setTimeout(() => { waiters.delete(check); resolve(null); }, ms);
    waiters.add(check);
    check();
  });
  box.ready = new Promise((resolve) => socket.addEventListener("open", resolve));
  return box;
}

const lastTurn = (view) => view.turns.at(-1);
const idle = (view) => view.runtime === "idle";
const request = (method) => (view) => view.requests.find((item) => item.method === method);
const approvedFile = path.join(WORK, "approved.txt");

try {
  // 安全检查
  const page = await fetch(`${BASE}/`);
  const html = await page.text();
  record("网页可访问且带 CSP", page.status === 200 && html.includes("Codex++ 远程控制") && (page.headers.get("content-security-policy") || "").includes("frame-ancestors 'none'"));
  record("没有令牌被拒", (await api("GET", "/status", null, { token: "" })).status === 401);
  record("错误令牌被拒", (await api("GET", "/status", null, { token: "x".repeat(64) })).status === 401);
  record("跨站 Origin 被拒", (await api("GET", "/status", null, { headers: { Origin: "https://evil.example" } })).status === 403);
  // fetch 会无视自定义 Host，用 node:http 发。
  const rebindStatus = await new Promise((resolve) => {
    http.get({ host: "127.0.0.1", port: PORT, path: "/api/status", headers: { Host: "evil.example", Authorization: `Bearer ${TOKEN}` } }, (res) => {
      res.resume();
      resolve(res.statusCode);
    }).on("error", () => resolve(null));
  });
  record("非本机 Host 被拒（DNS 重绑定）", rebindStatus === 421, `HTTP ${rebindStatus}`);

  const status = await api("GET", "/status");
  record("状态：IPC 已连接", status.ok && status.data.ipc.connected, `版本表来源 ${status.data?.ipcVersionSource}`);

  const list = await api("GET", "/threads");
  record("会话列表", list.ok && Array.isArray(list.data.threads), `${list.data?.threads?.length ?? 0} 个`);

  // 没在桌面打开的旧会话：离线查看历史
  const old = list.data.threads.find((thread) => thread.title?.startsWith("P0"));
  if (old) {
    const offline = await api("GET", `/threads/${old.id}`);
    record("查看会话历史", offline.ok && offline.data.turns.length > 0, `${offline.data?.live ? "已在桌面打开" : "离线"}，${offline.data?.turns?.length} 轮`);
  }

  // 新建会话：侧车建 → 放锁 → 桌面打开
  const t0 = Date.now();
  const created = await api("POST", "/threads", { cwd: WORK, name: `P1 ${new Date().toISOString().slice(11, 19)}` });
  record("新建会话", created.ok, created.ok ? `${Date.now() - t0} ms` : created.error);
  if (!created.ok) throw new Error(created.error);
  const threadId = created.data.threadId;
  // 新建默认走后台（桌面不切换，见 run-p3）；P1 验证的是跟随桌面这条路，显式让桌面打开。
  const opened = await api("POST", `/threads/${threadId}/open`);
  record("在桌面打开", opened.ok && opened.data.live && !opened.data.headless, opened.error ?? "");

  const live = stream(threadId);
  await live.ready;
  const first = await live.until((view) => view.live && view);
  record("WebSocket 实时推送", !!first, `revision=${first?.revision}`);

  // 发消息
  await api("POST", `/threads/${threadId}/messages`, { text: "你好，来自 P1" });
  const replied = await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("P1")));
  record("发消息并收到回复", !!replied);

  // 审批：同意
  fs.rmSync(approvedFile, { force: true });
  await api("POST", `/threads/${threadId}/messages`, { text: "RUN 同意" });
  let pending = await live.until(request("item/commandExecution/requestApproval"));
  const accepted = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { decision: "accept" });
  await live.until((view) => idle(view) && !view.requests.length);
  record("远程同意命令", accepted?.ok && fs.existsSync(approvedFile), pending?.params?.command);

  // 审批：拒绝
  fs.rmSync(approvedFile, { force: true });
  await api("POST", `/threads/${threadId}/messages`, { text: "RUN 拒绝" });
  pending = await live.until(request("item/commandExecution/requestApproval"));
  const declined = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { decision: "decline" });
  const declinedView = await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "commandExecution" && item.status === "declined"));
  record("远程拒绝命令", declined?.ok && !!declinedView && !fs.existsSync(approvedFile));

  // 运行中再发消息 = 插话
  await api("POST", `/threads/${threadId}/messages`, { text: "SLOW 慢慢说" });
  await live.until((view) => view.runtime === "active");
  await sleep(1500);
  const steer = await api("POST", `/threads/${threadId}/messages`, { text: "插一句：请简短" });
  const steered = await live.until((view) => lastTurn(view)?.items.some((item) => item.type === "steeringUserMessage"), 8000);
  record("运行中发消息自动变插话", steer.ok && steer.data.kind === "steer" && !!steered, steer.error ?? steer.data?.kind);
  await live.until(idle, 30000);

  // 中断
  await api("POST", `/threads/${threadId}/messages`, { text: "SLOW 再说一遍" });
  await live.until((view) => view.runtime === "active");
  await sleep(1500);
  const stop = await api("POST", `/threads/${threadId}/interrupt`);
  const stopped = await live.until((view) => idle(view) && lastTurn(view)?.status === "interrupted", 8000);
  record("中断", stop.ok && !!stopped, stop.error ?? "");

  // 回答模型提问
  await api("POST", `/threads/${threadId}/messages`, { text: "ASK 问我一个问题" });
  pending = await live.until(request("item/tool/requestUserInput"));
  const questionId = pending?.params?.questions?.[0]?.id;
  const answered = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { answers: { [questionId]: { answers: ["蓝色"] } } });
  const answeredView = await live.until((view) => lastTurn(view)?.items.some((item) => item.type === "userInputResponse"));
  record("回答模型提问", answered?.ok && !!answeredView);
  await live.until(idle, 15000);

  // 已处理的请求再提交要报错，不能重复执行
  const stale = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { answers: {} });
  record("重复提交已处理的请求被拒", stale && !stale.ok, stale?.error);

  // 会话设置：模型、推理强度、权限模式、计划模式；用假上游的请求记录核对真的生效。
  const MOCK_LOG = process.env.P1_MOCK_LOG;
  const lastRequest = () => MOCK_LOG ? JSON.parse(fs.readFileSync(MOCK_LOG, "utf8").trim().split("\n").at(-1)) : null;
  const options = await api("GET", `/threads/${threadId}/options`);
  const model = options.data?.models?.find((item) => item.efforts?.includes("high") && item.id !== options.data?.current?.model);
  record("读取可选项", options.ok && options.data.models.length > 0 && options.data.permissionModes.length === 4 && options.data.collaborationModes.some((mode) => mode.id === "plan"),
    `${options.data?.models?.length} 个模型，当前 ${options.data?.current?.model} / ${options.data?.current?.permissionMode}`);
  const changed = await api("POST", `/threads/${threadId}/settings`, { model: model?.id, effort: "high", permissionMode: "full-access" });
  record("改模型、推理强度与权限", changed.ok && changed.data.controls.model === model?.id && changed.data.controls.effort === "high" && changed.data.controls.permissionMode === "full-access",
    JSON.stringify(changed.data?.controls ?? changed.error));
  const planned = await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "plan" });
  record("切到计划模式", planned.ok && planned.data.controls.collaborationMode === "plan" && planned.data.controls.model === model?.id);
  await api("POST", `/threads/${threadId}/messages`, { text: "设置生效检查" });
  await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("设置生效检查")));
  if (MOCK_LOG) {
    const sent = lastRequest();
    record("上游请求用上了新设置", sent.model === model?.id && sent.effort === "high" && sent.sandbox === "full" && sent.mode === "plan", JSON.stringify(sent));
  }
  const unplanned = await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "default", permissionMode: "auto", effort: null });
  record("退出计划模式、改回默认权限", unplanned.ok && unplanned.data.controls.collaborationMode === "default" && unplanned.data.controls.permissionMode === "auto", JSON.stringify(unplanned.data?.controls ?? unplanned.error));
  await api("POST", `/threads/${threadId}/messages`, { text: "回到默认模式" });
  await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("回到默认模式")));
  if (MOCK_LOG) {
    const sent = lastRequest();
    record("上游请求回到默认模式", sent.mode === "default" && sent.sandbox !== "full", JSON.stringify(sent));
  }
  const bad = await api("POST", `/threads/${threadId}/settings`, { permissionMode: "root" });
  record("未知权限模式被拒", !bad.ok, bad.error);

  record("推送次数合理（有合并）", live.messages > 5 && live.messages < 600, `${live.messages} 条`);
  live.socket.close();
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  process.exit(failed.length ? 1 : 0);
}
