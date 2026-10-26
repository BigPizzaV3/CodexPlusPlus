import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/85-side-chat-save.js", import.meta.url), "utf8");

test("manager and in-app toggles share the persisted setting and obey the master switch", async () => {
  const style = readFileSync(new URL("../../../assets/inject/renderer-inject/10-style.js", import.meta.url), "utf8");
  const writes: any[] = [];
  const context: any = { window: {}, codexPlusBackendSettings: {}, conversationViewDefaultWidth: 900,
    codexPlusSettingsKey: "settings", localStorage: { getItem: () => '{"sideChatSave":true}', setItem: () => assert.fail("must use backend") },
    setBackendSetting: async (key: string, value: any) => { writes.push([key, value]); }, loadBackendSettings() {},
  };
  const start = style.indexOf("  function defaultCodexPlusSettings()");
  const setter = style.indexOf("  function setCodexPlusSetting(");
  vm.runInNewContext(style.slice(start, style.indexOf("  // Dream skin runtime", start))
    + style.slice(setter, style.indexOf("  function syncStepwisePanel(", setter)), context);
  assert.equal(context.defaultCodexPlusSettings().sideChatSave, false);
  context.codexPlusBackendSettings = { codexAppSideChatSave: false };
  assert.equal(context.codexPlusSettings().sideChatSave, false);
  context.codexPlusBackendSettings = { codexAppSideChatSave: true };
  assert.equal(context.codexPlusSettings().sideChatSave, true);
  context.codexPlusBackendSettings.enhancementsEnabled = false;
  assert.equal(context.codexPlusSettings().sideChatSave, false);
  context.setCodexPlusSetting("sideChatSave", false);
  await Promise.resolve();
  assert.deepEqual(writes, [["codexAppSideChatSave", false]]);
});

function harness(options: { source?: string; post?: (path: string, body: any) => Promise<any> } = {}) {
  const calls: any[] = [];
  const context: any = { window: { __CODEX_PLUS_TEST_SIDE_CHAT__: true }, WeakRef, Map, Set, Date, JSON,
    crypto: { randomUUID: () => "22222222-2222-4222-8222-222222222222" },
    codexPlusSettings: () => ({ sideChatSave: true }), registerCodexPlusExtensionSelector() {},
    postJson: async (path: string, body: any) => { calls.push({ path, body }); return options.post?.(path, body) ?? { status: "ok", source_id: body.source_id, content: options.source || "" }; },
    document: { querySelectorAll: () => [] },
    showToast: (...args: any[]) => calls.push({ toast: args }),
    loadOptionalCodexAppModule: async () => ({}),
    refreshRecentConversationsForHost: async () => true,
    codexServiceTierDispatcherFromModule: () => null,
  };
  vm.runInNewContext(source, context);
  return { api: context.window.__codexPlusSideChatTest, calls, context };
}

function conversation() {
  return { id: "side", hostId: "local", ephemeral: true, sideConversation: true, forkedFromId: "parent", createdAt: 2000,
    requests: [], turns: [{ turnId: "t1", status: "completed", items: [{ id: "u1", type: "userMessage", content: [{ type: "text", text: "hello" }] }] }] };
}

test("save mounts only on the native tab header, not its controller or content panel", () => {
  const { api, context } = harness();
  const close = {};
  const header = { querySelector: (selector: string) =>
    selector === ':scope > [data-app-shell-tab-close-button]' || selector === 'button[aria-label]' ? close : null };
  const controller = { querySelector: (selector: string) =>
    selector === '[data-tab-id^="sidechat:"]' ? header : selector === 'button[aria-label]' ? close : null };
  // 内容区有菜单按钮；错误挂载的保存按钮也有 aria-label，不能据此认成标签头。
  const panel = { querySelector: (selector: string) =>
    selector === 'button[aria-label]' || selector.includes('side-chat-save') ? {} : null };
  context.document.querySelectorAll = () => [controller, header, panel];
  const tabs = api.tabs();
  assert.equal(tabs.length, 1);
  assert.equal(tabs[0], header);
});

test("saving keeps its text and exactly one spinner across repeated scans", async () => {
  const { api, context } = harness();
  const children: any[] = [], attributes = new Map<string, string>();
  let label = "";
  const button: any = {
    dataset: {},
    get textContent() { return label; },
    set textContent(value: string) { label = value; children.length = 0; },
    getAttribute: (key: string) => attributes.get(key),
    setAttribute: (key: string, value: string) => attributes.set(key, value),
    removeAttribute: (key: string) => attributes.delete(key),
    appendChild: (child: any) => children.push(child),
  };
  const tab = { getAttribute: () => "sidechat:side", querySelector: (selector: string) =>
    selector.includes("side-chat-save") ? button : selector === ':scope > [data-app-shell-tab-close-button]' ? {} : null };
  context.document.querySelectorAll = () => [tab];
  context.document.createElement = () => ({ setAttribute() {} });
  const manager = { requestClient: {}, getConversation: () => conversation() };
  api.state.manager = new WeakRef(manager);
  const entry = { busy: true };
  api.state.entries.set("side", entry);
  for (let scan = 0; scan < 3; scan++) {
    api.refresh();
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(button.textContent, "保存中");
    assert.equal(children.length, 1);
    assert.equal(children[0].className, "codex-plus-side-chat-save-spinner");
    assert.equal(button.disabled, true);
  }
  entry.busy = false;
  api.refresh();
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(button.textContent, "保存");
  assert.equal(children.length, 0);
  assert.equal(button.disabled, false);
});

test("captures parent history before creating the side, without adding later parent changes", async () => {
  const reference = { byte_length: 160000000, sha256: "captured-prefix", last_turn_id: "t0" };
  const { api, calls } = harness({ post: async () => ({ status: "ok", source_id: "parent", source_ref: reference }) });
  let sends = 0;
  const result = await api.send({ hostId: "local" }, "thread/fork", { threadId: "parent", ephemeral: true, lastTurnId: "t0" }, async () => {
    sends++; assert.equal(calls[0].path, "/side-chat/source"); return { thread: { id: "side" } };
  });
  assert.equal(result.thread.id, "side");
  assert.equal(sends, 1);
  assert.deepEqual(api.state.entries.get("side").source.source_ref, reference);
  assert.equal(api.state.entries.get("side").source.content, undefined);
  assert.equal(calls[0].body.last_turn_id, "t0");
});

test("source capture failure keeps native creation working and blocks an inaccurate save", async () => {
  const { api } = harness({ post: async () => { throw Error("source unavailable"); } });
  await api.send({ hostId: "local" }, "thread/fork", { threadId: "parent", ephemeral: true }, async () => ({ thread: { id: "side" } }));
  await assert.rejects(api.source(conversation(), api.state.entries.get("side")), /source unavailable/);
});

test("does not capture remote forks or ordinary provider requests", async () => {
  const { api, calls } = harness();
  let sends = 0;
  for (const [hostId, method] of [["remote", "thread/fork"], ["local", "turn/start"]]) {
    await api.send({ hostId }, method, { threadId: "parent", ephemeral: true }, async () => { sends++; return {}; });
  }
  assert.equal(sends, 2); assert.equal(calls.length, 0);
});

test("closing a side releases its snapshot only after native success, including when disabled", async () => {
  const { api, context } = harness();
  context.codexPlusSettings = () => ({ sideChatSave: false });
  for (const method of ["thread/stop", "thread/unsubscribe"]) {
    api.state.entries.set("side", { source: "snapshot" });
    await assert.rejects(api.send({ hostId: "local" }, method, { threadId: "side" }, async () => { throw Error("close failed"); }));
    assert.ok(api.state.entries.has("side"));
    await api.send({ hostId: "local" }, method, { threadId: "side" }, async () => ({}));
    assert.equal(api.state.entries.has("side"), false);
  }
});

test("blocks active turns, approval requests, running tools, and incomplete paginated history", () => {
  const { api } = harness();
  const active = conversation(); active.turns[0].status = "inProgress";
  assert.throws(() => api.ready(active), /等待/);
  const approval: any = conversation(); approval.requests = [{}];
  assert.throws(() => api.ready(approval), /等待/);
  const tool: any = conversation(); tool.turns[0].items.push({ status: "inProgress" });
  assert.throws(() => api.ready(tool), /等待/);
  assert.throws(() => api.ready({ ...conversation(), turnsPagination: { hasLoadedOldest: false } }), /完整加载/);
});

test("uses canonical island order and deduplicates live turn overlays", () => {
  const { api } = harness();
  const c: any = conversation();
  c.turnHistory = { kind: "canonical", history: { isComplete: true, entitiesByKey: { z: { turnId: "t0" }, a: { turnId: "t1" } }, islands: [{ entries: [{ value: "z" }, { value: "a" }] }] } };
  assert.equal(JSON.stringify(api.turns(c).map((t: any) => t.turnId)), '["t0","t1"]');
});

test("an existing side can only use unchanged parent history", async () => {
  const { api } = harness({ source: '{"timestamp":"1970-01-01T00:00:01.000Z"}\n' });
  const entry: any = {};
  await api.source(conversation(), entry);
  assert.ok(entry.source);
  const changed = harness({ source: '{"timestamp":"1970-01-01T00:00:03.000Z"}\n' });
  await assert.rejects(changed.api.source(conversation(), {}), /主历史已有变化/);
});

test("old size failures recover only when the parent still predates the side", async () => {
  for (const error of ["会话文件超过分享大小限制", "主会话历史超过 16 MiB 上限"]) {
    for (const changed of [false, true]) {
      const reference = { byte_length: 160000000, sha256: "captured-prefix" };
      const { api, calls } = harness({ post: async () => ({ status: "ok", source_id: "parent", source_ref: reference,
        timestamps: [changed ? "1970-01-01T00:00:03.000Z" : "1970-01-01T00:00:01.000Z"],
      }) });
      const entry: any = { error };
      if (changed) await assert.rejects(api.source(conversation(), entry), /主历史已有变化/);
      else assert.deepEqual((await api.source(conversation(), entry)).source_ref, reference);
      assert.equal(calls[0].body.check_timestamps, true);
    }
  }
});

test("missing saved tools or a remaining ephemeral thread fail verification", () => {
  const { api } = harness();
  const turns = [{ turnId: "t1", status: "completed", items: [{ id: "cmd1" }] }];
  const thread: any = { id: "saved", path: "/saved.jsonl", ephemeral: false, turns: [{ id: "t1", status: "completed", items: [] }] };
  assert.throws(() => api.verify(thread, "saved", turns), /不完整/);
  thread.turns[0].items.push({ id: "cmd1" });
  assert.doesNotThrow(() => api.verify(thread, "saved", turns));
  thread.ephemeral = true;
  assert.throws(() => api.verify(thread, "saved", turns), /校验失败/);
});

test("fork naming numbers the same family and preserves intentional suffixes", () => {
  const { api } = harness();
  const parent = { id: "parent", title: "需求讨论" };
  const fork = { id: "fork", forkedFromId: "parent", title: "需求讨论 (2)" };
  const threads = [parent, fork,
    { id: "descendant", forkedFromId: "fork", title: "需求讨论 (5)" },
    { id: "unrelated", title: "需求讨论 (99)" },
  ];
  assert.equal(api.title(parent, []), "需求讨论 (2)");
  assert.equal(api.title(parent, threads), "需求讨论 (6)");
  assert.equal(api.title(fork, threads), "需求讨论 (6)");
  assert.equal(api.title({ id: "report", title: "报告 (2026)" }, threads), "报告 (2026) (2)");
  assert.equal(api.title({ id: "lost", forkedFromId: "missing", title: "需求讨论 (2)" }, []), "需求讨论 (3)");
  assert.equal(api.title({ ...fork, title: "另一个任务" }, threads), "另一个任务 (2)");
  assert.equal(api.title({ id: "empty", title: "  " }, threads), null);
  const long = { id: "long", title: "长".repeat(80) };
  const next = api.title(long, []);
  assert.equal(next, `${"长".repeat(55)}… (2)`);
  assert.equal(api.title(long, [long, { id: "long-fork", forkedFromId: long.id, title: next }]), `${"长".repeat(55)}… (3)`);
});

test("failed verification retains the original side and reuses the written file on retry", async () => {
  let writes = 0, resumes = 0;
  const { api, context } = harness({ post: async path => {
    assert.equal(path, "/side-chat/save"); writes++;
    return { status: "ok", session_id: "saved", path: "/saved.jsonl" };
  } });
  api.state.entries.set("side", { source: { source_id: "parent", content: "fixed source" } });
  const manager = { requestClient: {}, getConversation: () => conversation(), getRecentConversations: () => [], getThreadProjectAssignment: async () => null, sendRequest: async (method: string) => {
    if (method === "thread/resume") { resumes++; return {}; }
    if (method === "thread/read") return { thread: { id: "saved", path: "/saved.jsonl", ephemeral: false, turns: [] } };
    throw Error(`unexpected native write: ${method}`);
  } };
  api.state.manager = new WeakRef(manager);
  await Promise.all([api.save("side"), api.save("side")]);
  await api.save("side");
  assert.equal(writes, 1); assert.equal(resumes, 2);
  assert.ok(context.window.__codexPlusSideChatSaveState.entries.has("side"));
});

test("failed naming retains the side and retries the same saved conversation", async () => {
  let writes = 0, names = 0;
  const { api, calls, context } = harness({ post: async () => {
    writes++; return { status: "ok", session_id: "saved", path: "/saved.jsonl" };
  } });
  let refreshed = false;
  context.refreshRecentConversationsForHost = async () => { refreshed = true; };
  const manager = {
    requestClient: {}, getConversation: (id: string) => id === "parent" ? { id, title: "需求讨论" } : conversation(),
    getRecentConversations: () => [], getThreadProjectAssignment: async () => null,
    setThreadTitle: async (id: string, title: string) => {
      assert.equal(id, "saved"); assert.equal(title, "需求讨论 (2)");
      if (++names === 1) throw Error("title service unavailable");
    },
    sendRequest: async (method: string) => method === "thread/read" ? { thread: {
      id: "saved", ephemeral: false, path: "/saved.jsonl",
      turns: [{ id: "t1", status: "completed", items: [{ id: "u1" }] }],
    } } : {},
  };
  api.state.manager = new WeakRef(manager);
  api.state.entries.set("side", { source: { source_id: "parent", content: "fixed parent" } });
  await api.save("side");
  assert.equal(calls.at(-1).toast[0], "title service unavailable");
  assert.equal(refreshed, false);
  await api.save("side");
  assert.equal(writes, 1); assert.equal(names, 2); assert.equal(refreshed, true);
  assert.match(calls.at(-1).toast[0], /^已保存为普通会话/);
});

test("successful save opens the ordinary thread and only closes an unchanged side", async () => {
  for (const changedDuringSave of [false, true]) {
    let closes = 0;
    const nativeCalls: string[] = [];
    const c = conversation();
    const reference = { byte_length: 160000000, sha256: "captured-prefix" };
    const { api, calls, context } = harness({ post: async (path, body) => {
      assert.equal(path, "/side-chat/save");
      assert.deepEqual(body.source_ref, reference);
      assert.equal(Object.hasOwn(body, "source_content"), false);
      assert.ok(JSON.stringify(body).length < 4096);
      return { status: "ok", session_id: "saved", path: "/saved.jsonl" };
    } });
    context.document.querySelectorAll = () => [{
      getAttribute: () => "sidechat:side",
      querySelector: (selector: string) => selector.startsWith("[data-tab-id") ? null : {},
      querySelectorAll: () => [{ getAttribute: () => "Close tab", click: () => { closes++; } }],
    }];
    // 本测试关注保存完成后的行为，按钮刷新由扫描负责。
    context.codexPlusSettings = () => ({ sideChatSave: true });
    api.state.scanning = true;
    context.codexServiceTierDispatcherFromModule = () => ({ dispatchHostMessage: async (message: any) => {
      assert.equal(message.path, "/local/saved");
      if (changedDuringSave) c.turns.push({ turnId: "new-turn", status: "inProgress", items: [] });
    } });
    const manager = { requestClient: {}, getConversation: (id: string) => id === "parent" ? { id, title: "需求讨论" } : c,
      getRecentConversations: () => [], getThreadProjectAssignment: async () => null,
      setThreadTitle: async (id: string, title: string) => {
        assert.equal(id, "saved"); assert.equal(title, "需求讨论 (2)"); nativeCalls.push("setThreadTitle");
      }, sendRequest: async (method: string) => {
      nativeCalls.push(method);
      if (method === "thread/read") return { thread: { id: "saved", ephemeral: false, path: "/saved.jsonl", turns: [{ id: "t1", status: "completed", items: [{ id: "u1" }] }] } };
      return {};
    } };
    api.state.manager = new WeakRef(manager);
    api.state.entries.set("side", { source: { source_id: "parent", source_ref: reference } });
    await api.save("side");
    assert.deepEqual(nativeCalls, ["thread/resume", "thread/read", "setThreadTitle"]);
    assert.equal(closes, changedDuringSave ? 0 : 1);
    assert.equal(calls.at(-1).toast[0], "已保存为普通会话「需求讨论 (2)」");
  }
});

test("inherits the parent project ID even when two projects have the same name and directory", async () => {
  const projects = [
    { id: "old-chat", name: "chat", rootPaths: ["D:\\projects\\chat"] },
    { id: "current-chat", name: "chat", rootPaths: ["D:\\projects\\chat"] },
  ];
  for (const project of projects) {
    const assignment = { projectKind: "local", projectId: project.id };
    const assignments = new Map<string, any>([["parent", assignment]]);
    const order: string[] = [];
    const { api, calls, context } = harness({ post: async () => ({ status: "ok", session_id: "saved", path: "/saved.jsonl" }) });
    context.refreshRecentConversationsForHost = async () => { order.push("refresh"); };
    const manager = {
      requestClient: {}, getConversation: () => conversation(), getRecentConversations: () => [],
      getThreadProjectAssignment: async (id: string) => assignments.get(id) ?? null,
      runtimeSettings: { saveThreadProjectAssignment: async ({ threadId, assignment }: any) => {
        order.push("assign"); assignments.set(threadId, assignment);
      } },
      sendRequest: async (method: string) => method === "thread/read" ? { thread: {
        id: "saved", ephemeral: false, path: "/saved.jsonl",
        turns: [{ id: "t1", status: "completed", items: [{ id: "u1" }] }],
      } } : {},
    };
    api.state.manager = new WeakRef(manager);
    api.state.entries.set("side", { source: { source_id: "parent", content: "fixed parent" } });
    await api.save("side");
    assert.deepEqual(assignments.get("saved"), assignment);
    assert.deepEqual(order, ["assign", "refresh"]);
    assert.equal(calls.at(-1).toast.length, 1);
    assert.match(calls.at(-1).toast[0], /^已保存为普通会话/);
  }
});

test("failed or mismatched project assignment retains the side and reuses the saved file", async () => {
  for (const mode of ["reject", "mismatch", "unavailable"]) {
    let writes = 0, attempt = 0;
    const assignment = { projectKind: "local", projectId: "parent-project" };
    let savedAssignment: any = null;
    const { api, calls, context } = harness({ post: async () => {
      writes++; return { status: "ok", session_id: "saved", path: "/saved.jsonl" };
    } });
    context.refreshRecentConversationsForHost = async () => assert.fail("must not open an unassigned thread");
    const manager: any = {
      requestClient: {}, getConversation: () => conversation(), getRecentConversations: () => [],
      getThreadProjectAssignment: async (id: string) => id === "parent" ? assignment : savedAssignment,
      threadWorkspaceStorage: { setProjectAssignment: async (_id: string, value: any) => {
        attempt++;
        if (mode === "reject") throw Error("project service unavailable");
        savedAssignment = { ...value, projectId: "other-project" };
      } },
      sendRequest: async (method: string) => method === "thread/read" ? { thread: {
        id: "saved", ephemeral: false, path: "/saved.jsonl",
        turns: [{ id: "t1", status: "completed", items: [{ id: "u1" }] }],
      } } : {},
    };
    if (mode === "unavailable") delete manager.threadWorkspaceStorage;
    api.state.manager = new WeakRef(manager);
    api.state.entries.set("side", { source: { source_id: "parent", content: "fixed parent" } });
    await api.save("side");
    await api.save("side");
    assert.equal(writes, 1);
    assert.equal(attempt, mode === "unavailable" ? 0 : 2);
    assert.equal(calls.at(-1).toast.length, 1);
    assert.match(calls.at(-1).toast[0], /project service unavailable|项目归属校验失败|无法登记项目归属/);
    assert.ok(api.state.entries.has("side"));
  }
});
