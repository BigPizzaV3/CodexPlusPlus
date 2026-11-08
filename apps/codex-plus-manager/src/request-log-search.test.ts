import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8").replaceAll("\r\n", "\n");
const start = "  const filtered = useMemo(() => {\n";
const end = "\n  }, [entries, filter, query, sessionFilter, agentScope, payload]);";
const begin = app.indexOf(start);
const finish = app.indexOf(end, begin);
assert.ok(begin >= 0 && finish > begin, "找不到请求列表筛选逻辑");

// 执行页面实际的 useMemo 回调，覆盖搜索与其他本地筛选的组合。
const select = new Function(
  "entries", "query", "filter", "sessionFilter", "agentScope", "requestLogAgent", "groupDisplayName", "payload",
  app.slice(begin + start.length, finish),
);

const base = {
  state: "success",
  success: true,
  autoRetries: 0,
  attempts: 1,
  agent: "codex",
  threadId: "thread-a",
  model: "example-model",
  upstreamModel: "upstream-model",
  relayName: "provider-a",
  path: "/v1/responses",
  error: "",
  wireApi: "responses",
};
const rows = [
  { ...base, id: "429", statusCode: 429, success: false, error: "rate limited" },
  { ...base, id: "503", statusCode: 503, success: false, error: "unavailable", agent: "claude", threadId: "thread-b" },
  { ...base, id: "retried", statusCode: 200, attempts: 2, autoRetries: 1, attemptDetails: [{ code: "429" }] },
];

function ids(query: string, filter = "all", session = "all", agent = "all"): string[] {
  return select(rows, query, filter, session, agent, (entry: typeof base) => entry.agent || "codex", () => null, null)
    .map((entry: { id: string }) => entry.id);
}

test("可按最终 HTTP 状态码搜索，不把重试明细当作最终状态", () => {
  assert.deepEqual(ids("429"), ["429"]);
  assert.deepEqual(ids("503"), ["503"]);
  assert.deepEqual(ids("200"), ["retried"]);
  assert.deepEqual(ids(" 429 "), ["429"]);
});

test("现有文本搜索与状态、会话和 Agent 筛选仍然组合生效", () => {
  assert.deepEqual(ids("  "), ["429", "503", "retried"]);
  assert.deepEqual(ids("EXAMPLE"), ["429", "503", "retried"]);
  assert.deepEqual(ids("provider-a"), ["429", "503", "retried"]);
  assert.deepEqual(ids("/v1/responses"), ["429", "503", "retried"]);
  assert.deepEqual(ids("unavailable"), ["503"]);
  assert.deepEqual(ids("503", "failed", "thread-b", "claude"), ["503"]);
  assert.deepEqual(ids("503", "success", "thread-b", "claude"), []);
  assert.deepEqual(ids("503", "failed", "thread-a", "claude"), []);
  assert.deepEqual(ids("503", "failed", "thread-b", "codex"), []);
});

test("原有错误文本仍可匹配状态数字", () => {
  const loaded = [{ ...base, id: "error-text", statusCode: 200, error: "upstream 429" }];
  const selected = select(loaded, "429", "all", "all", "all", (entry: typeof base) => entry.agent, () => null, null);
  assert.deepEqual(selected.map((entry: { id: string }) => entry.id), ["error-text"]);
});

test("搜索提示在中英文中均包含状态码", () => {
  const placeholder = "搜索模型 / 供应商 / 路径 / 错误 / 状态码";
  const translations = readFileSync(new URL("./i18n-en.ts", import.meta.url), "utf8");
  assert.ok(app.includes(`placeholder={t("${placeholder}")}`));
  assert.ok(translations.includes(`"${placeholder}": "Search model / provider / path / error / status code"`));
});
