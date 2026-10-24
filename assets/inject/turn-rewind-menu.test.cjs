const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');
const shard = name => fs.readFileSync(path.join(__dirname, 'renderer-inject', name), 'utf8');

class Element {
  constructor() {
    this.nodeType = 1; this.children = []; this.attributes = {}; this.style = {};
    this.textContent = ''; this.listeners = {}; this.events = []; this.parentElement = null;
  }
  setAttribute(key, value) { this.attributes[key] = String(value); }
  getAttribute(key) { return this.attributes[key] ?? null; }
  hasAttribute(key) { return key in this.attributes; }
  matches(selector) {
    const match = selector.match(/^\[([^=\]]+)(?:="([^"]+)")?\]$/);
    return !!match && this.hasAttribute(match[1]) && (!match[2] || this.getAttribute(match[1]) === match[2]);
  }
  closest(selector) { return this.matches(selector) ? this : this.parentElement?.closest(selector); }
  appendChild(child) { child.parentElement = this; this.children.push(child); return child; }
  querySelectorAll(selector) {
    return this.children.flatMap(child => [...(child.matches(selector) ? [child] : []), ...child.querySelectorAll(selector)]);
  }
  querySelector(selector) { return this.querySelectorAll(selector)[0] || null; }
  addEventListener(name, callback) { this.listeners[name] = callback; }
  dispatchEvent(event) { this.events.push(event); return true; }
  remove() { this.parentElement.children = this.parentElement.children.filter(child => child !== this); }
  insertAdjacentElement(position, child) {
    assert.equal(position, 'afterend'); child.parentElement = this.parentElement;
    this.parentElement.children.splice(this.parentElement.children.indexOf(this) + 1, 0, child);
  }
  get previousElementSibling() { return this.parentElement?.children[this.parentElement.children.indexOf(this) - 1]; }
  click() { return this.listeners.click?.({ preventDefault() {}, stopPropagation() {} }); }
}

(async () => {
  const body = new Element(), menu = body.appendChild(new Element()); menu.setAttribute('role', 'menu');
  const branch = menu.appendChild(new Element()); branch.setAttribute('role', 'menuitem'); branch.textContent = '分支到新聊天';
  const other = menu.appendChild(new Element()); other.setAttribute('role', 'menuitem'); other.textContent = 'Other';
  const state = { historyMode: 'paginated', resumeState: 'resumed', threadRuntimeStatus: { type: 'idle' } };
  const manager = Object.fromEntries(['loadRemainingTurnItems', 'getCompleteConversationTurns', 'sendFollowUpMessage',
    'addTurnCompletedListener', 'addConversationStateCallback', 'interruptConversation', 'sendRequest', 'applyRevertResponseToConversation']
    .map(name => [name, () => {}]));
  manager.getHostId = () => 'local'; manager.getConversation = () => state;
  const identity = { threadId: 'fixture-chat', turnId: 'fixture-turn' };
  branch.__reactFiber$fixture = { memoizedProps: identity,
    memoizedState: { memoizedState: { registry: { getImplForHostId: () => manager } } } };
  const settings = { turnRewind: false };
  let observes = 0, disconnects = 0, mutationCallback, moduleLoads = 0, disposed = 0;
  const saveRequests = [], toasts = [], diagnostics = [];
  let scopeReads = 0, registryLoads = 0, registryAvailable = false;
  class NativeRegistry {
    getImplForHostId(host) { scopeReads++; return this.scope.get("hosts").includes(host) ? this.scope.get("managers", host) : null; }
    getForHostId() {} getAll() {} addRegistryCallback() {}
  }
  const scope = {
    get(key, host) { return key === "hosts" ? ["local"] : host === "local" ? manager : null; },
    set() { throw Error("Read-only lookup must never write scope"); }, watch() {}
  };
  const context = { KeyboardEvent: class { constructor(type, init) { this.type = type; Object.assign(this, init); } }, TextEncoder, setTimeout, clearTimeout, Symbol, crypto: require('node:crypto').webcrypto,
    document: { body, querySelectorAll: selector => body.querySelectorAll(selector), createElement: () => new Element() },
    MutationObserver: class {
      constructor(callback) { mutationCallback = callback; }
      observe() { observes++; } disconnect() { disconnects++; }
    },
    codexPlusBackendSettingsLoaded: false,
    codexPlusSettings: () => settings, showToast: message => toasts.push(message),
    sendCodexPlusDiagnostic: async (name, detail) => diagnostics.push({ name, detail }),
    loadCodexAppModule: async prefix => {
      if (prefix === 'app-server-manager-hooks-') {
        registryLoads++;
        return { get renamedRegistryExport() { return registryAvailable ? NativeRegistry : undefined; } };
      }
      moduleLoads++;
      return { changedExportName: { workspaceFiles: { saveCopy(request) {
        saveRequests.push(request);
        const operation = Promise.resolve({ path: '/fixture/receipt.json' });
        operation[Symbol.dispose] = () => { disposed++; };
        return operation;
      } } } };
    }
  };
  context.window = context;
  vm.createContext(context);
  vm.runInContext(shard('85-turn-rewind-controller.js') + shard('86-turn-rewind-menu.js'), context);
  const api = context.__codexPlusTurnRewind;
  context.syncCodexPlusTurnRewind();
  assert.equal(observes, 0); assert.equal(menu.children.length, 2); assert.equal(moduleLoads, 0);
  settings.turnRewind = true; context.syncCodexPlusTurnRewind(); assert.equal(observes, 0);
  context.codexPlusBackendSettingsLoaded = true; context.syncCodexPlusTurnRewind();
  const button = menu.children[1];
  assert.equal(button.textContent, '回退到本轮之前 · Codex++'); assert.equal(button.getAttribute('aria-disabled'), 'false');
  assert.equal(button.previousElementSibling, branch); assert.equal(menu.children[2], other);
  context.refreshTurnRewindMenus(); assert.equal(menu.children.length, 3); assert.equal(moduleLoads, 0);
  const activations = []; api.run = async (...args) => {
    assert.equal(menu.events.at(-1)?.key, 'Escape', 'close native modal menu before starting rollback');
    assert.equal(menu.events.at(-1)?.bubbles, true);
    activations.push(args);
  };
  identity.turnId = 'fixture-new-target'; await button.click();
  assert.deepEqual(Array.from(activations[0]), ['fixture-chat', 'fixture-new-target']);

  identity.hostId = 'remote'; context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'true');
  delete identity.hostId;
  state.threadRuntimeStatus.type = 'active'; context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'true');
  state.threadRuntimeStatus.type = 'idle';
  branch.__reactFiber$fixture.return = { memoizedProps: { ...identity, turnId: 'conflicting-turn' } };
  context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'true'); branch.__reactFiber$fixture.return = null;
  branch.__reactFiber$fixture.pendingProps = identity; branch.__reactFiber$fixture.memoizedProps = {};
  context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'true');
  branch.__reactFiber$fixture.memoizedProps = identity;
  const nativeApply = manager.applyRevertResponseToConversation; delete manager.applyRevertResponseToConversation;
  context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'true'); manager.applyRevertResponseToConversation = nativeApply;

  // 当前原生 hooks 只给出 public facade 和 scope；完整写能力不在 facade 上。
  await Promise.resolve(); await Promise.resolve();
  branch.__reactFiber$fixture.memoizedState.memoizedState = { current: { getConversation: manager.getConversation }, scope };
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'imported namespace may precede lazy class initialization');
  registryAvailable = true;
  context.refreshTurnRewindMenus();
  assert.equal(registryLoads, 1, 'retry live binding discovery without reloading app assets');
  assert.equal(button.getAttribute('aria-disabled'), 'false');
  assert.ok(scopeReads > 0, 'existing native registry accessor reads the React scope');
  await button.click();
  assert.equal(activations.length, 2);
  assert.deepEqual(Array.from(activations[1]), ['fixture-chat', 'fixture-new-target']);

  // 支持能力不可用时仍能点击并解释原因，不能无声吞掉，也不能启动回退。
  branch.__reactFiber$fixture.memoizedState.memoizedState = {};
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true');
  assert.equal(Boolean(button.disabled), false);
  const previousEscapes = menu.events.length;
  await button.click();
  assert.equal(menu.events.length, previousEscapes, 'unavailable clicks keep the menu and explain the issue');
  assert.match(toasts.at(-1), /原生回退接口/);
  assert.equal(activations.length, 2);
  assert.equal(diagnostics.at(-1).detail.identityCount, 1);
  assert.equal(JSON.stringify(diagnostics).includes('fixture-chat'), false, 'diagnostics contain counts, not chat IDs or content');
  branch.__reactFiber$fixture.memoizedProps = {};
  await button.click();
  assert.match(toasts.at(-1), /唯一识别/);
  branch.__reactFiber$fixture.memoizedProps = identity;
  branch.__reactFiber$fixture.memoizedState.memoizedState = { registry: { getImplForHostId: () => manager } };

  // 原生弹出菜单可能隔着很深的 Radix 包装，或由独立 portal 子树承载。
  const messageFiber = branch.__reactFiber$fixture;
  let deepFiber = messageFiber;
  for (let depth = 0; depth < 90; depth++) deepFiber = { memoizedProps: {}, return: deepFiber };
  branch.__reactFiber$fixture = deepFiber;
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'false', 'message identity beyond 64 native wrappers is reachable');

  const trigger = body.appendChild(new Element());
  trigger.setAttribute('id', 'fixture-trigger'); trigger.setAttribute('aria-haspopup', 'menu');
  trigger.setAttribute('aria-expanded', 'true'); trigger.setAttribute('aria-controls', 'fixture-menu');
  menu.setAttribute('id', 'fixture-menu'); menu.setAttribute('aria-labelledby', 'fixture-trigger');
  identity.onFork = () => {};
  let applicationRoot = null;
  for (let depth = 0; depth < 230; depth++) applicationRoot = { memoizedProps: {}, return: applicationRoot };
  trigger.__reactFiber$fixture = { memoizedProps: {}, return: {
    memoizedProps: identity, dependencies: { firstContext: { memoizedValue: scope } }, return: applicationRoot
  } };
  branch.__reactFiber$fixture = { memoizedProps: {} };
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'false', 'resolve the native message action boundary without scanning the entire deep application tree');
  assert.equal(context.turnRewindAvailability(branch).details.messageBoundaryResolved, true);
  // 复现日志中的另一种 192 层来源：无消息身份的菜单 portal 自身很深。
  let detachedMenuTree = null;
  for (let depth = 0; depth < 230; depth++) detachedMenuTree = { memoizedProps: {}, return: detachedMenuTree };
  branch.__reactFiber$fixture = detachedMenuTree;
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'false', 'optional deep menu wrappers cannot invalidate a proven native trigger boundary');
  const proven = context.turnRewindAvailability(branch).details;
  assert.equal(proven.menuScanLimitReached, true); assert.equal(proven.scanLimitReached, false);
  branch.__reactFiber$fixture = { memoizedProps: {} };

  const beforeOwnerRun = activations.length;
  await button.click();
  assert.deepEqual(Array.from(activations.at(-1)), ['fixture-chat', 'fixture-new-target']);
  assert.equal(activations.length, beforeOwnerRun + 1);

  identity.turnId = 'fixture-owner-new-target'; await button.click();
  assert.deepEqual(Array.from(activations.at(-1)), ['fixture-chat', 'fixture-owner-new-target']);
  identity.turnId = 'fixture-new-target';

  const nativeTriggerFiber = trigger.__reactFiber$fixture;
  let inaccessibleTrigger = nativeTriggerFiber;
  for (let depth = 0; depth < 230; depth++) inaccessibleTrigger = { memoizedProps: {}, return: inaccessibleTrigger };
  trigger.__reactFiber$fixture = inaccessibleTrigger;
  branch.__reactFiber$fixture = messageFiber;
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'an unresolved required trigger boundary must still fail closed');
  await button.click(); assert.match(toasts.at(-1), /安全识别范围/);
  trigger.__reactFiber$fixture = nativeTriggerFiber; branch.__reactFiber$fixture = { memoizedProps: {} };

  trigger.setAttribute('aria-expanded', 'false'); context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'closed trigger must not identify a visible menu');
  trigger.setAttribute('aria-expanded', 'true');
  trigger.setAttribute('aria-controls', 'fixture-other-menu'); context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'contradictory native control/label relations are not trusted');
  trigger.setAttribute('aria-controls', 'fixture-other-menu'); menu.setAttribute('aria-labelledby', 'fixture-other-trigger');
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'do not use another menu or the last clicked message');
  trigger.setAttribute('aria-controls', 'fixture-menu'); menu.setAttribute('aria-labelledby', 'fixture-trigger');

  branch.__reactFiber$fixture = { memoizedProps: { threadId: 'other-chat', turnId: 'other-turn' } };
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'conflicting menu and trigger identities fail closed');
  branch.__reactFiber$fixture = { memoizedProps: {} };
  const duplicateTrigger = body.appendChild(new Element());
  duplicateTrigger.setAttribute('aria-haspopup', 'menu'); duplicateTrigger.setAttribute('aria-expanded', 'true');
  duplicateTrigger.setAttribute('aria-controls', 'fixture-menu'); duplicateTrigger.__reactFiber$fixture = messageFiber;
  context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'multiple explicit menu owners are ambiguous');
  duplicateTrigger.remove();
  // 一项自身没有 fiber 时，最近的原生父节点仍可提供身份；不读取 pendingProps。
  delete branch.__reactFiber$fixture; menu.__reactFiber$fixture = messageFiber;
  context.refreshTurnRewindMenus(); assert.equal(button.getAttribute('aria-disabled'), 'false');
  delete menu.__reactFiber$fixture;
  trigger.remove(); branch.__reactFiber$fixture = messageFiber;

  let excessiveFiber = messageFiber;
  for (let depth = 0; depth < 230; depth++) excessiveFiber = { memoizedProps: {}, return: excessiveFiber };
  branch.__reactFiber$fixture = excessiveFiber; context.refreshTurnRewindMenus();
  assert.equal(button.getAttribute('aria-disabled'), 'true', 'bounded scans never guess a target beyond the ancestor limit');
  await button.click(); assert.equal(diagnostics.at(-1).detail.scanLimitReached, true);
  branch.__reactFiber$fixture = messageFiber;

  const exported = await api.exportRecord({ id: 'fixture', text: '中文恢复记录' });
  assert.equal(exported.path, '/fixture/receipt.json'); assert.equal(disposed, 1);
  assert.equal(JSON.parse(Buffer.from(saveRequests[0].bytes).toString('utf8')).text, '中文恢复记录');
  // 重注入保留控制器对象，不丢在途任务；旧观察器和菜单项被回收。
  vm.runInContext(`(() => {${shard('85-turn-rewind-controller.js') + shard('86-turn-rewind-menu.js')}
    window.fixtureSync = syncCodexPlusTurnRewind;
  })()`, context);
  assert.equal(context.__codexPlusTurnRewind, api); assert.equal(disconnects, 1);
  context.fixtureSync();
  settings.turnRewind = false; context.fixtureSync();
  assert.equal(menu.children.length, 2); assert.equal(disconnects, 2);
  assert.equal(api.canRun('fixture-chat', 'fixture-turn'), false);
  console.log('PASS: native trigger association, deep portal ancestry, lazy registry exports, current committed turn identity, opt-in lifecycle, busy/remote/conflicting-owner/unsupported guards, native JSON export and reinjection handoff.');
})().catch(error => { console.error(error); process.exitCode = 1; });
