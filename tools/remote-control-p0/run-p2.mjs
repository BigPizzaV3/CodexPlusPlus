// 远程控制 P2 端到端：像手机一样经局域网 HTTPS 访问 Rust 远程桥——
// 核对自签证书指纹、配对领设备令牌、按权限放行、wss 实时推送、远程审批与设置、吊销后推送连接立刻断开。
//
// 前提：隔离桌面与假上游已按 README 启动；远程桥带局域网入口启动：
//   P1_HOME=… P1_APP=… P1_CDP_PORT=9333 P1_PORT=57399 P1_TOKEN=… \
//   P1_LAN_PORT=57398 P1_STATE=/tmp/codex-p0/rc-state/remote-control.json P1_PAIR_ROLES=view,full \
//     target/debug/examples/remote_control_serve
// 然后：NODE_TLS_REJECT_UNAUTHORIZED=0 P1_PORT=57399 P1_TOKEN=… P2_LAN_PORT=57398 \
//   P2_DIR=/tmp/codex-p0/rc-state P1_WORK=… node run-p2.mjs
// （证书是自签的，这里关掉 Node 的默认校验，改为自己按状态文件里的证书核对指纹。）
import crypto from "node:crypto";
import fs from "node:fs";
import https from "node:https";
import os from "node:os";
import path from "node:path";
import tls from "node:tls";

const PORT = Number(process.env.P1_PORT || 57399);
const TOKEN = process.env.P1_TOKEN;
const LAN_PORT = Number(process.env.P2_LAN_PORT || 57398);
const DIR = process.env.P2_DIR;
const WORK = process.env.P1_WORK;
if (!TOKEN || !DIR || !WORK) {
  console.error("需要 P1_TOKEN、P2_DIR、P1_WORK");
  process.exit(2);
}
const lanIp = process.env.P2_HOST
  || Object.values(os.networkInterfaces()).flat().find((item) => item?.family === "IPv4" && !item.internal)?.address;
const LAN = `https://${lanIp}:${LAN_PORT}`;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = [];
function record(name, ok, detail = "") {
  results.push({ name, ok });
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
}

async function api(method, route, body, { token, base = LAN, headers = {} } = {}) {
  const response = await fetch(`${base}/api${route}`, {
    method,
    headers: { ...(token ? { Authorization: `Bearer ${token}` } : {}), ...(body ? { "Content-Type": "application/json" } : {}), ...headers },
    body: body ? JSON.stringify(body) : undefined,
  });
  const payload = await response.json().catch(() => ({}));
  return { status: response.status, ...payload };
}

/** 经 wss 订阅会话推送。 */
function stream(threadId, token) {
  const socket = new WebSocket(`wss://${lanIp}:${LAN_PORT}/api/threads/${threadId}/stream?token=${token}`);
  const box = { view: null, events: [], socket };
  const waiters = new Set();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    box.events.push(message.type);
    if (message.type === "view") box.view = message.view;
    for (const waiter of [...waiters]) waiter(message);
  });
  box.until = (predicate, ms = 15000) => new Promise((resolve) => {
    const check = (message) => {
      const value = predicate(box.view, message);
      if (value) {
        clearTimeout(timer);
        waiters.delete(check);
        resolve(value);
      }
    };
    const timer = setTimeout(() => { waiters.delete(check); resolve(null); }, ms);
    waiters.add(check);
    if (box.view) check(null);
  });
  box.ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve);
    socket.addEventListener("error", () => reject(new Error("wss 连不上")));
  });
  return box;
}

async function nextPairingCode(expectedRole) {
  for (let i = 0; i < 50; i += 1) {
    const text = fs.existsSync(path.join(DIR, "pairing-code.txt")) ? fs.readFileSync(path.join(DIR, "pairing-code.txt"), "utf8").trim() : "";
    const [code, role] = text.split(" ");
    if (code && role === expectedRole) return code;
    await sleep(200);
  }
  throw new Error(`等不到 ${expectedRole} 的配对码`);
}

const lastTurn = (view) => view?.turns.at(-1);
const idle = (view) => view?.runtime === "idle";

try {
  // 证书：服务端出示的就是状态目录里那张，且覆盖当前局域网地址
  const stored = JSON.parse(fs.readFileSync(path.join(DIR, "remote-control-tls.json"), "utf8"));
  const der = Buffer.from(stored.certPem.replace(/-----[^-]+-----/g, "").replace(/\s+/g, ""), "base64");
  const expected = crypto.createHash("sha256").update(der).digest("hex").toUpperCase().match(/../g).join(":");
  const served = await new Promise((resolve, reject) => {
    const socket = tls.connect({ host: lanIp, port: LAN_PORT, rejectUnauthorized: false, servername: undefined }, () => {
      resolve(socket.getPeerCertificate());
      socket.end();
    });
    socket.on("error", reject);
  });
  record("证书指纹与状态文件一致", served.fingerprint256 === expected, expected.slice(0, 23) + "…");
  record("证书覆盖局域网地址", (served.subjectaltname || "").includes(`IP Address:${lanIp}`), served.subjectaltname);

  const page = await fetch(`${LAN}/`);
  const html = await page.text();
  record("手机经 HTTPS 打开网页", page.status === 200 && html.includes("Codex++ 远程控制") && (page.headers.get("content-security-policy") || "").includes(`wss://${lanIp}:${LAN_PORT}`), lanIp);
  const manifest = await fetch(`${LAN}/manifest.webmanifest`);
  record("可添加到主屏幕（manifest）", manifest.status === 200 && (await manifest.json()).display === "standalone");

  record("局域网不认主令牌", (await api("GET", "/me", null, { token: TOKEN })).status === 401);
  const rebind = await new Promise((resolve) => {
    https.get({ host: lanIp, port: LAN_PORT, path: "/api/me", rejectUnauthorized: false, headers: { Host: `evil.example:${LAN_PORT}` } }, (res) => {
      res.resume();
      resolve(res.statusCode);
    }).on("error", () => resolve(null));
  });
  record("外部域名 Host 被拒（DNS 重绑定）", rebind === 421, `HTTP ${rebind}`);

  // 配对一台只读设备
  const viewCode = await nextPairingCode("view");
  const wrong = await api("POST", "/pair", { code: "WRONGCOD", name: "x" });
  record("错误配对码被拒", wrong.status === 400 && !wrong.ok, wrong.error);
  const viewPair = await api("POST", "/pair", { code: viewCode.toLowerCase(), name: "测试 iPhone" });
  record("配对只读设备", viewPair.ok && viewPair.data.token.length === 64, viewPair.error ?? viewPair.data.device.name);
  const viewToken = viewPair.data.token;
  const reuse = await api("POST", "/pair", { code: viewCode, name: "再来一次" });
  record("配对码只能用一次", !reuse.ok, reuse.error);
  const savedState = fs.readFileSync(path.join(DIR, "remote-control.json"), "utf8");
  record("状态文件里没有明文设备令牌", !savedState.includes(viewToken) && savedState.includes("tokenSha256"));

  const viewMe = await api("GET", "/me", null, { token: viewToken });
  record("只读设备身份", viewMe.ok && viewMe.data.role === "view" && viewMe.data.name === "测试 iPhone");
  const viewList = await api("GET", "/threads", null, { token: viewToken });
  record("只读设备能看会话列表", viewList.ok && Array.isArray(viewList.data.threads), `${viewList.data?.threads?.length} 个`);
  const viewCreate = await api("POST", "/threads", { cwd: WORK }, { token: viewToken });
  record("只读设备不能新建会话", viewCreate.status === 403, viewCreate.error);

  // 配对一台完全控制的设备，走完新建 → 发消息 → wss 实时 → 审批 → 设置
  const fullCode = await nextPairingCode("full");
  const fullPair = await api("POST", "/pair", { code: fullCode, name: "测试 Android" });
  const fullToken = fullPair.data?.token;
  record("配对完全控制设备", fullPair.ok);
  const created = await api("POST", "/threads", { cwd: WORK, name: `P2 ${new Date().toISOString().slice(11, 19)}` }, { token: fullToken });
  record("手机新建会话", created.ok, created.error ?? created.data.threadId);
  if (!created.ok) throw new Error(created.error);
  const threadId = created.data.threadId;

  const live = stream(threadId, fullToken);
  await live.ready;
  record("wss 实时推送", !!(await live.until((view) => view?.live && view)));
  const watcher = stream(threadId, viewToken);
  await watcher.ready;
  record("只读设备也能订阅推送", !!(await watcher.until((view) => view?.live && view)));

  await api("POST", `/threads/${threadId}/messages`, { text: "你好，来自手机" }, { token: fullToken });
  const replied = await live.until((view) => idle(view) && lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("手机")));
  record("手机发消息并收到回复", !!replied);
  record("只读设备同步看到回复", !!(await watcher.until((view) => lastTurn(view)?.items.some((item) => item.type === "agentMessage" && item.text?.includes("手机")))));
  const viewSend = await api("POST", `/threads/${threadId}/messages`, { text: "不该发出去" }, { token: viewToken });
  record("只读设备不能发消息", viewSend.status === 403, viewSend.error);

  const approvedFile = path.join(WORK, "approved.txt");
  fs.rmSync(approvedFile, { force: true });
  await api("POST", `/threads/${threadId}/messages`, { text: "RUN 手机审批" }, { token: fullToken });
  const pending = await live.until((view) => view?.requests.find((item) => item.method === "item/commandExecution/requestApproval"));
  const viewApprove = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { decision: "accept" }, { token: viewToken });
  record("只读设备不能审批", viewApprove?.status === 403, viewApprove?.error);
  const approved = pending && await api("POST", `/threads/${threadId}/requests/${pending.id}`, { decision: "accept" }, { token: fullToken });
  await live.until((view) => idle(view) && !view.requests.length);
  record("手机远程审批命令", approved?.ok && fs.existsSync(approvedFile));

  const planned = await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "plan" }, { token: fullToken });
  record("手机切计划模式", planned.ok && planned.data.controls.collaborationMode === "plan", planned.error);
  const viewSettings = await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "default" }, { token: viewToken });
  record("只读设备不能改设置", viewSettings.status === 403);
  await api("POST", `/threads/${threadId}/settings`, { collaborationMode: "default" }, { token: fullToken });

  // 吊销只读设备：已建立的推送连接要被断开，令牌立刻失效
  fs.writeFileSync(path.join(DIR, "revoke.txt"), viewPair.data.device.id);
  const t0 = Date.now();
  const revoked = await watcher.until((_, message) => message?.type === "revoked", 8000);
  record("吊销后推送连接立刻断开", !!revoked, `${Date.now() - t0} ms`);
  record("吊销后令牌失效", (await api("GET", "/me", null, { token: viewToken })).status === 401);
  record("其它设备不受影响", (await api("GET", "/me", null, { token: fullToken })).ok);

  // 本机入口：主令牌照常可用，也认设备令牌
  const master = await api("GET", "/me", null, { token: TOKEN, base: `http://127.0.0.1:${PORT}` });
  record("本机入口主令牌照常可用", master.ok && master.data.kind === "master");

  live.socket.close();
  watcher.socket.close();
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  process.exit(failed.length ? 1 : 0);
}
