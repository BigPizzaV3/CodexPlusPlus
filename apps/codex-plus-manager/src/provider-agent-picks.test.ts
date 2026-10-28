import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { defaultProviderAgents, draftFromBinding, bindingFromDraft, skippedAgents, type GatewayPayload } from "./gateway-model.ts";

const payload = {
  agents: [{ agent: "codex" }, { agent: "claude" }, { agent: "grok" }],
  gateway: { bindings: { claude: { autoJoinProviders: true }, grok: { autoJoinProviders: false } } },
} as unknown as GatewayPayload;

test("添加供应商默认勾选没关掉「自动加入」的 Agent，没勾选的作为 skipAgents 传给后端", () => {
  assert.deepEqual(defaultProviderAgents(payload), ["codex", "claude"]);
  assert.deepEqual(skippedAgents(payload, ["claude"]), ["codex", "grok"]);
  assert.deepEqual(skippedAgents(payload, ["codex", "claude", "grok"]), []);
});

test("「添加供应商时自动加入」旧数据视为开，表单来回保持", () => {
  assert.equal(draftFromBinding(undefined).autoJoinProviders, true);
  const draft = { ...draftFromBinding(undefined), autoJoinProviders: false };
  assert.equal(bindingFromDraft(draft, undefined).autoJoinProviders, false);
});

test("各条添加路径都带上 skipAgents", () => {
  const source = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  for (const command of ["create_gateway_provider", "import_gateway_providers", "finish_gateway_copilot_login"]) {
    const line = source.split("\n").find((item) => item.includes(`"${command}"`)) ?? "";
    assert.match(line, /skipAgents/, command);
  }
  assert.match(source, /BROWSER_LOGIN_COMMANDS\[kind\], skip \? \{ skipAgents: skip \}/);
});
