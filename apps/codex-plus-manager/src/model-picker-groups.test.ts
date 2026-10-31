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
const CATALOG_URL = new URL(
  "../../../assets/inject/renderer-inject/70-model-catalog.js",
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
  assert.match(source, /groups\.length < 2 &&/);
  assert.match(source, /catalogGroups\.size < 2/);
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
  // 只有整行精确命中目录（含 `（后缀）` 的完整展示名）时，才用目录里的分组名做显示归属：
  // 网关把同名模型的后几家写成 `名字（供应商 id）`，标题要显示打过码的完整账号名；别名命中仍不决定归属。
  assert.match(source, /const provider = exactKnown\?\.provider \|\| split\.provider \|\| known\?\.provider/);
  // 原生缩写也要保留零宽消歧标记，再回退到宽松归一化。
  assert.match(source, /const model = codexPlusModelPickerRowModel\(row\)/);
  assert.match(source, /exact\.get\(`model\\u0000\$\{model\}`\)/);
  assert.match(source, /exact\.get\(`raw\\u0000\$\{identity\.text\}`\)\s*\|\| exact\.get\(codexPlusModelPickerNativeLabelKey\(identity\.text\)\)\s*\|\| exact\.get\(codexPlusModelPickerNormalizeLoose\(identity\.text\)\)/);
  assert.match(source, /const rawKey = `raw\\u0000\$\{String\(displayName\)/);
  // 组内任意一行命中默认分组即认定该组是默认（裸名行的 id 可能来自目录）。
  assert.match(source, /group\.entries\.some\(\(entry\) => entry\.groupId === defaultGroupId/);
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

it("在 Codex++ 里收藏的模型在选择器行尾画星标，取消收藏或还原时拿掉", async () => {
  const source = await readShard();
  assert.match(source, /favorite: entry\.codex_plus_favorite === true/);
  assert.match(source, /favorite: exactKnown\?\.favorite === true/);
  assert.match(source, /codexPlusModelPickerFavoriteClass = "codex-plus-model-group-favorite"/);
  // 星标节点带归属标记，扫描不会把它当成原生标签，也不会自喂循环。
  assert.match(source, /star\.setAttribute\("data-codex-plus-ext", codexPlusModelPickerGroupsExtMark\)/);
  assert.match(source, /content\.querySelector\(`\.\$\{codexPlusModelPickerFavoriteClass\}`\)\?\.remove\(\)/);
});

it("模型选择器用单行工具栏、明确默认模型和缩进后的分组模型", async () => {
  const source = await readShard();
  assert.match(source, /spacer\.textContent = .*text\.selectModel/);
  assert.match(source, /codexPlusModelPickerDefaultModel\(\)/);
  assert.match(source, /codexModelCatalog\.effective_config_model/);
  assert.match(source, /codexModelCatalog\?\.status !== "ok"/);
  assert.match(source, /aggregate_model_catalog", "aggregate_pool_catalog/);
  assert.match(source, /codex-plus-model-default-value/);
  assert.match(source, /composer\.modelPicker\.default\.label/);
  assert.match(source, /codex-plus-model-default-recommendation/);
  assert.match(source, /codex-plus-model-group-toolbar-title\{flex:1/);
  assert.match(source, /\[role="menuitemradio"\]\[data-codex-plus-model-group\]\{padding-left:30px/);
  assert.match(source, /codexPlusModelPickerSyncDefaultRow\(list,/);
  assert.doesNotMatch(source, /codexPlusModelPickerSelectDefault/);
  assert.match(source, /codexPlusModelPickerScheduleRescan\(\)/);
  assert.match(source, /兼容 v1 早期把供应商直接存成字符串/);
  assert.doesNotMatch(source, /codex-plus-model-recommendation/);
});

it("复用原生标题且默认供应商显示真实名称，热重注入更新样式", async () => {
  const source = await readShard();
  assert.match(source, /\[data-codex-plus-model-groups="true"\] > \[data-menu-section-label\]\{display:none!important\}/);
  assert.match(source, /if \(label\) label\.textContent = group\.name/);
  assert.doesNotMatch(source, /group\.isDefault \? "默认" : group\.name/);
  assert.match(source, /content\.querySelector\("\.flex-1"\)/);
  assert.doesNotMatch(source, /if \(!isDefault \|\| !model\) continue/);
  assert.match(source, /document\.getElementById\(codexPlusModelPickerGroupsStyleId\) \|\| document\.createElement/);
});

it("不搬动原生节点也让同组模型连续排列，目录刷新清除旧归属", async () => {
  const source = await readShard();
  const catalog = await readFile(CATALOG_URL, "utf8");
  assert.match(source, /order:var\(--codex-plus-model-order,0\)/);
  assert.match(source, /row\.style\.removeProperty\("--codex-plus-model-order"\)/);
  assert.match(source, /const fallbackId = `provider:/);
  assert.match(source, /providerIds\.has\(split\.provider\)/);
  assert.match(source, /state\.providerNames\.clear\(\)/);
  assert.match(catalog, /codexPlusModelPickerCatalogChanged\(\)/);
});

it("打开模型菜单时补读尚未就绪的供应商设置，再同步默认模型", async () => {
  const source = await readShard();
  const start = source.indexOf("  function codexPlusModelPickerEnsureCatalog() {");
  const end = source.indexOf("\n  /**", start);
  for (const loaded of [false, true]) {
    const calls: string[] = [];
    const state = { catalogPending: false, catalogRetryAt: 0 };
    const ensure = new Function(
      "codexPlusModelPickerGroupsState", "loadCodexModelCatalog",
      "loadBackendSettingsState", "codexPlusModelPickerScheduleRescan",
      "codexPlusBackendSettingsLoaded", "codexAppServerPreparationTimeoutMs",
      `${source.slice(start, end)}\nreturn codexPlusModelPickerEnsureCatalog;`,
    )(state, async () => calls.push("catalog"), async () => calls.push("settings"),
      () => calls.push("sync"), loaded, 2000);
    ensure();
    ensure();
    await new Promise<void>((resolve) => setImmediate(resolve));
    assert.deepEqual(calls, loaded ? ["catalog", "sync"] : ["settings", "catalog", "sync"]);
    assert.equal(state.catalogPending, false);
  }
});

it("多开禁用供应商切换后仍显示固定供应商默认模型，拒绝其他目录来源", async () => {
  const source = await readShard();
  const start = source.indexOf("  function codexPlusModelPickerDefaultModel() {");
  const end = source.indexOf("\n  function codexPlusModelPickerSyncDefaultRow", start);
  const settings = {
    activeRelayId: "fixed", relayProfilesEnabled: false,
    relayProfiles: [{ id: "fixed", relayMode: "pureApi" }],
  };
  const resolve = new Function(
    "codexPlusBackendSettingsLoaded",
    "codexRemoteSessionActiveProfile", "codexPlusBackendSettings", "codexModelCatalog",
    `${source.slice(start, end)}\nreturn codexPlusModelPickerDefaultModel();`,
  );
  const catalog = {
    status: "ok", model: "gpt-6.1-sol", effective_config_model: "gpt-6.1-sol", models: ["gpt-6.1-sol"],
    sources: [{ id: "relay-profile:fixed", type: "relay_profile_model_list" }],
  };
  assert.equal(resolve(true, () => null, settings, catalog), "gpt-6.1-sol");
  assert.equal(resolve(true, () => null, settings, {
    ...catalog, sources: [{ id: "relay-profile:other", type: "relay_profile_model_list" }],
  }), "");
  assert.equal(resolve(false, () => null, settings, catalog), "");
  assert.equal(resolve(true, () => null, settings, { ...catalog, status: "not_applied" }), "");
  assert.equal(resolve(true, () => null, settings, { ...catalog, effective_config_model: "unknown" }), "");
});

it("默认模型只显示已确认的配置值，恢复失败不强制切换", async () => {
  const picker = await readShard();
  const catalog = await readFile(CATALOG_URL, "utf8");
  assert.match(catalog, /function codexAggregateAllowedModelSet\(hostId, allowUnpublished = false\)/);
  assert.match(catalog, /function codexPlusConfiguredDefaultModel\(\)/);
  assert.match(catalog, /previous\.attempts >= 3/);
  assert.doesNotMatch(catalog, /function codexFallbackThreadModelSelection\(/);
  assert.match(catalog, /allowed\.has\(model\)/);
  assert.match(picker, /codexModelCatalog\.effective_config_model/);
  assert.match(picker, /aggregate_model_catalog", "aggregate_pool_catalog/);
});

it("菜单认不出时记一次结构诊断：只在真有目录里的模型时记，不带模型名（#2753）", async () => {
  const source = await readShard();
  const body = source.match(/^  function codexPlusModelPickerReportUnrecognized\(\) \{[^]*?^  \}/m)?.[0];
  assert.ok(body, "诊断函数必须存在");
  const element = (attributes: Record<string, string>, text = "", children: unknown[] = []) => ({
    textContent: text,
    attributes: Object.entries(attributes).map(([name, value]) => ({ name, value })),
    getAttribute: (name: string) => attributes[name] ?? null,
    closest: () => null,
    querySelectorAll: (selector: string) => (selector === '[role="menuitemradio"]' ? children : selector === "[class]" ? children : []),
  });
  const radios = [
    element({ role: "menuitemradio", "data-state": "checked", class: "_Row_abc" }, "GPT-6 Sol（官方）"),
    element({ role: "menuitemradio", class: "_Row_abc" }, "DeepSeek-V4-Pro（DeepSeek）"),
  ];
  const run = (menus: unknown[], catalog: Map<string, unknown>) => {
    const sent: Array<{ event: string; detail: Record<string, unknown> }> = [];
    const state: Record<string, unknown> = {};
    const report = new Function(
      "document", "codexPlusModelPickerGroupsState", "codexPlusModelPickerCatalogExactMap",
      "codexPlusModelPickerNormalizeLoose", "visibleElement", "sendCodexPlusDiagnostic",
      `${body}\nreturn codexPlusModelPickerReportUnrecognized;`,
    )(
      { querySelectorAll: (selector: string) => (selector === '[role="menu"]' ? menus : []) },
      state, () => catalog, (value: string) => value.toLowerCase(), () => true,
      (event: string, detail: Record<string, unknown>) => { sent.push({ event, detail }); },
    );
    report();
    report();
    return sent;
  };
  const catalog = new Map([["gpt-6 sol（官方）", {}]]);
  const menu = element({ role: "menu", "data-side": "top", "data-state": "open", class: "_Menu_x1" }, "", radios);

  const sent = run([menu], catalog);
  assert.equal(sent.length, 1, "每个页面只记一次");
  assert.equal(sent[0].event, "model_picker_groups_menu_unrecognized");
  assert.equal(sent[0].detail.radios, 2);
  assert.equal(sent[0].detail.catalogMatches, 1);
  assert.deepEqual(sent[0].detail.menuData, ["data-side", "data-state"]);
  assert.deepEqual(sent[0].detail.modulePrefixes, ["Menu", "Row"]);
  assert.doesNotMatch(JSON.stringify(sent[0].detail), /GPT|DeepSeek|官方/, "诊断里不能有模型名");

  assert.equal(run([menu], new Map()).length, 0, "目录里没有对应模型（例如别的单选菜单）时不记");
  assert.equal(run([element({ role: "menu" }, "", radios.slice(0, 1))], catalog).length, 0, "只有一个选项的菜单不记");
});
