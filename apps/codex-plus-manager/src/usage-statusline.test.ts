// 输入框下方用量状态栏：口径与取数的行为回归。
//
// cdp_bridge.rs 只守「接线还在不在」，这里把分片里的纯计算函数抠出来放进沙箱真跑，
// 验证三件在真机上不好复现的事：
//   1. 生成速度按「同一轮内相邻事件差」算，按真实 turn_id 换轮，
//      过短/过长的间隔不能当样本；
//   2. 缓存命中与上下文占用是 Codex 自己的口径（cached/input、total/context_window），
//      窗口缺失时不能瞎算成 0%；
//   3. 会话 id 只认 composer 上的实时标识，新会话草稿（client-new-thread）没有 rollout，
//      必须当作无会话而不是拿草稿 id 去查。
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
  declaration("codexPlusUsageRate"),
  declaration("codexPlusUsageTurnCache"),
  declaration("codexPlusUsageSnapshot"),
  declaration("codexPlusUsageThreadId"),
  "return { codexPlusUsagePercent, codexPlusUsageRate, codexPlusUsageTurnCache, codexPlusUsageSnapshot, codexPlusUsageThreadId };",
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
};

function sandbox(nowMs: number, documentStub: unknown = { querySelector: () => null }): Sandbox {
  // 分片用 Date.now() 判新鲜度，注入一个只改 now 的子类，Date.parse 仍走原生实现。
  class FixedDate extends Date {
    static now() {
      return nowMs;
    }
  }
  const factory = new Function("document", "navigator", "Date", sandboxSource);
  return factory(documentStub, { language: "zh-CN" }, FixedDate) as Sandbox;
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

test("生成速度按同一轮内相邻事件差计算，换轮时切段", () => {
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
      "codexPlusUsagePercent", "codexPlusUsageLatestTurn", "codexPlusUsageRate", "codexPlusUsageTurnCache", "codexPlusUsageSnapshot",
      "codexPlusUsageHide", "codexPlusUsageShow", "codexPlusUsageSyncPlacement",
      "codexPlusUsageRemember", "codexPlusUsageRefresh", "codexPlusUsageTick"].map(declaration),
    "return { state: codexPlusUsageState, sync: codexPlusUsageSyncPlacement, tick: codexPlusUsageTick, remember: codexPlusUsageRemember };",
  ].join("\n");
  const runtime = new Function("document", "codexPlusUsageEnabled", "codexPlusUsageComposerCard",
    "codexPlusUsageThreadId", "codexPlusUsageEnsureNode", "codexPlusUsageRender",
    "codexPlusUsageSchedule", "postJson", "sendCodexPlusDiagnostic", source)(
    { hidden: false }, () => true, () => current.card, () => current.id,
    (card: unknown) => { node.previousElementSibling = card; return node; },
    (value: unknown) => renders.push(value), () => {},
    () => new Promise((resolve, reject) => { resolveRequest = resolve; rejectRequest = reject; }), () => {},
  );
  runtime.state.node = node;
  return {
    ...runtime, node, renders,
    switchTo(id: string) { current = { id, card: {} }; },
    respond(events: unknown[]) { resolveRequest({ status: "ok", events }); },
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
