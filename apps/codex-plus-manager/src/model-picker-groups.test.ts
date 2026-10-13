import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

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

it("默认标志不改变无障碍语义：读屏会念出「默认供应商」", async () => {
  const source = await readShard();
  assert.match(source, /（默认供应商）/);
  assert.match(source, /\(default provider\)/);
});
