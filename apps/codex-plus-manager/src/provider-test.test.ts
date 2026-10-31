import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { formatProbeMs, probeHints, probeKeyHint, probeWireLabel, type ProviderProbeResult } from "./provider-test.ts";

const read = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");

function result(overrides: Partial<ProviderProbeResult>): ProviderProbeResult {
  return {
    ok: false,
    category: "",
    httpStatus: null,
    model: "m",
    wireApi: "chat",
    firstTokenMs: null,
    totalMs: 0,
    reply: "",
    replyTruncated: false,
    error: "",
    keyId: "key-2",
    keyName: "",
    keyTail: "",
    ...overrides,
  };
}

test("供应商测试：耗时与协议的显示", () => {
  assert.equal(formatProbeMs(null), "—");
  assert.equal(formatProbeMs(850.4), "850 ms");
  assert.equal(formatProbeMs(1234), "1.2 s");
  assert.equal(probeWireLabel("chat"), "Chat Completions");
  assert.equal(probeWireLabel("anthropic"), "Anthropic Messages");
  assert.equal(probeWireLabel("responses"), "Responses");
});

test("供应商测试：按失败分类给排查提示，认证失败时说明用的是哪个 Key", () => {
  assert.deepEqual(probeHints(result({ ok: true })), []);
  const auth = probeHints(result({ category: "auth", httpStatus: 401, keyName: "备用", keyTail: "BBBB" }));
  assert.match(auth[0].text, /Key 无效/);
  assert.deepEqual(auth[1], { text: "本次用的是 Key「{0}」（末 4 位 {1}）。", args: ["备用", "BBBB"] });
  assert.deepEqual(probeKeyHint({ keyName: "", keyTail: "AAAA" }), { text: "本次用的是末 4 位为 {0} 的 Key。", args: ["AAAA"] });
  assert.equal(probeKeyHint({ keyName: "", keyTail: "" }), null, "不用 Key 或 Key 太短时不提");
  // 订阅账号：提示重新登录，没有 Key 可说。
  const account = probeHints(result({ category: "auth", keyId: "" }), true);
  assert.equal(account.length, 1);
  assert.match(account[0].text, /重新登录/);
  assert.match(probeHints(result({ category: "not_found" }))[0].text, /模型名不对/);
  assert.match(probeHints(result({ category: "rate_limited" }))[0].text, /限速.*额度/);
  assert.match(probeHints(result({ category: "timeout" }))[0].text, /Base URL 和代理/);
  assert.match(probeHints(result({ category: "connect" }))[0].text, /Base URL 和代理/);
  assert.match(probeHints(result({ category: "whatever" }))[0].text, /错误原文/);
});

test("供应商测试：详情底栏有测试按钮，Key 列表每个 Key 可单独测，按标识调用、可取消", () => {
  const providers = read("./GatewayProviders.tsx");
  assert.match(providers, /onClick=\{\(\) => setTestKey\(null\)\}[\s\S]{0,200}<Zap className="h-3\.5 w-3\.5" \/>\s*\{t\("测试"\)\}/);
  assert.match(providers, /<ProviderTestDialog entry=\{entry\} keyId=\{testKey\} onClose=\{\(\) => setTestKey\(undefined\)\} \/>/);
  const keys = read("./ProviderKeyList.tsx");
  assert.match(keys, /aria-label=\{tf\("测试 Key \{0\}", \[key\.masked\]\)\}/);
  assert.match(keys, /onClick=\{\(\) => onTest\(key\.id\)\}/, "按标识测，不碰 Key 原文");
  const dialog = read("./ProviderTestDialog.tsx");
  assert.match(dialog, /invoke<ProbeResponse>\("test_gateway_provider", \{ id: entry\.id, model, keyId: key \|\| null \}\)/);
  assert.match(dialog, /invoke\("cancel_gateway_provider_test"\)/);
  assert.match(dialog, /if \(current !== seq\.current\) return;/, "取消或关掉后回来的旧结果不显示");
  // 命令已注册。
  const lib = read("../src-tauri/src/lib.rs");
  assert.match(lib, /gateway_commands::test_gateway_provider,/);
  assert.match(lib, /gateway_commands::cancel_gateway_provider_test,/);
});

test("供应商测试：新样式带前缀，正式版样式顺序下不被通用规则盖掉", () => {
  const form = read("./ProviderFormDialog.css");
  for (const selector of ["pt-result", "pt-status", "pt-metrics", "pt-text", "pt-hints"]) {
    assert.match(form, new RegExp(`\\.gw-portal \\.${selector} \\{`), selector);
    assert.doesNotMatch(form, new RegExp(`^\\.${selector}\\b`, "m"), `${selector} 要带 .gw-portal 前缀`);
  }
  const page = read("./GatewayPage.css");
  assert.match(page, /\.shell \.gw-detail-foot \.gw-detail-foot-actions \{/);
});

test("供应商测试：新文案都有英文", () => {
  const dictionary = read("./i18n-en.ts");
  const sources = [read("./ProviderTestDialog.tsx"), read("./ProviderKeyList.tsx"), read("./GatewayProviders.tsx"), read("./provider-test.ts")].join("\n");
  const keys = new Set<string>();
  for (const match of sources.matchAll(/\b(?:t|tf|hint)\("((?:[^"\\]|\\.)+)"/g)) keys.add(match[1]);
  const missing = [...keys].filter((key) => /[一-龥]/.test(key) && !dictionary.includes(`"${key}":`));
  assert.deepEqual(missing, []);
  for (const message of ["测试成功。", "测试失败。", "测试已取消。", "没有进行中的测试。"]) {
    assert.ok(dictionary.includes(`"${message}":`), message);
  }
  assert.ok(dictionary.includes("[/^测试失败：(.+)$/,"));
});
