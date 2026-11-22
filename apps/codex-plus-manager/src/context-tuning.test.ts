import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 内置拓展「上下文与压缩」。
 *
 * 钉死：
 *   1. config/read 的生效值（include_* 默认 true）→ 「没设置」用 null 表示；
 *   2. 草稿校验范围，超范围不写（batchWrite 不查范围，写进去 Codex 就拒载）；
 *   3. 只写改过的键，删除用 replace + null；
 *   4. 回读不一致或读不了时把改过的键写回原值；
 *   5. 四处登记一致。
 */

const SHARD_URL = new URL("../../../assets/inject/renderer-inject/96-context-tuning.js", import.meta.url);

type Values = Record<string, unknown>;
type Edit = { key: string; value: unknown };

async function runtime() {
  const source = await readFile(SHARD_URL, "utf8");
  return new Function(
    `${source}\nreturn {
      fields: codexPlusContextTuningFields,
      fromConfig: codexPlusContextTuningValuesFromConfig,
      draftFrom: codexPlusContextTuningDraftFromValues,
      parse: codexPlusContextTuningParseDraft,
      edits: codexPlusContextTuningEdits,
      batch: codexPlusContextTuningBatchEdits,
      apply: codexPlusContextTuningApply,
      templates: codexPlusContextTuningTemplates,
    };`,
  )() as {
    fields: Array<{ key: string; kind: string }>;
    fromConfig(config: unknown): Values;
    draftFrom(values: Values): Values;
    parse(draft: Values): { values: Values; errors: Record<string, string> };
    edits(loaded: Values, values: Values): Edit[];
    batch(edits: Edit[]): Array<{ keyPath: string; value: unknown; mergeStrategy: string }>;
    apply(client: unknown, edits: Edit[], previous: Values): Promise<Values>;
    templates: Array<{ id: string; text: string }>;
  };
}

const defaults = {
  compact_prompt: null,
  model_post_turn_compact_threshold_percent: null,
  tool_output_token_limit: null,
  include_environment_context: true,
  include_environment_context_time: true,
  include_permissions_instructions: true,
  include_apps_instructions: true,
  include_collaboration_mode_instructions: true,
};

it("生效值里的默认 true 读成「没设置」，false 才算不发送", async () => {
  const api = await runtime();
  const values = api.fromConfig({ ...defaults, include_permissions_instructions: false, tool_output_token_limit: 200, compact_prompt: "  " });
  assert.equal(values.include_environment_context, null);
  assert.equal(values.include_permissions_instructions, false);
  assert.equal(values.tool_output_token_limit, 200);
  assert.equal(values.compact_prompt, null, "空白提示词等于没设置");
  const draft = api.draftFrom(values);
  assert.equal(draft.include_permissions_instructions, true, "勾上「不发送」");
  assert.equal(draft.include_environment_context, false);
  assert.equal(draft.tool_output_token_limit, "200");
  assert.equal(draft.model_post_turn_compact_threshold_percent, "");
});

it("草稿校验范围与格式", async () => {
  const api = await runtime();
  const base = api.draftFrom(api.fromConfig(defaults));
  assert.deepEqual(api.parse(base).errors, {});
  for (const [key, text] of [
    ["model_post_turn_compact_threshold_percent", "0"],
    ["model_post_turn_compact_threshold_percent", "150"],
    ["model_post_turn_compact_threshold_percent", "80%"],
    ["tool_output_token_limit", "-5"],
    ["tool_output_token_limit", "1.5"],
  ]) {
    assert.ok(api.parse({ ...base, [key]: text }).errors[key], `${key}=${text}`);
  }
  const ok = api.parse({ ...base, model_post_turn_compact_threshold_percent: " 85 ", compact_prompt: "  摘要  " });
  assert.equal(ok.values.model_post_turn_compact_threshold_percent, 85);
  assert.equal(ok.values.compact_prompt, "摘要");
  assert.ok(api.parse({ ...base, compact_prompt: "字".repeat(20001) }).errors.compact_prompt);
});

it("只写改过的键，删除用 replace + null", async () => {
  const api = await runtime();
  const loaded = api.fromConfig({ ...defaults, tool_output_token_limit: 200, include_apps_instructions: false });
  const draft = { ...api.draftFrom(loaded), tool_output_token_limit: "", include_environment_context: true, compact_prompt: "保留待办" };
  const edits = api.edits(loaded, api.parse(draft).values);
  assert.deepEqual(edits, [
    { key: "compact_prompt", value: "保留待办" },
    { key: "tool_output_token_limit", value: null },
    { key: "include_environment_context", value: false },
  ]);
  assert.deepEqual(api.batch(edits), [
    { keyPath: "compact_prompt", value: "保留待办", mergeStrategy: "upsert" },
    { keyPath: "tool_output_token_limit", value: null, mergeStrategy: "replace" },
    { keyPath: "include_environment_context", value: false, mergeStrategy: "upsert" },
  ]);
  assert.deepEqual(api.edits(loaded, api.parse(api.draftFrom(loaded)).values), [], "原样不算改动");
});

function fakeClient(config: Values, { breakAfterWrite = false } = {}) {
  const writes: unknown[] = [];
  let broken = false;
  return {
    writes,
    sendRequest: async (method: string, params: { edits: Array<{ keyPath: string; value: unknown }> }) => {
      if (method === "config/batchWrite") {
        writes.push(params.edits);
        for (const edit of params.edits) {
          config[edit.keyPath] = edit.value ?? (edit.keyPath.startsWith("include_") ? true : null);
        }
        broken = breakAfterWrite && writes.length === 1;
        return {};
      }
      if (method === "config/read") {
        if (broken) throw new Error("invalid config");
        return { config: { ...config } };
      }
      throw new Error(method);
    },
  };
}

it("写入后回读一致就返回新配置", async () => {
  const api = await runtime();
  const client = fakeClient({ ...defaults });
  const config = await api.apply(client, [{ key: "tool_output_token_limit", value: 300 }], api.fromConfig(defaults));
  assert.equal(config.tool_output_token_limit, 300);
  assert.equal(client.writes.length, 1);
});

it("回读失败时把改过的键写回原值", async () => {
  const api = await runtime();
  const client = fakeClient({ ...defaults, tool_output_token_limit: 200 }, { breakAfterWrite: true });
  const previous = api.fromConfig({ ...defaults, tool_output_token_limit: 200 });
  await assert.rejects(api.apply(client, [{ key: "tool_output_token_limit", value: 999 }], previous), /已恢复原值/);
  assert.equal(client.writes.length, 2);
  assert.deepEqual(client.writes[1], [{ keyPath: "tool_output_token_limit", value: 200, mergeStrategy: "upsert" }]);
});

it("模板非空，四处登记一致", async () => {
  const api = await runtime();
  assert.ok(api.templates.length >= 2 && api.templates.every((template) => template.text.length > 50));
  const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
  const registry = JSON.parse(await read("../../../crates/codex-plus-core/assets/enhancement-settings.json")) as { builtins: Array<{ id: string; title: string; description: string }> };
  const entry = registry.builtins.find((item) => item.id === "context-tuning");
  assert.ok(entry);
  const builtins = await read("../../../assets/inject/renderer-inject/96-builtin-extensions.js");
  assert.match(builtins, /id: "context-tuning".*codexPlusContextTuningExtension/);
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json")) as { fragments: Array<{ name: string }> };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const at = names.indexOf("96-context-tuning.js");
  assert.ok(at >= 0 && at < names.indexOf("96-builtin-extensions.js"));
  const english = await read("./i18n-en.ts");
  assert.ok(english.includes(`"${entry.title}"`) && english.includes(`"${entry.description}"`));
});
