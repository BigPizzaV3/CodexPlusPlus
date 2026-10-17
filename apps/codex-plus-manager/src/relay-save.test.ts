import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import ts from "typescript";
import { relaySettingsSaveRequiresRestart, relaySettingsSaveRestartReason, relaySettingsSaveCapabilityProfileNames, type RelaySaveChoice, type RelaySaveSettings } from "./relay-save.ts";

function fixture(aggregate = true): RelaySaveSettings {
  return {
    relayProfilesEnabled: true, enhancementsEnabled: true, activeRelayId: "active",
    relayProfiles: [{
      id: "active", name: "活动", baseUrl: "https://a.example/v1", apiKey: "fixture-a",
      relayMode: aggregate ? "aggregate" : "pureApi", protocol: "responses", officialMixApiKey: false,
      configContents: aggregate ? "" : 'model_provider = "custom"\n[model_providers.custom]\nbase_url = "https://a.example/v1"\nexperimental_bearer_token = "fixture-a"\n',
      authContents: aggregate ? "" : '{"OPENAI_API_KEY":"fixture-a"}',
    }, ...["a", "b"].map((id) => ({
      id, name: id, baseUrl: `https://${id}.example/v1`, apiKey: `fixture-${id}`,
      relayMode: "pureApi" as const, protocol: "responses" as const, officialMixApiKey: false,
    }))],
    aggregateRelayProfiles: aggregate ? [{
      id: "active", modelAggregationEnabled: true, members: [{ relayId: "a", weight: 1 }],
      defaultRelayId: "a", sessionProvider: "custom",
    }] : [],
  };
}

function check(before: RelaySaveSettings, after: RelaySaveSettings, running = true, live = "http://127.0.0.1:62021/v1"): boolean {
  return relaySettingsSaveRequiresRestart(before, after, live, "http://127.0.0.1:62021", running);
}

test("model aggregation member and default updates reuse the existing proxy", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 2 });
  after.aggregateRelayProfiles![0].defaultRelayId = "b";
  assert.equal(check(before, after), false);
  assert.equal(check(before, after, false), true);
  assert.equal(check(before, after, true, "https://vendor.example/v1"), true);
});

test("member credentials and lists are hot, member native capabilities are not silently applied", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.relayProfiles[1].apiKey = "fixture-new";
  after.relayProfiles[1].baseUrl = "https://new.example/v1";
  assert.equal(check(before, after), false);
  after.relayProfiles[1].modelWindows = '{"model-a":"100000"}';
  assert.equal(check(before, after), true);
});

test("adding a member with native model capabilities explains why list refresh is insufficient", () => {
  for (const capability of [
    { modelWindows: '{"kimi-k3":"1048576"}', modelAutoCompact: '{"kimi-k3":"90%"}' },
    { contextWindow: "1048576", autoCompactLimit: "90%" },
    { modelMetadata: '{"kimi-k3":{"context_window":1048576}}' },
  ]) {
    const before = fixture();
    Object.assign(before.relayProfiles[2], capability);
    const after = structuredClone(before);
    after.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 1 });
    assert.equal(check(before, after), true);
    assert.deepEqual(relaySettingsSaveCapabilityProfileNames(before, after), ["b"]);
    assert.equal(relaySettingsSaveRestartReason(before, after, "http://127.0.0.1:62021/v1", "http://127.0.0.1:62021", true), "modelCapabilities");
    assert.equal(check(before, structuredClone(before)), false, "inactive member capabilities alone are not a change");
  }
});

test("restart reasons distinguish proxy connectivity and native configuration from capabilities", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 1 });
  const reason = (running: boolean) => relaySettingsSaveRestartReason(before, after, "http://127.0.0.1:62021/v1", "http://127.0.0.1:62021", running);
  assert.equal(reason(true), null);
  assert.equal(reason(false), "proxyUnavailable");
  after.aggregateRelayProfiles![0].sessionProvider = "openai";
  assert.equal(reason(true), "clientConfig");
});

test("capability changes cannot mask simultaneous native configuration or proxy connection changes", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.relayProfiles[2].modelWindows = '{"kimi-k3":"1048576"}';
  after.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 1 });
  const reason = (running = true) => relaySettingsSaveRestartReason(before, after, "http://127.0.0.1:62021/v1", "http://127.0.0.1:62021", running);
  assert.equal(reason(), "modelCapabilities");
  assert.equal(reason(false), "proxyUnavailable");
  after.relayProfiles[0].configContents = 'model_reasoning_effort = "high"\n';
  assert.equal(reason(), "clientConfig");
});

test("inactive edits and metadata saves never require a healthy helper", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.relayProfiles[2].configContents = "arbitrary inactive configuration";
  after.relayProfiles[0].name = "新名称";
  assert.equal(check(before, after, false, ""), false);
  assert.equal(check(before, structuredClone(before), false, ""), false);
});

test("editor normalization of empty capability maps does not request a restart", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 1 });
  for (const profile of after.relayProfiles) {
    profile.modelVlm = "{}";
    profile.modelWindows = "{}";
    profile.modelAutoCompact = "{}";
    profile.modelMetadata = "{}";
  }
  assert.equal(check(before, after), false);
  after.relayProfiles[1].modelVlm = '{"m":"strip"}';
  assert.equal(check(before, after), true);
});

test("direct model-list saves tolerate normalization of empty route and header arrays", () => {
  const before = fixture(false);
  const after = structuredClone(before);
  after.relayProfiles[0].modelRoutes = [];
  Object.assign(after.relayProfiles[0], { customHeaders: [], modelList: "new-model" });
  assert.equal(check(before, after, false, ""), false);
  after.relayProfiles[0].modelRoutes = [{ model: "m", targetRelayId: "a", targetModel: "n" }];
  assert.equal(check(before, after, false, ""), true);
});

test("proxy endpoint and key changes leave client auth and arbitrary config changes guarded", () => {
  const before = fixture(false);
  const after = structuredClone(before);
  after.relayProfiles[0].baseUrl = "https://new.example/v1";
  after.relayProfiles[0].apiKey = "fixture-new";
  after.relayProfiles[0].configContents = before.relayProfiles[0].configContents!.replaceAll("https://a.example/v1", "https://new.example/v1").replaceAll("fixture-a", "fixture-new");
  after.relayProfiles[0].authContents = '{"OPENAI_API_KEY":"fixture-new"}';
  assert.equal(check(before, after), false);
  assert.equal(check(before, after, true, "https://a.example/v1"), true);
  after.relayProfiles[0].authContents = '{"OPENAI_API_KEY":"fixture-new","tokens":{"account_id":"other"}}';
  assert.equal(check(before, after), true);
  after.relayProfiles[0].authContents = before.relayProfiles[0].authContents;
  after.relayProfiles[0].configContents += '\n[mcp_servers.demo]\nurl = "http://localhost:9999"\n';
  assert.equal(check(before, after), true);
});

test("removing last route can retain a healthy proxy without rewriting live transport", () => {
  const before = fixture(false);
  before.enhancementsEnabled = false;
  before.relayProfiles[0].modelRoutes = [{ model: "m", targetRelayId: "a", targetModel: "n" }];
  const after = structuredClone(before);
  after.relayProfiles[0].modelRoutes = [];
  assert.equal(check(before, after), false);
});

test("default API route changes are expected, login sync and session identity changes are guarded", () => {
  const before = fixture();
  const after = structuredClone(before);
  after.aggregateRelayProfiles![0].defaultRelayId = "b";
  assert.equal(check(before, after), false);
  before.aggregateRelayProfiles![0].syncDefaultMemberLogin = true;
  after.aggregateRelayProfiles![0].syncDefaultMemberLogin = true;
  assert.equal(check(before, after), true);
  const identity = structuredClone(before);
  identity.aggregateRelayProfiles![0].sessionProvider = "openai";
  assert.equal(check(before, identity), true);
});

test("saving asks before persistence and live saves avoid replaying the full provider switch", async () => {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const start = source.indexOf("const restartReason = clientRunning ? relaySettingsSaveRestartReason(");
  const end = source.indexOf("const switchDraft =", start);
  const save = source.slice(start, end);
  assert.ok(save.includes("await actions.confirmRelaySaveRestart("));
  assert.ok(save.indexOf("await actions.confirmRelaySaveRestart(") < save.indexOf("await onFormChange(next)"));
  assert.match(save, /relaySettingsSaveCapabilityProfileNames\(normalizeSettings\(form\), next\)/);
  assert.match(save, /if \(!clientRunning && isActive && savedSettings\.relayProfilesEnabled\)/);
  assert.match(source, /confirmText: canDeferRestart \? t\("保存，暂不重启"\) : t\("继续保存并重启"\)/);
  assert.match(source, /cancelText: t\("取消"\)/);
  assert.match(source, /可能需要重启才能正确加载模型参数/);
  assert.match(source, /localProxyEndpointMatches\(rootTomlStringValue\(contents, "openai_base_url"\), runtimeProxyAddress\)/);
});

type SaveDialog = {
  title: string; message: string; confirmText: string; alternateText?: string;
  resolve: (confirmed: boolean) => void; resolveAlternate?: () => void;
};

async function promptRuntime() {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const start = source.indexOf("  const confirmRelaySaveRestart = ");
  const end = source.indexOf("\n  const setDreamSkinDraftSelection", start);
  assert.ok(start >= 0 && end > start);
  const compiled = ts.transpileModule(source.slice(start, end), {
    compilerOptions: { target: ts.ScriptTarget.ESNext, module: ts.ModuleKind.ESNext },
  }).outputText;
  let dialog!: SaveDialog;
  const prompt = new Function("setConfirmDialog", "t", "tf", `${compiled}\nreturn confirmRelaySaveRestart;`)(
    (value: SaveDialog) => { dialog = value; },
    (value: string) => value,
    (value: string, args: string[]) => value.replace("{0}", args[0]),
  ) as (reason: string, names?: string[]) => Promise<RelaySaveChoice>;
  return { prompt, dialog: () => dialog };
}

test("model capabilities offer save, restart and cancel; native/login changes retain two choices", async () => {
  const r = await promptRuntime();
  for (const choice of ["save", "restart", "cancel"] as const) {
    const result = r.prompt("modelCapabilities", ["示例供应商"]);
    const dialog = r.dialog();
    assert.equal(dialog.confirmText, "保存，暂不重启");
    assert.equal(dialog.alternateText, "继续保存并重启");
    assert.ok(dialog.message.includes("示例供应商") && dialog.message.includes("部分参数可能暂未生效"));
    if (choice === "restart") dialog.resolveAlternate!();
    else dialog.resolve(choice === "save");
    assert.equal(await result, choice);
  }
  for (const reason of ["clientConfig", "proxyUnavailable"]) {
    const result = r.prompt(reason);
    assert.equal(r.dialog().alternateText, undefined);
    assert.equal(r.dialog().resolveAlternate, undefined);
    r.dialog().resolve(true);
    assert.equal(await result, "restart");
  }
});

async function runSaveBranch(choice: RelaySaveChoice, options: {
  stopped?: boolean; persistenceFailed?: boolean; restartFailed?: boolean; unchanged?: boolean;
} = {}) {
  const source = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
  const start = source.indexOf("const restartReason = clientRunning ? relaySettingsSaveRestartReason(");
  const end = source.indexOf("    } finally {", start);
  assert.ok(start >= 0 && end > start);
  const execute = new Function("env", `return async function () {
    const {clientRunning,normalizeSettings,form,next,activeLiveBaseUrl,runtimeProxyAddress,helperRunning,
      relaySettingsSaveRestartReason,relaySettingsSaveCapabilityProfileNames,actions,onFormChange,isActive,onSaved} = env;
    ${source.slice(start, end)}
  };`);
  const form = fixture(), next = structuredClone(form);
  if (!options.unchanged) {
    next.relayProfiles[2].modelWindows = '{"kimi-k3":"1048576"}';
    next.aggregateRelayProfiles![0].members.push({ relayId: "b", weight: 1 });
  }
  const events: string[] = [];
  const prompt = await promptRuntime();
  const actions = {
    confirmRelaySaveRestart: (reason: string, names: string[]) => {
      events.push("confirm");
      const result = prompt.prompt(reason, names);
      if (choice === "restart") prompt.dialog().resolveAlternate!();
      else prompt.dialog().resolve(choice === "save");
      return result;
    },
    restart: async (silent: boolean) => { assert.equal(silent, true); events.push("restart"); return !options.restartFailed; },
    switchRelayProfile: async () => { events.push("switch"); },
  };
  await execute({
    clientRunning: !options.stopped, normalizeSettings: (value: unknown) => value, form, next,
    activeLiveBaseUrl: "http://127.0.0.1:62021/v1", runtimeProxyAddress: "http://127.0.0.1:62021", helperRunning: true,
    relaySettingsSaveRestartReason, relaySettingsSaveCapabilityProfileNames, actions, isActive: true,
    onFormChange: async (value: unknown) => { assert.equal(value, next); events.push("save"); return options.persistenceFailed ? null : next; },
    onSaved: () => { events.push("saved"); },
  })();
  return events;
}

test("choosing save without restart persists the aggregate and returns without restart or provider switch", async () => {
  assert.deepEqual(await runSaveBranch("save"), ["confirm", "save", "saved"]);
});

test("restart happens only after persistence, while cancellation and failed saves do not restart", async () => {
  assert.deepEqual(await runSaveBranch("restart"), ["confirm", "save", "restart", "saved"]);
  assert.deepEqual(await runSaveBranch("cancel"), ["confirm"]);
  assert.deepEqual(await runSaveBranch("restart", { persistenceFailed: true }), ["confirm", "save"]);
  assert.deepEqual(await runSaveBranch("save", { persistenceFailed: true }), ["confirm", "save"]);
  assert.deepEqual(await runSaveBranch("restart", { restartFailed: true }), ["confirm", "save", "restart"]);
});

test("an unchanged live save stays silent and a stopped client still receives full startup configuration", async () => {
  assert.deepEqual(await runSaveBranch("save", { unchanged: true }), ["save", "saved"]);
  assert.deepEqual(await runSaveBranch("save", { stopped: true }), ["save", "switch", "saved"]);
});
