import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { defaultPoolDialogKind, poolProviderTabs } from "./pool-provider-tabs.ts";

const read = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");

test("统一配置弹窗标签页按供应商类型和范围精简排列", () => {
  assert.deepEqual(
    poolProviderTabs({ models: ["gpt-5"] }).map((tab) => tab.kind),
    ["edit", "models", "windows", "reasoning", "price", "proxy", "limits"],
  );
  assert.deepEqual(
    poolProviderTabs({ models: [] }).map((tab) => tab.kind),
    ["edit", "models", "price", "proxy", "limits"],
  );
  // ChatGPT 官方订阅把代理和限速排在价格前面，且保留常规页。
  assert.deepEqual(
    poolProviderTabs({ account: "chatgpt", models: ["gpt-5"] }).map((tab) => tab.kind),
    ["edit", "models", "windows", "reasoning", "proxy", "limits", "price"],
  );
  assert.equal(defaultPoolDialogKind({ account: "chatgpt", models: ["gpt-5"] }), "edit");
  // 非 ChatGPT 官方订阅没有常规连接表单，默认落在模型页。
  assert.deepEqual(
    poolProviderTabs({ account: "copilot", models: ["gpt-5"] }).map((tab) => tab.kind),
    ["models", "windows", "reasoning", "proxy", "limits", "price"],
  );
  assert.equal(defaultPoolDialogKind({ account: "copilot", models: ["gpt-5"] }), "models");
  // 直连引用供应商只开放网关层配置（价格 / 代理 / 限速）。
  assert.deepEqual(
    poolProviderTabs({ linkedCodexRelay: "relay-1", models: ["gpt-5"] }).map((tab) => tab.kind),
    ["price", "proxy", "limits"],
  );
  assert.equal(defaultPoolDialogKind({ linkedCodexRelay: "relay-1", models: ["gpt-5"] }), "price");
});

test("Codex 与 Claude 聚合模式列表：停用供应商的开关可直接点开，三点菜单收拢为分组短菜单", () => {
  for (const file of ["./CodexConnectionMode.tsx", "./ClaudeProviders.tsx"]) {
    const source = read(file);
    // 开关不再因为全局停用而 disabled，点开时先全局启用并按需移出排除列表。
    assert.doesNotMatch(source, /<Switch\b(?:(?!\/>)[\s\S])*?disabled=\{!enabled\}/, `${file} 的开关不能在停用时灰死`);
    assert.match(source, /if \(!enabled\) \{[\s\S]*?"set_gateway_provider_enabled", \{ id: entry\.id, enabled: true \}/);
    // 菜单收拢进带标签栏的统一弹窗，不再平铺管理模型 / 上下文窗口 / 思考等级 / 编辑订阅额度策略。
    assert.doesNotMatch(source, /t\("编辑订阅额度策略"\)|t\("管理模型"\)|t\("上下文窗口"\)|t\("思考等级"\)/);
    assert.match(source, /<MenuSeparator \/>/);
    assert.match(source, /t\("请先启用供应商再测试"\)/);
  }
});

test("统一弹窗切换标签前走 useModalShell 的未保存确认，且切标签时不重播入场动画", () => {
  const host = read("./CodexPoolEditHost.tsx");
  assert.match(host, /role="tablist"/);
  assert.match(host, /runIfClean\(\(\) => \{/);
  assert.match(host, /noEnterAnimation=\{tabSwitched\}/);
  assert.match(host, /key=\{`\$\{entry\.id\}:\$\{dialog\.kind\}`\}/);
  for (const dialog of [
    "./ProviderFormDialog.tsx",
    "./ProviderModelsDialog.tsx",
    "./ProviderWindowsDialog.tsx",
    "./ProviderReasoningDialog.tsx",
    "./ProviderExtrasDialog.tsx",
  ]) {
    const source = read(dialog);
    assert.match(source, /renderTabs\?\.\(\(action\) => void confirmAction\(action\)\)/, `${dialog} 应渲染顶部标签栏`);
    assert.match(source, /data-no-enter=\{noEnterAnimation \? "true" : undefined\}/, `${dialog} 应支持关闭切页动画`);
  }
});
