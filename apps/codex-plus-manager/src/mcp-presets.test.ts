import assert from "node:assert/strict";
import test from "node:test";

import { MCP_PRESETS, mcpPresetById } from "./mcp-presets.ts";

test("npx 预设在 Windows 上经 cmd /c 执行", () => {
  const preset = mcpPresetById("context7");
  assert.ok(preset);

  const windows = preset.tomlBody({ windows: true });
  assert.match(windows, /^command = "cmd"$/m);
  assert.match(windows, /^args = \["\/c", "npx", "-y", "@upstash\/context7-mcp"\]$/m);

  const unix = preset.tomlBody({ windows: false });
  assert.match(unix, /^command = "npx"$/m);
  assert.match(unix, /^args = \["-y", "@upstash\/context7-mcp"\]$/m);
});

test("uvx 预设不分平台", () => {
  const preset = mcpPresetById("fetch");
  assert.ok(preset);
  assert.equal(preset.tomlBody({ windows: true }), preset.tomlBody({ windows: false }));
  assert.match(preset.tomlBody({ windows: false }), /^command = "uvx"$/m);
});

test("预设的 TOML 表体只含表头之下的内容", () => {
  for (const preset of MCP_PRESETS) {
    for (const windows of [true, false]) {
      const body = preset.tomlBody({ windows });
      // 表体会被拼到 [mcp_servers.<id>] 之下，自己不能再带表头
      assert.ok(!body.includes("["+"mcp_servers"), `${preset.id} 不该自带表头`);
      assert.match(body, /^command = /m, `${preset.id} 缺 command`);
      assert.ok(body.endsWith("\n"), `${preset.id} 应以换行结尾`);
    }
  }
});

test("预设 id 唯一且非空", () => {
  const ids = MCP_PRESETS.map((preset) => preset.id);
  assert.equal(new Set(ids).size, ids.length);
  assert.ok(ids.every((id) => id.length > 0));
});
