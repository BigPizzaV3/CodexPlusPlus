import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 内置拓展「Hooks 编辑器」。
 *
 * 钉死：
 *   1. hook key 的事件段是 snake_case，格式 `<路径>:<事件>:<分组>:<序号>`（与 Codex hooks/list 一致）；
 *   2. 开关与信任记录按位置记，增删挪动后保存时把记录搬到新 key，删掉的清掉，新建的不继承旧记录；
 *   3. 编辑就地改 handler（保留未知字段），换事件或拆 matcher 时挪到事件末尾的新分组；
 *   4. 试运行结果按 Codex 规则解读（退出码 2、JSON deny、Stop 输出必须是 JSON）；
 *   5. 三处登记一致，写文件 / 执行命令的路由不开放给拓展。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/96-hooks-editor.js", import.meta.url);

type Doc = { hooks: Record<string, Array<{ matcher?: string; hooks: Array<Record<string, unknown>> }>>; [key: string]: unknown };
type Row = { event: string; groupIndex: number; handlerIndex: number; position: string; group: Doc["hooks"][string][number]; handler: Record<string, unknown>; editable: boolean };
type Edit = { keyPath: string; value: unknown; mergeStrategy: string };

async function runtime() {
  const source = await readFile(SHARD_URL, "utf8");
  return new Function(
    `${source}\nreturn {
      snake: codexPlusHooksEditorSnake,
      key: codexPlusHooksEditorKey,
      parse: codexPlusHooksEditorParse,
      rows: codexPlusHooksEditorRows,
      origins: codexPlusHooksEditorOrigins,
      migrate: codexPlusHooksEditorMigrationEdits,
      add: codexPlusHooksEditorAdd,
      remove: codexPlusHooksEditorRemove,
      update: codexPlusHooksEditorUpdate,
      applyFields: codexPlusHooksEditorApplyFields,
      serialize: codexPlusHooksEditorSerialize,
      templates: codexPlusHooksEditorTemplates,
      sample: codexPlusHooksEditorSampleInput,
      interpret: codexPlusHooksEditorInterpret,
      listHooks: codexPlusHooksEditorListHooks,
      keyPrefix: codexPlusHooksEditorKeyPrefix,
      events: codexPlusHooksEditorEvents,
      extension: codexPlusHooksEditorExtension,
    };`,
  )() as {
    snake(event: string): string;
    key(prefix: string, position: string): string;
    parse(text: string): { doc: Doc | null; error: string };
    rows(doc: Doc): Row[];
    origins(doc: Doc): Map<object, string>;
    migrate(input: { doc: Doc; origins: Map<object, string>; prefix: string; state: Record<string, unknown> }): Edit[];
    add(doc: Doc, input: { event: string; matcher?: string; handler: Record<string, unknown> }): Record<string, unknown>;
    remove(doc: Doc, handler: object): boolean;
    update(doc: Doc, row: Row, input: { event: string; matcher: string; fields: Record<string, unknown> }): Record<string, unknown>;
    applyFields(handler: Record<string, unknown>, fields: Record<string, unknown>): Record<string, unknown>;
    serialize(doc: Doc): string;
    templates(platform?: string): Array<{ id: string; event: string; matcher: string; command: string; timeout: number }>;
    sample(event: string, options?: { cwd?: string; matcher?: string }): string;
    interpret(event: string, result: Record<string, unknown>): { tone: string; lines: string[] };
    listHooks(client: unknown, cwds: string[]): Promise<{ hooks: Map<string, Record<string, unknown>>; warnings: string[] }>;
    keyPrefix(file: Record<string, unknown> | null, hooks: Array<{ sourcePath?: string }>): string;
    events: Array<{ name: string }>;
    extension(ctx: unknown): void;
  };
}

const P = "/u/.codex/hooks.json";
const keyPath = (position: string, api: Awaited<ReturnType<typeof runtime>>) => `hooks.state.${JSON.stringify(api.key(P, position))}`;

function sampleDoc(): Doc {
  return {
    extra: { keep: true },
    hooks: {
      Stop: [{ hooks: [{ type: "command", command: "a" }, { type: "command", command: "b", custom: 1 }] }],
      PreToolUse: [{ matcher: "^Bash$", hooks: [{ type: "command", command: "c" }] }],
    },
  };
}

it("事件名转 snake_case，key 与 Codex 的格式一致", async () => {
  const api = await runtime();
  assert.equal(api.snake("PreToolUse"), "pre_tool_use");
  assert.equal(api.snake("UserPromptSubmit"), "user_prompt_submit");
  assert.equal(api.snake("Stop"), "stop");
  assert.equal(api.key("/a/hooks.json", "PostToolUse:1:0"), "/a/hooks.json:post_tool_use:1:0");
  assert.equal(api.events.length, 14);
});

it("解析：空文本当空文档，结构不对时报错交给原文编辑", async () => {
  const api = await runtime();
  assert.deepEqual(api.parse("").doc, { hooks: {} });
  assert.deepEqual(api.parse("{}").doc, { hooks: {} });
  assert.match(api.parse("{oops").error, /不是合法的 JSON/);
  assert.match(api.parse("[]").error, /对象/);
  assert.match(api.parse('{"hooks":[]}').error, /hooks 必须是对象/);
  assert.match(api.parse('{"hooks":{"Stop":{}}}').error, /hooks\.Stop 必须是数组/);
});

it("按文件顺序列出 handler 和位置，非 command 类型不可编辑", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  doc.hooks.Stop[0].hooks.push({ type: "prompt", prompt: "x" });
  const rows = api.rows(doc);
  assert.deepEqual(rows.map((row) => row.position), ["Stop:0:0", "Stop:0:1", "Stop:0:2", "PreToolUse:0:0"]);
  assert.deepEqual(rows.map((row) => row.editable), [true, true, false, true]);
});

it("没有改动时不发任何搬迁", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  assert.deepEqual(api.migrate({ doc, origins, prefix: P, state: { [api.key(P, "Stop:0:0")]: { enabled: false } } }), []);
});

it("删除前面的 handler：后面的记录前移，旧位置清掉", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  const first = doc.hooks.Stop[0].hooks[0];
  const state = {
    [api.key(P, "Stop:0:0")]: { enabled: false },
    [api.key(P, "Stop:0:1")]: { enabled: false, trusted_hash: "sha256:b" },
  };
  assert.ok(api.remove(doc, first));
  const edits = api.migrate({ doc, origins, prefix: P, state });
  assert.deepEqual(edits, [
    { keyPath: keyPath("Stop:0:0", api), value: { enabled: false, trusted_hash: "sha256:b" }, mergeStrategy: "replace" },
    { keyPath: keyPath("Stop:0:1", api), value: null, mergeStrategy: "replace" },
  ]);
});

it("删到分组和事件都空时一并删掉", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  api.remove(doc, doc.hooks.PreToolUse[0].hooks[0]);
  assert.equal(doc.hooks.PreToolUse, undefined);
  assert.equal(api.remove(doc, {}), false);
});

it("新建的 handler 不继承落点上残留的记录", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  api.add(doc, { event: "PreToolUse", matcher: "", handler: { type: "command", command: "new" } });
  const stale = { [api.key(P, "PreToolUse:1:0")]: { enabled: false, trusted_hash: "sha256:old" } };
  const edits = api.migrate({ doc, origins, prefix: P, state: stale });
  assert.deepEqual(edits, [{ keyPath: keyPath("PreToolUse:1:0", api), value: null, mergeStrategy: "replace" }]);
  // 新分组没有 matcher 字段（留空匹配全部）。
  assert.deepEqual(doc.hooks.PreToolUse[1], { hooks: [{ type: "command", command: "new" }] });
  assert.throws(() => api.add(doc, { event: "NoSuchEvent", handler: {} }), /未知的事件/);
});

it("编辑同事件同 matcher：就地改，保留未知字段，空值删字段", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  const row = api.rows(doc)[1];
  row.handler.timeout = 5;
  const handler = api.update(doc, row, { event: "Stop", matcher: "", fields: { command: "b2", timeout: "", statusMessage: " ", async: true } });
  assert.equal(handler, row.handler, "对象身份不变");
  assert.deepEqual(doc.hooks.Stop[0].hooks[1], { type: "command", command: "b2", custom: 1, async: true });
  assert.deepEqual(api.migrate({ doc, origins, prefix: P, state: {} }), []);
});

it("改 matcher 时分组只有它一个就改分组；共用分组则拆出去，记录跟着搬", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  const pre = api.rows(doc).find((row) => row.event === "PreToolUse")!;
  api.update(doc, pre, { event: "PreToolUse", matcher: "", fields: { command: "c" } });
  assert.equal(doc.hooks.PreToolUse[0].matcher, undefined, "单独一组：就地删掉 matcher");

  const shared = api.rows(doc)[0];
  const state = { [api.key(P, "Stop:0:0")]: { enabled: false } };
  api.update(doc, shared, { event: "Stop", matcher: "x", fields: { command: "a" } });
  assert.deepEqual(doc.hooks.Stop.map((group) => group.matcher ?? ""), ["", "x"]);
  const edits = api.migrate({ doc, origins, prefix: P, state });
  // a 从 Stop:0:0 挪到 Stop:1:0，b 从 Stop:0:1 前移到 Stop:0:0。
  assert.deepEqual(new Map(edits.map((edit) => [edit.keyPath, edit.value])), new Map<string, unknown>([
    [keyPath("Stop:0:0", api), null],
    [keyPath("Stop:1:0", api), { enabled: false }],
  ]));
});

it("换事件：挪到目标事件末尾", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  const row = api.rows(doc)[0];
  api.update(doc, row, { event: "SessionStart", matcher: "", fields: { command: "a" } });
  assert.deepEqual(doc.hooks.SessionStart, [{ hooks: [{ type: "command", command: "a" }] }]);
  const state = { [api.key(P, "Stop:0:0")]: { enabled: false } };
  const edits = api.migrate({ doc, origins, prefix: P, state });
  assert.deepEqual(new Map(edits.map((edit) => [edit.keyPath, edit.value])), new Map<string, unknown>([
    [keyPath("Stop:0:0", api), null],
    [keyPath("SessionStart:0:0", api), { enabled: false }],
  ]));
});

it("两条互换位置时记录对调，不会被清掉", async () => {
  const api = await runtime();
  const doc = sampleDoc();
  const origins = api.origins(doc);
  doc.hooks.Stop[0].hooks.reverse();
  const state = { [api.key(P, "Stop:0:0")]: { enabled: false }, [api.key(P, "Stop:0:1")]: { enabled: true } };
  const edits = api.migrate({ doc, origins, prefix: P, state });
  assert.deepEqual(new Map(edits.map((edit) => [edit.keyPath, edit.value])), new Map<string, unknown>([
    [keyPath("Stop:0:0", api), { enabled: true }],
    [keyPath("Stop:0:1", api), { enabled: false }],
  ]));
});

it("序列化保留未知顶层字段，两空格缩进加换行", async () => {
  const api = await runtime();
  const text = api.serialize(sampleDoc());
  assert.ok(text.endsWith("}\n"));
  assert.deepEqual(JSON.parse(text).extra, { keep: true });
});

it("模板：事件合法、matcher 是合法正则，三个平台都有命令", async () => {
  const api = await runtime();
  const names = new Set(api.events.map((event) => event.name));
  for (const platform of ["mac", "linux", "windows"]) {
    const templates = api.templates(platform);
    assert.ok(templates.length >= 4);
    for (const template of templates) {
      assert.ok(names.has(template.event), template.id);
      assert.ok(template.command.trim(), template.id);
      new RegExp(template.matcher);
    }
  }
  const stop = api.templates("mac").find((template) => template.event === "Stop")!;
  assert.doesNotMatch(stop.command, /exit 2/, "Stop 模板不能用退出码 2，会死循环");
});

it("示例事件：公共字段齐全，按事件补字段，matcher 是工具名时用它", async () => {
  const api = await runtime();
  const pre = JSON.parse(api.sample("PreToolUse", { cwd: "/p", matcher: "^apply_patch$" }));
  assert.equal(pre.hook_event_name, "PreToolUse");
  assert.equal(pre.cwd, "/p");
  assert.equal(pre.tool_name, "apply_patch");
  assert.ok(pre.tool_input.command);
  assert.equal(JSON.parse(api.sample("PreToolUse", { matcher: "Bash|Write" })).tool_name, "Bash");
  assert.ok("tool_response" in JSON.parse(api.sample("PostToolUse")));
  assert.equal(JSON.parse(api.sample("Stop")).stop_hook_active, false);
  assert.equal(JSON.parse(api.sample("UserPromptSubmit")).prompt.length > 0, true);
  assert.equal(JSON.parse(api.sample("SessionStart")).source, "startup");
});

it("试运行结果按 Codex 规则解读", async () => {
  const api = await runtime();
  assert.equal(api.interpret("Stop", { exitCode: 0, stdout: "", stderr: "" }).tone, "ok");
  assert.match(api.interpret("Stop", { exitCode: 2, stdout: "", stderr: "" }).lines.join(), /死循环/);
  assert.equal(api.interpret("Stop", { exitCode: 0, stdout: "done", stderr: "" }).tone, "error");
  assert.match(api.interpret("PreToolUse", { exitCode: 2, stdout: "", stderr: "危险" }).lines.join(), /拦截.*危险/);
  const deny = api.interpret("PreToolUse", {
    exitCode: 0,
    stdout: JSON.stringify({ hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: "no" } }),
  });
  assert.equal(deny.tone, "warn");
  assert.match(deny.lines.join(), /deny.*no/);
  const bare = api.interpret("PreToolUse", { exitCode: 0, stdout: '{"hookSpecificOutput":{"permissionDecision":"deny"}}' });
  assert.equal(bare.tone, "error");
  assert.match(api.interpret("SessionStart", { exitCode: 0, stdout: "branch main" }).lines.join(), /上下文/);
  assert.equal(api.interpret("PostToolUse", { exitCode: 1, stdout: "" }).tone, "error");
  assert.equal(api.interpret("Stop", { timedOut: true, timeoutSec: 3 }).tone, "error");
});

it("hooks/list 多个目录的结果按 key 合并，警告去重", async () => {
  const api = await runtime();
  const calls: unknown[] = [];
  const client = {
    async sendRequest(method: string, params: unknown) {
      calls.push([method, params]);
      return {
        data: [
          { cwd: "/a", hooks: [{ key: "u:stop:0:0", n: 1 }], warnings: ["w"], errors: [] },
          { cwd: "/b", hooks: [{ key: "u:stop:0:0", n: 2 }, { key: "p:stop:0:0" }], warnings: ["w"], errors: [{ message: "e" }] },
        ],
      };
    },
  };
  const { hooks, warnings } = await api.listHooks(client, ["/a", "/b"]);
  assert.deepEqual(calls, [["hooks/list", { cwds: ["/a", "/b"] }]]);
  assert.deepEqual([...hooks.keys()], ["u:stop:0:0", "p:stop:0:0"]);
  assert.equal(hooks.get("u:stop:0:0")!.n, 1);
  assert.deepEqual(warnings, ["w", "e"]);
});

it("注册为 Codex 设置页分区", async () => {
  const api = await runtime();
  const captured: { definition?: { id?: string; title?: string; render?: unknown } } = {};
  api.extension({
    registerSettingsSection(value: { id?: string; title?: string; render?: unknown }) {
      captured.definition = value;
      return () => {};
    },
  });
  assert.equal(captured.definition?.id, "hooks-editor");
  assert.equal(captured.definition?.title, "Hooks 编辑器");
  assert.equal(typeof captured.definition?.render, "function");
});

it("内置拓展清单、设置清单与分片清单三处登记一致", async () => {
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
  const registry = JSON.parse(await read("../../../crates/codex-plus-core/assets/enhancement-settings.json")) as {
    builtins: Array<{ id: string }>;
  };
  assert.ok(registry.builtins.some((item) => item.id === "hooks-editor"));
  const builtins = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(builtins, /id: "hooks-editor"/);
  assert.match(builtins, /codexPlusHooksEditorExtension/);
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json")) as {
    fragments: Array<{ name: string }>;
  };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const at = names.indexOf("96-hooks-editor.js");
  assert.ok(at >= 0 && at < names.indexOf("96-builtin-extensions.js"));
});

it("key 前缀以 hooks/list 的 sourcePath 为准；没有时用户层取规范化路径、项目层取原样路径", async () => {
  const api = await runtime();
  const user = { scope: "user", path: "/tmp/h/hooks.json", canonicalPath: "/private/tmp/h/hooks.json" };
  const project = { scope: "project", path: "/tmp/p/.codex/hooks.json", canonicalPath: "/private/tmp/p/.codex/hooks.json" };
  assert.equal(api.keyPrefix(user, []), "/private/tmp/h/hooks.json");
  assert.equal(api.keyPrefix(project, []), "/tmp/p/.codex/hooks.json");
  assert.equal(api.keyPrefix(project, [{ sourcePath: "/x" }, { sourcePath: "/private/tmp/p/.codex/hooks.json" }]), "/private/tmp/p/.codex/hooks.json");
  assert.equal(api.keyPrefix(null, []), "");
});
