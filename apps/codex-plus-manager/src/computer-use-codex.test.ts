import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const page = readFileSync(new URL("./ComputerUsePage.tsx", import.meta.url), "utf8");

test("Codex 不在接入列表里，页面也不再引导用命令行接入 Codex", () => {
  const targetIds = page.match(/const targetIds: CuTargetId\[\] = \[([^\]]*)\]/);
  assert.ok(targetIds, "targetIds should exist");
  assert.ok(!targetIds[1].includes("\"codex\""));
  assert.ok(!page.includes("install codex"));
});

test("Codex 里有残留条目时提示并只提供移除", () => {
  assert.ok(page.includes("status?.targets?.codex?.registered === true"));
  assert.ok(page.includes("mutateTarget(\"codex\", \"uninstall\")"));
  assert.ok(!page.includes("mutateTarget(\"codex\", \"install\")"));
});

test("非 macOS 平台显示「暂不支持」，不去探测运行时（#2718）", () => {
  assert.match(page, /const computerUseSupported = enhancementPlatformFromUserAgent\(navigator\.userAgent\) === "macos";/);
  assert.match(page, /return computerUseSupported \? <ComputerUseMacPage \/> : <ComputerUseUnsupported \/>;/);
  const unsupported = page.slice(page.indexOf("function ComputerUseUnsupported"), page.indexOf("function ComputerUseMacPage"));
  assert.ok(unsupported.includes("Computer Use 暂不支持 Windows"));
  assert.ok(!unsupported.includes("invoke("), "不支持的平台不应调用后端探测");
});
