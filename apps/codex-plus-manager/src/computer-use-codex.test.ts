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

test("非 macOS 平台（Windows、Linux）显示「仅支持 macOS」，不去探测运行时（#2718）", () => {
  assert.match(page, /const computerUseSupported = enhancementPlatformFromUserAgent\(navigator\.userAgent\) === "macos";/);
  assert.match(page, /return computerUseSupported \? <ComputerUseMacPage \/> : <ComputerUseUnsupported \/>;/);
  const unsupported = page.slice(page.indexOf("function ComputerUseUnsupported"), page.indexOf("function ComputerUseMacPage"));
  assert.ok(unsupported.includes("Computer Use 仅支持 macOS"));
  assert.ok(!page.includes("暂不支持 Windows"), "Linux 也会走到这里，不写死 Windows");
  assert.ok(!unsupported.includes("invoke("), "不支持的平台不应调用后端探测");
});

test("「全部清空」审批记录前先确认（会连 Codex 的「始终允许」一起清掉）", () => {
  const clear = page.slice(page.indexOf("const clearApprovals = async"), page.indexOf("const loadStatus"));
  assert.match(clear, /await appConfirm\([\s\S]*danger: true,[\s\S]*\}\);\n\s*if \(!confirmed \|\| !mounted\.current\) return;\n\s*await runExclusive\("approval:clear"/);
});

test("提示文案是整句：不以逗号开头，也不把一句话拆成几段 t() 拼接", () => {
  assert.doesNotMatch(page, /t\("，/);
  assert.doesNotMatch(page, /\{t\("与"\)\}|t\("删除后会先移除写入"\)|\$\{t\("已备份到"\)\}/);
});

test("各操作共用 runExclusive 上锁 / 记 busy / 丢弃卸载后的结果，不再各写一遍", () => {
  assert.equal(page.match(/lock\.current = true;/g)?.length, 1);
  assert.equal(page.match(/setBusy\(""\);/g)?.length, 1);
  for (const name of ["changeApprovalMode", "changeCuMode", "changePip", "forgetApproval", "loadStatus", "openChatGpt", "mutateTarget", "addCustomTarget", "renameCustomTarget", "removeCustomTarget", "runCheck"]) {
    const start = page.indexOf(`const ${name} =`);
    assert.ok(start >= 0, name);
    // 到下一个 const 声明为止就是这个操作的函数体。
    const body = page.slice(start, page.indexOf("\n  const ", start + 10));
    assert.match(body, /runExclusive\(|saveSetting\(/, name);
  }
});
