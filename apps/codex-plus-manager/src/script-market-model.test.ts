import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { defaultMarketSort, localScriptState, parseMarketTime, relativeAge, sortMarketScripts } from "./script-market-model.ts";

const script = (name: string, updatedAt = "") => ({ name, updatedAt });

test("market time parsing treats empty and invalid values as missing", () => {
  assert.equal(parseMarketTime(""), null);
  assert.equal(parseMarketTime(undefined), null);
  assert.equal(parseMarketTime("not a date"), null);
  assert.equal(parseMarketTime(" 2026-10-20 "), Date.parse("2026-10-20"));
  assert.equal(parseMarketTime("2026-10-20T08:00:00Z"), Date.parse("2026-10-20T08:00:00Z"));
});

test("market time parsing accepts space-separated date times that WKWebView rejects", () => {
  // 带空格的写法换成 T 再解析：不带时区按本地时间，与 ISO 本地时间写法一致。
  assert.equal(parseMarketTime("2026-10-29 12:00"), Date.parse("2026-10-29T12:00"));
  assert.equal(parseMarketTime("2026-10-29 12:00:30"), Date.parse("2026-10-29T12:00:30"));
  assert.equal(parseMarketTime("2026-10-29  9:05"), Date.parse("2026-10-29T09:05"));
  assert.equal(parseMarketTime("2026-10-29 12:00:00Z"), Date.parse("2026-10-29T12:00:00Z"));
  assert.equal(parseMarketTime("2026-10-29 12:00:00 +08:00"), Date.parse("2026-10-29T04:00:00Z"));
  assert.equal(parseMarketTime("2026-10-29 12:00 UTC"), Date.parse("2026-10-29T12:00Z"));
  assert.equal(parseMarketTime("2026-10-29 25:99"), null);
});

test("market time parsing accepts unix seconds and milliseconds", () => {
  const time = Date.parse("2026-10-29T12:00:00Z");
  assert.equal(parseMarketTime(String(time / 1000)), time);
  assert.equal(parseMarketTime(String(time)), time);
  assert.equal(parseMarketTime(time / 1000), time);
  assert.equal(parseMarketTime(time), time);
  // 太小的数字不当成时间戳。
  assert.equal(parseMarketTime(0), null);
  assert.equal(parseMarketTime("12345"), null);
});

test("default sort switches to recently updated only when every script has a date", () => {
  assert.equal(defaultMarketSort([]), "default");
  assert.equal(defaultMarketSort([script("a", "2026-10-01"), script("b")]), "default");
  assert.equal(defaultMarketSort([script("a", "2026-10-01"), script("b", "bad")]), "default");
  assert.equal(defaultMarketSort([script("a", "2026-10-01"), script("b", "2026-09-01")]), "updated");
});

test("sorting by update puts newest first, keeps ties in index order and undated last", () => {
  const entries = [
    script("old", "2026-01-01"),
    script("undated-a"),
    script("new", "2026-10-20"),
    script("tie-1", "2026-05-01"),
    script("undated-b", "garbage"),
    script("tie-2", "2026-05-01"),
  ];
  const sorted = sortMarketScripts(entries, "updated").map((item) => item.name);
  assert.deepEqual(sorted, ["new", "tie-1", "tie-2", "old", "undated-a", "undated-b"]);
  // 不改入参，默认顺序原样返回
  assert.equal(entries[0].name, "old");
  assert.deepEqual(sortMarketScripts(entries, "default").map((item) => item.name), entries.map((item) => item.name));
});

test("sorting by name is case-insensitive and numeric-aware", () => {
  const sorted = sortMarketScripts([script("b"), script("Item 10"), script("A"), script("item 2")], "name").map((item) => item.name);
  assert.deepEqual(sorted, ["A", "b", "item 2", "Item 10"]);
});

test("relative age buckets updates into hours, days, months and years", () => {
  const now = Date.parse("2026-10-29T12:00:00Z");
  const hour = 60 * 60 * 1000;
  const day = 24 * hour;
  assert.deepEqual(relativeAge(now + day, now), { unit: "justNow" });
  assert.deepEqual(relativeAge(now - 10 * 60 * 1000, now), { unit: "justNow" });
  assert.deepEqual(relativeAge(now - 5 * hour, now), { unit: "hours", value: 5 });
  assert.deepEqual(relativeAge(now - 3 * day, now), { unit: "days", value: 3 });
  assert.deepEqual(relativeAge(now - 75 * day, now), { unit: "months", value: 2 });
  assert.deepEqual(relativeAge(now - 800 * day, now), { unit: "years", value: 2 });
});

test("local script state merges the user switch with the runtime load status", () => {
  assert.equal(localScriptState({ enabled: false, status: "disabled" }, true), "off");
  assert.equal(localScriptState({ enabled: false, status: "loaded" }, true), "off");
  assert.equal(localScriptState({ enabled: true, status: "disabled" }, false), "globalOff");
  assert.equal(localScriptState({ enabled: true, status: "loaded" }, true), "loaded");
  assert.equal(localScriptState({ enabled: true, status: "loading" }, true), "loading");
  assert.equal(localScriptState({ enabled: true, status: "failed" }, true), "failed");
  assert.equal(localScriptState({ enabled: true, status: "not_loaded" }, true), "notLoaded");
  assert.equal(localScriptState({ enabled: true, status: "" }, true), "notLoaded");
});

test("local script rows no longer render the switch label and raw status as two columns", () => {
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  const row = app.slice(app.indexOf("function ScriptRow("), app.indexOf("function AdGrid("));
  assert.ok(row.includes("localScriptState("), "ScriptRow should derive one merged state");
  assert.ok(!row.includes("{script.status}"), "raw loaded/disabled column should be gone");
  assert.ok(!row.includes('t("启用") : t("关闭")'), "separate enabled/disabled text column should be gone");
});
