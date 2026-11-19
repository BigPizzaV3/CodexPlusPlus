// 远程控制 P3 端到端：没在桌面打开的会话由后台执行，桌面窗口不切换。
// 核对：发消息 / 实时推送 / 审批 / 插话 / 中断 / 改设置都在后台完成，桌面当前页面始终不变、
// 会话没有桌面 owner；跑完放锁后「在桌面打开」能接过去。
//
// 前提同 run-p1（隔离桌面 + 假上游 + example 远程桥），另需桌面调试端口：
//   P1_PORT=57399 P1_TOKEN=… P1_WORK=/tmp/codex-p0/work2 P1_MOCK_LOG=… P3_CDP_PORT=9333 node run-p3.mjs
import fs from "node:fs";
import path from "node:path";

const PORT = Number(process.env.P1_PORT || 57399);
const TOKEN = process.env.P1_TOKEN;
const WORK = process.env.P1_WORK;
const CDP_PORT = Number(process.env.P3_CDP_PORT || 9333);
const MOCK_LOG = process.env.P1_MOCK_LOG;
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

async function api(method, route, body) {
  const response = await fetch(`${BASE}/api${route}`, {
    method,
    headers: { Authorization: `Bearer ${TOKEN}`, ...(body ? { "Content-Type": "application/json" } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const payload = await response.json().catch(() => ({}));
  return { status: response.status, ...payload };
}

/** 桌面当前显示的页面（路由），用来证明桌面没被切走。 */
async function desktopRoute() {
  const targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
  const page = targets.find((target) => target.type === "page" && target.url.startsWith("app://-/index.html"));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));
  const value = await new Promise((resolve) => {
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id === 1) resolve(message.result?.result?.value);
    });
    // 路由在 history state / 路径里；再带上主区域标题，切会话时一定会变。
    ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: {
      expression: `JSON.stringify({ path: location.pathname + location.search + location.hash, route: (history.state && (history.state.usr?.path || history.state.key)) || "", title: document.title, thread: (document.querySelector('[data-thread-id]')?.getAttribute('data-thread-id')) || "" })`,
      returnByValue: true,
    } }));
  });
  ws.close();
  return value;
}

function stream(threadId) {
  const socket = new WebSocket(`ws://127.0.0.1:${PORT}/api/threads/${threadId}/stream?token=${TOKEN}`);
  const box = { view: null, socket, messages: 0 };
  const waiters = new Set();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    box.messages += 1;
    if (message.type === "view") box.view = message.view;
    for (const waiter of [...waiters]) waiter();
  });
  box.until = (predicate, ms = 20000) => new Promise((resolve) => {
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
const lastRequest = () => (MOCK_LOG ? JSON.parse(fs.readFileSync(MOCK_LOG, "utf8").trim().split("\n").at(-1)) : null);

try {
  const routeBefore = await desktopRoute();
  record("读到桌面当前页面", !!routeBefore, routeBefore);

  const created = await api("POST", "/threads", { cwd: WORK, name: `P3 后台 ${new Date().toISOString().slice(11, 19)}` });
  record("新建会话（不让桌面打开）", created.ok, created.error ?? created.data.threadId);
  if (!created.ok) throw new Error(created.error);
  const threadId = created.data.threadId;
  record("新建后桌面没切走", (await desktopRoute()) === routeBefore);

  const live = stream(threadId);
  await live.ready;
  const first = await live.until((view) => view);
  record("推送来自后台会话", first?.headless === true, `live=${first?.live} headless=${first?.headless}`);

  const sent = await api("POST", `/threads/${threadId}/messages`, { text: "你好，来自后台" });
  record("发消息走后台", sent.ok && sent.data.background === true, sent.error ?? sent.data.kind);
  const replied = await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("后台")));
  record("后台跑完并实时推送回复", !!replied);
  record("发消息后桌面仍没切走", (await desktopRoute()) === routeBefore);

  // 审批
  const approvedFile = path.join("/tmp/codex-p0/work", "approved.txt");
  fs.rmSync(approvedFile, { force: true });
  await api("POST", `/threads/${threadId}/messages`, { text: "RUN 后台审批" });
  const pending = await live.until((view) => view.requests.find((item) => item.method === "item/commandExecution/requestApproval"));
  record("后台运行时审批出现在网页上", !!pending, pending?.params?.command);
  const accepted = pending && await api("POST", `/threads/${threadId}/requests/${encodeURIComponent(JSON.stringify(pending.id))}`, { decision: "accept" });
  await live.until((view) => idle(view) && !view.requests.length);
  await sleep(300);
  record("远程同意后命令执行", accepted?.ok && fs.existsSync(approvedFile), accepted?.error ?? "");
  const stale = pending && await api("POST", `/threads/${threadId}/requests/${encodeURIComponent(JSON.stringify(pending.id))}`, { decision: "accept" });
  record("重复提交已处理的请求被拒", stale && !stale.ok, stale?.error);

  // 插话
  await api("POST", `/threads/${threadId}/messages`, { text: "SLOW 慢慢说" });
  await live.until((view) => view.runtime === "active");
  await sleep(1200);
  const steer = await api("POST", `/threads/${threadId}/messages`, { text: "插一句：后台插话" });
  record("运行中发消息变插话", steer.ok && steer.data.kind === "steer", steer.error ?? steer.data?.kind);
  await live.until(idle, 40000);

  // 中断
  await api("POST", `/threads/${threadId}/messages`, { text: "SLOW 再说一遍" });
  await live.until((view) => view.runtime === "active");
  await sleep(1200);
  const stop = await api("POST", `/threads/${threadId}/interrupt`);
  const stopped = await live.until((view) => idle(view) && lastTurn(view)?.status === "interrupted", 10000);
  record("后台中断", stop.ok && !!stopped, stop.error ?? lastTurn(live.view)?.status);

  // 改设置：记在后台会话上，随下一轮发出，不打开桌面
  const options = await api("GET", `/threads/${threadId}/options`);
  const model = options.data?.models?.find((item) => item.efforts?.includes("high"));
  const changed = await api("POST", `/threads/${threadId}/settings`, { model: model?.id, effort: "high", permissionMode: "full-access" });
  record("后台会话改模型 / 推理 / 权限", changed.ok && changed.data.headless && changed.data.controls.permissionMode === "full-access", changed.error ?? JSON.stringify(changed.data?.controls));
  await api("POST", `/threads/${threadId}/messages`, { text: "后台设置检查" });
  await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("后台设置检查")));
  if (MOCK_LOG) {
    const request = lastRequest();
    record("上游请求用上了新设置", request.model === model?.id && request.effort === "high" && request.sandbox === "full", JSON.stringify(request));
  }
  record("全程桌面没被切走", (await desktopRoute()) === routeBefore);
  live.socket.close();
  const plan = await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "plan" }).catch(() => null);

  // 计划模式需要桌面：这一步会让桌面打开它（也就验证了跑完后锁已放掉、桌面能接过去）
  record("切计划模式时转到桌面打开", plan?.ok && plan.data.live && !plan.data.headless, plan?.error ?? "");
  const routeAfter = await desktopRoute();
  record("这时桌面才切到该会话", routeAfter !== routeBefore, routeAfter);
  const view = await api("GET", `/threads/${threadId}`);
  record("桌面看到后台跑过的全部轮次", view.ok && view.data.turns.length >= 5, `${view.data?.turns?.length} 轮`);
  await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "default", permissionMode: "auto" });
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  process.exit(failed.length ? 1 : 0);
}
