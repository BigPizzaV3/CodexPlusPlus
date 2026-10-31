// 输入框下方用量状态栏：口径与取数的行为回归。
//
// cdp_bridge.rs 只守「接线还在不在」，这里把分片里的纯计算函数抠出来放进沙箱真跑，
// 验证三件在真机上不好复现的事：
//   1. 输出吞吐按「同一轮内相邻事件差」求和后相除（不能等权平均各区间速率，#2636），
//      按真实 turn_id 换轮，过短/过长的间隔不能当样本；
//   2. 缓存命中与上下文占用是 Codex 自己的口径（cached/input、total/context_window），
//      窗口缺失时不能瞎算成 0%；
//   3. 会话 id 只认 composer 上的实时标识，新会话草稿（client-new-thread）没有 rollout，
//      必须当作无会话而不是拿草稿 id 去查；
//   4. 旧版 Codex（26.825 之前，Intel Mac 只能停在旧版）没有新版 composer 标记时，
//      退回旧类名 / 输入框所在 form 挂载，会话退回侧边栏当前行 + 地址栏，挂不上要写诊断（#2753）。
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import path from "node:path";
import test from "node:test";
import ts from "typescript";

const fragmentDir = path.resolve(import.meta.dirname, "../../../assets/inject/renderer-inject");
const shardSource = readFileSync(path.join(fragmentDir, "83-usage-statusline.js"), "utf8");
const shard = ts.createSourceFile("83-usage-statusline.js", shardSource, ts.ScriptTarget.Latest, true, ts.ScriptKind.JS);

/** 按名字取分片里的顶层声明源码；找不到就直接失败，避免锚点漂移后静默跳过。 */
function declaration(name: string): string {
  for (const statement of shard.statements) {
    if (ts.isFunctionDeclaration(statement) && statement.name?.text === name) return statement.getText(shard);
    if (ts.isVariableStatement(statement)) {
      const hit = statement.declarationList.declarations.some(
        (item) => ts.isIdentifier(item.name) && item.name.text === name,
      );
      if (hit) return statement.getText(shard);
    }
  }
  assert.fail(`分片里找不到 ${name}`);
}

const sandboxSource = [
  declaration("codexPlusUsageStaleMs"),
  declaration("codexPlusUsagePercent"),
  declaration("codexPlusUsageLatestTurn"),
  declaration("codexPlusUsageMaxIntervalSeconds"),
  declaration("codexPlusUsageRate"),
  declaration("codexPlusUsageTurnCache"),
  declaration("codexPlusUsageSnapshot"),
  declaration("codexPlusUsageValidThreadId"),
  declaration("codexPlusUsageLegacyThreadId"),
  declaration("codexPlusUsageResolveThread"),
  declaration("codexPlusUsageThreadId"),
  "return { codexPlusUsagePercent, codexPlusUsageRate, codexPlusUsageTurnCache, codexPlusUsageSnapshot, codexPlusUsageThreadId, codexPlusUsageResolveThread };",
].join("\n");

type Sandbox = {
  codexPlusUsagePercent: (numerator: number, denominator: number) => number | null;
  codexPlusUsageRate: (events: Array<Record<string, unknown>>) => number | null;
  codexPlusUsageTurnCache: (events: Array<Record<string, unknown>>) => number | null;
  codexPlusUsageSnapshot: (events: Array<Record<string, unknown>>) => {
    speed: number | null;
    cache: number | null;
    context: number | null;
    stale: boolean;
    modelContextWindow: number;
  } | null;
  codexPlusUsageThreadId: (card: unknown) => string;
  codexPlusUsageResolveThread: (card: unknown) => { id: string; source: string; reason: string; placement: string };
};

function sandbox(
  nowMs: number,
  documentStub: unknown = { querySelector: () => null },
  currentSessionRef?: () => { session_id: string },
): Sandbox {
  // 分片用 Date.now() 判新鲜度，注入一个只改 now 的子类，Date.parse 仍走原生实现。
  class FixedDate extends Date {
    static now() {
      return nowMs;
    }
  }
  const factory = new Function("document", "navigator", "Date", "currentSessionRef", sandboxSource);
  return factory(documentStub, { language: "zh-CN" }, FixedDate, currentSessionRef) as Sandbox;
}

function event(secondsFromStart: number, turnOutputTokens: number, extra: Record<string, unknown> = {}) {
  return {
    t: new Date(Date.UTC(2026, 5, 2, 5, 0, secondsFromStart)).toISOString(),
    turn_output_tokens: turnOutputTokens,
    turn_id: "turn-1",
    input_tokens: 1000,
    cached_input_tokens: 0,
    output_tokens: 0,
    total_tokens: 1000,
    context_window: 100000,
    ...extra,
  };
}

test("输出吞吐按同一轮内相邻事件差计算，换轮时切段", () => {
  const { codexPlusUsageRate } = sandbox(Date.UTC(2026, 5, 2, 5, 1, 0));
  const events = [
    event(0, 100),
    event(10, 600),
    event(20, 1100),
    // 累计输出跨轮仍然递增，只有 turn_id 能确认边界。
    event(30, 2100, { turn_id: "turn-2" }),
    event(40, 2600, { turn_id: "turn-2" }),
  ];
  assert.equal(codexPlusUsageRate(events), 500 / 10);
  assert.equal(codexPlusUsageRate([event(0, 100)]), null, "只有一条事件时算不出速度");
});

test("输出吞吐按总输出 ÷ 总时长加权，不对各区间速率等权平均（#2636 算例）", () => {
  const { codexPlusUsageRate } = sandbox(Date.UTC(2026, 5, 2, 5, 1, 0));
  // issue 里的三个区间：前两段含工具调用，最后一段是大段回答。
  const at = (ms: number) => new Date(Date.UTC(2026, 5, 2, 5, 0, 0) + ms).toISOString();
  const events = [
    { ...event(0, 1000), t: at(0) },
    { ...event(0, 1221), t: at(21_132) },
    { ...event(0, 1528), t: at(21_132 + 21_752) },
    { ...event(0, 2781), t: at(21_132 + 21_752 + 11_409) },
  ];
  const rate = codexPlusUsageRate(events);
  assert.ok(rate !== null);
  // 旧实现：(10.46 + 14.11 + 109.83) / 3 ≈ 44.8。小输出的工具步骤和大段回答同权，口径错误。
  assert.notEqual(Math.round(rate * 10) / 10, 44.8);
  // 新口径：(221 + 307 + 1253) / (21.132 + 21.752 + 11.409) = 1781 / 54.293 ≈ 32.80。
  assert.equal(Number(rate.toFixed(2)), Number((1781 / 54.293).toFixed(2)));

  // 长停顿区间分子分母一起剔除，不能只丢掉速率而把 token 留在分子里。
  const withPause = [
    ...events,
    { ...event(0, 3781), t: at(21_132 + 21_752 + 11_409 + 200_000) },
  ];
  assert.equal(Number(codexPlusUsageRate(withPause)!.toFixed(2)), Number((1781 / 54.293).toFixed(2)));

  // 重复记录 / 累计输出回落的区间不算样本，也不能把后面的增量算重。
  const withDuplicate = [
    { ...event(0, 1000), t: at(0) },
    { ...event(0, 1221), t: at(21_132) },
    { ...event(0, 1221), t: at(22_000) },
    { ...event(0, 1528), t: at(21_132 + 21_752) },
  ];
  assert.equal(
    Number(codexPlusUsageRate(withDuplicate)!.toFixed(4)),
    Number(((221 + 307) / (21.132 + (21_132 + 21_752 - 22_000) / 1000)).toFixed(4)),
  );
});

test("过短与过长的间隔都不作为速度样本", () => {
  const { codexPlusUsageRate } = sandbox(Date.UTC(2026, 5, 2, 5, 1, 0));
  // 0.2 秒是同一批写入，200 秒说明中间停了很久。
  assert.equal(codexPlusUsageRate([
    { ...event(0, 0), t: "2026-06-02T05:00:00.000Z" },
    { ...event(0, 500), t: "2026-06-02T05:00:00.200Z" },
  ]), null);
  assert.equal(codexPlusUsageRate([
    { ...event(0, 0), t: "2026-06-02T05:00:00.000Z" },
    { ...event(0, 50000), t: "2026-06-02T05:03:20.000Z" },
  ]), null);
});

test("缓存命中与上下文占用按 Codex 的口径取值", () => {
  const now = Date.UTC(2026, 5, 2, 5, 0, 10);
  const { codexPlusUsageSnapshot } = sandbox(now);
  const snapshot = codexPlusUsageSnapshot([
    event(0, 500, { input_tokens: 200000, cached_input_tokens: 199000, total_tokens: 250000, context_window: 1000000 }),
  ]);
  assert.ok(snapshot);
  assert.equal(Number(snapshot.cache?.toFixed(1)), 99.5);
  assert.equal(snapshot.context, 25);
  assert.equal(snapshot.modelContextWindow, 1000000);
  assert.equal(snapshot.stale, false, "刚写入的事件不算过期");

  // 窗口缺失时上下文占用必须是 null，不能算成 0% 骗人。
  const noWindow = codexPlusUsageSnapshot([event(0, 1, { context_window: 0 })]);
  assert.equal(noWindow?.context, null);

  assert.equal(codexPlusUsageSnapshot([]), null);
});

test("同一轮缓存命中率按输入 token 加权，换轮后重新计算", () => {
  const { codexPlusUsageTurnCache } = sandbox(Date.UTC(2026, 5, 2, 5, 0, 10));
  assert.equal(codexPlusUsageTurnCache([
    event(0, 100, { input_tokens: 100, cached_input_tokens: 40 }),
    event(1, 200, { input_tokens: 900, cached_input_tokens: 810 }),
  ]), 85);
  assert.equal(codexPlusUsageTurnCache([
    event(0, 100, { input_tokens: 100, cached_input_tokens: 40 }),
    event(1, 200, { input_tokens: 1000, cached_input_tokens: 100, turn_id: "turn-2" }),
  ]), 10);
});

test("会话累计输出持续增长时，第二轮 0% 不能混成 50%", () => {
  const { codexPlusUsageTurnCache, codexPlusUsageRate } = sandbox(Date.UTC(2026, 5, 2, 5, 0, 10));
  const events = [
    event(0, 100, { cached_input_tokens: 1000 }),
    event(1, 200, { turn_id: "turn-2" }),
  ];
  assert.equal(codexPlusUsageTurnCache(events), 0);
  assert.equal(codexPlusUsageRate(events), null, "新一轮只有一次调用，不能跨轮估算速度");
});

test("缺少真实轮边界的旧记录只取最后一次调用", () => {
  const { codexPlusUsageTurnCache, codexPlusUsageRate } = sandbox(Date.UTC(2026, 5, 2, 5, 0, 10));
  const events = [
    event(0, 100, { cached_input_tokens: 1000, turn_id: undefined }),
    event(1, 200, { turn_id: undefined }),
  ];
  assert.equal(codexPlusUsageTurnCache(events), 0);
  assert.equal(codexPlusUsageRate(events), null);
  assert.equal(codexPlusUsageTurnCache([]), null);
  assert.equal(codexPlusUsageTurnCache([
    event(0, 100, { input_tokens: 0, cached_input_tokens: 100, turn_id: undefined }),
  ]), null);
});

test("最近一条事件超过阈值后标记为过期", () => {
  const observed = Date.UTC(2026, 5, 2, 5, 0, 0);
  const fresh = sandbox(observed + 60_000).codexPlusUsageSnapshot([event(0, 1)]);
  const stale = sandbox(observed + 130_000).codexPlusUsageSnapshot([event(0, 1)]);
  assert.equal(fresh?.stale, false);
  assert.equal(stale?.stale, true);
});

test("三项目作为一组居中，过期只淡化速度项", () => {
  // 布局对齐 DSH 那条：三项居中成组，不是两端对齐。
  // 两端对齐会把「上下文占用」甩到最右边，和速度、缓存拉成两段信息。
  assert.match(shardSource, /justify-content:center/);
  assert.doesNotMatch(shardSource, /data-usage="context"\]\{margin-left:auto\}/);

  // 本轮结束后速度是历史读数，但缓存命中与上下文占用仍是当前值，不能一起变灰。
  const staleRule = shardSource.match(/\[data-stale="true"\][^{]*\{opacity:([^}]*)\}/g) || [];
  assert.equal(staleRule.length, 1, "只应有一条过期样式");
  assert.match(staleRule[0], /data-usage="speed"/);
  // 容器自身不能再挂 opacity：会叠在文字颜色的透明度上，实测掉到 0.36 看不清。
  assert.doesNotMatch(shardSource, /\[data-stale="true"\]\{/);
});

test("会话 id 只认 composer 上的实时标识，新会话草稿视为无会话", () => {
  const card = (composerId: string | null, placement = "thread") => ({
    closest: () => ({
      getAttribute: () => placement,
      querySelector: () => (composerId === null ? null : { getAttribute: () => composerId }),
    }),
  });
  const sidebar = (sidebarId: string | null) => ({
    querySelector: (selector: string) => {
      assert.match(selector, /data-app-action-sidebar-thread-selected/);
      return sidebarId === null ? null : { getAttribute: () => sidebarId };
    },
  });

  const { codexPlusUsageThreadId } = sandbox(Date.now(), sidebar(null));
  assert.equal(
    codexPlusUsageThreadId(card("local:01a1352f-f8a9-71f2-9ae4-f2de48a74d8d")),
    "01a1352f-f8a9-71f2-9ae4-f2de48a74d8d",
    "去掉 local: 前缀后交给宿主",
  );
  assert.equal(
    codexPlusUsageThreadId(card("01a1352f-f8a9-71f2-9ae4-f2de48a74d8d")),
    "01a1352f-f8a9-71f2-9ae4-f2de48a74d8d",
    "没有前缀时原样使用",
  );
  // thread 里 composer 没带标识时回落侧边栏选中行。
  assert.equal(
    sandbox(Date.now(), sidebar("local:01a133b6-fec1-7641-a1d1-f27100cee5b6"))
      .codexPlusUsageThreadId(card(null)),
    "01a133b6-fec1-7641-a1d1-f27100cee5b6",
  );
  // 首页 composer 是 placement=home，侧边栏这时仍选中着上一个 thread，
  // 兜底会把上一个会话的读数挂到新会话上，必须拒绝。
  assert.equal(
    sandbox(Date.now(), sidebar("local:01a133b6-fec1-7641-a1d1-f27100cee5b6"))
      .codexPlusUsageThreadId(card(null, "home")),
    "",
  );
  assert.equal(codexPlusUsageThreadId(card("local:client-new-thread:dac7b41a-c092-4147-827e-9010f1ba7600")), "", "新会话草稿没有 rollout");
  assert.equal(codexPlusUsageThreadId(card("__proto__")), "");
  assert.equal(codexPlusUsageThreadId(card("short")), "");
  assert.equal(codexPlusUsageThreadId(undefined), "");
});

function lifecycle() {
  const node = { isConnected: true, previousElementSibling: null as unknown, dataset: {} as Record<string, string> };
  let current = { id: "thread-a-123", card: {} };
  const renders: any[] = [];
  let resolveRequest: (value: unknown) => void = () => {};
  let rejectRequest: (error: Error) => void = () => {};
  const source = [
    ...["codexPlusUsageState", "codexPlusUsageStaleMs", "codexPlusUsageIdleIntervalMs",
      "codexPlusUsageActiveIntervalMs", "codexPlusUsageWarmupIntervalMs", "codexPlusUsageFailureLimit",
      "codexPlusUsagePercent", "codexPlusUsageLatestTurn", "codexPlusUsageMaxIntervalSeconds", "codexPlusUsageRate", "codexPlusUsageTurnCache", "codexPlusUsageSnapshot",
      "codexPlusUsageHide", "codexPlusUsageShow", "codexPlusUsageSyncPlacement",
      "codexPlusUsageDiagnosticLimit", "codexPlusUsageDiagnoseOnce", "codexPlusUsageNoteDataGaps",
      "codexPlusUsageRemember", "codexPlusUsageRefresh", "codexPlusUsageTick"].map(declaration),
    "return { state: codexPlusUsageState, sync: codexPlusUsageSyncPlacement, tick: codexPlusUsageTick, remember: codexPlusUsageRemember };",
  ].join("\n");
  const diagnostics: Array<{ event: string; detail: any }> = [];
  const runtime = new Function("document", "codexPlusUsageEnabled", "codexPlusUsageComposerCard",
    "codexPlusUsageResolveThread", "codexPlusUsageReportPlacement", "codexPlusUsageEnsureNode", "codexPlusUsageRender",
    "codexPlusUsageSchedule", "postJson", "sendCodexPlusDiagnostic", source)(
    { hidden: false }, () => true, () => ({ card: current.card, source: "surface-variant" }),
    () => ({ id: current.id, source: current.id ? "composer" : "", reason: current.id ? "" : "home", placement: "" }),
    () => {},
    (card: unknown) => { node.previousElementSibling = card; return node; },
    (value: unknown) => renders.push(value), () => {},
    () => new Promise((resolve, reject) => { resolveRequest = resolve; rejectRequest = reject; }),
    (event: string, detail: unknown) => diagnostics.push({ event, detail }),
  );
  runtime.state.node = node;
  return {
    ...runtime, node, renders, diagnostics,
    switchTo(id: string) { current = { id, card: {} }; },
    respond(events: unknown[]) { resolveRequest({ status: "ok", events }); },
    respondStatus(status: string, message: string) { resolveRequest({ status, message, events: [] }); },
    fail() { rejectRequest(new Error("bridge unavailable")); },
  };
}

test("数据请求完成前就显示栏位，切换会话立即恢复各自缓存", async () => {
  const ui = lifecycle();
  const pending = ui.tick();
  assert.notEqual(ui.node.dataset.hidden, "true");
  assert.equal(ui.renders.at(-1), null);
  ui.respond([event(0, 100, { total_tokens: 27000 })]);
  await pending;
  assert.equal(ui.renders.at(-1).context, 27);
  const original = ui.state.node;
  ui.switchTo("thread-b-123");
  ui.sync();
  assert.equal(ui.renders.at(-1), null, "不能沿用其他会话的数字");
  ui.switchTo("thread-a-123");
  ui.sync();
  assert.equal(ui.renders.at(-1).context, 27);
  assert.equal(ui.state.node, original, "会话切换复用同一栏位");
});

test("旧请求返回不能覆盖当前会话，也不能在新建或设置页面重新显示", async () => {
  const ui = lifecycle();
  const pending = ui.tick();
  ui.switchTo("thread-b-123");
  ui.sync();
  ui.respond([event(0, 100, { total_tokens: 27000 })]);
  await pending;
  assert.equal(ui.renders.at(-1), null);
  assert.equal(ui.state.threadId, "thread-b-123");
  const other = ui.tick();
  ui.switchTo("");
  ui.sync();
  ui.respond([event(0, 100)]);
  await other;
  assert.equal(ui.node.dataset.hidden, "true");
  assert.equal(ui.state.threadId, "");
});

test("空数据及连续失败仍保留已有会话栏位，缓存有界", async () => {
  const ui = lifecycle();
  const pending = ui.tick();
  ui.respond([]);
  await pending;
  assert.notEqual(ui.node.dataset.hidden, "true");
  for (let i = 0; i < 5; i++) {
    const failed = ui.tick();
    ui.fail();
    await failed;
  }
  assert.equal(ui.state.failures, 5);
  assert.notEqual(ui.node.dataset.hidden, "true", "桥接不可用仍保留栏位");
  for (let i = 0; i < 40; i++) ui.remember(`thread-${i}`, [event(0, i)]);
  assert.equal(ui.state.cache.size, 32);
  assert.equal(ui.state.cache.has("thread-0"), false);
});

test("宿主没有数据或记录缺字段时照样显示，只各记一次诊断（#2753）", async () => {
  const ui = lifecycle();
  for (let i = 0; i < 2; i++) {
    const pending = ui.tick();
    ui.respondStatus("not_found", "Thread not found in local storage");
    await pending;
  }
  assert.notEqual(ui.node.dataset.hidden, "true", "没数据也保留占位栏位");
  assert.deepEqual(ui.diagnostics.map((item: { event: string }) => item.event), ["usage_statusline_data_unavailable"]);
  assert.equal(ui.diagnostics[0].detail.status, "not_found");

  // 旧版 Codex 的 rollout 没有窗口与轮 id：上下文占用显示 --，并记下缺了什么。
  const partial = ui.tick();
  ui.respond([event(0, 100, { context_window: undefined, turn_id: undefined })]);
  await partial;
  assert.equal(ui.renders.at(-1).context, null);
  const gap = ui.diagnostics.find((item: { event: string }) => item.event === "usage_statusline_data_partial");
  assert.deepEqual(gap?.detail.missing, ["context_window", "turn_id"]);
});

/**
 * 极简 DOM 桩：按选择器原文查表。分片里的选择器是常量，查表比引一个 DOM 实现更直接，
 * 选择器改了这里会立刻失配报错。
 */
function fakeElement(options: { editor?: boolean; form?: unknown; root?: unknown; visible?: boolean } = {}) {
  return {
    visible: options.visible !== false,
    querySelector: (selector: string) => (selector.includes("ProseMirror") && options.editor ? {} : null),
    closest: (selector: string) => {
      if (selector === "form") return options.form || null;
      if (selector === "[data-codex-composer-root]") return options.root || null;
      return null;
    },
  };
}

function composerSandbox(table: Record<string, unknown[]>) {
  const source = [
    ...["codexPlusUsageState", "codexPlusUsageCardSelector", "codexPlusUsageLegacyCardSelector",
      "codexPlusUsageEditorSelector", "codexPlusUsageComposerCard"].map(declaration),
    "return { state: codexPlusUsageState, find: codexPlusUsageComposerCard };",
  ].join("\n");
  // 逗号分隔的选择器按各部分合并，和浏览器一致（否则组合选择器永远命中不到，测不出误判）。
  const all = (selector: string) => table[selector] || selector.split(/,\s*/).flatMap((part) => table[part] || []);
  const documentStub = {
    querySelectorAll: all,
    querySelector: (selector: string) => all(selector)[0] || null,
  };
  return new Function("document", "visibleElement", source)(
    documentStub, (element: { visible?: boolean }) => element?.visible !== false,
  );
}

test("旧版 Codex 没有 data-composer-surface-variant 时退回旧类名与输入框所在 form（#2753）", () => {
  const modern = fakeElement({ editor: true });
  assert.deepEqual(
    composerSandbox({ "[data-composer-surface-variant]": [modern] }).find(),
    { card: modern, source: "surface-variant" },
  );

  const decorative = fakeElement();
  const chrome = fakeElement({ editor: true });
  assert.deepEqual(
    composerSandbox({ '.composer-surface-chrome, [class*="_ComposerLayoutRoot_"]': [decorative, chrome] }).find(),
    { card: chrome, source: "legacy-chrome" },
    "旧类名必须真的包着输入框",
  );

  const form = fakeElement();
  const editor = fakeElement({ form });
  assert.deepEqual(
    composerSandbox({ '.ProseMirror[contenteditable="true"]': [editor] }).find(),
    { card: form, source: "editor-form" },
  );

  // 新版 Codex 停在没有输入卡片的页面：不能退回旧版兜底，把别处的 form 当输入框。
  const modernPage = composerSandbox({
    "[data-composer-surface-variant]": [fakeElement({ visible: false })],
    '.ProseMirror[contenteditable="true"]': [editor],
  });
  assert.equal(modernPage.find(), null);
  assert.equal(modernPage.state.modernDom, true);
  assert.equal(composerSandbox({}).find(), null);
});

test("旧版 Codex 已有 composer 根标记、还没有新版卡片标记时仍走旧版兜底（#2753 Intel Mac 日志）", () => {
  // 日志：surfaceVariant=0、composerRoot=1、legacyChrome=1、editors=1。
  const root = fakeElement({ editor: true });
  const chrome = fakeElement({ editor: true, root });
  const page = composerSandbox({
    "[data-codex-composer-root]": [root],
    '.composer-surface-chrome, [class*="_ComposerLayoutRoot_"]': [chrome],
  });
  assert.deepEqual(page.find(), { card: chrome, source: "legacy-chrome" });
  assert.equal(page.state.modernDom, false, "只有根标记不算新版");

  // 没有旧类名时挂在包着输入框的根节点上。
  assert.deepEqual(composerSandbox({ "[data-codex-composer-root]": [root] }).find(), { card: root, source: "composer-root" });
  // 根节点没包输入框（例如装饰容器）不算。
  assert.equal(composerSandbox({ "[data-codex-composer-root]": [fakeElement()] }).find(), null);
});

test("旧版 Codex 的 composer 没有根标记与会话标识时，会话退回侧边栏当前行 + 地址栏（#2753）", () => {
  const legacyCard = fakeElement();
  const uuid = "019a1352-f8a9-71f2-9ae4-f2de48a74d8d";
  const withRef = (id: string) => sandbox(Date.now(), { querySelector: () => null }, () => ({ session_id: id }));

  assert.deepEqual(
    withRef(`local:${uuid}`).codexPlusUsageResolveThread(legacyCard),
    { id: uuid, source: "legacy", reason: "", placement: "" },
  );
  assert.equal(withRef("").codexPlusUsageResolveThread(legacyCard).reason, "no-thread-id");
  assert.equal(withRef("local:client-new-thread:dac7b41a").codexPlusUsageThreadId(legacyCard), "");
  // 测试沙箱里没有 currentSessionRef：按缺失处理，不能抛错。
  assert.equal(sandbox(Date.now()).codexPlusUsageResolveThread(legacyCard).reason, "no-thread-id");

  const rooted = (placement: string) => fakeElement({
    root: { getAttribute: () => placement, querySelector: () => null },
  });
  // 新版 thread 里两处标识都缺时也能用兜底；首页与其它 placement 仍然拒绝。
  assert.equal(withRef(uuid).codexPlusUsageThreadId(rooted("thread")), uuid);
  assert.equal(withRef(uuid).codexPlusUsageResolveThread(rooted("home")).reason, "home");
  assert.equal(withRef(uuid).codexPlusUsageResolveThread(rooted("side-chat")).reason, "other-placement");
});

function reportSandbox(table: Record<string, unknown[]> = {}) {
  const sent: Array<{ event: string; detail: any }> = [];
  const source = [
    ...["codexPlusUsageState", "codexPlusUsageCardSelector", "codexPlusUsageDiagnosticLimit",
      "codexPlusUsageDiagnoseOnce", "codexPlusUsageDomProbe", "codexPlusUsageVisibleEditor",
      "codexPlusUsageReportPlacement"].map(declaration),
    "return { report: codexPlusUsageReportPlacement };",
  ].join("\n");
  const documentStub = { querySelectorAll: (selector: string) => table[selector] || [] };
  const runtime = new Function("document", "visibleElement", "sendCodexPlusDiagnostic", source)(
    documentStub, () => true, (event: string, detail: any) => sent.push({ event, detail }),
  );
  return { ...runtime, sent };
}

test("挂载结果写进诊断：旧版兜底与挂不上各记一次，首页和草稿不记（#2753）", () => {
  const ui = reportSandbox({
    ".composer-surface-chrome": [{}],
    '.ProseMirror[contenteditable="true"]': [{ closest: () => null }],
  });
  const card = { card: {}, source: "legacy-chrome" };
  ui.report(card, { id: "019a1352-f8a9", source: "legacy", reason: "", placement: "" });
  ui.report(card, { id: "019a1352-f8a9", source: "legacy", reason: "", placement: "" });
  ui.report({ card: {}, source: "surface-variant" }, { id: "x", source: "composer", reason: "", placement: "thread" });
  ui.report(card, { id: "", source: "", reason: "home", placement: "home" });
  ui.report(card, { id: "", source: "", reason: "draft", placement: "home" });
  ui.report(card, { id: "", source: "", reason: "no-thread-id", placement: "" });
  ui.report(null, { id: "", source: "", reason: "no-card", placement: "" });

  assert.deepEqual(ui.sent.map((item: { event: string }) => item.event), [
    "usage_statusline_fallback_mount",
    "usage_statusline_mount_missing",
    "usage_statusline_mount_missing",
  ]);
  assert.deepEqual(
    { card: ui.sent[0].detail.card, thread: ui.sent[0].detail.thread },
    { card: "legacy-chrome", thread: "legacy" },
  );
  assert.equal(ui.sent[1].detail.reason, "no-thread-id");
  assert.equal(ui.sent[2].detail.reason, "no-composer-card");
  // DOM 快照只有计数与布尔值，不带会话 id。
  assert.equal(ui.sent[0].detail.dom.legacyChrome, 1);
  assert.equal(ui.sent[0].detail.dom.surfaceVariant, 0);
  assert.ok(Object.values(ui.sent[0].detail.dom).every((value) => typeof value !== "string"));

  // 没有输入框的页面（设置页等）找不到卡片不算异常。
  const empty = reportSandbox();
  empty.report(null, { id: "", source: "", reason: "no-card", placement: "" });
  assert.equal(empty.sent.length, 0);
});
