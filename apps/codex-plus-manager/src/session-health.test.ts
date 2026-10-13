import assert from "node:assert/strict";
import { test } from "node:test";

import { readFile } from "node:fs/promises";

const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
const source = renderer.slice(renderer.indexOf('  const invalidSessionStorageKey ='), renderer.indexOf('  function sessionRefFromRow(row) {'));
assert.ok(source.includes("function checkAndHideInvalidSessions"), "检测函数必须位于主注入作用域");
const lost = "01000000-0000-7000-8000-000000000001";
const healthy = "01000000-0000-7000-8000-000000000002";

function row(id = lost, host: string | null = "local") {
  const attributes = new Map<string, string>();
  if (host !== null) attributes.set("data-app-action-sidebar-thread-host-id", host);
  return {
    ref: { session_id: id },
    getAttribute: (key: string) => attributes.get(key) ?? null,
    setAttribute: (key: string, value: string) => attributes.set(key, value),
    removeAttribute: (key: string) => attributes.delete(key),
    hidden: () => attributes.get("data-codex-invalid-session-hidden") === "true",
  };
}

type Client = { hostId?: string; sendRequest: (method: string, params: { threadId: string; includeTurns: boolean }) => Promise<unknown> };
type HealthPayload = { threadIds: string[]; observedOnly: boolean };
const missingClient: Client = {
  async sendRequest(method, params) {
    assert.equal(method, "thread/read");
    assert.equal(params.includeTurns, true);
    throw new Error(`no rollout found for thread id ${params.threadId}`);
  },
};

function harness(options: {
  rows?: ReturnType<typeof row>[];
  saved?: string[];
  clients?: Client[];
  backend?: (payload: HealthPayload) => Promise<unknown>;
} = {}) {
  const rows = options.rows ?? [row()];
  const storage = new Map<string, string>();
  if (options.saved) storage.set("codexPlus.hiddenInvalidSessions.v1", JSON.stringify(options.saved));
  const status = { textContent: "" };
  const button = { disabled: false };
  const documentState = {
    visibilityState: "visible",
    querySelectorAll: (selector: string) => selector.includes("status") ? [status] : [button],
  };
  const requests: HealthPayload[] = [];
  const backend = options.backend ?? (async () => ({ status: "ok", scanned: 2, missingIds: [lost] }));
  const api = new Function("localStorage", "document", "sessionRows", "sessionRefFromRow", "normalizedCodexThreadUuid", "uuidV7TimestampMs", "postJson", "loadAppServerRequestCandidates", `
    const modelResponseHostId = client => client.hostId ?? "local"; let codexPlusBackendSettings = { enhancementsEnabled: true }, codexPlusBackendSettingsLoaded = true;
    ${source}
    return { check: checkAndHideInvalidSessions, reset: resetInvalidSessionVisibility,
      refresh: refreshInvalidSessionVisibility, apply: applyInvalidSessionVisibility,
      nativeMissing: nativeSessionIsMissing, request: sessionHealthRequest,
      busy: () => sessionHealthBusy, due: () => { sessionHealthCheckedAt = 0; },
      disable: () => { codexPlusBackendSettings.enhancementsEnabled = false; } };
  `)(
    { getItem: (key: string) => storage.get(key), setItem: (key: string, value: string) => storage.set(key, value), removeItem: (key: string) => storage.delete(key) },
    documentState,
    () => rows, (entry: ReturnType<typeof row>) => entry.ref,
    (id: string) => /^[0-9a-f-]{36}$/i.test(id.replace(/^local:/, "")) ? id.replace(/^local:/, "") : "",
    (id: string) => Number.parseInt(id.replaceAll("-", "").slice(0, 12), 16),
    (path: string, payload: HealthPayload) => { assert.equal(path, "/session/health"); requests.push(payload); return backend(payload); },
    async () => ({ candidates: options.clients ?? [missingClient] }),
  );
  return { api, rows, status, button, storage, requests, documentState };
}

test("检查只隐藏本机明确失效的会话，按钮可以恢复显示", async () => {
  const h = harness({ rows: [row(), row(healthy), row(lost, "remote-ssh:server"), row(lost, null)] });
  await h.api.check();
  assert.deepEqual(h.rows.map((entry) => entry.hidden()), [true, false, false, false]);
  assert.match(h.status.textContent, /隐藏 1 个/);
  h.api.reset();
  assert.ok(h.rows.every((entry) => !entry.hidden()));
  assert.equal(h.storage.size, 0);
});

test("只发现远端 RPC 时不能隐藏本机会话", async () => {
  const h = harness({ clients: [{ ...missingClient, hostId: "remote-ssh:server" }] });
  await h.api.check();
  assert.equal(h.rows[0].hidden(), false);
});

test("设置页、扫描与显示样式接入同一套检查实现", () => {
  assert.match(renderer, /data-codex-session-health-scan/);
  assert.match(renderer, /void checkAndHideInvalidSessions\(\)/);
  assert.match(renderer, /resetInvalidSessionVisibility\(\)/);
  assert.match(renderer, /runScanStep\(refreshInvalidSessionVisibility\)/);
  assert.match(renderer, /\[data-codex-invalid-session-hidden="true"\] \{ display: none !important; \}/);
});

test("文件缺失还必须由原生读取接口确认，连接错误和可读取会话不会隐藏", async () => {
  for (const client of [
    { sendRequest: async () => { throw new Error("connection closed"); } },
    { sendRequest: async () => ({ thread: { id: lost, turns: [] } }) },
    { sendRequest: async () => { throw new Error("permission denied"); } },
  ]) {
    const h = harness({ clients: [missingClient, client] });
    await h.api.check();
    assert.equal(h.rows[0].hidden(), false);
  }
});

test("新建会话尚未落盘时保留，读取超时会返回错误", async () => {
  const h = harness();
  const hex = Date.now().toString(16).padStart(12, "0");
  const id = `${hex.slice(0, 8)}-${hex.slice(8)}-7000-8000-000000000001`;
  assert.equal(await h.api.nativeMissing(id, [missingClient]), false);
  await assert.rejects(h.api.request(() => new Promise(() => {}), 1), /检查超时/);
});

test("兼容新版 thread/read 的持久记录和内存会话均不存在错误", async () => {
  const h = harness({ clients: [{ sendRequest: async () => { throw new Error(`thread not loaded: ${lost}`); } }] });
  await h.api.check();
  assert.equal(h.rows[0].hidden(), true);
  const unavailable = harness({ clients: [{ sendRequest: async () => { throw new Error("thread not loaded"); } }] });
  await unavailable.api.check();
  assert.equal(unavailable.rows[0].hidden(), false);
});

test("忽略不支持会话读取的 IPC 客户端，但不能把接口缺失当作会话失效", async () => {
  const unsupported = { sendRequest: async () => { throw Object.assign(new Error("Method not found"), { code: -32601 }); } };
  const h = harness({ clients: [unsupported, missingClient] });
  await h.api.check();
  assert.equal(h.rows[0].hidden(), true);
  const unavailable = harness({ clients: [unsupported] });
  await unavailable.api.check();
  assert.equal(unavailable.rows[0].hidden(), false);
});

test("隐藏前复核恢复来源，检查期间恢复的会话保留", async () => {
  let reads = 0;
  const h = harness({ backend: async () => ({ status: "ok", scanned: 1, missingIds: reads++ === 0 ? [lost] : [] }) });
  await h.api.check();
  assert.equal(reads, 2);
  assert.equal(h.rows[0].hidden(), false);
});

test("检查过程中恢复显示，会使未完成的隐藏结果作废", async () => {
  let complete!: () => void;
  let entered!: () => void;
  const started = new Promise<void>((resolve) => { entered = resolve; });
  const h = harness({ clients: [{ sendRequest: async () => {
    entered();
    await new Promise<void>((resolve) => { complete = resolve; });
    throw new Error(`no rollout found for thread id ${lost}`);
  } }] });
  const check = h.api.check();
  await started;
  h.api.reset();
  complete();
  await check;
  assert.equal(h.rows[0].hidden(), false);
  assert.equal(h.storage.size, 0);
  assert.match(h.status.textContent, /已显示全部/);
});

test("页面重载后立即应用持久化隐藏，恢复来源出现后重新显示", async () => {
  const h = harness({ saved: [lost] });
  h.api.apply();
  assert.equal(h.rows[0].hidden(), true);
  assert.equal(h.requests.length, 0);
  await h.api.check(true);
  assert.ok(h.requests.every((request) => request.observedOnly));
  assert.equal(h.rows[0].hidden(), true);
  const recovered = harness({ saved: [lost], backend: async () => ({ status: "ok", scanned: 1, missingIds: [] }) });
  await recovered.api.check(true);
  assert.equal(recovered.rows[0].hidden(), false);
  assert.equal(recovered.storage.get("codexPlus.hiddenInvalidSessions.v1"), "[]");
});

test("自动复核启动时不抢占资源，页面隐藏时继续暂停", async () => {
  const h = harness({ saved: [lost] });
  h.api.refresh();
  assert.equal(h.requests.length, 0);

  h.api.due();
  h.documentState.visibilityState = "hidden";
  h.api.refresh();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.requests.length, 0);

  h.documentState.visibilityState = "visible";
  h.api.refresh();
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(h.requests.length, 2);
  assert.ok(h.requests.every((request) => request.observedOnly));
});

test("虚拟列表复用行或关闭增强时，移除旧的隐藏标记", async () => {
  const h = harness();
  await h.api.check();
  assert.equal(h.rows[0].hidden(), true);
  h.rows[0].ref.session_id = healthy;
  h.api.apply();
  assert.equal(h.rows[0].hidden(), false);
  h.rows[0].ref.session_id = lost;
  h.api.apply();
  assert.equal(h.rows[0].hidden(), true);
  h.api.disable();
  h.api.apply();
  assert.equal(h.rows[0].hidden(), false);
});

test("后端扫描失败或原生接口缺失时保留列表并给出原因", async () => {
  for (const options of [
    { backend: async () => ({ status: "failed", message: "数据库被占用" }) },
    { clients: [] },
  ]) {
    const h = harness(options);
    await h.api.check();
    assert.equal(h.rows[0].hidden(), false);
    assert.match(h.status.textContent, /未隐藏会话/);
    assert.equal(h.button.disabled, false);
  }
});
