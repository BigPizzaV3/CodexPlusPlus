import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");

/** 从 `start` 起到下一个同缩进的函数结尾。 */
function body(source: string, start: string, end: string) {
  const from = source.indexOf(start);
  assert.ok(from >= 0, start);
  const to = source.indexOf(end, from);
  assert.ok(to > from, `${start} 没有结尾`);
  return source.slice(from, to);
}

// 安装确认框读的是 Rust 的 manifest_permissions：那里报出的每一项，两处确认框都要展示。
// 漏掉一项，只声明它的包就会显示成「没有声明任何能力」（launch、mcpServers 曾经就是这样）。
const rust = body(read("../../../crates/codex-plus-core/src/extension_discovery.rs"), "pub fn manifest_permissions(", "\n}\n");
const keys = [...rust.slice(rust.indexOf("json!({")).matchAll(/^\s+"(\w+)":/gm)].map((match) => match[1]);

test("权限摘要报出的每一项都在类型里，管理器与 Codex 页内的安装确认框都会展示", () => {
  assert.ok(keys.length >= 20, `只解析出 ${keys.length} 项，解析方式可能失效了`);
  for (const key of ["launchBefore", "launchAfter", "mcpServers", "preload", "toolPipeline", "gatewayReplaces", "providers", "replaces"]) {
    assert.ok(keys.includes(key), `权限摘要缺少 ${key}`);
  }
  const panel = read("./CommunityExtensionsPanel.tsx");
  const type = body(panel, "export type Permissions = {", "\n};\n");
  const manager = body(panel, "export function permissionLines(", "\n}\n");
  const codex = body(read("../../../assets/inject/renderer-inject/40-backend-settings.js"), "function codexPlusPackagePermissionLines(", "\n  }\n");
  for (const key of keys) {
    assert.match(type, new RegExp(`\\b${key}\\??:`), `Permissions 类型缺少 ${key}`);
    assert.match(manager, new RegExp(`permissions\\.${key}\\b`), `管理器确认框没有展示 ${key}`);
    assert.match(codex, new RegExp(`permissions\\.${key}\\b`), `Codex 页内确认框没有展示 ${key}`);
  }
});

test("两处确认框是同一套文案", () => {
  const manager = body(read("./CommunityExtensionsPanel.tsx"), "export function permissionLines(", "\n}\n");
  const codex = body(read("../../../assets/inject/renderer-inject/40-backend-settings.js"), "function codexPlusPackagePermissionLines(", "\n  }\n");
  // 取每句文案冒号后的说明部分比较：占位写法两边不同（{0} 与 ${…}），说明文字应当一字不差。
  // 只看带中文的片段，代码里的冒号（三元、类型标注）不算。
  const explanations = (source: string) => new Set(
    [...source.matchAll(/[：:]([^"`：:{}$]{6,})["`]/g)].map((match) => match[1]).filter((text) => /[\u4e00-\u9fff]/.test(text)),
  );
  const inManager = explanations(manager);
  const inCodex = explanations(codex);
  assert.ok(inManager.size >= 10, `只解析出 ${inManager.size} 句`);
  assert.deepEqual([...inManager].sort(), [...inCodex].sort());
});

test("只带启动脚本或 MCP 的包在社区列表里也有风险徽标", () => {
  const panel = read("./CommunityExtensionsPanel.tsx");
  const manager = body(panel, "export function riskBadges(", "\n}\n");
  const codex = body(read("../../../assets/inject/renderer-inject/41-extension-sources.js"), "function codexPlusRiskBadges(", "\n  }\n");
  for (const key of ["backend", "dsh", "launchBefore", "launchAfter", "mcpServers", "hooks", "toolPipeline", "gatewayRequest", "gatewayResponse", "gatewayError", "gatewayReplaces", "providers", "tools"]) {
    assert.match(manager, new RegExp(`permissions\\.${key}\\b`), `管理器徽标没有看 ${key}`);
    assert.match(codex, new RegExp(`permissions\\.${key}\\b`), `Codex 页内徽标没有看 ${key}`);
  }
});
