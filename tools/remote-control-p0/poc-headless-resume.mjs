// 验证「侧车接管已有会话」：thread/resume 后 turn/start、插话（turn/steer）、中断，以及放锁后桌面能再打开。
// 用法：P0_CODEX=… P0_HOME=… node poc-headless-resume.mjs <threadId> <cwd>
import { spawn } from "node:child_process";

const codexBin = process.env.P0_CODEX;
const codexHome = process.env.P0_HOME;
const threadId = process.argv[2];
const cwd = process.argv[3] || "/tmp/codex-p0/work";
if (!codexBin || !codexHome || !threadId) {
  console.error("需要 P0_CODEX、P0_HOME、threadId");
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
const requests = [];
const send = (message) => child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
const call = (method, params) => new Promise((resolve) => {
  const id = next++;
  pending.set(id, resolve);
  send({ id, method, params });
});
let resolveTurnDone = null;

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
      requests.push(msg.method);
      if (msg.method.includes("requestApproval")) send({ id: msg.id, result: { decision: "accept" } });
      else send({ id: msg.id, result: { answers: {} } });
      continue;
    }
    if (msg.method) {
      notifications.push(msg.method);
      if (msg.method === "turn/completed") resolveTurnDone?.(msg.params);
    }
  }
});
child.stderr.on("data", (chunk) => { const t = String(chunk).trim(); if (t) console.log("stderr:", t.slice(0, 160)); });

const waitTurn = (ms = 90000) => new Promise((resolve) => {
  const timer = setTimeout(() => { resolveTurnDone = null; resolve("timeout"); }, ms);
  resolveTurnDone = (params) => { clearTimeout(timer); resolveTurnDone = null; resolve(params); };
});

await call("initialize", { clientInfo: { name: "poc-resume", version: "0" }, capabilities: { experimentalApi: true } });
send({ method: "initialized", params: {} });

const resumed = await call("thread/resume", { threadId, cwd, approvalPolicy: "on-request", sandbox: "workspace-write" });
console.log("thread/resume:", resumed.error ? JSON.stringify(resumed.error) : "ok", "轮数:", resumed.result?.thread?.turns?.length);
if (resumed.error) process.exit(1);

let done = waitTurn();
const started = await call("turn/start", { threadId, input: [{ type: "text", text: "SLOW 慢慢说，等插话" }] });
console.log("turn/start:", JSON.stringify(started.error ?? started.result?.turn?.id));
await new Promise((resolve) => setTimeout(resolve, 1500));
const steer = await call("turn/steer", { threadId, input: [{ type: "text", text: "插一句：简短点" }] });
console.log("turn/steer:", JSON.stringify(steer.error ?? "ok"));
console.log("第一轮:", JSON.stringify((await done)?.turn?.status ?? done));

// 中断：起一轮然后 interrupt
done = waitTurn();
await call("turn/start", { threadId, input: [{ type: "text", text: "SLOW 再说一遍" }] });
await new Promise((resolve) => setTimeout(resolve, 1500));
const interrupted = await call("turn/interrupt", { threadId });
console.log("turn/interrupt:", JSON.stringify(interrupted.error ?? "ok"), "结果:", JSON.stringify((await done)?.turn?.status ?? done));

const read = await call("thread/read", { threadId, includeTurns: true });
const turns = read.result?.thread?.turns ?? [];
console.log("历史轮数:", turns.length, "最后两轮类型:", turns.slice(-2).map((turn) => `${turn.status}:[${(turn.items ?? []).map((item) => item.type).join(",")}]`));
console.log("审批请求:", [...new Set(requests)].join(", ") || "无");

// 放锁：unsubscribe 后卸载，桌面才能再打开
await call("thread/unsubscribe", { threadId });
for (let i = 0; i < 50; i += 1) {
  const loaded = await call("thread/loaded/list", {});
  if (!(loaded.result?.data ?? []).includes(threadId)) { console.log("已放锁"); break; }
  await new Promise((resolve) => setTimeout(resolve, 100));
}
child.kill();
