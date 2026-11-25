import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { readAppSource } from "./app-source.test-helper.ts";

/**
 * 「连接与供应商」页不再分「无需转换 / 模型聚合 / 需协议转换 / 故障转移 / 共享」五个分页：
 * 顶部选连接方式（聚合 / 直连），下面一份供应商列表。这里守住分页别被加回来。
 */
const source = readAppSource();

test("provider page has a connection mode instead of provider view tabs", () => {
  assert.doesNotMatch(source, /as ListView\[\]/);
  assert.doesNotMatch(source, /onChange=\{selectProviderView\}/);
  assert.match(source, /aria-label=\{label\} className="mg-segmented cx-connection-switch" role="radiogroup"/);
  assert.match(source, /<ConnectionModeSwitch[\s\S]*?label=\{t\("Codex 的连接方式"\)\}/);
  // 直连时需协议转换、模型路由、直连聚合的那几家标「需 Codex++」：它们离不开 Codex++ 的本地代理。
  assert.match(source, /!gatewayManaged && relayProviderView\(profile\) !== "direct" \? \(/);
});

test("claude page gives Claude Code the same aggregate / direct switch over the shared pool", () => {
  const claude = readFileSync(new URL("./ClaudeProviders.tsx", import.meta.url), "utf8");
  // 和 Codex 页同一个切换；Claude Desktop 始终经本地服务，只有 Claude Code 有两种。
  assert.match(claude, /<ConnectionModeSwitch[\s\S]*?label=\{t\("Claude Code 的连接方式"\)\}/);
  assert.match(claude, /const codeMode: CodexConnectionMode = claudeCode\?\.enabled \? "aggregate" : "direct";/);
  // 直连某一家走专门的命令，接入着时先看断开预览（带 previewToken）。
  assert.match(claude, /"use_claude_code_pool_provider", \{ id: entry\.id \}/);
  assert.match(claude, /"use_claude_code_pool_provider", \{ id: directTarget\.id, previewToken \}/);
  // 只有 Anthropic 协议、不是订阅账号的能直连，和后端 claude_direct::rejection 一致。
  assert.match(claude, /entry\.protocol === "anthropicMessages" && !entry\.account/);
});
