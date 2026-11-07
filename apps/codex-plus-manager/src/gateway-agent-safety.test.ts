import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { agentDiagnosticPresentation, canReconnectAgent, diagnosticErrorLabel, diagnosticEvidenceLabel, disconnectFileLabel, disconnectPreviewReady, fullSwitchReady } from "./gateway-agent-safety.ts";
import { disabledGroup } from "./gateway-model.ts";
import type { GatewayAgentDiagnostics, GatewayAgentView, GatewayDisconnectPreview } from "./gateway-model.ts";

const agent = (state?: GatewayAgentDiagnostics["state"]): GatewayAgentView => ({
  agent: "opencode", name: "OpenCode", enabled: true, baseUrl: "http://127.0.0.1:57321/gw/opencode",
  defaultModel: "gpt", models: ["gpt"],
  ...(state ? { diagnostics: {
    state, endpoint: null, expectedEndpoint: "", configEvidence: [], requests: 0, errors: 0, inFlight: 0,
    lastSeenMs: null, recentError: null,
  } } : {}),
});
const preview = (token = "snapshot-1"): GatewayDisconnectPreview => ({
  agent: "opencode", previewToken: token, canDisconnect: true, warnings: ["保留接入后修改的字段"],
  files: [{ path: "/test/opencode.jsonc", kind: "modify", diff: "-  \"apiKey\": \"sk-old-secret\"\n+  // 用户注释" }],
});

test("无请求只表示待验证；配置、helper、上游失败分别显示且保留旧drift兼容", () => {
  assert.deepEqual(agentDiagnosticPresentation(agent("awaitingTraffic"), true), { badge: "等待请求验证", tone: "pending", reconnect: false });
  assert.equal(agentDiagnosticPresentation(agent(), true).tone, "pending", "旧后端没有证据也不标为已验证");
  assert.equal(agentDiagnosticPresentation(agent("active"), true).badge, "请求已到达");
  assert.equal(agentDiagnosticPresentation(agent("upstreamError"), true).badge, "上游请求失败");
  assert.equal(agentDiagnosticPresentation(agent("configUnreadable"), true).reconnect, false, "不可读不建议覆盖");
  assert.deepEqual(agentDiagnosticPresentation(agent("configMissing"), true), { badge: "配置缺失", tone: "error", reconnect: true });
  assert.deepEqual(agentDiagnosticPresentation(agent("configUnverified"), true), { badge: "配置无法验证", tone: "error", reconnect: false });
  assert.equal(agentDiagnosticPresentation(agent("futureState" as GatewayAgentDiagnostics["state"]), true).badge, "状态待确认");
  assert.equal(agentDiagnosticPresentation(agent("active"), false).badge, "网关未运行");
  assert.equal(agentDiagnosticPresentation({ ...agent(), drift: { endpoint: "http://elsewhere" } }, true).reconnect, true);
  assert.equal(agentDiagnosticPresentation({ ...agent("awaitingTraffic"), drift: { endpoint: "http://old" } }, true).tone, "pending", "新诊断优先于兼容字段");
  assert.equal(agentDiagnosticPresentation({ ...agent("active"), enabled: false }, true).badge, "未接入");
});

test("供应商配置总开关关闭时只阻止 Codex 重新接入，旧 payload 与其它 Agent 保持兼容", () => {
  assert.equal(canReconnectAgent({ agent: "codex" }, false), false);
  assert.equal(canReconnectAgent({ agent: "claude" }, false), true);
  assert.equal(canReconnectAgent({ agent: "grok" }, false), true);
  assert.equal(canReconnectAgent({ agent: "codex" }, true), true);
  assert.equal(canReconnectAgent({ agent: "codex" }), true);

  const agents = readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8");
  assert.match(agents, /reconnectAllowed \? \([\s\S]*?重新接入[\s\S]*?: \([\s\S]*?供应商配置切换已关闭[\s\S]*?onOpenCodexProviderSettings/);
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(app, /onOpenCodexProviderSettings=\{\(\) => \{ void switchTool\("codex", "relay"\); \}\}/);
  assert.match(app, /takenOverByGateway && normalized\.relayProfilesEnabled \? null : \(/);
});

test("预览授权绑定Agent与非空token；文件状态和未知诊断证据不被误标成功", () => {
  assert.equal(disconnectPreviewReady(preview(), "opencode"), true);
  assert.equal(disconnectPreviewReady(preview("  "), "opencode"), false);
  assert.equal(disconnectPreviewReady(preview(), "codex"), false);
  assert.equal(disconnectPreviewReady({ ...preview(), canDisconnect: false }, "opencode"), false);
  assert.equal(disconnectPreviewReady(null, "opencode"), false);
  assert.deepEqual(["modify", "remove", "create"].map((kind) => disconnectFileLabel(kind as "modify")), ["将修改", "将移除", "将创建"]);
  assert.equal(diagnosticEvidenceLabel("endpointMatched"), "与网关配置一致");
  assert.equal(diagnosticEvidenceLabel("endpointPortMismatch"), "端口与网关配置不符");
  assert.equal(diagnosticEvidenceLabel("unrecognized_evidence"), "诊断信息待确认");
  assert.equal(diagnosticErrorLabel("authInvalid"), "认证已失效");
  assert.equal(diagnosticErrorLabel("unrecognized_error"), "错误原因待确认");
  for (const code of [
    "endpoint_match", "credential_match", "credential_sealed", "selection_match", "managed_config_match", "config_missing", "config_unreadable",
    "config_invalid", "config_unverified", "unsupported_agent", "endpoint_missing", "endpoint_invalid", "endpoint_changed",
    "endpoint_host_mismatch", "endpoint_port_mismatch", "endpoint_path_mismatch", "selection_changed", "credential_changed", "managed_config_changed",
  ]) assert.notEqual(diagnosticEvidenceLabel(code), "诊断信息待确认", `后端证据码 ${code} 应有可读文案`);
  for (const kind of ["auth_invalid", "rate_limited", "quota_exhausted", "server_error", "client_error", "upstream_error", "transport", "cooling"]) {
    assert.notEqual(diagnosticErrorLabel(kind), "错误原因待确认", `后端错误码 ${kind} 应有可读文案`);
  }
});

const source = ts.transpileModule(readFileSync(new URL("./AgentDisconnectDialog.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;
type Element = { type: unknown; props: Record<string, any> };
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}
const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function harness(own = false) {
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const pendingEffects: Array<() => void> = [];
  const loads: Array<ReturnType<typeof deferred<unknown>>> = [];
  const applies: Array<{ token: string; result: ReturnType<typeof deferred<boolean>> }> = [];
  let cursor = 0;
  let closed = 0;
  let escape = () => {};
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  const modules: Record<string, unknown> = {
    react: {
      useState: (initial: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = initial;
        return [hooks[index], (value: unknown) => { hooks[index] = value; }];
      },
      useRef: (initial: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = { current: initial };
        return hooks[index];
      },
      useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
        const index = cursor++;
        if (hooks[index]?.every((value: unknown, i: number) => Object.is(value, deps[i]))) return;
        hooks[index] = deps.slice();
        pendingEffects.push(() => {
          cleanups.get(index)?.();
          const cleanup = effect();
          if (typeof cleanup === "function") cleanups.set(index, cleanup);
        });
      },
    },
    "react/jsx-runtime": { jsx, jsxs: jsx },
    "react-dom": { createPortal: (child: unknown) => child },
    "@tauri-apps/api/core": { invoke: (command: string, args: unknown) => {
      assert.equal(command, "preview_gateway_disconnect");
      assert.equal((args as { agent: string }).agent, "opencode");
      const load = deferred<unknown>();
      loads.push(load);
      return load.promise;
    } },
    "lucide-react": Object.fromEntries(["FileDiff", "Loader2", "RefreshCw", "ShieldCheck", "Unplug", "X"].map((name) => [name, name])),
    "@/components/ui/button": { Button: "button" },
    "./i18n": {
      t: (text: string) => text,
      tf: (text: string, args: string[]) => text.replace(/\{(\d+)\}/g, (_, index) => args[Number(index)]),
    },
    "./gateway-agent-safety": { disconnectFileLabel, disconnectPreviewReady },
    "./use-escape-layer": { useEscapeLayer: (_active: boolean, close: () => void) => { escape = close; } },
    "./use-modal-focus": { useModalFocus: () => {} },
  };
  vm.runInNewContext(source, {
    exports, document: { body: {} },
    require: (name: string) => { assert.ok(name in modules, `unexpected module ${name}`); return modules[name]; },
  });
  return {
    loads, applies,
    render: (busy = false) => {
      cursor = 0;
      const result = exports.AgentDisconnectDialog({
        agent: agent(), busy,
        ownModel: own ? { model: "gpt-own", label: "Own GPT", direct: "OpenAI" } : null,
        onClose: () => { closed += 1; },
        onApply: (token: string) => {
          const result = deferred<boolean>();
          applies.push({ token, result });
          return result.promise;
        },
      }) as Element;
      pendingEffects.splice(0).forEach((effect) => effect());
      return result;
    },
    closeCount: () => closed,
    escape: () => escape(),
    unmount: () => cleanups.forEach((cleanup) => cleanup()),
  };
}
function elements(element: any): Element[] {
  if (!element || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(elements);
  return [element, ...elements(element.props?.children)];
}
function text(element: any): string {
  if (element === null || element === undefined || typeof element === "boolean") return "";
  if (typeof element !== "object") return String(element);
  if (Array.isArray(element)) return element.map(text).join("");
  return text(element.props?.children);
}
function button(tree: Element, label: string) {
  const result = elements(tree).find((element) => element.type === "button" && text(element) === label);
  assert.ok(result, `missing button ${label}`);
  return result;
}
async function ready(dialog: ReturnType<typeof harness>, value = preview()) {
  dialog.loads.at(-1)!.resolve({ status: "ok", message: "", preview: value });
  await flush();
  return dialog.render();
}

test("断开前等待预览；保留字段警告、文件diff按原文可读", async () => {
  const dialog = harness();
  let tree = dialog.render();
  assert.equal(button(tree, "断开并还原").props.disabled, true);
  assert.equal(dialog.applies.length, 0);
  tree = await ready(dialog);
  assert.equal(button(tree, "断开并还原").props.disabled, false);
  assert.match(text(tree), /OpenCode 将恢复接入前的设置，以下为要改回的项/);
  assert.match(text(tree), /保留接入后修改的字段/);
  assert.match(text(tree), /\/test\/opencode.jsonc/);
  assert.match(text(tree), /sk-old-secret/);
  assert.match(text(tree), /用户注释/);
  assert.equal(elements(tree).find((element) => element.props.role === "dialog")?.props["aria-modal"], "true");
});

test("预览失败或无法安全还原时不能提交；刷新可恢复", async () => {
  const dialog = harness();
  dialog.render();
  dialog.loads[0].resolve({ status: "error", message: "文件无法解析", preview: null });
  await flush();
  let tree = dialog.render();
  assert.match(text(tree), /文件无法解析/);
  assert.equal(button(tree, "断开并还原").props.disabled, true);
  button(tree, "重新获取预览").props.onClick();
  tree = await ready(dialog, { ...preview(), files: [], canDisconnect: false });
  assert.equal(button(tree, "断开并还原").props.disabled, true);
  assert.match(text(tree), /当前无法安全还原/);
  assert.doesNotMatch(text(tree), /没有需要改写的配置文件/);
  assert.equal(dialog.applies.length, 0);
  button(tree, "重新获取预览").props.onClick();
  tree = await ready(dialog, { ...preview(), files: [] });
  assert.equal(button(tree, "断开并还原").props.disabled, false);
  assert.match(text(tree), /没有需要改写的配置文件/);
});

test("确认使用所见token；应用中拦重复操作与关闭，失败后旧token作废", async () => {
  const dialog = harness();
  dialog.render();
  let tree = await ready(dialog);
  button(tree, "断开并还原").props.onClick();
  button(tree, "断开并还原").props.onClick();
  button(tree, "重新获取预览").props.onClick();
  assert.equal(dialog.loads.length, 1, "确认提交后即刻禁止旧按钮触发刷新");
  assert.deepEqual(dialog.applies.map((item) => item.token), ["snapshot-1"]);
  dialog.escape();
  assert.equal(dialog.closeCount(), 0);
  tree = dialog.render();
  assert.equal(button(tree, "正在还原…").props.disabled, true);
  dialog.applies[0].result.resolve(false);
  await flush();
  tree = dialog.render();
  assert.equal(button(tree, "断开并还原").props.disabled, true);
  assert.match(text(tree), /重新获取预览后再确认/);
  button(tree, "重新获取预览").props.onClick();
  tree = await ready(dialog, preview("snapshot-2"));
  button(tree, "断开并还原").props.onClick();
  assert.equal(dialog.applies[1].token, "snapshot-2");
  dialog.applies[1].result.resolve(true);
  await flush();
  assert.equal(dialog.closeCount(), 1);
});

test("刷新只接纳最后的预览；跨Agent返回的token被拒绝", async () => {
  const dialog = harness();
  dialog.render();
  let tree = await ready(dialog);
  const refresh = button(tree, "重新获取预览");
  refresh.props.onClick();
  refresh.props.onClick();
  tree = await ready(dialog, preview("newest"));
  dialog.loads[1].resolve({ status: "ok", message: "", preview: preview("old") });
  await flush();
  tree = dialog.render();
  button(tree, "断开并还原").props.onClick();
  assert.equal(dialog.applies[0].token, "newest");
  dialog.applies[0].result.resolve(false);
  await flush();
  tree = dialog.render();
  button(tree, "重新获取预览").props.onClick();
  tree = await ready(dialog, { ...preview(), agent: "codex" });
  assert.equal(button(tree, "断开并还原").props.disabled, true);
});

test("断开并直连明确预览范围，携带相同确认token", async () => {
  const dialog = harness(true);
  dialog.render();
  const tree = await ready(dialog);
  assert.match(text(tree), /还原后将选择 Own GPT，使用 OpenAI/);
  assert.match(text(tree), /仅预览接入配置的还原差异/);
  button(tree, "断开并直连").props.onClick();
  assert.equal(dialog.applies[0].token, "snapshot-1");
  dialog.unmount();
  dialog.applies[0].result.resolve(true);
  await flush();
  assert.equal(dialog.closeCount(), 0, "卸载后的结果不再次触发关闭");
});

function renderTrayAgent(connected: boolean, requiresDisconnectPreview = false, group?: { disabled: boolean }) {
  const sourceFile = ts.createSourceFile("TrayPanel.tsx", readFileSync(new URL("./TrayPanel.tsx", import.meta.url), "utf8"), ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
  const component = sourceFile.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "PanelAgentRow");
  assert.ok(component);
  const compiled = ts.transpileModule(`export ${component.getText(sourceFile)}`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const hooks: any[] = [];
  let cursor = 0;
  const calls: Array<{ command: string; args: Record<string, unknown> }> = [];
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    require: (name: string) => {
      assert.equal(name, "react/jsx-runtime");
      return { jsx, jsxs: jsx };
    },
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = initial;
      return [hooks[index], (value: unknown) => { hooks[index] = value; }];
    },
    useRef: (initial: unknown) => {
      const index = cursor++;
      if (!(index in hooks)) hooks[index] = { current: initial };
      return hooks[index];
    },
    AgentIcon: "AgentIcon", ModelViewGlyph: "ModelViewGlyph", AutoTag: "AutoTag", ChevronDown: "ChevronDown", Check: "Check", AlertTriangle: "AlertTriangle",
    Floating: "Floating", ModelList: "ModelList", AgentDisconnectDialog: "AgentDisconnectDialog",
    draftFromBinding: () => ({ model: group ? "group/daily" : "gpt", poolExclude: [] }),
    disabledGroup,
    modelDisplay: (_payload: unknown, model: string) => ({ title: model === "group/daily" ? "Daily" : "GPT" }),
    ownCurrent: () => null,
    visibleOwnModels: () => [],
    agentModels: () => [],
    bindingFromDraft: (value: unknown) => value,
    t: (value: string) => value,
    tf: (value: string, values: string[]) => value.replace(/\{(\d+)\}/g, (_, index) => values[Number(index)]),
  });
  return {
    calls,
    render: () => {
      cursor = 0;
      return exports.PanelAgentRow({
        agent: { ...agent(), enabled: connected, requiresDisconnectPreview },
        payload: group ? { groups: [{ id: "daily", name: "Daily", disabled: group.disabled }], gateway: { bindings: {} } } : null,
        onRun: async (command: string, args: Record<string, unknown>) => { calls.push({ command, args }); return true; },
      }) as Element;
    },
  };
}

test("托盘在模型组停用时显示回退模型，重新启用后恢复原组", () => {
  const group = { disabled: true };
  const tray = renderTrayAgent(true, false, group);
  let tree = tray.render();
  let picker = elements(tree).find((element) => element.props.className === "gw-picker-button tp-agent-picker")!;
  assert.match(text(picker), /GPT/);
  assert.equal(picker.props.title, "原模型组「Daily」已停用，当前使用 GPT");
  assert.equal(elements(tree).find((element) => element.type === "ModelList")!.props.selectedModel, "gpt");

  group.disabled = false;
  tree = tray.render();
  picker = elements(tree).find((element) => element.props.className === "gw-picker-button tp-agent-picker")!;
  assert.match(text(picker), /Daily/);
  assert.equal(picker.props.title, undefined);
  assert.equal(elements(tree).find((element) => element.type === "ModelList")!.props.selectedModel, "group/daily");
});

test("托盘已连接Agent的默认与直连选择共用预览确认，未连接直连保持直接切换", async () => {
  const tray = renderTrayAgent(true);
  let tree = tray.render();
  const list = elements(tree).find((element) => element.type === "ModelList")!;
  list.props.head.props.onClick();
  assert.equal(tray.calls.length, 0, "选择默认不应先执行断开");
  tree = tray.render();
  let dialog = elements(tree).find((element) => element.type === "AgentDisconnectDialog")!;
  assert.ok(dialog);
  assert.equal(await dialog.props.onApply("reviewed-default"), true);
  assert.equal(tray.calls[0].command, "disable_gateway_binding");
  assert.equal(tray.calls[0].args.previewToken, "reviewed-default");
  dialog.props.onClose();
  tree = tray.render();
  elements(tree).find((element) => element.type === "ModelList")!.props.onPickOwn({ model: "gpt-own", label: "Own GPT" });
  tree = tray.render();
  dialog = elements(tree).find((element) => element.type === "AgentDisconnectDialog")!;
  assert.equal(dialog.props.ownModel.model, "gpt-own");
  assert.equal(tray.calls.length, 1, "已连接选自带模型也需等确认");
  await dialog.props.onApply("reviewed-own");
  assert.equal(tray.calls[1].command, "use_gateway_own_model");
  assert.equal(tray.calls[1].args.model, "gpt-own");
  assert.equal(tray.calls[1].args.previewToken, "reviewed-own");

  const direct = renderTrayAgent(false);
  tree = direct.render();
  elements(tree).find((element) => element.type === "ModelList")!.props.onPickOwn({ model: "gpt-own", label: "Own GPT" });
  assert.equal(direct.calls[0].command, "use_gateway_own_model");
  assert.equal("previewToken" in direct.calls[0].args, false);
  assert.equal(elements(direct.render()).some((element) => element.type === "AgentDisconnectDialog"), false);

  const orphan = renderTrayAgent(false, true);
  tree = orphan.render();
  elements(tree).find((element) => element.type === "ModelList")!.props.onPickOwn({ model: "gpt-own", label: "Own GPT" });
  assert.equal(orphan.calls.length, 0, "绑定丢失但仍在使用托管供应商时也必须预览");
  dialog = elements(orphan.render()).find((element) => element.type === "AgentDisconnectDialog")!;
  assert.ok(dialog);
  await dialog.props.onApply("reviewed-orphan");
  assert.equal(orphan.calls[0].args.previewToken, "reviewed-orphan");
});

test("展开详情才展示配置证据；无证据不声称配置已验证，错误类型也显示中文", () => {
  const sourceFile = ts.createSourceFile("GatewayAgents.tsx", readFileSync(new URL("./GatewayAgents.tsx", import.meta.url), "utf8"), ts.ScriptTarget.ES2022, true, ts.ScriptKind.TSX);
  const component = sourceFile.statements.find((node) => ts.isFunctionDeclaration(node) && node.name?.text === "AgentDiagnostics");
  assert.ok(component);
  const compiled = ts.transpileModule(`export ${component.getText(sourceFile)}`, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
  const exports: Record<string, any> = {};
  const jsx = (type: unknown, props: Record<string, any>) => ({ type, props });
  vm.runInNewContext(compiled, {
    exports,
    require: (name: string) => { assert.equal(name, "react/jsx-runtime"); return { jsx, jsxs: jsx }; },
    agentDiagnosticPresentation, diagnosticEvidenceLabel, diagnosticErrorLabel,
    t: (value: string) => value,
    tf: (value: string, values: string[]) => value.replace(/\{(\d+)\}/g, (_, index) => values[Number(index)]),
  });
  let tree = exports.AgentDiagnostics({ agent: agent(), helperRunning: true });
  assert.match(text(tree), /尚无足够信息确认请求路径/);
  assert.doesNotMatch(text(tree), /配置已指向网关/);
  const viewed = agent("awaitingTraffic");
  viewed.diagnostics!.endpoint = "http://127.0.0.1:57321/gw/opencode";
  viewed.diagnostics!.expectedEndpoint = viewed.diagnostics!.endpoint;
  viewed.diagnostics!.configEvidence = [{
    path: "/test/opencode.jsonc", field: "provider", code: "endpoint_match",
    endpoint: viewed.diagnostics!.endpoint, expectedEndpoint: viewed.diagnostics!.endpoint,
  }];
  viewed.diagnostics!.recentError = { kind: "auth_invalid", status: 401, atMs: 1 };
  tree = exports.AgentDiagnostics({ agent: viewed, helperRunning: true });
  assert.match(text(tree), /配置已指向网关；尚未观察到请求/);
  assert.match(text(tree), /请求 0 · 失败 0 · 进行中 0/);
  assert.match(text(tree), /当前地址.*127.0.0.1:57321/);
  assert.match(text(tree), /opencode.jsoncprovider与网关配置一致/);
  assert.match(text(tree), /认证已失效 · HTTP 401/);
  assert.doesNotMatch(text(tree), /endpoint_match|auth_invalid/);
});

test("没法逐项预览的 Codex 断开：只在后端给了完整切换目标和 token 时才给「完整切换并断开」，普通断开仍不放行", () => {
  const blocked = { ...preview(), agent: "codex", canDisconnect: false, fullSwitch: { relayId: "direct", name: "Direct" } };
  assert.equal(fullSwitchReady(blocked, "codex"), true);
  assert.equal(disconnectPreviewReady(blocked, "codex"), false, "普通断开不放行");
  assert.equal(fullSwitchReady({ ...blocked, fullSwitch: null }, "codex"), false);
  assert.equal(fullSwitchReady({ ...blocked, previewToken: " " }, "codex"), false);
  assert.equal(fullSwitchReady(blocked, "claude"), false);
  assert.equal(fullSwitchReady({ ...blocked, canDisconnect: true }, "codex"), false, "能逐项预览时走普通断开");
  const dialog = readFileSync(new URL("./AgentDisconnectDialog.tsx", import.meta.url), "utf8");
  // 改用自带模型时不走完整切换；调用方没给 onFullSwitch 时也不给这条路。
  assert.match(dialog, /const fullSwitch = !ownModel && onFullSwitch && fullSwitchReady\(preview, agent\.agent\) \? preview : null;/);
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(app, /invoke<GatewayResult>\("disable_gateway_binding_full_switch", \{ agent: "codex", previewToken \}\)/);
});
