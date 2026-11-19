// 远程控制 P0 回归：在隔离的 Codex 桌面上跑一遍全部已验证的链路。
// 前提（见 README.md）：隔离桌面已启动（独立 CODEX_HOME、独立 user-data-dir、开着 CDP），假上游已启动。
//
// 环境变量：
//   P0_HOME      隔离的 CODEX_HOME（必填）
//   P0_WORK      会话工作目录（必填，需在 config.toml 里 trusted）
//   P0_CODEX     侧车用的 codex 可执行文件（必填，通常是 <ChatGPT.app>/Contents/Resources/codex-cli/bin/codex）
//   P0_CDP_PORT  隔离桌面的调试端口（默认 9333）
//
// 绝不连接用户自己的 ~/.codex：P0_HOME 指向 ~/.codex 时直接退出。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { IpcClient, defaultSocketPath } from "./ipc-client.mjs";
import { ThreadFollower } from "./follower.mjs";
import { startSidecar } from "./sidecar.mjs";

const HOME = process.env.P0_HOME;
const WORK = process.env.P0_WORK;
const CODEX = process.env.P0_CODEX;
const CDP_PORT = Number(process.env.P0_CDP_PORT || 9333);
if (!HOME || !WORK || !CODEX) {
  console.error("需要 P0_HOME、P0_WORK、P0_CODEX，见 README.md");
  process.exit(2);
}
if (path.resolve(HOME) === path.join(os.homedir(), ".codex")) {
  console.error("P0_HOME 不能是 ~/.codex：只在隔离实例上跑");
  process.exit(2);
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = [];
function record(name, ok, detail = "") {
  results.push({ name, ok, detail });
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
}

async function cdpEval(expression) {
  const targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json`)).json();
  const page = targets.find((t) => t.type === "page" && t.url.startsWith("app://-/index.html"));
  if (!page) throw new Error("隔离桌面没有主窗口");
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve) => ws.addEventListener("open", resolve));
  const value = await new Promise((resolve) => {
    ws.addEventListener("message", (event) => {
      const msg = JSON.parse(event.data);
      if (msg.id === 1) resolve(msg.result?.result?.value);
    });
    ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
  });
  ws.close();
  return value;
}

/** 让桌面打开会话：正式实现由 Codex++ 的注入脚本发同一条页面消息。 */
const openInDesktop = (threadId) =>
  cdpEval(`window.dispatchEvent(new MessageEvent("message", { data: { type: "navigate-to-route", path: "/local/${threadId}" } })); document.title`);

async function waitOwner(ipc, threadId, ms = 8000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const r = await ipc.request("thread-owner-discovery", { hostId: "local", conversationId: threadId }, { timeoutMs: 800 });
    if (r.resultType === "success") return r.handledByClientId;
    await sleep(250);
  }
  return null;
}

function waitFor(follower, predicate, ms) {
  return new Promise((resolve) => {
    const timer = setTimeout(() => {
      follower.off("state", check);
      resolve(null);
    }, ms);
    const check = (state) => {
      const value = predicate(state);
      if (value) {
        clearTimeout(timer);
        follower.off("state", check);
        resolve(value);
      }
    };
    follower.on("state", check);
    if (follower.state) check(follower.state);
  });
}

const pendingRequest = (method) => (state) => (state.requests || []).find((r) => r.method === method);
const idle = (state) => state.threadRuntimeStatus?.type === "idle" && state;
const approvedFile = path.join(WORK, "approved.txt");

const sidecar = await startSidecar({ codexBin: CODEX, codexHome: HOME });
const ipc = new IpcClient(defaultSocketPath(HOME), { clientType: "codex-plus-remote-p0" });

try {
  await ipc.connect();
  record("IPC 握手", true, `clientId=${ipc.clientId.slice(0, 8)}`);

  // 1. 侧车新建会话 → 桌面打开 → 成为 owner
  const threadId = await sidecar.createThread({ cwd: WORK, name: `P0 ${new Date().toISOString().slice(11, 19)}` });
  record("侧车建会话后放掉写锁", !(await sidecar.isLoaded(threadId)));
  // 侧车读历史不能重新占住写锁，否则桌面打不开
  await sidecar.call("thread/read", { threadId, includeTurns: true });
  record("侧车读会话不占写锁", !(await sidecar.isLoaded(threadId)));
  const t0 = Date.now();
  await openInDesktop(threadId);
  const owner = await waitOwner(ipc, threadId);
  record("新建会话并由桌面接管", !!owner, owner ? `${Date.now() - t0} ms` : "8 秒内没有 owner");
  if (!owner) throw new Error("没有 owner，后续无法继续");

  const follower = new ThreadFollower(ipc, threadId);
  await follower.start();
  record("跟随会话并收到快照", follower.state != null, `revision=${follower.revision}`);

  // 2. 发消息
  const sent = await follower.startTurn("你好，来自远程");
  const reply = await waitFor(follower, (s) => idle(s) && follower.turns().at(-1)?.items?.some((i) => i.type === "agentMessage" && i.text?.includes("来自远程")), 15000);
  record("远程发消息并收到流式回复", sent.resultType === "success" && !!reply);

  // 3. 审批：同意
  fs.rmSync(approvedFile, { force: true });
  await follower.startTurn("RUN 同意");
  let req = await waitFor(follower, pendingRequest("item/commandExecution/requestApproval"), 15000);
  if (req) await follower.decideCommand(req.id, "accept");
  await waitFor(follower, (s) => idle(s) && !(s.requests || []).length, 15000);
  record("远程同意命令", !!req && fs.existsSync(approvedFile), req ? req.params.command : "没收到审批请求");

  // 4. 审批：拒绝
  fs.rmSync(approvedFile, { force: true });
  await follower.startTurn("RUN 拒绝");
  req = await waitFor(follower, pendingRequest("item/commandExecution/requestApproval"), 15000);
  if (req) await follower.decideCommand(req.id, "decline");
  await waitFor(follower, (s) => idle(s) && !(s.requests || []).length, 15000);
  const declined = follower.turns().at(-1)?.items?.some((i) => i.type === "commandExecution" && i.status === "declined");
  record("远程拒绝命令", !!req && !fs.existsSync(approvedFile) && declined);

  // 5. 插话
  const slow = await follower.startTurn("SLOW 慢慢说");
  const turnId = slow.result?.result?.turn?.id;
  await sleep(2500);
  const steer = await follower.steer("插一句：请简短", WORK);
  const steered = await waitFor(follower, () => follower.turns().at(-1)?.items?.some((i) => i.type === "steeringUserMessage"), 8000);
  record("运行中插话", steer.resultType === "success" && !!steered, steer.error ?? "");
  await waitFor(follower, idle, 30000);

  // 6. 中断
  const slow2 = await follower.startTurn("SLOW 再说一遍");
  await sleep(2000);
  const stop = await follower.interrupt(slow2.result?.result?.turn?.id ?? turnId);
  const stopped = await waitFor(follower, (s) => idle(s) && follower.turns().at(-1)?.status === "interrupted", 8000);
  record("中断", stop.resultType === "success" && !!stopped);

  // 7. 回答模型提问（需要 features.default_mode_request_user_input，或会话处于 Plan 模式）
  await follower.startTurn("ASK 问我一个问题");
  req = await waitFor(follower, pendingRequest("item/tool/requestUserInput"), 15000);
  if (req) await follower.answerUserInput(req.id, { [req.params.questions[0].id]: { answers: ["蓝色"] } });
  const answered = await waitFor(follower, () => follower.turns().at(-1)?.items?.some((i) => i.type === "userInputResponse"), 15000);
  record("回答模型提问", !!req && !!answered, req ? "" : "没收到提问（检查 default_mode_request_user_input）");
  await waitFor(follower, idle, 15000);

  // 8. 侧车能读到桌面写入的历史
  const read = await sidecar.call("thread/read", { threadId, includeTurns: true });
  record("侧车读取历史", (read.result?.thread?.turns?.length ?? 0) >= 6, `${read.result?.thread?.turns?.length ?? 0} 轮`);

  follower.stop();
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  sidecar.close();
  ipc.close();
  const failed = results.filter((r) => !r.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  setTimeout(() => process.exit(failed.length ? 1 : 0), 200);
}
