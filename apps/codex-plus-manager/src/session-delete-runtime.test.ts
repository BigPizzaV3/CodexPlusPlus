import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import vm from "node:vm";

const shard = (name: string) => readFileSync(new URL(`../../../assets/inject/renderer-inject/${name}`, import.meta.url), "utf8");
const refs = shard("60-plugin-marketplace.js");
const deletion = shard("80-session-share.js");
const refHelpers = refs.slice(refs.indexOf("  function isClientNewThreadId(value)"), refs.indexOf("  if (window.__CODEX_PLUS_TEST_SESSION_REF__)"));
const singleFunction = (source: string, name: string) => source.match(new RegExp(`^  (?:async )?function ${name}\\([^]*?^  \\}`, "m"))?.[0] ?? "";
const uuid = "11111111-1111-4111-8111-111111111111";
const otherUuid = "22222222-2222-4222-8222-222222222222";

class Element {
  attributes = new Map<string, string>();
  parentElement: Element | null = null;
  visible = true;
  disabled = false;
  removed = false;
  clicks = 0;
  textContent = "";
  __reactFiber$fixture: any = null;
  constructor(attributes: Record<string, string> = {}) { for (const [key, value] of Object.entries(attributes)) this.attributes.set(key, value); }
  getAttribute(key: string) { return this.attributes.get(key) ?? null; }
  setAttribute(key: string, value: string) { this.attributes.set(key, value); }
  querySelector() { return null; }
  get isConnected() { return !this.removed; }
  closest(selector: string): Element | null {
    if (selector.includes("data-codex-plus-ext") && this.getAttribute("data-codex-plus-ext")) return this;
    if (selector.includes("aside") || selector.includes("nav")) return this.parentElement;
    return null;
  }
  contains() { return false; }
  blur() {}
  remove() { this.removed = true; }
  click() { this.clicks += 1; }
}

function row(id = uuid, host: string | null = null, props: any = null) {
  const element = new Element({ "data-app-action-sidebar-thread-id": id, href: `/thread/${id}` });
  if (host !== null) element.setAttribute("data-app-action-sidebar-thread-host-id", host);
  element.__reactFiber$fixture = { pendingProps: props, memoizedProps: null, return: null };
  return element;
}

function refFixture() {
  const context = vm.createContext({ selectors: { threadTitle: ".title" } });
  vm.runInContext(refHelpers, context);
  return (element: Element) => JSON.parse(JSON.stringify(context.sessionRefFromRow(element)));
}

// 26.930 静态原生合同：sidebarThreadRow({id,hostId,active})；kDo 的
// {conversationId,hostId,threadSummary} 以 hostId ?? threadSummary?.hostId 配对；
// 行组件还构造 {hostId,threadId:conversationId} locator。不是从 UUID 猜本机。
test("session host resolves paired native locators and thread summaries", () => {
  const resolve = refFixture();
  for (const props of [
    { conversationId: uuid, hostId: "local" },
    { threadId: uuid, hostId: "local" },
    { id: uuid, hostId: "local" },
    { conversationId: uuid, threadSummary: { id: uuid, hostId: "local" } },
    { conversationId: uuid, threadSummary: { hostId: "local" } },
    { thread: { id: uuid, hostId: "local" } },
    { children: { props: { threadId: uuid, hostId: "local" } } },
    { threadId: `local:${uuid}` },
  ]) assert.equal(resolve(row(uuid, null, props)).host_id, "local", JSON.stringify(props));
  assert.equal(resolve(row(uuid, null, { thread: { id: uuid, hostId: "ssh:fixture" } })).host_id, "ssh:fixture");
  assert.equal(resolve(row(`local:${uuid}`)).host_id, "local");
});

test("unknown, unrelated and conflicting host metadata remains refused", () => {
  const resolve = refFixture();
  for (const props of [
    null,
    { hostId: "local" },
    { threadId: otherUuid, hostId: "local" },
    { conversationId: uuid, hostId: "local", threadSummary: { id: uuid, hostId: "ssh:fixture" } },
    { conversationId: uuid, threadId: otherUuid, hostId: "local" },
    { conversationId: uuid, threadSummary: { id: otherUuid, hostId: "local" } },
  ]) assert.equal(resolve(row(uuid, null, props)).host_id, null, JSON.stringify(props));
  assert.equal(resolve(row(`local:${uuid}`, null, { threadId: uuid, hostId: "ssh:fixture" })).host_id, null);
  const element = row(uuid);
  element.__reactFiber$fixture.return = { pendingProps: { conversationId: otherUuid, hostId: "local" }, return: null };
  assert.equal(resolve(element).host_id, null);
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
const flush = () => new Promise<void>((done) => setImmediate(done));

function deleteFixture() {
  const rows = [row(uuid, "local"), row(otherUuid, "local")];
  const button = new Element();
  const native = new Element({ "aria-label": "New chat" }); native.parentElement = new Element();
  const outside = new Element({ "aria-label": "New chat" });
  const extension = new Element({ "aria-label": "New chat", "data-codex-plus-ext": "fixture" }); extension.parentElement = new Element();
  const buttons = [native, outside, extension];
  const request = deferred<any>();
  const confirmations: string[] = [], requests: any[] = [], diagnostics: any[] = [], toasts: string[] = [], timers: number[] = [], refreshes: any[] = [];
  let reloads = 0;
  const location: any = { href: "", pathname: "", search: "", hash: "", reload: () => { reloads += 1; } };
  const setLocation = (href: string) => { const url = new URL(href); Object.assign(location, { href: url.href, pathname: url.pathname, search: url.search, hash: url.hash }); };
  const activate = (id: string) => {
    setLocation(`app://-/thread/${id}`);
    rows.forEach((element) => element.setAttribute("data-app-action-sidebar-thread-active", String(element.getAttribute("data-app-action-sidebar-thread-id") === id)));
  };
  activate(uuid);
  // 删除后隐藏行用的样式表（#2614）：stub 只需支持按 id 取回与追加到 head。
  const styles = new Map<string, Element & { id?: string }>();
  const head = { appendChild: (node: Element & { id?: string }) => { node.removed = false; node.remove = () => { node.removed = true; styles.delete(node.id ?? ""); }; styles.set(node.id ?? "", node); return node; } };
  const context = vm.createContext({ Element, HTMLElement: Element, URL, window: { location },
    CSS: { escape: (value: string) => value.replace(/["\\]/g, "\\$&") },
    document: {
      activeElement: null, querySelectorAll: () => buttons, head,
      getElementById: (id: string) => styles.get(id) ?? null,
      createElement: () => new Element(),
    }, selectors: { threadTitle: ".title" },
    sessionRows: () => rows.filter((element) => element.isConnected),
    visibleElement: (element: Element) => element.visible && element.isConnected,
    isExtensionUiNode: (element: Element) => !!element.getAttribute("data-codex-plus-ext"),
    confirmDelete: (title: string) => { confirmations.push(title); return Promise.resolve(true); },
    postJson: (_path: string, ref: any) => { requests.push(ref); return request.promise; },
    showToast: (text: string) => { toasts.push(text); },
    sendCodexPlusDiagnostic: (event: string, details: any) => { diagnostics.push({ event, details }); },
    setTimeout: (_fn: Function, delay: number) => { timers.push(delay); return 1; },
    rpc: { sendRequest: (method: string, params: any) => { refreshes.push({ method, params }); return Promise.resolve({}); } },
  });
  vm.runInContext(`${refHelpers}\n${singleFunction(refs, "locationThreadId")}\n${singleFunction(refs, "currentSessionRef")}\n${deletion.slice(deletion.indexOf("  function rowHref(row)"), deletion.indexOf("  async function exportMarkdown(ref)"))}\n${singleFunction(deletion, "refreshRecentConversationsForHost")}\nfunction signal(e,t){return rpc.sendRequest(e,t)}; loadOptionalCodexAppModule=async()=>({signal});`, context);
  const begin = async (element = rows[0]) => { context.openDeleteConfirmForRow(element, button, context.sessionRefFromRow(element), { preventDefault() {}, stopPropagation() {}, stopImmediatePropagation() {} }); await flush(); };
  const finish = async (result = { status: "local_deleted", session_id: requests[0]?.session_id, message: "deleted" }) => { request.resolve(result); await flush(); };
  return { context, rows, native, buttons, requests, confirmations, diagnostics, timers, toasts, refreshes, styles, begin, finish, activate, setLocation, reloads: () => reloads };
}

test("local current deletion uses native navigation and refresh without a page reload", async () => {
  const f = deleteFixture(); await f.begin(); await f.finish();
  assert.equal(f.native.clicks, 1);
  assert.equal(f.rows[0].removed, true);
  assert.equal(f.refreshes.length, 1);
  assert.equal(f.timers.length, 0);
  assert.equal(f.reloads(), 0);
});

test("local deletion hides React-rebuilt rows of the deleted thread until undo", async () => {
  // #2614：进设置页再返回时 React 会按内存列表重建侧栏，被删行需按 id 持续隐藏。
  const f = deleteFixture(); await f.begin(); await f.finish();
  const style = f.styles.get("codex-plus-deleted-session-rows");
  assert.ok(style, "本地删除后应写入隐藏样式表");
  assert.equal(style.getAttribute("data-codex-plus-ext"), "deleted-session-rows");
  // #2671：React 重建时同一会话可能换成 `local:<uuid>` 等写法，按 uuid 后缀匹配覆盖全部变体。
  assert.match(style.textContent, new RegExp(`data-app-action-sidebar-thread-id\\$="${uuid}"`));
  assert.doesNotMatch(style.textContent, new RegExp(otherUuid), "不能隐藏其它会话");
  f.context.unhideDeletedSessionRows([uuid]);
  assert.equal(f.styles.has("codex-plus-deleted-session-rows"), false, "撤销后应移除样式表");
});

test("remote or mismatched deletions never add hide rules", async () => {
  const remote = deleteFixture(); remote.rows[0].setAttribute("data-app-action-sidebar-thread-host-id", "ssh:fixture");
  await remote.begin(); await remote.finish({ status: "server_deleted", session_id: uuid, message: "deleted" });
  assert.equal(remote.styles.size, 0);
  const mismatch = deleteFixture(); await mismatch.begin();
  await mismatch.finish({ status: "local_deleted", session_id: otherUuid, message: "deleted" });
  assert.equal(mismatch.styles.size, 0);
});

test("async deletion cannot navigate away from a conversation selected in the meantime", async () => {
  const f = deleteFixture(); await f.begin(); f.activate(otherUuid); await f.finish();
  assert.equal(f.native.clicks, 0);
  assert.equal(f.rows[0].removed, true);
  assert.equal(f.rows[1].removed, false);
  assert.equal(f.timers.length, 0);
});

test("async deletion cannot remove a DOM row reused for another thread or host", async () => {
  for (const changeHost of [false, true]) {
    const f = deleteFixture(); await f.begin();
    f.rows[0].setAttribute(changeHost ? "data-app-action-sidebar-thread-host-id" : "data-app-action-sidebar-thread-id", changeHost ? "ssh:fixture" : otherUuid);
    await f.finish();
    assert.equal(f.rows[0].removed, false);
    assert.equal(f.native.clicks, 0);
    assert.equal(f.timers.length, 0);
  }
});

test("native navigation cannot remove a row synchronously repurposed by React", async () => {
  const f = deleteFixture();
  f.native.click = () => { f.native.clicks += 1; f.rows[0].setAttribute("data-app-action-sidebar-thread-id", otherUuid); };
  await f.begin(); await f.finish();
  assert.equal(f.native.clicks, 1); assert.equal(f.rows[0].removed, false);
  assert.equal(f.timers.length, 0);
});

test("a row changing host while confirmation is pending is rejected before deletion", async () => {
  const f = deleteFixture(), confirmation = deferred<boolean>();
  f.context.confirmDelete = () => confirmation.promise;
  await f.begin(); f.rows[0].setAttribute("data-app-action-sidebar-thread-host-id", "ssh:fixture");
  confirmation.resolve(true); await flush();
  assert.equal(f.requests.length, 0); assert.equal(f.rows[0].removed, false);
  assert.match(f.toasts[0], /会话已变化/);
});

test("remote deletion continues to rely on native host notifications", async () => {
  const f = deleteFixture(); f.rows[0].setAttribute("data-app-action-sidebar-thread-host-id", "ssh:fixture");
  await f.begin(); await f.finish({ status: "server_deleted", session_id: uuid, message: "deleted" });
  assert.equal(f.requests[0].host_id, "ssh:fixture");
  assert.equal(f.native.clicks, 0); assert.equal(f.rows[0].removed, false);
  assert.equal(f.refreshes.length, 0); assert.equal(f.timers.length, 0);
});

test("a mismatched deletion result does not mutate the requested conversation view", async () => {
  const f = deleteFixture(); await f.begin(); await f.finish({ status: "local_deleted", session_id: otherUuid, message: "deleted" });
  assert.equal(f.rows[0].removed, false); assert.equal(f.native.clicks, 0);
  assert.match(f.toasts[0], /结果与请求会话不一致/);
});

test("non-current deletion refreshes the list without changing the selected thread", async () => {
  const f = deleteFixture(); await f.begin(f.rows[1]); await f.finish();
  assert.equal(f.native.clicks, 0);
  assert.equal(f.rows[1].removed, true);
  assert.equal(f.rows[0].removed, false);
  assert.equal(f.refreshes.length, 1);
  assert.equal(f.timers.length, 0);
});

test("unknown hosts retain the safe refusal and emit metadata-only evidence", async () => {
  const f = deleteFixture(); f.rows[0].attributes.delete("data-app-action-sidebar-thread-host-id"); await f.begin();
  assert.equal(f.requests.length, 0); assert.equal(f.confirmations.length, 0);
  assert.match(f.toasts[0], /无法确定会话主机归属/);
  assert.equal(f.diagnostics.length, 1);
  assert.ok(!JSON.stringify(f.diagnostics).includes(uuid));
});

test("route identity must match rather than merely sharing the same pathname", () => {
  const f = deleteFixture(); f.rows[0].attributes.delete("data-app-action-sidebar-thread-active");
  f.rows[0].setAttribute("href", `app://-/index.html?thread=${uuid}`);
  f.setLocation(`app://-/index.html?thread=${otherUuid}`);
  assert.equal(f.context.isCurrentSessionRow(f.rows[0], f.context.sessionRefFromRow(f.rows[0])), false);
});

test("native new-chat affordances never trigger a reload", async () => {
  for (const ambiguous of [false, true]) {
    const f = deleteFixture();
    const duplicate = new Element({ "aria-label": "New chat" }); duplicate.parentElement = new Element();
    if (ambiguous) f.buttons.push(duplicate);
    else f.native.visible = false;
    await f.begin(); await f.finish();
    // 不可见但唯一时照常跳转（#2671）；真正有歧义时不点。两种情况都不能整页刷新。
    assert.equal(f.native.clicks, ambiguous ? 0 : 1);
    assert.equal(duplicate.clicks, 0);
    assert.equal(f.timers.length, 0); assert.equal(f.reloads(), 0);
  }
});

test("hide rules follow the thread uuid across id spellings (#2671)", async () => {
  const f = deleteFixture();
  f.rows[0].setAttribute("data-app-action-sidebar-thread-id", `local:${uuid}`);
  await f.begin(); await f.finish();
  const style = f.styles.get("codex-plus-deleted-session-rows");
  assert.ok(style);
  assert.match(style.textContent, new RegExp(`data-app-action-sidebar-thread-id\\$="${uuid}"`));
  // 非 uuid 的 id 没有后缀可依，退回精确匹配原值。
  f.context.hideDeletedSessionRows("opaque-thread-key");
  assert.match(style.textContent, /data-app-action-sidebar-thread-id="opaque-thread-key"/);
  assert.equal(style.textContent.split("\n").length, 2, "同一会话只留一条规则");
});

test("deletion still navigates when the new chat button is not visible (#2671)", async () => {
  const f = deleteFixture();
  f.native.visible = false;
  await f.begin(); await f.finish();
  assert.equal(f.native.clicks, 1, "侧栏折叠时也要能跳到新会话");
  assert.doesNotMatch(f.toasts.join(" "), /请点击新聊天/, "跳转成功就不再提示手动点击");
  assert.ok(f.diagnostics.some((entry: any) => entry.event === "delete_session_new_chat_fallback"));
});

test("ambiguous invisible new chat candidates are not clicked (#2671)", async () => {
  const f = deleteFixture();
  f.native.visible = false;
  const twin = new Element({ "aria-label": "新聊天" });
  twin.parentElement = new Element();
  twin.visible = false;
  f.buttons.push(twin);
  await f.begin(); await f.finish();
  assert.equal(f.native.clicks + twin.clicks, 0, "多个不可见候选无法判定，不能乱点");
  assert.match(f.toasts.join(" "), /新聊天/);
});
