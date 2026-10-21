import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";
import vm from "node:vm";

/**
 * 模型选择器分组折叠（聚合供应商场景）的契约测试。
 *
 * 这层跑不了真 DOM（仓库的注入层测试都是"读取分片源码 + 断言结构"这种风格，
 * 浏览器行为由 renderer-inject.test.ts 里的静态契约 + 人工/CDP 实测覆盖）。
 * 所以这里钉死三件事：
 *   1. 与后端约定的字段名不能漂（`codex_plus_group` / `codex_plus_group_id`）；
 *   2. 影响别人的行为约束：不搬动 React 管理的行、隐藏原生标签要退出无障碍树、
 *      单供应商不接管、聚合目录里"默认成员"那一组要能看懂；
 *   3. 自己的扫描/清理路径不能漏（否则会闪、会自喂扫描循环）。
 */

const SHARD_URL = new URL(
  "../../../assets/inject/renderer-inject/71-model-picker-groups.js",
  import.meta.url,
);
const MANIFEST_URL = new URL(
  "../../../assets/inject/renderer-inject/manifest.json",
  import.meta.url,
);

async function readShard() {
  return await readFile(SHARD_URL, "utf8");
}

it("未发布聚合目录仅提供默认行显示和显式选择，不放宽自动迁移", async () => {
  const source = await readShard();
  const block = source.match(/  function codexPlusModelPickerDefaultModel\(\)[^]*?\n  \}/)?.[0];
  assert.ok(block);
  const catalog = {
    status: "not_applied", model: "configured", default_model: "configured", models: ["configured"],
    sources: [{ id: "aggregate:active", type: "aggregate_model_catalog" }],
  };
  let profile = { id: "active", relayMode: "aggregate" };
  const resolve = vm.runInNewContext(`(() => { ${block}; return codexPlusModelPickerDefaultModel; })()`, {
    codexPlusConfiguredDefaultModel: () => "",
    codexModelCatalog: catalog, codexPlusBackendSettingsLoaded: true,
    codexRemoteSessionActiveProfile: () => profile,
    codexPlusModelNames: () => ["configured"],
  });
  assert.equal(resolve(), "configured");
  profile = { id: "other", relayMode: "aggregate" };
  assert.equal(resolve(), "", "旧供应商目录不能污染新配置");
  profile = { id: "active", relayMode: "aggregate" };
  catalog.status = "failed";
  assert.equal(resolve(), "", "失败目录不能用于默认项");
  catalog.status = "not_applied";
  catalog.model = catalog.default_model = "unknown";
  assert.equal(resolve(), "", "配置默认值必须属于当前目录");
});

it("默认行通过菜单关联的触发器匹配本地选择器，不猜测无标识菜单", async () => {
  const source = await readShard();
  const block = source.match(/  function codexPlusModelPickerLocalPickerForList\(list\)[^]*?\n  \}/)?.[0];
  assert.ok(block);
  const picker = { node: { id: "local-trigger" }, picker: {} };
  const resolve = vm.runInNewContext(`(() => { ${block}; return codexPlusModelPickerLocalPickerForList; })()`, {
    codexAggregateModelPickers: () => [picker],
  });
  const list = (label: string) => ({ closest: () => ({ getAttribute: () => label }) });
  assert.equal(resolve(list("local-trigger")), picker);
  assert.equal(resolve(list("other-label local-trigger")), picker);
  assert.equal(resolve(list("remote-trigger")), null);
  assert.equal(resolve(list("")), null);
  assert.equal(resolve(null), null);
});

it("默认动作行不继承原生默认勾号，当前会话标记落在具体模型行", async () => {
  const source = await readShard();
  assert.match(source, /codexPlusModelPickerCurrentIdentity/);
  assert.match(source, /codexPlusModelPickerSyncCurrentRow/);
  assert.match(source, /codex-plus-model-default-suppress-selection svg\{display:none!important\}/);
  assert.match(source, /codex-plus-model-current-synthetic/);
  assert.match(source, /selected\?\.picker/);
  assert.match(source, /\.codex-plus-model-current-row"\) \|\| list\.querySelector\("\[data-model-selected\]"\)/);
});

it("补丁默认行在目录失败或归属不明时阻止旧推荐动作，未打补丁的行保持原生行为", async () => {
  const source = await readShard();
  const block = source.match(/  function codexPlusModelPickerSelectConfiguredDefault\(event\)[^]*?\n  \}/)?.[0];
  assert.ok(block);
  let target = "";
  let selected: unknown = { picker: { disabled: false, models: [{ model: "configured" }] } };
  const handle = vm.runInNewContext(`(() => { ${block}; return codexPlusModelPickerSelectConfiguredDefault; })()`, {
    codexPlusModelPickerGroupsState: { disposed: false },
    codexPlusModelPickerDefaultModel: () => target,
    codexPlusModelPickerLocalPickerForList: () => selected,
  });
  const row = { getAttribute: () => null, closest: () => ({}) };
  const event = (type: string, key?: string, patched = true) => {
    const calls = { prevented: 0, stopped: 0 };
    return { type, key, target: { closest: () => patched ? row : null },
      preventDefault: () => calls.prevented++, stopImmediatePropagation: () => calls.stopped++, calls };
  };
  for (const type of ["click", "keydown"]) {
    const failed = event(type, "Enter");
    handle(failed);
    assert.deepEqual(failed.calls, { prevented: 1, stopped: 1 });
    selected = null;
    target = "configured";
    const remote = event(type, " ");
    handle(remote);
    assert.deepEqual(remote.calls, { prevented: 1, stopped: 1 });
    const native = event(type, "Enter", false);
    handle(native);
    assert.deepEqual(native.calls, { prevented: 0, stopped: 0 });
    target = "";
    selected = { picker: { disabled: false, models: [{ model: "configured" }] } };
  }
});

it("分组折叠分片在 manifest 里排在模型目录之后、并在组装产物里", async () => {
  const manifest = JSON.parse(await readFile(MANIFEST_URL, "utf8")) as {
    fragments: Array<{ name: string }>;
  };
  const names = manifest.fragments.map((fragment) => fragment.name);
  const index = names.indexOf("71-model-picker-groups.js");
  assert.ok(index > 0, "分片必须登记进 manifest.json");
  assert.ok(
    names.indexOf("70-model-catalog.js") < index,
    "要排在 70-model-catalog.js 之后：它依赖 codexModelCatalog 快照",
  );
  assert.ok(
    names.indexOf("99-tail.js") > index,
    "必须排在 IIFE 收尾之前，否则代码不在主闭包里",
  );

  const artifact = await readFile(
    new URL("../../../assets/inject/renderer-inject.js", import.meta.url),
    "utf8",
  );
  assert.match(artifact, /codexPlusModelPickerGroupsRuntime/, "组装产物里要有本分片");
});

it("分组折叠读的字段名与后端发布一致", async () => {
  const source = await readShard();
  // 后端 aggregate_catalog.rs 发布 / model_suffix.rs 放行的两个键。
  assert.match(source, /entry\.codex_plus_group\b/);
  assert.match(source, /entry\.codex_plus_group_id\b/);
});

it("不搬动 React 管理的行，只用类名隐藏", async () => {
  const source = await readShard();
  // 行必须留在原位：只允许插标题/工具条、加类名和属性。
  assert.doesNotMatch(source, /appendChild\(entry\.row\)|appendChild\(row\)/);
  assert.doesNotMatch(source, /\.row\.remove\(\)/);
  assert.match(source, /codex-plus-model-group-collapsed\{display:none!important\}/);
  // 折叠状态必须落在行自己的类名上，而不是重排 DOM。
  assert.match(source, /classList\.toggle\("codex-plus-model-group-collapsed"/);
});

it("隐藏原生标签时同步退出无障碍树，避免读屏念两遍", async () => {
  const source = await readShard();
  assert.match(source, /native\.setAttribute\("aria-hidden", "true"\)/);
  assert.match(source, /native\.removeAttribute\("aria-hidden"\)/);
});

it("只有一个供应商时不接管原生列表", async () => {
  const source = await readShard();
  assert.match(source, /if \(groups\.length < 2\) \{/);
  // 单组时要能退回原始状态。
  assert.match(source, /codexPlusModelPickerClear\(list\)/);
  assert.match(source, /list\.removeAttribute\("data-codex-plus-model-groups"\)/);
});

it("默认成员的裸名模型归到默认供应商分组，不产生空标题", async () => {
  const source = await readShard();
  // 分组名解析：优先 codex_plus_group，其次 description 的 "上游 · 供应商"。
  assert.match(source, /const separator = description\.lastIndexOf\(" · "\)/);
  assert.match(source, /codexPlusModelPickerDefaultProviderName/);
  // 空名兜底成固定 id，避免折叠状态串组。
  assert.match(source, /group\.id = "__ungrouped__"/);
});

it("扫描器自己插的节点都带归属标记，避免自喂扫描循环", async () => {
  const source = await readShard();
  assert.match(
    source,
    /registerCodexPlusExtensionSelector\('\[data-codex-plus-ext="builtin-model-picker-groups"\]'\)/,
    "必须登记扩展选择器，否则扫描调度器会把我们插的标题当成 Codex 原生节点",
  );
  const marks = source.match(/data-codex-plus-ext", codexPlusModelPickerGroupsExtMark/g) ?? [];
  assert.ok(marks.length >= 4, "标题、工具条、行、标签都要打标记");
});

it("菜单关闭或组件重建后能自愈，不留下清不掉的残留", async () => {
  const source = await readShard();
  // React 回收兄弟节点后标题会消失，指纹没变也必须重建。
  assert.match(source, /codexPlusModelPickerHeadersIntact/);
  assert.match(source, /header\.nextElementSibling === first/);
  // 恢复时要摘干净自己加的类名与属性。
  assert.match(source, /row\.removeAttribute\(codexPlusModelPickerGroupAttr\)/);
  assert.match(source, /native\.removeAttribute\("data-codex-plus-ext"\)/);
});

it("重注入时与上一个实例交接，不先拆再建造成闪烁", async () => {
  const source = await readShard();
  assert.match(source, /window\.__codexPlusModelPickerGroupsRuntime\?\.dispose\?\.\(\)/);
  const dispose = source.match(/dispose\(\) \{[\s\S]*?\n    \},/)?.[0];
  assert.ok(dispose, "要有 dispose 实现");
  assert.doesNotMatch(dispose, /codexPlusModelPickerClear/, "dispose 不能顺手拆 DOM：重注入会闪");
  assert.match(source, /releaseDom\(\)/, "彻底还原要有单独出口");
});

it("折叠状态按分组 id 持久化，并且有中文文案", async () => {
  const source = await readShard();
  assert.match(source, /codexPlus\.modelPicker\.collapsedGroups\.v1/);
  assert.match(source, /全部展开/);
  assert.match(source, /全部折叠/);
  // 读屏文案要带展开/折叠状态。
  assert.match(source, /groupAria/);
});

it("默认供应商带「默认」标志，身份来自目录而不是猜名字", async () => {
  const source = await readShard();
  // 顶层 id 优先，其次目录条目的 default 标记，最后才是会话内记住的值。
  assert.match(source, /codex_plus_default_group_id/);
  assert.match(source, /codex_plus_default\b/);
  assert.match(source, /defaultGroupId/);
  // 标志要能随扫描补齐/摘掉，而不是建标题时定一次。
  assert.match(source, /codexPlusModelPickerSyncDefaultBadge/);
  assert.match(source, /codex-plus-model-group-default/);
  assert.match(source, /defaultBadge: "默认"/);
  // 默认分组在 id 去重改写之前先认出来，且按组内任意一行命中判定。
  assert.match(source, /group\.isDefault = !!defaultGroupId/);
  // 没有选中模型时优先展开默认供应商。
  assert.match(source, /keep = groups\.find\(\(group\) => group\.isDefault\)\?\.id/);
});

it("目录条目按别名匹配，且只用它补分组 id、不用它决定显示归属", async () => {
  const source = await readShard();
  // 目录的展示名（GPT-6.1-Sol）与选择器里的行（6.1 Sol）不一样，必须登记别名。
  assert.match(source, /codexPlusModelPickerCatalogAliases/);
  assert.match(source, /replace\(\/\^gpt\[-\\s\]\*\/iu, ""\)/);
  // 早期版本的教训：拿目录展示名去拟合行文本会把同一家切成好几段。
  assert.match(source, /const groupId = known\?\.groupId/);
  assert.match(source, /const provider = split\.provider \|\| known\?\.provider/);
  // 组内任意一行命中默认分组即认定该组是默认（裸名行的 id 可能来自目录）。
  assert.match(source, /group\.entries\.some\(\(entry\) => entry\.groupId === defaultGroupId/);
});

it("目录热替换会清除旧默认组和旧供应商映射并安排重扫", async () => {
  const source = await readShard();
  const changed = source.match(/function codexPlusModelPickerCatalogChanged\(\) \{[\s\S]*?\n  \}/)?.[0];
  assert.ok(changed, "要提供目录替换通知入口");
  assert.match(changed, /state\.defaultGroupId = ""/);
  assert.match(changed, /state\.providerNames\.clear\(\)/);
  assert.match(changed, /state\.signature = ""/);
  assert.match(changed, /codexPlusModelPickerScheduleRescan\(\)/);
});

it("表头右端对齐：标题撑满剩余空间，不靠 auto 外边距推计数", async () => {
  const source = await readShard();
  // 回归：用 `margin-left:auto` 把计数推到右边时，多出「默认」徽标的那一行 auto 外边距
  // 会被吃掉，数字紧跟在徽标后面，与其它行错位。
  assert.match(source, /codex-plus-model-group-title\{flex:1 1 auto/);
  assert.doesNotMatch(
    source,
    /codex-plus-model-group-count\{flex:0 0 auto;margin-left:auto/,
    "计数不能再用 margin-left:auto 定位",
  );
});

it("默认标志不改变无障碍语义：读屏会念出「默认供应商」", async () => {
  const source = await readShard();
  assert.match(source, /（默认供应商）/);
  assert.match(source, /\(default provider\)/);
});

async function groupingRuntime(catalog: any) {
  const source = await readShard();
  const names = [
    "providerNameOf", "codexPlusModelPickerSplitLabel",
    "codexPlusModelPickerNormalizeStrict", "codexPlusModelPickerNormalize",
    "codexPlusModelPickerCatalogAliases", "codexPlusModelPickerCatalogProviderMap",
    "codexPlusModelPickerRows", "codexPlusModelPickerGroups",
  ];
  const functions = names.map((name) => {
    const match = source.match(new RegExp(`  function ${name}\\([^]*?\\n  \\}`));
    assert.ok(match, name);
    return match[0];
  }).join("\n");
  return vm.runInNewContext(`(() => {${functions}; return {
    rows: (labels) => codexPlusModelPickerRows({querySelectorAll: () => labels}),
    groups: codexPlusModelPickerGroups,
    providers: codexPlusModelPickerCatalogProviderMap
  };})()`, {
    codexModelCatalog: catalog,
    codexPlusModelPickerGroupsState: { providerNames: new Map([["5.5", { name: "Old", parsed: false }]]) },
    codexPlusModelPickerGroupNameMaxLength: 24,
    codexPlusModelPickerRememberProvider: () => {},
    codexPlusModelPickerRowIdentity: (text: string) => ({ text, groupable: true }),
    codexPlusModelPickerDefaultGroupId: () => "a",
    codexPlusModelPickerDefaultProviderName: () => "Alpha",
  });
}

it("display aliases and newly added models share their authoritative provider group", async () => {
  const metadata = (displayName: string) => ({
    displayName, codex_plus_group: "Beta", codex_plus_group_id: "b",
  });
  const runtime = await groupingRuntime({
    models: ["cpx-b/gpt-6.1-sol", "cpx-b/gpt-5.5", "cpx-b/gpt-test"],
    modelMetadata: {
      "cpx-b/gpt-6.1-sol": metadata("GPT-6.1-Sol (Beta)"),
      "cpx-b/gpt-5.5": metadata("GPT-5.5 (Beta)"),
      "cpx-b/gpt-test": metadata("gpt-test (Beta)"),
    },
  });
  const groups = runtime.groups(runtime.rows(["6.1 Sol (Beta)", "5.5 (Beta)", "gpt-test (Beta)"]));
  assert.equal(groups.length, 1);
  assert.equal(groups[0].id, "b");
  assert.equal(groups[0].entries.length, 3);
});

it("same model names resolve by provider and stale metadata outside the catalog is ignored", async () => {
  const runtime = await groupingRuntime({
    models: ["gpt-5.5", "cpx-b/gpt-5.5"],
    modelMetadata: {
      "gpt-5.5": { displayName: "GPT-5.5", codex_plus_group: "Alpha", codex_plus_group_id: "a" },
      "cpx-b/gpt-5.5": { displayName: "GPT-5.5 (Beta)", codex_plus_group: "Beta", codex_plus_group_id: "b" },
      "cpx-old/kimi": { displayName: "Kimi (Old)", codex_plus_group: "Old", codex_plus_group_id: "old" },
    },
  });
  const rows = runtime.rows(["5.5 (Alpha)", "5.5 (Beta)"]);
  assert.deepEqual(Array.from(rows, (row: any) => row.groupId), ["a", "b"]);
  assert.equal(runtime.providers().has("kimi"), false);
});
