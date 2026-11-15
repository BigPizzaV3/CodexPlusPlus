// 抓后台执行时 app-server 推送的通知结构（用于 Rust 侧把它转成网页视图）。
// 用法：P0_CODEX=… P0_HOME=… node poc-headless-dump.mjs [cwd]
import { spawn } from "node:child_process";
import fs from "node:fs";

const codexBin = process.env.P0_CODEX;
const codexHome = process.env.P0_HOME;
const cwd = process.argv[2] || "/tmp/codex-p0/work2";
const out = "/tmp/codex-p0/headless-notifications.jsonl";
fs.writeFileSync(out, "");

const child = spawn(codexBin, ["-c", "thread_unload_delay_secs=0", "app-server"], {
  env: { ...process.env, CODEX_HOME: codexHome, RUST_LOG: "error" },
  stdio: ["pipe", "pipe", "pipe"],
});
let buf = "";
let next = 1;
const pending = new Map();
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
      fs.appendFileSync(out, `${JSON.stringify({ __serverRequest: true, method: msg.method, id: msg.id, params: msg.params })}\n`);
      if (msg.method.includes("requestApproval")) send({ id: msg.id, result: { decision: "accept" } });
      else send({ id: msg.id, result: { answers: {} } });
      continue;
    }
    if (msg.method) fs.appendFileSync(out, `${JSON.stringify({ method: msg.method, params: msg.params })}\n`);
  }
});
child.stderr.on("data", () => {});

await call("initialize", { clientInfo: { name: "poc-dump", version: "0" }, capabilities: { experimentalApi: true } });
send({ method: "initialized", params: {} });
const started = await call("thread/start", { cwd, model: "mock-model", approvalPolicy: "on-request", sandbox: "read-only" });
const threadId = started.result?.thread?.id;
console.log("thread:", threadId, "keys:", Object.keys(started.result?.thread ?? {}).join(","));
console.log("turn/start keys:", Object.keys((await call("turn/start", { threadId, input: [{ type: "text", text: "RUN 通知结构" }] })).result ?? {}).join(","));
await new Promise((resolve) => setTimeout(resolve, 6000));
const read = await call("thread/read", { threadId, includeTurns: true });
console.log("history item types:", JSON.stringify((read.result?.thread?.turns ?? []).flatMap((turn) => (turn.items ?? []).map((item) => item.type))));
child.kill();
console.log("已写入", out);
