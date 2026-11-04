import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";

const root = path.resolve(import.meta.dirname, "../../..");
const template = readFileSync(path.join(root, "assets/claude-extensions/register.js"), "utf8");
const nativeTest = `
import { expect, test } from 'claude-code/testing';

test('native desktop button, tool and session bridge', async ($, on) => {
  const server = 'plugin:codexpp-extension-host:codexpp-extensions';
  const calls = [];
  const tools = [];
  const toasts = [];
  const errors = [];
  on('mcp.connect', (_, e) => {
    expect(e.server).toBe('codexpp-extensions');
    return { value: { isConnected: true, server } };
  });
  on('mcp.call', (_, e) => {
    expect(e.server).toBe(server);
    expect(e.tool).toBe('codexpp_extension_call');
    calls.push(e.args);
    return {
      value: {
        content: [{ type: 'text', text: JSON.stringify({ text: 'Native OK' }) }],
        isError: false,
      },
    };
  });
  on('session.id', () => ({ value: 'native-session' }));
  on('tool.register', (_, e) => {
    tools.push(e);
    return { value: { tool: 'mcp__codexpp-extension-host__' + e.name } };
  });
  on('ui.toast', (_, e) => {
    toasts.push(e.text);
    return { value: undefined };
  });
  on('ui.log', (_, e) => {
    errors.push(e.text);
    return { value: undefined };
  });
  on('session.start', () => ({ cwd: '/work' }));
  on('classic.SessionStart', () => ({}));
  on('turn.start', (_, e) => ({ turnId: e.turnId }));
  on('turn.complete', () => ({ text: '' }));
  on('session.end', () => ({ sessionId: 'native-session' }));
  on('tool.call', () => ({ result: 'Existing tool' }));
  on('ui.render', () => ({ type: 'Text', props: {}, children: ['Existing UI'] }));

  await $.session.start({ surface: 'desktop', isInteractive: true, cwd: '/work' });
  expect(tools[0].name).toBe('cpx_0_lookup');
  expect(calls[0]).toMatchObject({
    key: 'native-validation', method: 'onSessionStart', sessionId: 'native-session',
    params: { host: 'claudeDesktop', event: 'session.start', sessionId: 'native-session' },
  });
  await $.turn.start({ turnId: 'turn-1' });
  expect(calls.at(-1).method).toBe('onTurnStart');
  await $.turn.complete({
    turnId: 'turn-1', answer: 'Finished', durationMs: 5, isAborted: false, usage: null,
  });
  expect(calls.at(-1)).toMatchObject({
    method: 'onTurnComplete', params: { event: 'turn.complete', turnId: 'turn-1', isAborted: false },
  });
  await $.classic.SessionStart({ source: 'clear' });
  expect(calls.at(-1)).toMatchObject({
    method: 'onSessionStart', params: { event: 'session.start', source: 'clear' },
  });
  const ui = await $.ui.mount({
    plugin: 'codexpp-extension-host',
    component: 'AbovePrompt',
    requestId: 'native-band',
    surface: 'desktop',
    viewport: { columns: 100, rows: 30 },
    props: {
      hasSurvey: false, isWorking: false, maxRows: 8, bodyColumns: 100,
      scroll: { offset: 0, bodyRows: 8 }, view: {},
    },
  });
  expect(await ui.find({ type: 'Text', text: 'Existing UI' })).toBeDefined();
  expect(await ui.find({ key: 'native-validation:run' })).toBeDefined();
  await ui.press({ key: 'native-validation:run' });
  expect(calls.at(-1)).toMatchObject({
    method: 'run',
    params: { host: 'claudeDesktop', source: 'button', buttonId: 'run', sessionId: 'native-session' },
  });
  expect(toasts).toEqual(['Native OK']);
  await ui.unmount();
  const tool = await $.tool.call({
    tool: 'mcp__codexpp-extension-host__cpx_0_lookup', id: 'item-1', tool_use_id: 'call-1',
  });
  expect(tool.result).toBe('Native OK');
  expect(calls.at(-1)).toMatchObject({ method: 'lookup', params: { id: 'item-1' } });
  expect(calls.at(-1).params.tool_use_id).toBeUndefined();
  const untouched = await $.tool.call({ tool: 'Read', file_path: '/work/file' });
  expect(untouched.result).toBe('Existing tool');
  await $.session.end({ reason: 'clear', sessionId: 'native-session', resume: 'claude --resume native-session' });
  expect(calls.at(-1)).toMatchObject({ method: 'onSessionEnd', params: { event: 'session.end', reason: 'clear' } });
  expect(calls.length).toBe(7);
  expect(errors).toEqual([]);
});
`;

test("Claude Code 原生分析和测试引擎验证按钮、工具及会话闭环", t => {
  const cli = process.env.CODEX_PLUS_CLAUDE_VALIDATE_CLI || "claude";
  const version = spawnSync(cli, ["--version"], { encoding: "utf8", timeout: 10_000 });
  const match = version.stdout?.match(/(\d+)\.(\d+)\.(\d+)/);
  if (version.error || version.status !== 0 || !match) {
    t.skip("本机未安装 Claude Code CLI，跳过官方静态分析");
    return;
  }
  const components = match.slice(1).map(Number);
  const supportedVersion = components[0] > 2 || (components[0] === 2
    && (components[1] > 1 || (components[1] === 1 && components[2] >= 314)));
  if (!supportedVersion) {
    t.skip("本机 Claude Code CLI 低于本适配已验证的 2.1.314");
    return;
  }

  const plugin = mkdtempSync(path.join(os.tmpdir(), "codexpp-claude-mod-validation-"));
  mkdirSync(path.join(plugin, ".claude-plugin"));
  mkdirSync(path.join(plugin, "hooks"));
  writeFileSync(path.join(plugin, ".claude-plugin/plugin.json"), JSON.stringify({
    name: "codexpp-extension-host",
    version: "0.1.0",
    description: "CodexPlusPlus extension adapter for Claude Desktop",
    author: { name: "CodexPlusPlus" },
  }));
  writeFileSync(path.join(plugin, "hooks/hooks.json"), JSON.stringify({ modules: ["./register.js"] }));
  writeFileSync(path.join(plugin, ".mcp.json"), JSON.stringify({
    mcpServers: {
      "codexpp-extensions": {
        command: process.execPath,
        args: ["-e", "throw new Error('Static validation must not start the backend')"],
      },
    },
  }));
  const config = {
    serverName: "plugin:codexpp-extension-host:codexpp-extensions",
    toolName: "codexpp_extension_call",
    extensions: [{
      key: "native-validation",
      id: "native-validation",
      name: "Native validation",
      buttons: [{ id: "run", label: "Run", method: "run" }],
      events: {
        "session.start": "onSessionStart",
        "turn.start": "onTurnStart",
        "turn.complete": "onTurnComplete",
        "session.end": "onSessionEnd",
      },
      tools: [{
        name: "lookup",
        description: "Look up an item",
        method: "lookup",
        inputSchema: {
          type: "object",
          properties: { id: { type: "string" } },
          required: ["id"],
          additionalProperties: false,
        },
      }],
    }],
  };
  writeFileSync(
    path.join(plugin, "hooks/register.js"),
    template.replace("/*CODEXPP_CONFIG*/{}", JSON.stringify(config)),
  );
  mkdirSync(path.join(plugin, "tests"));
  writeFileSync(path.join(plugin, "tests/native.test.ts"), nativeTest);
  const result = spawnSync(cli, ["plugin", "validate", plugin, "--strict", "--json"], {
    encoding: "utf8",
    timeout: 30_000,
  });
  assert.equal(result.error, undefined);
  assert.equal(result.status, 0, result.stdout + result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.success, true, result.stdout);
  for (const event of ["session.start", "classic.SessionStart", "turn.start", "turn.complete", "session.end", "tool.call", "ui.render"]) {
    assert.ok(result.stdout.includes(event), `官方静态分析遗漏事件 ${event}: ${result.stdout}`);
  }
  for (const method of ["mcp.call", "mcp.connect", "session.id", "tool.register", "ui.resolve", "ui.toast"]) {
    assert.ok(result.stdout.includes(method), `官方静态分析遗漏调用 ${method}: ${result.stdout}`);
  }
  const execution = spawnSync(cli, ["plugin", "test", plugin], {
    encoding: "utf8",
    timeout: 30_000,
  });
  assert.equal(execution.error, undefined);
  assert.equal(execution.status, 0, execution.stdout + execution.stderr);
  assert.match(execution.stdout + execution.stderr, /1 pass/);
});
