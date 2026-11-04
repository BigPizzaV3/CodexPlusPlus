import assert from "node:assert/strict";
import test from "node:test";
import { diagnosticFacts, filterLogEntries, formatLogTime, logTone, parseLogLine, parseLogText, summarizeDetail } from "./log-view-model.ts";

test("诊断日志一行 JSON 解析成事件名、时间、摘要和格式化的明细", () => {
  const raw = JSON.stringify({ timestamp_ms: 1793689309389, pid: 17040, event: "manager.save_settings", detail: { activeRelayId: "codex-plus-gateway", relayProfiles: [{ id: "a" }, { id: "b" }], ok: true } });
  const entry = parseLogLine(raw, 6);
  assert.equal(entry.event, "manager.save_settings");
  assert.equal(entry.timestampMs, 1793689309389);
  assert.equal(entry.pid, 17040);
  assert.equal(entry.tone, "info");
  // 简单字段排前面，数组只给数量。
  assert.equal(entry.summary, "activeRelayId=codex-plus-gateway · ok=true · relayProfiles=[2]");
  assert.match(entry.body, /\n {2}"activeRelayId": "codex-plus-gateway"/);
});

test("解析不了的行（截断留下的半行、纯文本）原样显示", () => {
  const entry = parseLogLine('ount":0,"models":[]}', 1);
  assert.equal(entry.event, "");
  assert.equal(entry.summary, 'ount":0,"models":[]}');
  assert.equal(entry.body, entry.summary);
});

test("出错、重试的事件分别标红、标黄", () => {
  assert.equal(logTone("manager.navigation_failed", {}), "error");
  assert.equal(logTone("gateway.request", { error: "upstream 429" }), "error");
  assert.equal(logTone("gateway.request", { status: "failed" }), "error");
  assert.equal(logTone("gateway.retry", {}), "warn");
  assert.equal(logTone("manager.save_settings", { status: "ok" }), "info");
});

test("只读了文件末尾时，第一行被截断的半行丢掉；没截断时原样保留", () => {
  const text = ['ount":0,"models":[]}', JSON.stringify({ timestamp_ms: 1, event: "a.start", detail: {} })].join("\n");
  assert.deepEqual(parseLogText(text, true).map((entry) => entry.event), ["a.start"]);
  assert.equal(parseLogText(text, false).length, 2);
});

test("列表最新的在前，跳过空行；可按文字和「只看出错」筛选", () => {
  const text = [
    JSON.stringify({ timestamp_ms: 1, event: "a.start", detail: {} }),
    "",
    JSON.stringify({ timestamp_ms: 2, event: "b.failed", detail: { message: "boom" } }),
  ].join("\n");
  const entries = parseLogText(text);
  assert.deepEqual(entries.map((entry) => entry.event), ["b.failed", "a.start"]);
  assert.deepEqual(entries.map((entry) => entry.index), [3, 1]);
  assert.deepEqual(filterLogEntries(entries, "", true).map((entry) => entry.event), ["b.failed"]);
  assert.deepEqual(filterLogEntries(entries, "START", false).map((entry) => entry.event), ["a.start"]);
});

test("摘要过长时截断", () => {
  const detail = Object.fromEntries(Array.from({ length: 30 }, (_, index) => [`key${index}`, "value-value-value"]));
  const summary = summarizeDetail(detail);
  assert.ok(summary.length <= 180);
  assert.ok(summary.endsWith("…"));
});

test("时间：当天只显示时分秒，跨天带月日", () => {
  const now = new Date(2026, 10, 3, 18, 0, 0).getTime();
  assert.equal(formatLogTime(new Date(2026, 10, 3, 9, 5, 7).getTime(), now), "09:05:07");
  assert.equal(formatLogTime(new Date(2026, 10, 2, 23, 59, 1).getTime(), now), "11-02 23:59:01");
  assert.equal(formatLogTime(null, now), "");
});

test("诊断报告挑出版本、Codex 应用、快捷方式和最近启动；不是 JSON 时不挑", () => {
  const report = JSON.stringify({
    generatedAtMs: 1,
    version: "2.2.0",
    overview: {
      codex_app: { status: "found", path: "/Applications/ChatGPT.app" },
      codex_version: "26.1027.61615",
      silent_shortcut: { status: "missing", path: "/Applications/Codex++.app" },
      management_shortcut: { status: "missing", path: "/Applications/Codex++.app" },
      latest_launch: { status: "stopped", phase: "user_stopped" },
    },
  });
  const facts = diagnosticFacts(report);
  assert.deepEqual(facts.map((fact) => fact.label), ["Codex++ 版本", "Codex 应用", "Codex 版本", "静默启动入口", "管理入口", "最近一次启动"]);
  // 状态词翻成中文，路径原样。
  assert.equal(facts[1].status, "已找到");
  assert.equal(facts[1].value, "/Applications/ChatGPT.app");
  assert.equal(facts[1].tone, "ok");
  assert.equal(facts[3].status, "未找到");
  assert.equal(facts[3].tone, "warn");
  assert.equal(facts[5].status, "已停止");
  assert.equal(facts[5].value, "手动停止");
  assert.equal(facts[0].status, undefined);
  assert.equal(facts[0].value, "2.2.0");
  assert.deepEqual(diagnosticFacts("尚未生成诊断报告。"), []);
});
