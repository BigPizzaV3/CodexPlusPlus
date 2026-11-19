// 跟随一个会话：维护会话状态的本地副本，并代理 follower 请求。
//
// owner（打开着这个会话的桌面窗口）通过广播 thread-stream-state-changed 推送：
//   { type: "snapshot", revision, conversationState }
//   { type: "patches", baseRevision, revision, patches: [{ op, path: [...], value }] }   // immer 补丁
// baseRevision 对不上时向 owner 要一次完整快照（thread-follower-load-complete-history）。
import { EventEmitter } from "node:events";

export function applyPatches(state, patches) {
  for (const { op, path, value } of patches) {
    if (path.length === 0) {
      if (op !== "remove") state = structuredClone(value);
      continue;
    }
    let parent = state;
    for (let i = 0; i < path.length - 1; i++) {
      if (parent[path[i]] == null) parent[path[i]] = typeof path[i + 1] === "number" ? [] : {};
      parent = parent[path[i]];
    }
    const key = path[path.length - 1];
    if (op === "remove") {
      if (Array.isArray(parent)) parent.splice(key, 1);
      else delete parent[key];
    } else if (op === "add" && Array.isArray(parent) && key !== "length") {
      parent.splice(key, 0, structuredClone(value));
    } else {
      parent[key] = structuredClone(value);
    }
  }
  return state;
}

export class ThreadFollower extends EventEmitter {
  constructor(ipc, threadId, { hostId = "local" } = {}) {
    super();
    this.ipc = ipc;
    this.threadId = threadId;
    this.hostId = hostId;
    this.state = null;
    this.revision = null;
    this.owner = null;
    this.onChange = (params, msg) => {
      if (params.conversationId !== this.threadId) return;
      const change = params.change;
      if (change.type === "snapshot") {
        this.state = structuredClone(change.conversationState);
        this.revision = change.revision;
        this.owner = msg.sourceClientId;
      } else {
        if (this.state == null || change.baseRevision !== this.revision) {
          this.emit("gap", { have: this.revision, base: change.baseRevision });
          void this.resync();
          return;
        }
        this.state = applyPatches(this.state, change.patches || []);
        this.revision = change.revision;
      }
      this.emit("state", this.state, change);
    };
    ipc.on("thread-stream-state-changed", this.onChange);
  }

  async start() {
    const owner = await this.ipc.request("thread-owner-discovery", { hostId: this.hostId, conversationId: this.threadId });
    if (owner.resultType !== "success") throw new Error(`no owner: ${JSON.stringify(owner)}`);
    this.owner = owner.handledByClientId;
    this.ipc.broadcast("thread-stream-following-changed", { conversationId: this.threadId, hostId: this.hostId, following: true });
    await this.resync();
  }

  resync() {
    return this.ipc.request("thread-follower-load-complete-history", { conversationId: this.threadId }, { targetClientId: this.owner, timeoutMs: 8000 });
  }

  stop() {
    this.ipc.broadcast("thread-stream-following-changed", { conversationId: this.threadId, hostId: this.hostId, following: false });
    this.ipc.off("thread-stream-state-changed", this.onChange);
  }

  call(method, params, timeoutMs = 20000) {
    return this.ipc.request(method, { conversationId: this.threadId, ...params }, { targetClientId: this.owner, timeoutMs });
  }

  /** 按显示顺序展开的轮次（历史在 turnHistory.history 里，快照的 turns 字段通常为空）。 */
  turns() {
    const history = this.state?.turnHistory?.history;
    if (!history) return this.state?.turns || [];
    return history.islands.flatMap((island) => island.entries.map((entry) => history.entitiesByKey[entry.value])).filter(Boolean);
  }

  // ---- 已验证的 follower 操作 ----

  startTurn(text, clientUserMessageId = crypto.randomUUID()) {
    return this.call("thread-follower-start-turn", {
      turnStart: {
        request: { threadId: this.threadId, input: [{ type: "text", text, text_elements: [] }], clientUserMessageId },
        context: {},
      },
    });
  }

  /** restoreMessage 不能为 null：owner 会读它的 cwd 和 context.commentAttachments。 */
  steer(text, cwd, clientUserMessageId = crypto.randomUUID()) {
    return this.call("thread-follower-steer-turn", {
      input: [{ type: "text", text, text_elements: [] }],
      clientUserMessageId,
      restoreMessage: {
        id: clientUserMessageId,
        text,
        cwd,
        context: { workspaceRoots: [cwd], commentAttachments: [], fileAttachments: [], collaborationMode: null },
      },
      attachments: [],
      serviceTier: null,
    });
  }

  interrupt(expectedTurnId) {
    return this.call("thread-follower-interrupt-turn", { mode: "user-stop", expectedTurnId });
  }

  /** decision：accept / decline / cancel，或 { acceptWithExecpolicyAmendment: {...} }（以请求里的 availableDecisions 为准）。 */
  decideCommand(requestId, decision) {
    return this.call("thread-follower-command-approval-decision", { requestId, decision });
  }

  decideFileChange(requestId, decision) {
    return this.call("thread-follower-file-approval-decision", { requestId, decision });
  }

  /** answers：{ [questionId]: { answers: [string] } } */
  answerUserInput(requestId, answers) {
    return this.call("thread-follower-submit-user-input", { requestId, response: { answers } });
  }
}
