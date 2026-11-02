import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

// Codex「拓展」详情页（41-extension-detail.js）：README 的 Markdown 渲染、页签、设置表单、功能清单与右侧栏。
const fragment = readFileSync(new URL("../../../assets/inject/renderer-inject/41-extension-detail.js", import.meta.url), "utf8");

function escapeHtml(value: unknown) {
  return String(value).replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;").replaceAll('"', "&quot;").replaceAll("'", "&#39;");
}

type Script = Record<string, any>;

function runtime(options: { scripts?: Script[]; panels?: Record<string, unknown>; post?: (path: string, payload: any) => Promise<any> } = {}) {
  const posts: Array<[string, any]> = [];
  const saved: Array<[string, any]> = [];
  const readmeNodes: Array<{ key: string; innerHTML: string; getAttribute: (name: string) => string }> = [];
  const document = {
    querySelectorAll(selector: string) {
      assert.equal(selector, "[data-codex-extensions-readme]");
      return readmeNodes;
    },
  };
  const api = new Function(
    "escapeHtml", "postJson", "document", "codexPlusUserScripts", "codexPlusSettingsPanelFor", "codexPlusExtensionConstants", "userScriptStatusLabel", "saveCodexPlusExtensionConfig",
    `${fragment}
return { renderCodexPlusMarkdown, renderCodexPlusExtensionDetailTabs, codexPlusExtensionHasSettings, saveCodexPlusExtensionConfigField,
  selectCodexPlusExtensionsDetailTab, resetCodexPlusExtensionsDetailTab, clearCodexPlusExtensionReadmes, tab: () => codexPlusExtensionsDetailTab };`,
  )(
    escapeHtml,
    async (path: string, payload: any) => {
      posts.push([path, payload]);
      return options.post ? options.post(path, payload) : { status: "ok", markdown: "", assets: {} };
    },
    document,
    { scripts: options.scripts || [] },
    (key: string) => options.panels?.[key] || null,
    { extensionAttribute: "data-codex-plus-ext" },
    (status: string) => ({ loaded: "已加载", disabled: "已禁用" } as Record<string, string>)[status] || status,
    async (key: string, values: any) => {
      saved.push([key, values]);
      return values;
    },
  );
  const addReadmeNode = (key: string) => {
    const node = { key, innerHTML: "", getAttribute: () => key };
    readmeNodes.push(node);
    return node;
  };
  return { ...api, posts, saved, addReadmeNode };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

test("README 的 Markdown：常用语法都渲染，原始 HTML 当文字，危险链接与外链图片不放行", () => {
  const { renderCodexPlusMarkdown: md } = runtime();
  const html = md([
    "# 便签 *拓展*",
    "",
    "介绍 **粗体**、_斜体_、~~删除~~、`<code>` 和 snake_case_name。",
    "第二行  ",
    "换行后",
    "",
    "- 一",
    "  - 嵌套",
    "- [x] 已完成",
    "- [ ] 未完成",
    "",
    "3. 三",
    "4. 四",
    "",
    "> 引用 [文档](https://example.test/docs?a=1&b=2)",
    "",
    "```js",
    "const a = '<script>';",
    "```",
    "",
    "| 名称 | 值 |",
    "| :--- | ---: |",
    "| a | `1` |",
    "",
    "---",
    "",
    "<script>alert(1)</script><!-- 隐藏的注释 -->",
    "[点我](javascript:alert(1)) [跳到用法](#用法) <https://auto.test/x> 裸链接 https://bare.test/p.",
    "![截图](./assets/shot.png) ![外链](http://insecure.test/a.png) ![徽章](https://img.test/b.svg)",
    "",
    "用法",
    "===",
  ].join("\n"), { assets: { "assets/shot.png": "data:image/png;base64,AAAA" } });

  assert.match(html, /<h1 id="codex-plus-md-便签-拓展">便签 <em>拓展<\/em><\/h1>/);
  assert.match(html, /<strong>粗体<\/strong>、<em>斜体<\/em>、<del>删除<\/del>、<code>&lt;code&gt;<\/code> 和 snake_case_name。/);
  assert.match(html, /第二行<br \/>换行后/);
  assert.match(html, /<ul><li>一<ul><li>嵌套<\/li><\/ul><\/li><li class="codex-plus-markdown-task"><input type="checkbox" disabled checked \/> 已完成<\/li><li class="codex-plus-markdown-task"><input type="checkbox" disabled \/> 未完成<\/li><\/ul>/);
  assert.match(html, /<ol start="3"><li>三<\/li><li>四<\/li><\/ol>/);
  assert.match(html, /<blockquote><p>引用 <a href="https:\/\/example\.test\/docs\?a=1&amp;b=2" target="_blank" rel="noreferrer">文档<\/a><\/p><\/blockquote>/);
  assert.match(html, /<pre><code data-lang="js">const a = &#39;&lt;script&gt;&#39;;<\/code><\/pre>/);
  assert.match(html, /<th style="text-align:left">名称<\/th><th style="text-align:right">值<\/th>/);
  assert.match(html, /<td style="text-align:right"><code>1<\/code><\/td>/);
  assert.match(html, /<hr \/>/);
  assert.ok(!html.includes("<script>"), "原始 HTML 不能进页面");
  assert.match(html, /&lt;script&gt;alert\(1\)&lt;\/script&gt;/);
  assert.ok(!html.includes("隐藏的注释"), "HTML 注释直接去掉");
  assert.ok(!html.includes("javascript:"), "javascript: 链接只留文字");
  assert.match(html, /点我 <a href="#" data-codex-plus-md-anchor="用法">跳到用法<\/a>/);
  assert.match(html, /<a href="https:\/\/auto\.test\/x" target="_blank" rel="noreferrer">https:\/\/auto\.test\/x<\/a>/);
  assert.match(html, /<a href="https:\/\/bare\.test\/p" target="_blank" rel="noreferrer">https:\/\/bare\.test\/p<\/a>\./);
  assert.match(html, /<img src="data:image\/png;base64,AAAA" alt="截图" loading="lazy" \/>/);
  assert.ok(!html.includes("insecure.test"), "http 图片不加载，只留替代文字");
  assert.match(html, /外链 <img src="https:\/\/img\.test\/b\.svg" alt="徽章"/);
  assert.match(html, /<h1 id="codex-plus-md-用法">用法<\/h1>/);
});

test("Markdown：属性里的引号与缩进代码块都转义，未知的相对图片不加载", () => {
  const { renderCodexPlusMarkdown: md } = runtime();
  const html = md('[x](https://a.test/"onmouseover="alert(1)) ![y](assets/missing.png)\n\n    <b>code</b>\n');
  assert.ok(!html.includes('"onmouseover'), html);
  assert.ok(!html.includes("missing.png"));
  assert.match(html, /<pre><code>&lt;b&gt;code&lt;\/b&gt;<\/code><\/pre>/);
});

const pkgScript: Script = {
  key: "user:notes",
  name: "便签",
  kind: "package",
  source: "user",
  enabled: true,
  status: "loaded",
  source_url: "https://example.test/notes.zip",
  package: {
    version: "1.2.0",
    readme: true,
    renderer: 2,
    tools: ["notes_add"],
    hookEvents: ["Stop"],
    routes: ["/diagnostics/log"],
    backend: true,
    codex: { skills: ["notes-skill"], agents: [], mcpServers: [], prompts: [], instructions: false },
    configSchema: [
      { key: "title", type: "string", title: { zh: "标题", en: "Title" }, description: "显示在顶部", default: "" },
      { key: "token", type: "string", title: "Token", secret: true, default: "" },
      { key: "count", type: "number", title: "数量", min: 1, max: 9, default: 3 },
      { key: "dark", type: "boolean", title: "深色", default: false },
      { key: "mode", type: "select", title: "模式", options: [{ value: "a", label: "甲" }, { value: 2, label: "乙" }], default: "a" },
      { key: "tags", type: "array", title: "标签", items: { type: "string" }, default: [] },
    ],
    config: { title: "我的<便签>", token: "s3cret", count: 5, dark: true, mode: 2, tags: ["x"] },
  },
};

test("已安装的拓展包：细节 / 设置 / 功能三个页签，README 按需读一次，右侧栏是安装信息", async () => {
  const rt = runtime({ scripts: [pkgScript], post: async () => ({ status: "ok", markdown: "# 便签\n\n说明", assets: {}, truncated: true }) });
  const node = rt.addReadmeNode("user:notes");
  const html = rt.renderCodexPlusExtensionDetailTabs({ local: pkgScript, marketItem: null, sourceLabel: "用户" });
  assert.deepEqual([...html.matchAll(/role="tab"[^>]*data-codex-extensions-tab="(\w+)">([^<]+)</g)].map((match) => [match[1], match[2]]), [["readme", "细节"], ["settings", "设置"], ["features", "功能"]]);
  assert.match(html, /aria-selected="true" data-codex-extensions-tab="readme"/);
  assert.match(html, /data-codex-extensions-tabpanel="settings" hidden>/);
  assert.match(html, /正在读取 README…/);
  assert.match(html, /<dt>标识符<\/dt><dd>user:notes<\/dd><dt>版本<\/dt><dd>1\.2\.0<\/dd><dt>来源<\/dt><dd>用户<\/dd><dt>状态<\/dt><dd>已加载<\/dd>/);
  assert.match(html, /<a href="https:\/\/example\.test\/notes\.zip" target="_blank" rel="noreferrer">源文件<\/a>/);
  // 功能页签列出清单接管的能力。
  assert.match(html, /<th scope="row">页面脚本<\/th><td>2 个文件<\/td>/);
  assert.match(html, /<th scope="row">模型工具<\/th><td>notes_add<\/td>/);
  assert.match(html, /<th scope="row">Codex 钩子<\/th><td>Stop<\/td>/);
  assert.match(html, /<th scope="row">Skills<\/th><td>notes-skill<\/td>/);
  assert.match(html, /<th scope="row">设置项<\/th><td>标题、Token、数量、深色、模式、标签<\/td>/);
  await flush();
  assert.deepEqual(rt.posts, [["/user-scripts/readme", { key: "user:notes" }]]);
  assert.match(node.innerHTML, /<div class="codex-plus-markdown"><h1 id="codex-plus-md-便签">便签<\/h1><p>说明<\/p><\/div>/);
  assert.match(node.innerHTML, /README 太长/);
  // 已经读过就不再请求；刷新清掉缓存后重新读。
  rt.renderCodexPlusExtensionDetailTabs({ local: pkgScript, marketItem: null, sourceLabel: "用户" });
  await flush();
  assert.equal(rt.posts.length, 1);
  rt.clearCodexPlusExtensionReadmes();
  rt.renderCodexPlusExtensionDetailTabs({ local: pkgScript, marketItem: null, sourceLabel: "用户" });
  await flush();
  assert.equal(rt.posts.length, 2);
});

test("设置页签：按清单 config 生成表单，值与类型都对；运行中的拓展登记了面板就交给面板", () => {
  const rt = runtime({ scripts: [pkgScript] });
  const html = rt.renderCodexPlusExtensionDetailTabs({ local: { ...pkgScript, package: { ...pkgScript.package, readme: false } }, marketItem: null, sourceLabel: "用户" });
  assert.match(html, /<label for="codex-plus-ext-config-0">标题<\/label>/);
  assert.match(html, /type="text"[^>]*data-codex-ext-config-field="title"[^>]*value="我的&lt;便签&gt;"/);
  assert.match(html, /type="password"[^>]*data-codex-ext-config-field="token"/);
  assert.match(html, /type="number"[^>]*data-codex-ext-config-field="count" data-codex-ext-config-type="number" min="1" max="9" value="5"/);
  assert.match(html, /class="codex-plus-toggle"[^>]*data-codex-ext-config-field="dark"[^>]*data-enabled="true"/);
  assert.match(html, /<option value="&quot;a&quot;">甲<\/option><option value="2" selected>乙<\/option>/);
  assert.match(html, /<textarea[^>]*data-codex-ext-config-field="tags"[^>]*>\[\n  &quot;x&quot;\n\]<\/textarea>/);
  assert.match(html, /这个拓展没有提供 README/);
  assert.ok(!html.includes("data-codex-plus-settings-panel"));

  const withPanel = runtime({ scripts: [pkgScript], panels: { "user:notes": { render() {} } } });
  const panelHtml = withPanel.renderCodexPlusExtensionDetailTabs({ local: pkgScript, marketItem: null, sourceLabel: "用户" });
  assert.match(panelHtml, /<div class="codex-plus-extensions-detail-settings-host" data-codex-plus-settings-panel="user:notes" data-codex-plus-ext="user:notes"><\/div>/);
  assert.ok(!panelHtml.includes("data-codex-ext-config-field"), "有面板时不再画通用表单");

  // 停用的拓展：自己的设置界面不在跑，提示启用，同时给通用表单。
  const disabled = { ...pkgScript, enabled: false, package: { ...pkgScript.package, settingsPage: true } };
  const disabledHtml = runtime({ scripts: [disabled] }).renderCodexPlusExtensionDetailTabs({ local: disabled, marketItem: null, sourceLabel: "官方" });
  assert.match(disabledHtml, /启用这个拓展后，这里会显示它自己的设置界面。下面是按清单生成的通用设置。/);
  assert.match(disabledHtml, /data-codex-ext-config-field="title"/);
});

test("改一项就把整份配置存回去；JSON 写错不保存并提示", async () => {
  const local = structuredClone(pkgScript);
  const rt = runtime({ scripts: [local] });
  const status = { textContent: "", dataset: {} as Record<string, string> };
  const form = { getAttribute: () => "user:notes", querySelector: () => status };
  const control = (field: string, type: string, value: string, attrs: Record<string, string> = {}) => {
    const all: Record<string, string> = { "data-codex-ext-config-field": field, "data-codex-ext-config-type": type, ...attrs };
    return { value, closest: () => form, getAttribute: (name: string) => all[name] ?? null, setAttribute: (name: string, next: string) => { all[name] = next; } };
  };
  await rt.saveCodexPlusExtensionConfigField(control("count", "number", "7"));
  assert.deepEqual(rt.saved.at(-1), ["user:notes", { ...local.package.config, count: 7 }]);
  assert.equal(status.textContent, "已保存");

  const toggle = control("dark", "boolean", "", { "data-enabled": "true" });
  await rt.saveCodexPlusExtensionConfigField(toggle);
  assert.equal(rt.saved.at(-1)[1].dark, false);
  assert.equal(toggle.getAttribute("aria-checked"), "false");

  await rt.saveCodexPlusExtensionConfigField(control("mode", "select", '"a"'));
  assert.equal(rt.saved.at(-1)[1].mode, "a");

  const before = rt.saved.length;
  await rt.saveCodexPlusExtensionConfigField(control("tags", "array", "[oops"));
  assert.equal(rt.saved.length, before);
  assert.equal(status.textContent, "JSON 格式不对");
  assert.equal(status.dataset.kind, "error");
  await rt.saveCodexPlusExtensionConfigField(control("tags", "array", '{"a":1}'));
  assert.equal(status.textContent, "要填一个 JSON 数组");
});

test("单文件脚本和市场条目只有「细节」；切页签只改显隐并记住，换选中项回到「细节」", () => {
  const single = { key: "user:solo.js", name: "solo", kind: "script", source: "user", enabled: true, status: "loaded" };
  const rt = runtime({ scripts: [single] });
  const html = rt.renderCodexPlusExtensionDetailTabs({ local: single, marketItem: null, sourceLabel: "用户" });
  assert.deepEqual([...html.matchAll(/role="tab"[^>]*data-codex-extensions-tab="(\w+)"/g)].map((match) => match[1]), ["readme"]);
  assert.match(html, /单文件脚本没有 README/);
  assert.equal(rt.posts.length, 0);

  const market = { id: "m", name: "M", version: "2.0.0", author: "作者", updated_at: "2026-10-01", homepage: "https://m.test", tags: ["效率"], requirements: ["需要网络"], limitations: [] };
  const marketHtml = rt.renderCodexPlusExtensionDetailTabs({ local: null, marketItem: market, sourceLabel: "" });
  assert.match(marketHtml, /<li>需要网络<\/li>/);
  assert.match(marketHtml, /<dt>作者<\/dt><dd>作者<\/dd><dt>最新版本<\/dt><dd>v2\.0\.0<\/dd><dt>更新时间<\/dt><dd>2026-10-01<\/dd>/);
  assert.match(marketHtml, /<h3>标签<\/h3><div class="codex-plus-extensions-detail-tags"><span>效率<\/span>/);

  const tabs = ["readme", "settings"].map((id) => ({ id, attrs: { "data-codex-extensions-tab": id, role: "tab", "aria-selected": "false" } as Record<string, string> }));
  const panels = ["readme", "settings"].map((id) => ({ id, hidden: false, getAttribute: () => id }));
  const root = {
    querySelectorAll: (selector: string) => (selector === "[data-codex-extensions-tab]"
      ? tabs.map((tab) => ({ getAttribute: (name: string) => tab.attrs[name], setAttribute: (name: string, value: string) => { tab.attrs[name] = value; } }))
      : panels),
  };
  assert.equal(rt.selectCodexPlusExtensionsDetailTab(root, "settings"), true);
  assert.equal(rt.tab(), "settings");
  assert.deepEqual(tabs.map((tab) => tab.attrs["aria-selected"]), ["false", "true"]);
  assert.deepEqual(panels.map((panel) => panel.hidden), [true, false]);
  assert.equal(rt.selectCodexPlusExtensionsDetailTab(root, "features"), false, "没有的页签不切");
  rt.resetCodexPlusExtensionsDetailTab();
  assert.equal(rt.tab(), "readme");
});

test("拓展包清单里的图标用在侧栏、卡片和详情头部，市场图标优先", () => {
  const list = readFileSync(new URL("../../../assets/inject/renderer-inject/40-backend-settings.js", import.meta.url), "utf8");
  assert.match(list, /codex-plus-page-nav-item-icon" aria-hidden="true">\$\{extensionIconMarkup\(marketItem\?\.icon \|\| entry\.script\?\.package\?\.icon\)\}/);
  assert.match(list, /codex-plus-extension-card-icon" aria-hidden="true">\$\{extensionIconMarkup\(item\?\.icon \|\| entry\.script\?\.package\?\.icon\)\}/);
  assert.match(list, /codex-plus-extensions-detail-icon" aria-hidden="true">\$\{extensionIconMarkup\(marketItem\?\.icon \|\| local\?\.package\?\.icon\)\}/);
  const manifest = JSON.parse(readFileSync(new URL("../../../assets/extensions/agent-pet/codexpp.json", import.meta.url), "utf8"));
  assert.equal(manifest.icon, "assets/icon.svg");
});
