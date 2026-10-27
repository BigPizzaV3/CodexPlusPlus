import assert from "node:assert/strict";
import test from "node:test";
import { filterCommands, type PaletteCommand } from "./command-palette-model.ts";

const command = (id: string, group: string, label: string, keywords = ""): PaletteCommand => ({ id, group, label, keywords, run: () => undefined });
const commands = [
  command("a", "网关", "供应商池", "gatewayProviders gateway"),
  command("b", "Codex", "供应商配置", "relay codex"),
  command("c", "Claude Code", "供应商配置", "claudeRelay claude"),
  command("d", "操作", "重启 Codex", "codex restart"),
];

test("空查询返回全部", () => {
  assert.equal(filterCommands(commands, "  ").length, 4);
});

test("多个词都要命中，可以跨标签、分组与关键词", () => {
  assert.deepEqual(filterCommands(commands, "供应商").map((item) => item.id), ["a", "b", "c"]);
  assert.deepEqual(filterCommands(commands, "claude 供应商").map((item) => item.id), ["c"]);
  assert.deepEqual(filterCommands(commands, "RESTART").map((item) => item.id), ["d"]);
  assert.deepEqual(filterCommands(commands, "不存在"), []);
});
