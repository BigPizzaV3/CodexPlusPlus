// 侧车 app-server：和桌面共用 CODEX_HOME，只做读、建会话、排队、目标这类不发起推理的操作。
// thread/queue/* 需要 initialize 时声明 capabilities.experimentalApi = true（桌面自己也这样做）。
//
// 写锁：一个会话同一时间只能有一个 app-server 持有写入权。侧车 thread/start 之后就持有了，
// 桌面再去 thread/resume 会报「thread … already has an active writer」，打不开这个会话。
// 所以侧车以 thread_unload_delay_secs=0 启动，建完会话立刻 unsubscribe 并等它卸载。
import { spawn } from "node:child_process";

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export async function startSidecar({ codexBin, codexHome }) {
  const child = spawn(codexBin, ["-c", "thread_unload_delay_secs=0", "app-server"], {
    env: { ...process.env, CODEX_HOME: codexHome, RUST_LOG: "error" },
    stdio: ["pipe", "pipe", "pipe"],
  });
  let buf = "";
  let next = 1;
  const pending = new Map();
  child.stdout.on("data", (chunk) => {
    buf += chunk;
    let index;
    while ((index = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, index).trim();
      buf = buf.slice(index + 1);
      if (!line) continue;
      let msg;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      if (msg.id != null && !msg.method && pending.has(msg.id)) {
        pending.get(msg.id)(msg);
        pending.delete(msg.id);
      } else if (msg.method && msg.id != null) {
        // 侧车不跑推理，不应该收到审批之类的服务端请求；收到就拒绝。
        child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id: msg.id, error: { code: -32601, message: "sidecar does not handle server requests" } })}\n`);
      }
    }
  });
  child.stderr.on("data", () => {});
  const call = (method, params) => new Promise((resolve) => {
    const id = next++;
    pending.set(id, resolve);
    child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
  });
  await call("initialize", { clientInfo: { name: "codex-plus-remote-sidecar", version: "0" }, capabilities: { experimentalApi: true } });
  child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", method: "initialized", params: {} })}\n`);

  async function isLoaded(threadId) {
    const loaded = await call("thread/loaded/list", {});
    return (loaded.result?.data ?? []).includes(threadId);
  }

  /** 放掉写锁：unsubscribe 后等会话卸载。 */
  async function release(threadId, timeoutMs = 5000) {
    await call("thread/unsubscribe", { threadId });
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
      if (!(await isLoaded(threadId))) return;
      await sleep(100);
    }
    throw new Error(`侧车 ${timeoutMs} ms 内没有卸载会话 ${threadId}`);
  }

  return {
    call,
    isLoaded,
    release,
    close: () => child.kill(),
    /**
     * 新建会话并落盘：thread/start 之后没有 rollout，别的进程（包括桌面）读不到；
     * thread/name/set 会把它落盘，且不往历史里塞假消息。最后放掉写锁交给桌面。
     */
    async createThread({ cwd, name }) {
      const started = await call("thread/start", { cwd });
      if (started.error) throw new Error(started.error.message);
      const threadId = started.result.thread.id;
      const named = await call("thread/name/set", { threadId, name });
      if (named.error) throw new Error(named.error.message);
      await release(threadId);
      return threadId;
    },
  };
}
