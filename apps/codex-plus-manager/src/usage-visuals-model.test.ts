import assert from "node:assert/strict";
import test from "node:test";
import type { UsageRouteDay } from "./gateway-model.ts";
import { calendarValue, heatLevel, matchUsageFilters, usageCalendar, usageFlow, visualDays, weekDay } from "./usage-visuals-model.ts";

function route(overrides: Partial<UsageRouteDay> = {}): UsageRouteDay {
  return {
    date: "2026-11-21", agent: "codex", providerId: "relay-a", provider: "Relay A", model: "model", calls: 1, errors: 0,
    inputTokens: 30, outputTokens: 10, cachedTokens: 10, reasoningTokens: 5, tokenMeteredCalls: 1, ...overrides,
  };
}

test("calendar dates stay consecutive across DST, start in the correct weekday, and are capped at 53 weeks", () => {
  assert.deepEqual(visualDays(["2026-03-07", "2026-03-10"]), ["2026-03-07", "2026-03-08", "2026-03-09", "2026-03-10"]);
  assert.equal(weekDay("2026-03-09"), 0);
  assert.equal(weekDay("2026-03-08"), 6);
  const days = visualDays(["2022-09-01", "2026-11-22"]);
  assert.equal(days.length, 371);
  assert.equal(days.at(-1), "2026-11-22");
});

test("calendar distinguishes an unobserved day, a filtered zero, an unmetered request, and measured zero", () => {
  const days = ["2026-11-20", "2026-11-21", "2026-11-22", "2026-11-23", "2026-11-24"];
  const rows = [
    route({ providerId: "other" }),
    route({ date: days[2], tokenMeteredCalls: undefined, inputTokens: 0, outputTokens: 0 }),
    route({ date: days[3], inputTokens: 0, outputTokens: 0 }),
    route({ date: days[4], tokenMeteredCalls: undefined }),
  ];
  const calendar = usageCalendar(days, rows, { providerId: "relay-a" });
  assert.deepEqual(calendar.map(day => calendarValue(day.usage, "calls")), [undefined, 0, 1, 1, 1]);
  assert.deepEqual(calendar.map(day => calendarValue(day.usage, "tokens")), [undefined, 0, undefined, 0, 40]);
  assert.deepEqual([undefined, 0, 2, 4, 6, 8].map(value => heatLevel(value, 8)), [-1, 0, 1, 2, 3, 4]);
});

test("Sankey preserves joint routes and volume without double-counting cached or reasoning tokens", () => {
  const rows = [
    route({ calls: 3 }),
    route({ agent: "claude", providerId: "relay-b", calls: 7, inputTokens: 0, outputTokens: 0 }),
    route({ agent: "grok", providerId: "relay-b", calls: 2 }),
  ];
  const calls = usageFlow(rows, "calls");
  assert.equal(calls.total, 12);
  assert.deepEqual(calls.columns[2].map(node => [node.providerId, node.key]), [["relay-b", "model"], ["relay-a", "model"]]);
  const first = calls.links.filter(link => link.stage === 0);
  assert.deepEqual(first.map(link => [link.source.key, link.target.key, link.value]), [
    ["claude", "relay-b", 7], ["codex", "relay-a", 3], ["grok", "relay-b", 2],
  ]);
  const tokens = usageFlow(rows, "tokens");
  assert.equal(tokens.total, 80);
  assert.equal(tokens.columns[0].some(node => node.key === "claude"), false);
  for (const flow of [calls, tokens]) {
    for (const stage of [0, 1]) {
      assert.equal(flow.links.filter(link => link.stage === stage).reduce((sum, link) => sum + link.value, 0), flow.total);
    }
  }
});

test("folding rare paths preserves totals and does not hide or conflate explicit unknown dimensions", () => {
  const rows = [
    route({ agent: "", providerId: "", model: "" }),
    ...Array.from({ length: 8 }, (_, index) => route({ agent: `agent-${index}`, providerId: `relay-${index}`, model: `model-${index}` })),
  ];
  assert.deepEqual(rows.filter(row => matchUsageFilters(row, { providerId: "", model: "" })), rows.slice(0, 1));
  const flow = usageFlow(rows, "calls");
  for (const column of flow.columns) {
    assert.equal(column.find(node => node.key === "")?.value, 1);
    assert.equal(column.find(node => node.key === undefined)?.folded, 2);
    assert.equal(column.reduce((sum, node) => sum + node.value, 0), 9);
  }
});
