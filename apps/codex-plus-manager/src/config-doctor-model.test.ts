import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import {
  aiBlockedHint,
  aiReady,
  aiStatusView,
  CURRENT_SOURCE_MISSING,
  currentSourceNote,
  DOCTOR_SOURCES,
  draftFromSettings,
  formatCheckedAt,
  formatElapsed,
  groupFindings,
  LOCAL_SOURCE_MISSING,
  localSourceNote,
  matchedPresetId,
  normalizeSource,
  OLLAMA_DOCS_URL,
  ollamaDocsUrl,
  presetFields,
  savedPayload,
  sortFindings,
  sourceLabel,
  sourceOptions,
  summarize,
  summaryLine,
  summaryTotal,
  type AiReport,
  type DoctorPreset,
  type DoctorSettings,
  type DoctorSources,
  type Finding,
} from "./config-doctor-model.ts";

const read = (file: string) => readFileSync(new URL(file, import.meta.url), "utf8");

function finding(id: string, level: Finding["level"]): Finding {
  return { id, level, title: `${id} 标题`, detail: `${id} 说明` };
}

const settings = (overrides: Partial<DoctorSettings> = {}): DoctorSettings => ({
  enabled: false,
  source: "current",
  baseUrl: "https://api.example.com/v1",
  model: "free-model",
  hasKey: false,
  apiKeyMasked: "",
  ...overrides,
});

const sources = (overrides: Partial<DoctorSources> = {}): DoctorSources => ({
  current: { available: true, model: "gpt-5.3-codex", profile: "哈基米 国模", label: "复用当前供应商（哈基米 国模 · gpt-5.3-codex）" },
  local: { found: false, baseUrl: "", model: "", label: "" },
  ...overrides,
});

const preset = (overrides: Partial<DoctorPreset> = {}): DoctorPreset => ({
  id: "demo",
  label: "示例免费服务",
  baseUrl: "https://api.example.com/v1",
  model: "free-model",
  needsKey: true,
  note: "",
  docsUrl: "",
  ...overrides,
});

const ai = (overrides: Partial<AiReport> = {}): AiReport => ({
  status: "skipped",
  message: "",
  model: "",
  endpoint: "",
  elapsedMs: 0,
  summary: "",
  items: [],
  ...overrides,
});

test("配置体检：结果按错误→警告→提示排序，同级保持后端顺序，空组不出现", () => {
  const items = [finding("c", "info"), finding("a", "error"), finding("b", "warn"), finding("d", "error")];
  assert.deepEqual(sortFindings(items).map((item) => item.id), ["a", "d", "b", "c"]);
  assert.deepEqual(items.map((item) => item.id), ["c", "a", "b", "d"], "不改动入参");

  const groups = groupFindings(items);
  assert.deepEqual(groups.map((group) => group.level), ["error", "warn", "info"]);
  assert.deepEqual(groups.map((group) => group.label), ["错误", "警告", "提示"]);
  assert.deepEqual(groups[0].items.map((item) => item.id), ["a", "d"]);
  assert.deepEqual(groupFindings([finding("only", "warn")]).map((group) => group.level), ["warn"]);
  assert.deepEqual(groupFindings([]), []);
});

test("配置体检：计数与汇总一句话", () => {
  const summary = summarize([finding("a", "error"), finding("b", "warn"), finding("c", "warn"), finding("d", "info")]);
  assert.deepEqual(summary, { error: 1, warn: 2, info: 1 });
  assert.equal(summaryTotal(summary), 4);
  assert.deepEqual(summaryLine(summary), {
    text: "共 {0} 项：{1} 个错误、{2} 个警告、{3} 条提示",
    args: ["4", "1", "2", "1"],
  });
  assert.equal(summaryLine({ error: 0, warn: 0, info: 0 }), null, "全绿时不显示计数句");
});

test("配置体检：预设一键填表、回显选中项，自定义地址不回显", () => {
  const demo = preset();
  assert.deepEqual(presetFields(demo), { baseUrl: "https://api.example.com/v1", model: "free-model" });
  assert.equal(presetFields(null), null);
  assert.equal(matchedPresetId([demo], { baseUrl: "https://api.example.com/v1/" }), "demo", "尾斜杠不影响匹配");
  assert.equal(matchedPresetId([demo], { baseUrl: "  HTTPS://API.EXAMPLE.COM/v1  " }), "demo");
  assert.equal(matchedPresetId([demo], { baseUrl: "https://other.example.com/v1" }), "");
});

test("配置体检：保存带上来源，Key 留空不覆盖已存的 Key", () => {
  const saved = savedPayload({ enabled: true, source: "current", baseUrl: " https://api.example.com/v1 ", model: " free-model ", apiKey: "  " });
  assert.deepEqual(saved, { enabled: true, source: "current", baseUrl: "https://api.example.com/v1", model: "free-model" });
  assert.equal("apiKey" in saved, false);
  assert.equal(savedPayload({ enabled: false, source: "custom", baseUrl: "", model: "", apiKey: " sk-1 " }).apiKey, "sk-1");
  const draft = draftFromSettings(settings({ hasKey: true, apiKeyMasked: "sk-••••1234", source: "local" }));
  assert.deepEqual(draft, { enabled: false, source: "local", baseUrl: "https://api.example.com/v1", model: "free-model", apiKey: "" });
});

test("配置体检：来源三选一默认复用当前供应商，标签与说明来自后端整句", () => {
  assert.deepEqual(DOCTOR_SOURCES, ["current", "local", "custom"]);
  assert.deepEqual(DOCTOR_SOURCES.map(sourceLabel), ["复用当前供应商（推荐）", "本机模型", "自定义"]);
  // 历史配置没有 source 一律按 current；未知值也退回 current。
  assert.equal(normalizeSource(undefined), "current");
  assert.equal(normalizeSource(""), "current");
  assert.equal(normalizeSource("bogus"), "current");
  assert.equal(normalizeSource(" local "), "local");
  assert.equal(normalizeSource("custom"), "custom");

  const options = sourceOptions(sources());
  assert.deepEqual(options.map((option) => option.value), ["current", "local", "custom"]);
  assert.deepEqual(options.map((option) => option.label), ["复用当前供应商（推荐）", "本机模型", "自定义"]);
  assert.equal(options[0].hint, "复用当前供应商（哈基米 国模 · gpt-5.3-codex）");
  assert.equal(options[1].hint, LOCAL_SOURCE_MISSING, "没探测到本机服务时说明探测范围");
  assert.equal(
    sourceOptions(sources({ local: { found: true, baseUrl: "http://127.0.0.1:11434/v1", model: "qwen3:8b", label: "Ollama（本机）" } }))[1].hint,
    "Ollama（本机）",
  );
  // 没有探测结果（首次加载）时也给一句能看的说明。
  assert.equal(sourceOptions(null)[0].hint.includes("Codex"), true);
  assert.equal(sourceOptions(null)[1].hint, LOCAL_SOURCE_MISSING);
});

test("配置体检：当前供应商/本机模型的说明行与 Ollama 文档地址", () => {
  const missing = currentSourceNote(sources({ current: { available: false, model: "", profile: "默认", label: "复用当前供应商（还没读到默认模型）" } }).current);
  assert.equal(missing.label, "复用当前供应商（还没读到默认模型）");
  assert.equal(missing.available, false);
  assert.equal(missing.hint, CURRENT_SOURCE_MISSING);
  const ready = currentSourceNote(sources().current);
  assert.equal(ready.available, true);
  assert.equal(ready.hint, null);
  assert.equal(currentSourceNote(undefined).hint, CURRENT_SOURCE_MISSING);

  const local = localSourceNote({ found: true, baseUrl: "http://127.0.0.1:11434/v1", model: "qwen3:8b", label: "Ollama（本机）" });
  assert.deepEqual(local, { label: "Ollama（本机）", found: true });
  assert.deepEqual(localSourceNote(undefined), { label: LOCAL_SOURCE_MISSING, found: false });

  assert.equal(ollamaDocsUrl([]), OLLAMA_DOCS_URL);
  assert.equal(ollamaDocsUrl([preset({ id: "ollama", docsUrl: "https://docs.ollama.com/api/openai-compatibility" })]), "https://docs.ollama.com/api/openai-compatibility");
  assert.equal(ollamaDocsUrl([preset({ id: "ollama", docsUrl: "" })]), OLLAMA_DOCS_URL);
});

test("配置体检：current 不用填字段即可 AI 体检，custom 缺地址或模型不行，local 不需要 Key", () => {
  // 未启用：一律不可用。
  assert.equal(aiReady(settings()), false);
  assert.equal(aiBlockedHint(settings())?.text, "先启用 AI 解释并保存，再用 AI 深度体检。");

  // current：零配置，空 baseUrl / model 也不影响。
  assert.equal(aiReady(settings({ enabled: true, source: "current", baseUrl: "", model: "" })), true);
  assert.equal(aiBlockedHint(settings({ enabled: true, source: "current", baseUrl: "", model: "" })), null);

  // local：不需要 Key，也不需要填地址。
  assert.equal(aiReady(settings({ enabled: true, source: "local", baseUrl: "", model: "", hasKey: false })), true);
  assert.equal(aiBlockedHint(settings({ enabled: true, source: "local", hasKey: false })), null);

  // custom：baseUrl 与模型都要有，Key 不是门槛。
  assert.equal(aiReady(settings({ enabled: true, source: "custom", baseUrl: "https://api.example.com/v1", model: "free-model", hasKey: false })), true);
  assert.equal(aiReady(settings({ enabled: true, source: "custom", baseUrl: "  ", model: "free-model" })), false);
  assert.equal(aiBlockedHint(settings({ enabled: true, source: "custom", baseUrl: "", model: "free-model" }))?.text, "先填好 Base URL 和模型并保存，再用 AI 深度体检。");
  assert.equal(aiReady(settings({ enabled: true, source: "custom", baseUrl: "https://api.example.com/v1", model: "   " })), false);
  assert.equal(aiReady(settings({ enabled: true, source: "custom", baseUrl: "", model: "" })), false);
});

test("配置体检：AI 三态文案（跳过 / 成功 / 失败）", () => {
  const skipped = aiStatusView(ai());
  assert.deepEqual(skipped, { tone: "off", label: "未用 AI 解释", detail: "这次只跑了本地规则体检。", facts: [] });
  assert.equal(aiStatusView(ai({ message: "AI 未启用" })).detail, "AI 未启用");

  const ok = aiStatusView(ai({ status: "ok", model: "free-model", endpoint: "https://api.example.com/v1", elapsedMs: 1234, summary: "配置看起来没问题" }));
  assert.equal(ok.tone, "ok");
  assert.equal(ok.label, "AI 解释完成");
  assert.equal(ok.detail, "配置看起来没问题");
  assert.deepEqual(ok.facts, ["free-model", "https://api.example.com/v1", "1.2 s"]);

  const failed = aiStatusView(ai({ status: "failed", message: "401 未授权", model: "free-model", endpoint: "https://api.example.com/v1", elapsedMs: 30 }));
  assert.equal(failed.tone, "danger");
  assert.equal(failed.label, "AI 解释失败");
  assert.equal(failed.detail, "401 未授权");
  assert.deepEqual(failed.facts, ["free-model", "https://api.example.com/v1"], "失败时不显示耗时噪音");
});

test("配置体检：耗时与体检时间格式", () => {
  assert.equal(formatElapsed(null), "—");
  assert.equal(formatElapsed(Number.NaN), "—");
  assert.equal(formatElapsed(850.4), "850 ms");
  assert.equal(formatElapsed(1234), "1.2 s");
  assert.equal(formatCheckedAt(""), "");
  assert.equal(formatCheckedAt("not-a-date"), "");
  // 用本地时间构造，避免测试受时区影响。
  assert.equal(formatCheckedAt(new Date(2026, 10, 7, 9, 5, 42).toISOString()), "2026-11-07 09:05");
});

test("配置体检：面板按契约调用命令，本地体检不带 useAi，AI 体检带 useAi", () => {
  const panel = read("./ConfigDoctorPanel.tsx");
  assert.match(panel, /invoke<DoctorState>\("config_doctor_request", \{ action: "status" \}\)/, "进入时拉 status");
  assert.match(panel, /action: "save", payload: savedPayload\(target\)/);
  assert.match(panel, /action: "run", payload: useAi \? \{ useAi: true \} : \{\}/, "本地体检不传 useAi");
  assert.match(panel, /id="app-settings-doctor"/, "卡片 id 必须以 app-settings- 开头，设置页才显示");
  assert.ok(!/safety|policy/i.test(panel));
});

test("配置体检：面板三态——来源三选一、current/local 隐藏输入框、切来源立即保存", () => {
  const panel = read("./ConfigDoctorPanel.tsx");
  // 三选一来自纯函数，不是写死的一段。
  assert.match(panel, /sourceOptions\(state\?\.sources\)/);
  assert.match(panel, /role="radiogroup"/);
  // current 只显示后端整句，未就绪时补提示。
  assert.match(panel, /draft\.source === "current"/);
  assert.match(panel, /currentNote\.hint/);
  // local 未找到时给 Ollama 文档按钮。
  assert.match(panel, /draft\.source === "local"/);
  assert.match(panel, /ollamaDocsUrl\(presets\)/);
  assert.match(panel, /invoke\("open_external_url", \{ url \}\)/);
  // 预设下拉与三个输入框只在 custom 下渲染。
  assert.match(panel, /const isCustom = draft\.source === "custom"/);
  assert.match(panel, /isCustom \? \(/);
  assert.match(panel, /config-doctor-fields/);
  // 开关与来源变化都走保存流程。
  assert.match(panel, /chooseEnabled\(!draft\.enabled\)/);
  assert.match(panel, /onClick=\{\(\) => chooseSource\(option\.value\)\}/);
  // 隐私说明区分额度归属。
  assert.match(panel, /约几千 token/);
});
