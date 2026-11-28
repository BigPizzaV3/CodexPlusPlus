import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { readAppSource } from "./app-source.test-helper.ts";

// Windows 检出（core.autocrlf）下源码是 CRLF，先归一化换行。
const app = readAppSource().replace(/\r\n/g, "\n");
const gatewayProviders = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("Codex「连接与供应商」改用新版 ProviderAdder 目录与表单弹窗，老版全屏 ProviderCatalogScreen 已移除", () => {
  assert.doesNotMatch(app, /function ProviderCatalogScreen|<ProviderCatalogScreen/);
  assert.match(app, /const openCatalog = \(\) => \{ setNewProfileDraft\(null\); setDetailProfileId\(null\); setPoolAdding\(true\); \};/);
  assert.match(app, /<PoolProviderAdder[\s\S]*?directOnly=\{mode === "direct"\}[\s\S]*?onSubmitDraft=\{submitCodexFromAdder\}[\s\S]*?scope="codex"/);
});

test("新版 ProviderAdder 从目录进入表单后点取消会返回目录；独立 Agent 模式下隐藏底部全局 Agent 勾选", () => {
  const adder = gatewayProviders.slice(gatewayProviders.indexOf("export function ProviderAdder("));
  assert.match(adder, /if \(startWith === "catalog"\) setCatalogOpen\(true\);/);
  assert.match(adder, /const agentPicks = effectiveScope \? null : <AgentPicks/);
});
