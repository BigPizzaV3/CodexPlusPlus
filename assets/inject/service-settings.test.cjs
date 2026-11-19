// Codex++ 设置页「语音输入服务」「下一步建议服务」「原生浏览器状态」三节（46-service-settings.js）的行为测试：
// Key 不回页面、只发改了的字段、数值按后端范围收口、写失败回滚、原生浏览器状态只读。
const assert = require("node:assert/strict");
const fs = require("node:fs");
const vm = require("node:vm");

const source = fs.readFileSync(require.resolve("./renderer-inject/46-service-settings.js"), "utf8");

function harness({ settings = {}, windows = false, respond } = {}) {
  const posts = [];
  const toasts = [];
  let reloads = 0;
  const context = vm.createContext({
    window: {},
    console,
    URL,
    CSS: { escape: (value) => String(value) },
    document: { activeElement: null, querySelectorAll: () => [], querySelector: () => null },
    setTimeout: () => 0,
    clearTimeout: () => {},
    escapeHtml: (value) => String(value ?? "").replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;"),
    codexPlusBackendSettings: settings,
    codexPlusBackendSettingsSeq: 0,
    codexPlusIsWindowsPlatform: windows,
    codexPlusIsMacPlatform: false,
    postJson: async (path, payload) => {
      posts.push([path, JSON.parse(JSON.stringify(payload))]);
      return respond ? respond(path, payload) : { status: "ok" };
    },
    showToast: (message) => toasts.push(message),
    loadBackendSettings: async () => { reloads += 1; },
    emitCodexPlusSettingsChanged: () => {},
    codexPlusAsk: async () => "clear",
    openCodexPlusPage: () => {},
  });
  vm.runInContext(source, context);
  return { context, posts, toasts, reloads: () => reloads, settings: () => vm.runInContext("codexPlusBackendSettings", context) };
}

/** 只带一个 data-* 属性的假输入框，够 closest / getAttribute / hasAttribute 用。 */
function field(attr, key, value) {
  const node = {
    value,
    closest: (selector) => (selector.includes(`[${attr}]`) ? node : null),
    getAttribute: (name) => (name === attr ? key : null),
    hasAttribute: (name) => name === attr,
  };
  return node;
}

const flush = () => new Promise((resolve) => setImmediate(resolve));

async function runServiceSettingsTests() {
  // 1. 渲染：Key 本身永远不进 HTML，只显示「已配置」；预设按地址认出来。
  {
    const { context } = harness({ settings: { dictation: {
      enabled: true, provider: "api", baseUrl: "https://api.groq.com/openai/v1", model: "whisper-large-v3-turbo",
      apiKeyConfigured: true, apiKey: "sk-private-fixture", apiKeyEnv: "GROQ_API_KEY",
    }, codexAppStepwiseApiKeyConfigured: false } });
    const dictation = context.renderCodexPlusDictationServiceSection();
    assert.ok(!dictation.includes("sk-private-fixture"), "语音 Key 不能出现在页面里");
    assert.match(dictation, /data-codex-plus-dictation-secret="apiKey" type="password" value="" placeholder="已配置（留空不改）"/);
    assert.match(dictation, /<option value="groq" selected>Groq<\/option>/);
    assert.match(dictation, /data-codex-plus-dictation-secret-clear="apiKey"/);
    assert.doesNotMatch(dictation, /配置不完整/);
    const stepwise = context.renderCodexPlusStepwiseServiceSection();
    assert.match(stepwise, /data-codex-plus-stepwise-secret="codexAppStepwiseApiKey" type="password" value="" placeholder="未配置"/);
    assert.doesNotMatch(stepwise, /data-codex-plus-stepwise-secret-clear/);
    // 非 Windows 不显示原生浏览器状态；其他分组后面不接东西。
    assert.equal(context.renderCodexPlusServiceSettingsAfterGroup("plugins"), "");
    assert.equal(context.renderCodexPlusServiceSettingsAfterGroup("pet"), "");
  }

  // 2. 填了 Key 只发 Key 本身；页面只记「已配置」，输入框清空。留空不写。
  {
    const h = harness({ settings: { dictation: { enabled: true, provider: "api", model: "m" } } });
    const empty = field("data-codex-plus-dictation-secret", "apiKey", "   ");
    assert.equal(h.context.handleCodexPlusServiceSettingsChange(empty), true);
    await flush();
    assert.deepEqual(h.posts, [], "留空表示不改");
    const secret = field("data-codex-plus-dictation-secret", "apiKey", " sk-new ");
    assert.equal(h.context.handleCodexPlusServiceSettingsChange(secret), true);
    await flush();
    assert.equal(secret.value, "");
    assert.deepEqual(h.posts, [["/settings/set", { dictation: { apiKey: "sk-new" } }]]);
    assert.equal(h.settings().dictation.apiKeyConfigured, true);
    assert.equal("apiKey" in h.settings().dictation, false, "本地副本不留 Key");
  }

  // 3. 普通字段只发这一项（不带 enabled），下一步建议的 Key 写进自己的字段。
  {
    const h = harness({ settings: { dictation: { enabled: false, provider: "api", model: "old" } } });
    h.context.handleCodexPlusServiceSettingsChange(field("data-codex-plus-dictation-field", "model", " whisper-1 "));
    h.context.handleCodexPlusServiceSettingsChange(field("data-codex-plus-stepwise-secret", "codexAppStepwiseApiKey", "sk-step"));
    await flush();
    assert.deepEqual(h.posts, [
      ["/settings/set", { dictation: { model: "whisper-1" } }],
      ["/settings/set", { codexAppStepwiseApiKey: "sk-step" }],
    ]);
    assert.equal(h.settings().codexAppStepwiseApiKeyConfigured, true);
    assert.equal(h.settings().codexAppStepwiseApiKey, undefined);
    assert.equal(h.settings().dictation.enabled, false);
  }

  // 4. 数值按后端范围收口，输入框也改成收口后的值。
  {
    const h = harness({ settings: { codexAppStepwiseMaxItems: 3, dictation: { timeoutSeconds: 120 } } });
    const items = field("data-codex-plus-stepwise-field", "codexAppStepwiseMaxItems", "99");
    const timeout = field("data-codex-plus-dictation-field", "timeoutSeconds", "0");
    h.context.handleCodexPlusServiceSettingsChange(items);
    h.context.handleCodexPlusServiceSettingsChange(timeout);
    await flush();
    assert.equal(items.value, "6");
    assert.equal(timeout.value, "1");
    assert.deepEqual(h.posts, [
      ["/settings/set", { codexAppStepwiseMaxItems: 6 }],
      ["/settings/set", { dictation: { timeoutSeconds: 1 } }],
    ]);
  }

  // 5. 写失败：提示原因并重新读后端设置，页面回到真实值。
  {
    const h = harness({ settings: { codexAppStepwiseModel: "a" }, respond: () => ({ status: "failed", message: "设置已被其他窗口修改" }) });
    h.context.handleCodexPlusServiceSettingsChange(field("data-codex-plus-stepwise-field", "codexAppStepwiseModel", "b"));
    await flush();
    await flush();
    assert.deepEqual(h.toasts, ["保存失败：设置已被其他窗口修改"]);
    assert.equal(h.reloads(), 1);
  }

  // 6. 选预设只改端点、模型与 provider；「自定义」不动现有值。
  {
    const h = harness({ settings: { dictation: { provider: "api", baseUrl: "https://example.test/v1", model: "x" } } });
    const select = { value: "custom", closest: (selector) => (selector.includes("[data-codex-plus-dictation-preset]") ? select : null) };
    h.context.handleCodexPlusServiceSettingsChange(select);
    select.value = "sensevoice";
    h.context.handleCodexPlusServiceSettingsChange(select);
    await flush();
    assert.deepEqual(h.posts, [["/settings/set", { dictation: { provider: "sensevoice", baseUrl: "local://sensevoice", model: "SenseVoiceSmall (INT8)" } }]]);
  }

  // 7. 原生浏览器状态：只读，文案与管理器一致；读失败显示检测失败。
  {
    const h = harness({ windows: true, respond: (path) => (path === "/native-browser/status" ? {
      compatibility: { state: "prepared", detail: "" },
      connection: { state: "available", failedChecks: 1, browsers: [{ family: "edge", headerEnabled: true, recognized: false }] },
    } : { status: "ok" }) });
    assert.match(h.context.renderCodexPlusServiceSettingsAfterGroup("plugins"), /正在检测浏览器连接/);
    await h.context.refreshCodexPlusNativeBrowserStatus();
    const html = h.context.renderCodexPlusNativeBrowserStatusSection();
    assert.match(html, /Edge 浏览器可用/);
    assert.match(html, /Edge · 请求标识：已开启（扩展报告）（扩展版本未登记）/);
    assert.match(html, /兼容服务文件已准备，等待原生工作进程加载/);
    assert.match(html, /1 个原生连接端点未通过检测/);
    assert.deepEqual(h.posts.map(([path]) => path), ["/native-browser/status"], "状态那一节不写设置");
    const failed = harness({ windows: true, respond: () => ({ status: "failed", message: "未连接" }) });
    await failed.context.refreshCodexPlusNativeBrowserStatus();
    assert.match(failed.context.renderCodexPlusNativeBrowserStatusSection(), /浏览器状态检测失败/);
  }

  // 8. 下一步建议「测试连接」只读已保存的配置，按结果显示。
  {
    const h = harness({ respond: () => ({ status: "ok", items: [{ text: "a" }, { text: "b" }] }) });
    h.context.handleCodexPlusServiceSettingsClick({ closest: (selector) => (selector === "[data-codex-plus-stepwise-test]" ? {} : null) });
    await flush();
    assert.deepEqual(h.posts, [["/stepwise/test", {}]]);
    assert.match(h.context.renderCodexPlusStepwiseServiceSection(), /连接正常 · 2 条建议/);
  }

  // 9. 预设按地址认出来；SenseVoice 认 provider；其余地址算自定义。
  {
    const { context } = harness();
    const preset = (dictation) => context.codexPlusDictationPreset(dictation);
    assert.equal(preset({ provider: "api", baseUrl: "https://api.groq.com/openai/v1/" }), "groq");
    assert.equal(preset({ provider: "api", baseUrl: "https://api.openai.com/v1" }), "openai");
    assert.equal(preset({ provider: "api", baseUrl: "http://127.0.0.1:8000/v1" }), "local");
    assert.equal(preset({ provider: "sensevoice", baseUrl: "anything" }), "sensevoice");
    assert.equal(preset({ provider: "api", baseUrl: "http://localhost:9090/asr/v1" }), "custom");
  }

  // 10. 校验只在开启时提示；SenseVoice 不要求地址；地址、模型、环境变量名各有各的提示。
  {
    const { context } = harness();
    const issue = (dictation) => context.codexPlusDictationIssue(dictation);
    const local = { enabled: true, provider: "api", baseUrl: "http://127.0.0.1:8000/v1", model: "whisper-1", apiKeyEnv: "" };
    assert.equal(issue(local), "");
    for (const baseUrl of ["", "file:///tmp/asr", "https://user:pass@example.com/v1", "https://example.com/v1?key=x", "https://example.com/v1#fragment"]) {
      assert.match(issue({ ...local, baseUrl }), /Base URL/, baseUrl);
    }
    assert.match(issue({ ...local, model: " " }), /语音模型/);
    assert.match(issue({ ...local, apiKeyEnv: "1INVALID" }), /环境变量名/);
    assert.equal(issue({ ...local, apiKeyEnv: "ASR_API_KEY" }), "");
    assert.equal(issue({ ...local, enabled: false, baseUrl: "" }), "");
    assert.equal(issue({ enabled: true, provider: "sensevoice", baseUrl: "local://sensevoice", model: "SenseVoiceSmall (INT8)" }), "");
  }
}

module.exports = runServiceSettingsTests;
if (require.main === module) {
  runServiceSettingsTests().then(() => console.log("Codex++ service settings sections passed")).catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
