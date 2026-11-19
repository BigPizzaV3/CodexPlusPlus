// 验证「侧车自己跑一轮」：thread/start + turn/start，审批由侧车接，桌面完全不用动。
// 前提：隔离 CODEX_HOME + 假上游。
// 用法：P0_CODEX=… P0_HOME=… node poc-headless-turn.mjs [cwd]
import { spawn } from "node:child_process";

const codexBin = process.env.P0_CODEX;
const codexHome = process.env.P0_HOME;
const cwd = process.argv[2] || "/tmp/codex-p0/work2";
if (!codexBin || !codexHome) {
  console.error("需要 P0_CODEX、P0_HOME");
  process.exit(2);
}

const child = spawn(codexBin, ["-c", "thread_unload_delay_secs=0", "app-server"], {
  env: { ...process.env, CODEX_HOME: codexHome, RUST_LOG: "error" },
  stdio: ["pipe", "pipe", "pipe"],
});
let buf = "";
let next = 1;
const pending = new Map();
const notifications = [];
const serverRequests = [];
const send = (message) => child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
const call = (method, params) => new Promise((resolve) => {
  const id = next++;
  pending.set(id, resolve);
  send({ id, method, params });
});

child.stdout.on("data", (chunk) => {
  buf += chunk;
  let index;
  while ((index = buf.indexOf("\n")) >= 0) {
    const line = buf.slice(0, index).trim();
    buf = buf.slice(index + 1);
    if (!line) continue;
    let msg;
    try { msg = JSON.parse(line); } catch { continue; }
    if (msg.id != null && !msg.method) { pending.get(msg.id)?.(msg); pending.delete(msg.id); continue; }
    if (msg.method && msg.id != null) {
      serverRequests.push(msg);
      // 审批：命令执行一律同意，其它提交空回答。
      if (msg.method.includes("requestApproval")) {
        const decision = msg.params?.availableDecisions?.includes("accept") ? "accept" : "accept";
        send({ id: msg.id, result: { decision } });
        console.log("→ 已同意:", msg.method, msg.params?.command ?? "");
      } else {
        send({ id: msg.id, result: { answers: {} } });
        console.log("→ 已回答:", msg.method);
      }
      continue;
    }
    if (msg.method) {
      notifications.push(msg);
      if (/turn\/(completed|failed)|item\/(started|completed)/.test(msg.method)) {
        console.log("·", msg.method, msg.params?.item?.type ?? msg.params?.turn?.status ?? "");
      }
    }
  }
});
child.stderr.on("data", (chunk) => {
  const text = String(chunk).trim();
  if (text) console.log("stderr:", text.slice(0, 200));
});

await call("initialize", { clientInfo: { name: "poc-headless", version: "0" }, capabilities: { experimentalApi: true } });
send({ method: "initialized", params: {} });

const started = await call("thread/start", { cwd });
console.log("thread/start:", started.error ? started.error : started.result.thread.id);
const threadId = started.result?.thread?.id;
if (!threadId) process.exit(1);

const turnStart = await call("turn/start", { threadId, input: [{ type: "text", text: "RUN 后台执行检查" }] });
console.log("turn/start:", JSON.stringify(turnStart.error ?? turnStart.result?.turn?.id));

const deadline = Date.now() + 60000;
let last = null;
while (Date.now() < deadline) {
  const read = await call("thread/read", { threadId, includeTurns: true });
  const turn = read.result?.thread?.turns?.at(-1);
  last = { status: turn?.status, items: (turn?.items ?? []).map((item) => item.type) };
  if (turn?.status && turn.status !== "inProgress") break;
  await new Promise((resolve) => setTimeout(resolve, 700));
}
console.log("最后一轮:", JSON.stringify(last));
console.log("审批请求:", serverRequests.map((request) => request.method).join(", ") || "无");
console.log("通知数:", notifications.length, "类型:", [...new Set(notifications.map((n) => n.method))].join(", "));
child.kill();
