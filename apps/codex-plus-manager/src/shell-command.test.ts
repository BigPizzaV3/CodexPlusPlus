import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 内置拓展「! 命令」。
 *
 * 钉死：
 *   1. 只认 `!` 开头且后面有内容的输入；
 *   2. app-server 推来的 userShell 条目按 commandActions 或包过 shell 的 command 对上等待中的命令，
 *      之后按条目 id 更新；别的会话、agent 自己跑的命令不算；
 *   3. 输出只留末尾一段；
 *   4. 三处登记一致。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/96-shell-command.js", import.meta.url);

type Run = { command: string; status: string; itemId: string; output: string; exitCode: number | null; startedAt: number; durationMs?: number; trimmed?: boolean };

async function runtime() {
  const source = await readFile(SHARD_URL, "utf8");
  return new Function(
    `${source}\nreturn {
      parse: codexPlusShellCommandParse,
      apply: codexPlusShellCommandApplyItem,
      matches: codexPlusShellCommandMatches,
      trim: codexPlusShellCommandTrimOutput,
      status: codexPlusShellCommandStatusText,
      max: codexPlusShellCommandMaxOutput,
    };`,
  )() as {
    parse(text: string): string;
    apply(runs: Map<string, Run>, threadId: string, item: unknown, phase: string): Run | null;
    matches(item: unknown, command: string): boolean;
    trim(output: string): { text: string; trimmed: boolean };
    status(run: Partial<Run> & { error?: string }): string;
    max: number;
  };
}

const running = (command: string): Run => ({ command, status: "running", itemId: "", output: "", exitCode: null, startedAt: Date.now() });
const item = (overrides: Record<string, unknown>) => ({
  type: "commandExecution",
  id: "item-1",
  source: "userShell",
  status: "inProgress",
  command: "/bin/zsh -lc 'echo hi'",
  commandActions: [{ type: "unknown", command: "echo hi" }],
  aggregatedOutput: null,
  exitCode: null,
  durationMs: null,
  ...overrides,
});

it("只认 ! 开头且有命令的输入", async () => {
  const api = await runtime();
  assert.equal(api.parse("!ls -la"), "ls -la");
  assert.equal(api.parse("  ! git status  "), "git status");
  assert.equal(api.parse("!echo a\nb"), "echo a\nb");
  assert.equal(api.parse("!"), "");
  assert.equal(api.parse("!   "), "");
  assert.equal(api.parse("ls"), "");
  assert.equal(api.parse("请看 !ls"), "");
});

it("按 commandActions 或包过 shell 的 command 对上命令", async () => {
  const api = await runtime();
  assert.equal(api.matches(item({}), "echo hi"), true);
  assert.equal(api.matches(item({ commandActions: [] }), "echo hi"), true, "zsh -lc '...'");
  assert.equal(api.matches(item({ commandActions: [], command: "bash -c 'echo hi'" }), "echo hi"), true);
  assert.equal(api.matches(item({ commandActions: [], command: "echo hi" }), "echo hi"), true);
  assert.equal(api.matches(item({}), "echo other"), false);
});

it("开始时记下条目 id，结束时写入输出、退出码与耗时", async () => {
  const api = await runtime();
  const runs = new Map([["t1", running("echo hi")]]);
  const started = api.apply(runs, "t1", item({}), "started");
  assert.equal(started?.itemId, "item-1");
  assert.equal(started?.status, "running");
  // 结束时只认 id：即便命令文字不同（不会发生，但防御）。
  const done = api.apply(runs, "t1", item({ status: "completed", aggregatedOutput: "hi\n", exitCode: 0, durationMs: 22, commandActions: [] , command: "x" }), "completed");
  assert.equal(done?.status, "completed");
  assert.equal(done?.output, "hi\n");
  assert.equal(done?.exitCode, 0);
  assert.equal(done?.durationMs, 22);
  assert.match(api.status(done!), /退出码 0/);
  const failed = api.apply(new Map([["t1", { ...running("false"), itemId: "x" }]]), "t1", item({ id: "x", status: "failed", exitCode: 1, aggregatedOutput: "" }), "completed");
  assert.equal(failed?.status, "failed");
  assert.equal(failed?.exitCode, 1);
});

it("不认别的会话、agent 跑的命令、已结束后的同名命令", async () => {
  const api = await runtime();
  const runs = new Map([["t1", running("echo hi")]]);
  assert.equal(api.apply(runs, "t2", item({}), "started"), null, "别的会话");
  assert.equal(api.apply(runs, "t1", item({ source: "agent" }), "started"), null, "agent 自己的命令");
  assert.equal(api.apply(runs, "t1", { type: "agentMessage" }, "started"), null);
  const finished = new Map([["t1", { ...running("echo hi"), status: "completed", itemId: "old" }]]);
  assert.equal(api.apply(finished, "t1", item({ id: "new" }), "started"), null, "上一条已结束、新条目 id 不同");
});

it("输出只留末尾一段", async () => {
  const api = await runtime();
  assert.deepEqual(api.trim("short"), { text: "short", trimmed: false });
  const long = `${"a".repeat(api.max)}TAIL`;
  const trimmed = api.trim(long);
  assert.equal(trimmed.trimmed, true);
  assert.equal(trimmed.text.length, api.max);
  assert.ok(trimmed.text.endsWith("TAIL"));
});

it("内置拓展清单、设置清单与分片清单三处登记一致", async () => {
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
  const registry = JSON.parse(await read("../../../crates/codex-plus-core/assets/enhancement-settings.json")) as { builtins: Array<{ id: string }> };
  assert.ok(registry.builtins.some((entry) => entry.id === "shell-command"));
  const builtins = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(builtins, /id: "shell-command"/);
  assert.match(builtins, /codexPlusShellCommandExtension/);
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json")) as { fragments: Array<{ name: string }> };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const at = names.indexOf("96-shell-command.js");
  assert.ok(at >= 0 && at < names.indexOf("96-builtin-extensions.js"));
});
