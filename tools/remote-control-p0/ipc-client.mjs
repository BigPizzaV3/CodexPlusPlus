// Codex 桌面 IPC 路由的客户端（远程控制 P0 参考实现）。
//
// 传输：macOS / Linux 为 <CODEX_HOME>/ipc/ipc.sock，Windows 为 \\.\pipe\codex-ipc。
// 帧：[u32 LE 长度][UTF-8 JSON]。消息类型：request / response / broadcast /
// client-discovery-request / client-discovery-response。
//
// 我们注册成一个普通客户端：对路由器的 discovery 一律回 canHandle:false，
// 不接任何别人的请求，只发 follower 请求和跟随广播。
import net from "node:net";
import os from "node:os";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { EventEmitter } from "node:events";

/** 26.1111（build 27194）里的方法版本表；正式实现应从已安装 app 的 asar 里读（见 ipc-versions.mjs）。 */
export const DEFAULT_VERSIONS = {
  "thread-stream-state-changed": 13,
  "thread-stream-following-changed": 1,
  "thread-stream-following-status-requested": 1,
  "ipc-connection-reset": 1,
  "thread-read-state-changed": 3,
  "thread-archived": 2,
  "thread-unarchived": 1,
  "thread-owner-discovery": 1,
  "thread-follower-start-turn": 2,
  "thread-follower-load-complete-history": 1,
  "thread-follower-compact-thread": 1,
  "thread-follower-steer-turn": 1,
  "thread-follower-interrupt-turn": 4,
  "thread-follower-update-thread-settings": 2,
  "thread-follower-update-daybreak": 1,
  "thread-follower-read-model-settings": 1,
  "thread-follower-edit-last-user-turn": 2,
  "thread-follower-command-approval-decision": 1,
  "thread-follower-file-approval-decision": 1,
  "thread-follower-permissions-request-approval-response": 1,
  "thread-follower-submit-user-input": 1,
  "thread-follower-submit-mcp-server-elicitation-response": 1,
  "thread-follower-set-queued-follow-ups-state": 3,
  "thread-follower-remove-queued-message": 1,
  "thread-follower-clear-queued-messages": 1,
  "thread-queued-followups-changed": 3,
};

/** 与桌面主进程相同的路径规则（不处理 macOS 路径过长时的 /tmp 符号链接回退）。 */
export function defaultSocketPath(codexHome = process.env.CODEX_HOME || path.join(os.homedir(), ".codex")) {
  if (process.platform === "win32") return "\\\\.\\pipe\\codex-ipc";
  return path.join(codexHome, "ipc", "ipc.sock");
}

export class IpcClient extends EventEmitter {
  constructor(socketPath = defaultSocketPath(), { clientType = "codex-plus-remote", versions = DEFAULT_VERSIONS } = {}) {
    super();
    this.socketPath = socketPath;
    this.clientType = clientType;
    this.versions = versions;
    this.clientId = "initializing-client";
    this.pending = new Map();
    this.buf = Buffer.alloc(0);
  }

  frame(obj) {
    const body = Buffer.from(JSON.stringify(obj), "utf8");
    const head = Buffer.alloc(4);
    head.writeUInt32LE(body.length, 0);
    return Buffer.concat([head, body]);
  }

  write(obj) {
    this.sock.write(this.frame(obj));
  }

  connect() {
    return new Promise((resolve, reject) => {
      this.sock = net.connect(this.socketPath);
      this.sock.once("error", reject);
      this.sock.on("data", (chunk) => this.onData(chunk));
      this.sock.on("close", () => this.emit("close"));
      this.sock.on("connect", async () => {
        const init = await this.request("initialize", { clientType: this.clientType });
        if (init.resultType !== "success") return reject(new Error(`initialize failed: ${JSON.stringify(init)}`));
        this.clientId = init.result.clientId;
        resolve(this.clientId);
      });
    });
  }

  onData(chunk) {
    this.buf = Buffer.concat([this.buf, chunk]);
    while (this.buf.length >= 4) {
      const size = this.buf.readUInt32LE(0);
      if (this.buf.length < 4 + size) break;
      const msg = JSON.parse(this.buf.subarray(4, 4 + size).toString("utf8"));
      this.buf = this.buf.subarray(4 + size);
      if (msg.type === "response" && this.pending.has(msg.requestId)) {
        const entry = this.pending.get(msg.requestId);
        this.pending.delete(msg.requestId);
        clearTimeout(entry.timer);
        entry.resolve(msg);
        continue;
      }
      if (msg.type === "client-discovery-request") {
        this.write({ type: "client-discovery-response", requestId: msg.requestId, response: { canHandle: false } });
        continue;
      }
      if (msg.type === "request") {
        this.write({ type: "response", requestId: msg.requestId, resultType: "error", error: "unsupported" });
        continue;
      }
      if (msg.type === "broadcast") {
        this.emit("broadcast", msg);
        this.emit(msg.method, msg.params, msg);
        continue;
      }
      this.emit("other", msg);
    }
  }

  /**
   * 发请求。路由器在 targetClientId 缺省时向所有客户端做 discovery；
   * 当某个客户端不回 discovery 时路由器会等 10 秒，所以本地超时要比 timeoutMs 略长。
   */
  request(method, params, { targetClientId, hostId, timeoutMs = 10000 } = {}) {
    return new Promise((resolve) => {
      const requestId = randomUUID();
      const timer = setTimeout(() => {
        this.pending.delete(requestId);
        resolve({ resultType: "error", error: "local-timeout" });
      }, timeoutMs + 2000);
      this.pending.set(requestId, { resolve, timer });
      this.write({
        type: "request", requestId, sourceClientId: this.clientId, version: this.versions[method] ?? 0,
        method, params, targetClientId, hostId, timeoutMs,
      });
    });
  }

  broadcast(method, params, targetClientIds) {
    this.write({ type: "broadcast", method, sourceClientId: this.clientId, version: this.versions[method] ?? 0, params, targetClientIds });
  }

  close() {
    for (const entry of this.pending.values()) clearTimeout(entry.timer);
    this.pending.clear();
    this.sock?.end();
  }
}
