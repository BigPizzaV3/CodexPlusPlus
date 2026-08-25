import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

type FakeElementOptions = {
  className?: string;
  dismissLabel?: string;
  hasProgress?: boolean;
  styleDisplay?: string;
};

class FakeElement {
  children: FakeElement[] = [];
  dataset: Record<string, string> = {};
  parentElement: FakeElement | null = null;
  style: { display: string };
  private readonly className: string;
  private readonly dismissLabel: string;
  private readonly hasProgress: boolean;

  constructor(options: FakeElementOptions = {}) {
    this.className = options.className ?? "";
    this.dismissLabel = options.dismissLabel ?? "";
    this.hasProgress = options.hasProgress ?? false;
    this.style = { display: options.styleDisplay ?? "" };
  }

  appendChild(child: FakeElement) {
    child.parentElement = this;
    this.children.push(child);
  }

  getAttribute(name: string) {
    return name === "aria-label" ? this.dismissLabel : null;
  }

  matches(selector: string) {
    return selector === "div.w-full" && this.className.split(/\s+/).includes("w-full");
  }

  querySelector(selector: string) {
    return selector === 'progress[max="100"]' && this.hasProgress ? new FakeElement() : null;
  }

  querySelectorAll(selector: string) {
    return selector === "button" && this.dismissLabel ? [this] : [];
  }
}

function usageAlertRuntime(renderer: string, cards: FakeElement[], managed: FakeElement[]) {
  const start = renderer.indexOf("  function officialUsageAlertHidden(");
  const end = renderer.indexOf("\n  let zedRemoteStatusPromise", start);
  assert.ok(start >= 0 && end > start);
  const source = renderer.slice(start, end);
  const selectors: string[] = [];
  const document = {
    querySelectorAll(selector: string) {
      selectors.push(selector);
      return selector === '[data-codex-plus-usage-alert-hidden="true"]'
        ? managed.filter((node) => node.dataset.codexPlusUsageAlertHidden === "true")
        : cards;
    },
  };
  const windowValue: Record<string, unknown> = {};
  const create = new Function(
    "window",
    "document",
    "HTMLElement",
    `${source}\nreturn { officialUsageAlertHidden, refreshOfficialUsageAlertVisibility };`,
  ) as (
    windowValue: Record<string, unknown>,
    documentValue: typeof document,
    elementType: typeof FakeElement,
  ) => {
    officialUsageAlertHidden: () => boolean;
    refreshOfficialUsageAlertVisibility: () => void;
  };
  return { runtime: create(windowValue, document, FakeElement), selectors, windowValue };
}

function installRendererStyle(renderer: string) {
  const start = renderer.indexOf("  function installStyle()");
  const end = renderer.indexOf("\n  function defaultCodexPlusSettings", start);
  assert.ok(start >= 0 && end > start);
  const source = renderer.slice(start, end);
  const requiredNames = new Set([
    "styleId",
    "codexDeleteStyleVersion",
    ...Array.from(source.matchAll(/\$\{([A-Za-z_$][A-Za-z0-9_$]*)/g), (match) => match[1]),
  ]);
  const declarations = Array.from(requiredNames, (name) => {
    const declaration = renderer.match(new RegExp(`^  const ${name} = .+;$`, "m"))
      ?? renderer.match(new RegExp(`^  const ${name} = [\\s\\S]*?^  };$`, "m"));
    assert.ok(declaration, `missing renderer declaration for ${name}`);
    return declaration[0];
  }).join("\n");
  const appended: Array<{ dataset: Record<string, string>; id?: string; textContent?: string }> = [];
  const document = {
    getElementById() {
      return null;
    },
    createElement() {
      return { dataset: {} };
    },
    documentElement: {
      appendChild(node: (typeof appended)[number]) {
        appended.push(node);
      },
    },
  };
  const install = new Function("document", `${declarations}\n${source}\ninstallStyle();`) as (documentValue: typeof document) => void;

  install(document);
  return appended;
}

describe("renderer injection header compatibility", () => {
  it("adds the session copy shortcut through the native fork action", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /原地复制会话 - Codex\+\+/);
    assert.match(renderer, /createSessionMoreMenuItem\("原地复制会话 - Codex\+\+"/);
    assert.match(renderer, /getAttribute\("aria-label"\)[\s\S]*聊天操作/);
    assert.match(renderer, /从这里创建聊天分支/);
    assert.match(renderer, /data-app-action-sidebar-thread-selected/);
    assert.match(renderer, /sessionCopyMenuActivationTimeoutMs/);
    assert.doesNotMatch(renderer, /\n\s*refreshSessionCopyMenuItems\(\);/);
  });

  it("adds an encrypted session sharing button to the active Codex conversation", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /sessionShareButtonClass\s*=\s*"codex-session-share-button"/);
    assert.match(renderer, /function installSessionShareButton\(\)/);
    assert.match(renderer, /function sessionShareMarkdown\(\)/);
    assert.match(renderer, /crypto\.subtle\.generateKey\(\{ name: "AES-GCM", length: 256 \}/);
    assert.match(renderer, /https:\/\/share\.codexpp\.cc/);
    assert.match(renderer, /postJson\("\/share\/create", payload\)/);
    assert.match(renderer, /postJson\("\/session\/export"/);
    assert.match(renderer, /postJson\("\/session\/import"/);
    assert.match(renderer, /codex-rollout/);
    assert.match(renderer, /function sessionImportMarkdown\(session\)/);
    assert.match(renderer, /codexpp-import-session/);
    assert.match(renderer, /nativeShare\?\.closest\?\.\("\.ms-auto"\)/);
    assert.match(renderer, /#k=\$\{encrypted\.key\}/);
    assert.match(renderer, /navigator\.clipboard\.writeText\(shareUrl\)/);
    assert.match(renderer, /data-testid\*=\"message\"/);
    assert.match(renderer, /function sessionActionTrigger\(row\)/);
    assert.match(renderer, /const sessionMenuEnabled = codexPlusBackendSettings\.enhancementsEnabled !== false/);
    assert.doesNotMatch(renderer, /window\.location\.(?:href|assign)\s*=\s*[^;]*markdown/);
  });

  it("automatically renames a session through the native title suggestion", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /自动重命名当前会话/);
    assert.match(renderer, /activateSessionAutoRenameMenuItem/);
    assert.match(renderer, /input\[aria-label="聊天标题"\], input\[aria-label="Chat title"\]/);
    assert.match(renderer, /button\.classList\.contains\("text-info"\)/);
    assert.match(renderer, /\^\(保存\|Save\)\$/);
    assert.match(renderer, /Codex 未能生成新名称/);
  });

  it("removes the legacy Codex++ top-bar entry", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.doesNotMatch(renderer, /function installCodexPlusMenu\(\)/);
    assert.doesNotMatch(renderer, /function findNativeMenuInsertionPoint\(\)/);
    assert.doesNotMatch(renderer, /codex-plus-trigger/);
  });

  it("places Codex++ in the native sidebar and opens a main-content page", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusSidebarNavId\s*=\s*"codex-plus-sidebar-nav"/);
    assert.match(renderer, /function installCodexPlusSidebarNavigation\(\)/);
    assert.match(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\]/);
    assert.match(renderer, /const insertionButton = pluginButton \|\| navButtons\.find/);
    assert.match(renderer, /selectors\.pluginNavButton/);
    assert.match(renderer, /button\.querySelector\(selectors\.pluginSvgPath\)/);
    assert.match(renderer, /\^\(插件\|Plugins\)\$/);
    assert.match(renderer, /openCodexPlusPage\(\)/);
    assert.match(renderer, /codex-plus-page-overlay/);
    assert.match(renderer, /positionCodexPlusPage/);
    assert.match(renderer, /function closeCodexPlusPage\(\)/);
    assert.match(renderer, /target\?\.closest\("button, a"\)\) closeCodexPlusPage\(\)/);
    assert.match(renderer, /installCodexPlusSidebarNavigation\(\);/);
    assert.match(renderer, /document\.querySelectorAll\(`#\$\{codexPlusMenuId\}/);
  });

  it("does not install Codex++ UI in embedded browser documents", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /window\.top\s*!==\s*window/);
    assert.match(renderer, /!window\.electronBridge/);
    assert.ok(renderer.includes("/^app:\\\/\\\/\\-\\//i.test(window.location.href)"));
    assert.match(renderer, /codexPlusIsNodeTestHarness/);
  });

  it("initializes renderer styles without unresolved template identifiers", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    const appended = installRendererStyle(renderer);

    assert.equal(appended.length, 1);
    assert.match(appended[0].textContent ?? "", /#codex-plus-sidebar-nav/);
  });

  it("hides only the official usage alert and restores it without changing upstream styles", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const wrapper = new FakeElement({ className: "w-full", styleDisplay: "grid" });
    const usageAlert = new FakeElement({ dismissLabel: "Dismiss usage alert", hasProgress: true });
    const otherStatus = new FakeElement({ dismissLabel: "Dismiss sync status", hasProgress: true });
    wrapper.appendChild(usageAlert);
    const { runtime, selectors, windowValue } = usageAlertRuntime(renderer, [usageAlert, otherStatus], [wrapper]);

    windowValue.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = true;
    runtime.refreshOfficialUsageAlertVisibility();

    assert.equal(wrapper.dataset.codexPlusUsageAlertHidden, "true");
    assert.equal(wrapper.style.display, "grid");
    assert.equal(otherStatus.dataset.codexPlusUsageAlertHidden, undefined);
    assert.deepEqual(selectors, [
      '[data-codex-plus-usage-alert-hidden="true"]',
      'aside.app-shell-left-panel [role="status"][aria-live="polite"]',
    ]);

    windowValue.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = false;
    runtime.refreshOfficialUsageAlertVisibility();

    assert.equal(wrapper.dataset.codexPlusUsageAlertHidden, undefined);
    assert.equal(wrapper.style.display, "grid");
    assert.equal(wrapper.children[0], usageAlert);
    assert.equal(selectors.at(-1), '[data-codex-plus-usage-alert-hidden="true"]');
  });

  it("refreshes active-profile usage alert settings through the existing backend heartbeat", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /typeof nextStatus\.hideOfficialUsageAlert === "boolean"/);
    assert.match(renderer, /window\.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = nextStatus\.hideOfficialUsageAlert/);
    assert.match(renderer, /\[data-codex-plus-usage-alert-hidden="true"\] \{ display: none !important; \}/);
    assert.doesNotMatch(renderer, /container\.style\.(?:setProperty|removeProperty)\("display"/);
  });

  it("keeps Windows Dream Skin compatible with the modern Codex main surface", async () => {
    const windowsRenderers = await Promise.all([
      readFile(new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url), "utf8"),
      readFile(new URL("../../../assets/inject/upstream/cidala-tiger/windows/renderer-inject.js", import.meta.url), "utf8"),
    ]);

    for (const renderer of windowsRenderers) {
      assert.match(renderer, /MainContentSurface/);
      assert.match(renderer, /data-codex-plus-dream-surface/);
      assert.match(renderer, /ensureShellMain/);
    }
  });
});

/** 从注入脚本里取出 `shouldScheduleScan`，配上可控的依赖来跑。 */
function shouldScheduleScanRuntime(renderer: string) {
  const start = renderer.indexOf("  function shouldScheduleScan(");
  const end = renderer.indexOf("\n  function runScheduledScan(", start);
  assert.ok(start >= 0 && end > start, "shouldScheduleScan not found in renderer-inject.js");
  const source = renderer.slice(start, end);
  const factory = new Function(
    "isChatContentMutation",
    "isExtensionUiNode",
    "nodeSelfOrAncestorMatchesScanRelevance",
    "isScanRelevantNode",
    `${source}\nreturn shouldScheduleScan;`,
  );
  return factory(
    () => false,
    (node: { extension?: boolean }) => Boolean(node?.extension),
    // Codex 的容器（header / 侧栏 nav）本身就是 scan-relevant，这是自喂循环的关键前提。
    (node: { relevant?: boolean }) => Boolean(node?.relevant),
    (node: { relevant?: boolean; extension?: boolean }) =>
      Boolean(node?.relevant) && !node?.extension,
  ) as (mutations: unknown[]) => boolean;
}

const codexContainer = { nodeType: 1, relevant: true };

function mutation(addedNodes: unknown[] = [], removedNodes: unknown[] = []) {
  return { target: codexContainer, addedNodes, removedNodes };
}

describe("renderer injection scan scheduling", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：我们把自己的节点挂进 Codex 的容器，容器是 scan-relevant，
  // 于是每次写入都会再排一次 scan，scan 又重新写入，空闲时 CPU 被吃满。
  it("ignores mutations that only move the extension's own nodes", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([ownNode])]), false);
    // appendChild 一个已经在位的子节点会同时报 removed + added。
    assert.equal(shouldScheduleScan([mutation([ownNode], [ownNode])]), false);
  });

  it("still scans when Codex itself changes the same container", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const codexNode = { nodeType: 1, relevant: true };
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([codexNode])]), true);
    // 混合变更里只要有一个不是我们的，就不能跳过。
    assert.equal(shouldScheduleScan([mutation([ownNode, codexNode])]), true);
    // 属性变更没有 added/removed 节点，仍按容器相关性判定。
    assert.equal(shouldScheduleScan([mutation()]), true);
  });
});

interface MarketplacePatchHarness {
  install: () => void;
  sweeps: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function marketplacePatchRuntime(renderer: string, patchSucceeds: boolean): MarketplacePatchHarness {
  const start = renderer.indexOf("  const pluginMarketplaceRequestPatchMaxMisses = ");
  const end = renderer.indexOf("\n  function pluginPatchDisabledInRelayMode(", start);
  assert.ok(start >= 0 && end > start, "marketplace patch block not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexPluginMarketplaceUnlockVersion",
    "pluginPatchDisabledInRelayMode",
    "codexPlusSettings",
    "loadAppServerRequestCandidates",
    "patchPluginMarketplaceRequestClient",
    "sendCodexPlusDiagnostic",
    "__note",
    `${source}\nreturn installPluginMarketplaceRequestPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    () => false,
    () => ({ pluginMarketplaceUnlock: true }),
    // 每轮 sweep 在真实实现里会 fetch 全部 app asset，这里只计数并挂起，
    // 好让测试能在「上一轮尚未结束」的时刻再次调用 install。
    () =>
      new Promise((resolve) => {
        sweeps += 1;
        pending.push(() => resolve({ modules: [{}], candidates: [{}], sources: [], discovery: "fallback" }));
      }),
    () => patchSucceeds,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    // 放行所有挂起的 sweep，并把微任务队列排空。
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, sweeps: () => sweeps, diagnostics: () => diagnostics, settle };
}

/** 取出共用的 module loader，用假时钟驱动它的失败冷却。 */
function moduleLoaderRuntime(renderer: string) {
  const start = renderer.indexOf("  // issue #1960：失败必须被记住。");
  const end = renderer.indexOf("\n  async function loadOptionalCodexAppModule(", start);
  assert.ok(start >= 0 && end > start, "loadCodexAppModule not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let clock = 1_000_000;
  const factory = new Function(
    "codexServiceTierModulePromises",
    "codexAppModuleFailures",
    "codexAppModuleRetryCooldownMs",
    "codexAppModuleMaxAttempts",
    "codexAppAssetUrl",
    "codexAppAssetUrlFromScriptText",
    "Date",
    `${source}\nreturn loadCodexAppModule;`,
  );
  const load = factory(
    new Map(),
    new Map(),
    30000,
    8,
    () => "",
    // 真实实现在这里会把全部 app asset 拉一遍；这里只计数并同样返回“没找到”。
    async () => {
      sweeps += 1;
      return "";
    },
    { now: () => clock },
  ) as (namePart: string) => Promise<unknown>;

  const attempt = async (namePart = "vscode-api-") => {
    try {
      await load(namePart);
    } catch {
      /* 预期失败 */
    }
  };
  return { attempt, sweeps: () => sweeps, advance: (ms: number) => { clock += ms; } };
}

describe("renderer injection codex app module loader", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：失败以前只是把 promise 删掉，等于没有负缓存，
  // 调用方一重试就重新 fetch 全部 app asset（实测 301 次请求/秒）。
  it("does not re-sweep every asset while the failure is still in cooldown", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    for (let i = 0; i < 20; i += 1) await loader.attempt();

    assert.equal(loader.sweeps(), 1);
  });

  it("retries once per cooldown window, then gives up for good", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    // 冷却期满就允许再试一次，避免 Codex 更新后 asset 回来了却永远发现不了。
    for (let i = 0; i < 30; i += 1) {
      await loader.attempt();
      loader.advance(30001);
    }

    // 连续失败达到上限(8)后彻底停手，而不是每个冷却窗口都再扫一遍。
    assert.equal(loader.sweeps(), 8);
  });

  it("keeps failures separate per asset prefix", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    await loader.attempt("vscode-api-");
    await loader.attempt("app-initial-");
    await loader.attempt("vscode-api-");

    // 两个前缀各自试了一次；第三次命中 vscode-api- 自己的冷却。
    assert.equal(loader.sweeps(), 2);
  });
});

interface DispatcherPatchHarness {
  install: () => void;
  attempts: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function dispatcherPatchRuntime(renderer: string, dispatcherFound: boolean): DispatcherPatchHarness {
  const start = renderer.indexOf("  const serviceTierDispatcherPatchMaxMisses = ");
  const end = renderer.indexOf("\n  async function loadBackendSettingsState(", start);
  assert.ok(start >= 0 && end > start, "service tier dispatcher patch block not found");
  const source = renderer.slice(start, end);

  let attempts = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexServiceTierRequestOverrideVersion",
    "loadCodexAppModule",
    "codexServiceTierDispatcherFromModule",
    "dispatchCodexPlusMessage",
    "installCodexRemoteSessionDispatcherSubscription",
    "sendCodexPlusDiagnostic",
    `${source}\nreturn installCodexServiceTierDispatcherPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    // 真实实现每轮会依次试三个前缀，每个 miss 都触发一轮全量 asset 扫描。
    () =>
      new Promise((resolve, reject) => {
        attempts += 1;
        pending.push(() => (dispatcherFound ? resolve({}) : reject(new Error("未找到 Codex App asset"))));
      }),
    () => (dispatcherFound ? { dispatchMessage() {}, subscribe() {} } : null),
    () => undefined,
    () => undefined,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, attempts: () => attempts, diagnostics: () => diagnostics, settle };
}

describe("renderer injection service tier dispatcher patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：这个补丁挂在 scanLightweight() 里每轮都跑，是 #1324 同一缺陷的第三个实例。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    await harness.settle();
  });

  it("stops retrying and stops re-reporting once the dispatcher is clearly gone", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 三个前缀里第一个就抛，loadDispatcher 会继续试下一个，所以每轮不止一次尝试；
    // 关键是达到 maxMisses(8) 之后彻底停手。
    assert.equal(harness.diagnostics().filter((e) => e === "service_tier_dispatcher_patch_failed").length, 1);
    assert.deepEqual(harness.diagnostics().at(-1), "service_tier_dispatcher_patch_skipped");
    const settled = harness.attempts();
    harness.install();
    await harness.settle();
    assert.equal(harness.attempts(), settled);
  });

  it("keeps working normally when the dispatcher is found", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    assert.deepEqual(harness.diagnostics(), ["service_tier_dispatcher_patch_installed"]);
  });
});

describe("renderer injection plugin marketplace patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：scanDeferred() 每轮都调用这个补丁，而早退守卫只在打上补丁后才写入。
  // Codex 侧 asset 改名后这层永远成功不了，过去既不去重也不放弃，
  // 于是每轮 scan 都把全部 app asset 重新 fetch 一遍（实测 530 次 fetch/秒）。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 模拟连续多轮 scan：上一轮还挂着，后续调用必须被 in-flight 守卫挡掉。
    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    await harness.settle();
  });

  it("stops retrying once the asset is clearly unavailable", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 每次都跑完再发起下一轮，模拟长时间运行中的反复 scan。
    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 达到 maxMisses(8) 之后必须彻底停掉，而不是无限重试。
    assert.equal(harness.sweeps(), 8);
    // 首次 miss 上报一次，停用时再报一次，中间保持噤声。
    assert.deepEqual(harness.diagnostics(), [
      "plugin_marketplace_request_patch_not_found",
      "plugin_marketplace_request_patch_skipped",
    ]);
  });

  it("keeps working normally when the patch actually lands", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    // 打上补丁后守卫生效，后续 scan 不再重复扫描。
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    assert.deepEqual(harness.diagnostics(), ["plugin_marketplace_request_patch_installed"]);
  });
});
