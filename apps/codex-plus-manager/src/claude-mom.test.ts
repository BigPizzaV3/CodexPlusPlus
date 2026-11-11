import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { claudeState, emptyProfile, formatWindow, hostOf, normalizeModels, profileProblem, setupSteps, type ClaudeStatus } from "./claude-mom/model.ts";

const status = (patch: Partial<ClaudeStatus> = {}): ClaudeStatus => ({
  supported: true, sourceApp: "", version: "1.0", claudeRunning: false, running: false, connected: false,
  caReady: true, caTrusted: true, nodeAvailable: true, dictationReady: true, models: [], needsRestart: false,
  config: { enabled: true, model: "", localization: true, voiceInput: false, computerUse: false, backgroundMode: false },
  ...patch,
});

test("上游表单：要有效的 Base URL 和默认模型，不能写到 /v1/messages", () => {
  const profile = { ...emptyProfile(), baseUrl: "https://api.example.com", model: "glm-5" };
  assert.equal(profileProblem(profile), null);
  assert.equal(profileProblem({ ...profile, baseUrl: "" }), "填一个 Base URL");
  assert.equal(profileProblem({ ...profile, baseUrl: "api.example.com" }), "Base URL 不是有效地址");
  assert.equal(profileProblem({ ...profile, baseUrl: "ftp://x" }), "Base URL 要以 http:// 或 https:// 开头");
  assert.match(profileProblem({ ...profile, baseUrl: "https://x/v1/messages" }) ?? "", /不要带/);
  assert.equal(profileProblem({ ...profile, model: " " }), "选一个默认模型");
  assert.equal(hostOf("http://127.0.0.1:3425"), "127.0.0.1:3425");
});

test("模型列表去重，默认模型排第一", () => {
  assert.deepEqual(normalizeModels(["b", " a ", "b", ""], "a"), ["a", "b"]);
});

test("状态文案：已连接、运行中、外部打开、未启动", () => {
  assert.deepEqual(claudeState(null, null), { text: "正在检测 Claude…", tone: "busy" });
  assert.equal(claudeState(status({ connected: true }), null).tone, "ok");
  assert.equal(claudeState(status({ claudeRunning: true }), null).tone, "warn");
  assert.equal(claudeState(status(), null).text, "未启动");
  assert.equal(claudeState(status(), "launch").text, "正在启动…");
  assert.equal(claudeState(status({ supported: false }), null).text, "没有找到 Claude Desktop");
});

test("启动前清单按顺序列出缺的步骤", () => {
  const ready = { ...emptyProfile(), baseUrl: "https://a.example", model: "m" };
  assert.deepEqual(setupSteps(status(), ready), []);
  assert.deepEqual(setupSteps(status({ caTrusted: false, nodeAvailable: false }), null).map((step) => step.id), ["upstream", "node", "trust"]);
});

test("窗口显示", () => {
  assert.equal(formatWindow(null), "200k");
  assert.equal(formatWindow(1_000_000), "1M");
  assert.equal(formatWindow(1_100_000), "1.1M");
  assert.equal(formatWindow(400_000), "400k");
});

test("前端调用的命令都在后端注册了", () => {
  const app = readFileSync(new URL("./claude-mom/ClaudeMomApp.tsx", import.meta.url), "utf8");
  const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  // 命令名有的直接传给 invoke，有的经 bookCall 转一手：按字面量收。
  const called = new Set([...app.matchAll(/"(claude_(?:mom|local)_[a-z_]+)"/g)].map((match) => match[1]));
  assert.ok(called.size >= 9, [...called].join(","));
  for (const command of called) assert.ok(new RegExp(`::${command},`).test(lib), `${command} 没有注册`);
});
