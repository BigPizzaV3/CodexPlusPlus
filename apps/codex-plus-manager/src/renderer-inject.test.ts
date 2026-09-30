import assert from "node:assert/strict";
import { describe, it } from "node:test";
import { readFile } from "node:fs/promises";

const STEPWISE_FRAGMENT_PATHS = [
  "floating-panel/runtime/state.js",
  "floating-panel/core/appearance-runtime.js",
  "floating-panel/runtime/dom.js",
  "floating-panel/runtime/bridge-client.js",
  "floating-panel/runtime/answer-context.js",
  "floating-panel/stepwise/suggestions.js",
  "floating-panel/stepwise/generation.js",
  "floating-panel/runtime/lifecycle.js",
  "floating-panel/runtime/settings.js",
  "floating-panel/core/appearance.js",
  "floating-panel/core/host.js",
  "floating-panel/core/geometry.js",
  "floating-panel/core/interaction.js",
  "floating-panel/core/views.js",
  "floating-panel/outline/parser.js",
  "floating-panel/outline/navigation.js",
  "floating-panel/outline/feature.js",
  "floating-panel/outline/view.js",
  "floating-panel/core/scroll-state.js",
  "floating-panel-inject.js",
].map((name) => new URL(`../../../assets/inject/${name}`, import.meta.url));

async function readStepwiseSource() {
  const fragments = await Promise.all(STEPWISE_FRAGMENT_PATHS.map((url) => readFile(url, "utf8")));
  return `(() => {\n${fragments.join("\n")}\n})();\n`;
}

it("only reveals floating-panel content after the shell has settled open", async () => {
  const source = await readFile(
    new URL("../../../assets/inject/floating-panel/core/appearance.js", import.meta.url),
    "utf8",
  );
  const panelRules = Array.from(
    source.replace(/\$\{[^}]+\}/g, "0").matchAll(/(?:^|\n)\s*([^{}\n]*\.csw-panel)\s*\{([^}]+)\}/g),
    ([, selector, declarations]) => ({ selector: selector.trim(), declarations }),
  );
  const hidden = panelRules.find((rule) => rule.selector === ".csw-panel");
  assert.ok(hidden, "panel needs a hidden default for collapsed and interrupted states");
  assert.match(hidden.declarations, /opacity:\s*0\s*;/);
  assert.match(hidden.declarations, /visibility:\s*hidden\s*;/);
  assert.match(hidden.declarations, /transition:\s*none\s*!important\s*;/);

  const visible = panelRules.filter((rule) =>
    /opacity:\s*1\s*;|visibility:\s*visible\s*;/.test(rule.declarations),
  );
  assert.ok(visible.length > 0, "settled content must remain visible");
  for (const rule of visible) {
    assert.ok(rule.selector.includes('[data-open="true"]'), rule.selector);
    assert.ok(rule.selector.includes('[data-morphing="false"]'), rule.selector);
  }
});

function installRendererStyle(renderer: string) {
  const start = renderer.indexOf("  function installStyle()");
  const end = renderer.indexOf("\n  function defaultCodexPlusSettings", start);
  assert.ok(start >= 0 && end > start);
  const source = renderer.slice(start, end);
  const requiredNames = new Set([
    "styleId",
    "codexDeleteStyleVersion",
    ...Array.from(source.matchAll(/\$\{([A-Za-z_$][A-Za-z0-9_$]*)/g), (match) => match[1]),
  ]);
  const declarations = Array.from(requiredNames, (name) => {
    const declaration = renderer.match(new RegExp(`^  const ${name} = .+;$`, "m"))
      ?? renderer.match(new RegExp(`^  const ${name} = [\\s\\S]*?^  };$`, "m"));
    assert.ok(declaration, `missing renderer declaration for ${name}`);
    return declaration[0];
  }).join("\n");
  const appended: Array<{ dataset: Record<string, string>; id?: string; textContent?: string }> = [];
  const document = {
    getElementById() {
      return null;
    },
    createElement() {
      return { dataset: {} };
    },
    documentElement: {
      appendChild(node: (typeof appended)[number]) {
        appended.push(node);
      },
    },
  };
  const install = new Function("document", `${declarations}\n${source}\ninstallStyle();`) as (documentValue: typeof document) => void;

  install(document);
  return appended;
}

describe("renderer injection header compatibility", () => {
  it("纯 API 会话使用当前真实 provider，不强行改成 custom", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.doesNotMatch(
      renderer,
      /if \(String\(profile\?\.relayMode \|\| ""\) === "pureApi"\) return "custom";/,
    );
    assert.match(renderer, /codexPlusBackendSettings\.activeRelayCodexProvider/);
    assert.match(renderer, /codexModelCatalog\?\.codex_model_provider/);
  });

  it("adds the session copy shortcut through the native fork action", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /原地复制会话 - Codex\+\+/);
    assert.match(renderer, /createSessionMoreMenuItem\("原地复制会话 - Codex\+\+"/);
    assert.match(renderer, /getAttribute\("aria-label"\)[\s\S]*聊天操作/);
    assert.match(renderer, /从这里创建聊天分支/);
    assert.match(renderer, /data-app-action-sidebar-thread-selected/);
    assert.match(renderer, /sessionCopyMenuActivationTimeoutMs/);
    assert.doesNotMatch(renderer, /\n\s*refreshSessionCopyMenuItems\(\);/);
  });

  it("adds an encrypted session sharing button to the active Codex conversation", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /sessionShareButtonClass\s*=\s*"codex-session-share-button"/);
    assert.match(renderer, /function installSessionShareButton\(\)/);
    assert.match(renderer, /function sessionShareMarkdown\(\)/);
    assert.match(renderer, /crypto\.subtle\.generateKey\(\{ name: "AES-GCM", length: 256 \}/);
    assert.match(renderer, /https:\/\/share\.codexpp\.cc/);
    assert.match(renderer, /postJson\("\/share\/create", payload\)/);
    assert.match(renderer, /postJson\("\/session\/export"/);
    assert.match(renderer, /postJson\("\/session\/import"/);
    assert.match(renderer, /codex-rollout/);
    assert.match(renderer, /function sessionImportMarkdown\(session\)/);
    assert.match(renderer, /codexpp-import-session/);
    assert.match(renderer, /nativeShare\?\.closest\?\.\("\.ms-auto"\)/);
    assert.match(renderer, /#k=\$\{encrypted\.key\}/);
    assert.match(renderer, /navigator\.clipboard\.writeText\(shareUrl\)/);
    assert.match(renderer, /data-testid\*=\"message\"/);
    assert.match(renderer, /function sessionActionTrigger\(row\)/);
    assert.match(renderer, /const sessionMenuEnabled = codexPlusBackendSettings\.enhancementsEnabled !== false/);
    assert.doesNotMatch(renderer, /window\.location\.(?:href|assign)\s*=\s*[^;]*markdown/);
  });

  it("automatically renames a session through the native title suggestion", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /自动重命名当前会话/);
    assert.match(renderer, /activateSessionAutoRenameMenuItem/);
    assert.match(renderer, /input\[aria-label="聊天标题"\], input\[aria-label="Chat title"\]/);
    assert.match(renderer, /button\.classList\.contains\("text-info"\)/);
    assert.match(renderer, /\^\(保存\|Save\)\$/);
    assert.match(renderer, /Codex 未能生成新名称/);
  });

  it("removes the legacy Codex++ top-bar entry", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.doesNotMatch(renderer, /function installCodexPlusMenu\(\)/);
    assert.doesNotMatch(renderer, /function findNativeMenuInsertionPoint\(\)/);
    assert.doesNotMatch(renderer, /codex-plus-trigger/);
  });

  it("places Codex++ in the native sidebar and opens a main-content page", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusSidebarNavId\s*=\s*"codex-plus-sidebar-nav"/);
    assert.match(renderer, /function installCodexPlusSidebarNavigation\(\)/);
    assert.match(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\]/);
    // 新版把 role="navigation" 挪去了缩略图面板/演示目录，兜底要限定在 aside 内；
    // 而图标栏本身也是 aside 里的 <nav> 且文档顺序在前，必须显式排除，
    // 否则 "点导航就关页面" 的监听会挂到图标栏上，点自己的入口就把页面关掉。
    assert.doesNotMatch(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\], nav\[role="navigation"\]/);
    assert.match(renderer, /aside\.app-shell-left-panel nav\[role="navigation"\]'\)/);
    assert.match(renderer, /!nav\.hasAttribute\("data-app-navigation-rail"\)/);
    assert.match(renderer, /const insertionButton = pluginButton \|\| navButtons\.find/);
    assert.match(renderer, /selectors\.pluginNavButton/);
    assert.match(renderer, /button\.querySelector\(selectors\.pluginSvgPath\)/);
    assert.match(renderer, /\^\(插件\|Plugins\)\$/);
    assert.match(renderer, /openCodexPlusPage\(\)/);
    assert.match(renderer, /codex-plus-page-overlay/);
    assert.match(renderer, /positionCodexPlusPage/);
    assert.match(renderer, /overlay\.remove\(\);\s*if \(pageMode\) setCodexPlusSidebarNavActive\(false\);/);
    assert.match(renderer, /function closeCodexPlusPage\(\)/);
    assert.match(renderer, /function installCodexPlusPageNavigationCloseHandler\(\)/);
    assert.match(renderer, /target\?\.closest\(selectors\.sidebarThread\)/);
    assert.match(renderer, /closeCodexPlusPageAfterNativeNavigation\(\)/);
    assert.match(renderer, /setTimeout\(\(\) => \{\s*window\.__codexPlusPageNavigationCloseTimer = null;\s*closeCodexPlusPage\(\);/);
    assert.match(renderer, /installCodexPlusNavigationEntries\(\);/);
    assert.match(renderer, /document\.querySelectorAll\(`#\$\{codexPlusMenuId\}/);
  });

  it("mounts Codex++ and 拓展 entries into the new navigation rail", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusRailNavId\s*=\s*"codex-plus-rail-nav"/);
    assert.match(renderer, /codexPlusRailExtensionsId\s*=\s*"codex-plus-rail-extensions"/);
    assert.match(renderer, /codexPlusRailSelector\s*=\s*"nav\[data-app-navigation-rail\]"/);
    assert.match(renderer, /function installCodexPlusRailNavigation\(\)/);
    // 模板按钮取自原生 destination，clone 后必须清掉这几个属性，
    // 否则会被 Codex 的自定义/排序逻辑当成真 destination。
    assert.match(renderer, /codexPlusRailDestinationSelector\s*=\s*"\[data-sidebar-destination\]"/);
    assert.match(renderer, /button\.removeAttribute\("data-sidebar-destination"\)/);
    assert.match(renderer, /button\.removeAttribute\("aria-current"\)/);
    // 图标栏是纯图标，不塞文字标签。
    assert.match(renderer, /class="codex-plus-rail-icon"/);
    assert.doesNotMatch(renderer, /codex-plus-rail-icon"[^]*?<span class="truncate">/);
  });

  it("falls back to the legacy sidebar entry when the rail is absent", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function installCodexPlusNavigationEntries\(\)/);
    // 两条路径互斥：有 rail 就移除旧入口，没有就移除 rail 入口。
    assert.match(renderer, /if \(installCodexPlusRailNavigation\(\)\) \{\s*detachCodexPlusSidebarNavigation\(\);\s*return;\s*\}/);
    assert.match(renderer, /removeCodexPlusRailNavigation\(\);\s*installCodexPlusSidebarNavigation\(\);/);
    assert.match(renderer, /function detachCodexPlusSidebarNavigation\(\)/);
    // 启动补扫要认两条路径任意一条已装上。
    assert.match(renderer, /const installed = document\.getElementById\(codexPlusSidebarNavId\)\s*\|\| document\.getElementById\(codexPlusRailNavId\);/);
  });

  it("opens 拓展 as a standalone page instead of a Codex++ tab", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /codexPlusExtensionsTab\s*=\s*"extensions"/);
    assert.match(renderer, /function openCodexPlusExtensions\(\)/);
    assert.match(renderer, /openCodexPlusModal\(\{ page: true, tab: codexPlusExtensionsTab \}\)/);
    // 旧名 userScripts 仍要能映射过去，避免存量调用点失效。
    assert.match(renderer, /if \(tab === "extensions" \|\| tab === "userScripts"\) return codexPlusExtensionsTab;/);
    // 弹窗必须尊重传入的初始 tab，而不是硬编码 home。
    assert.match(renderer, /selectCodexPlusTab\(initialTab\);/);
    // 激活态要靠 data-codex-plus-active-tab 区分页面，必须排在 selectCodexPlusTab 之后。
    assert.match(renderer, /selectCodexPlusTab\(initialTab\);\s*\/\/[^]*?if \(pageMode\) setCodexPlusSidebarNavActive\(true, codexPlusActiveEntry\(\) \|\| "home"\);/);
  });

  it("gives the page mode a two-column layout with its own left panel", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function installCodexPlusPageLayout\(overlay, tab\)/);
    assert.match(renderer, /function refreshCodexPlusPageNav\(tab\)/);
    assert.match(renderer, /function renderCodexPlusPageNavItems\(tab\)/);
    // 三栏容器是 createElement + className 赋值的，不是 markup 字面量。
    assert.match(renderer, /layout\.className = "codex-plus-page-layout"/);
    assert.match(renderer, /main\.className = "codex-plus-page-main"/);
    // 只搬动已有的 modal-body，不重建里面的 data-codex-* 挂载点，
    // 否则 renderUserScripts / 各 toggle 的 querySelector 会找不到目标。
    assert.match(renderer, /main\.appendChild\(body\)/);
    // 左面板只有「拓展」需要（它是脚本列表）；主页与推荐内容是单栏内容页，
    // 页面切换交给图标栏那三个入口，所以导航容器要在条件分支里创建。
    assert.match(renderer, /if \(tab === codexPlusExtensionsTab\) \{[\s\S]{0,400}nav\.className = "codex-plus-page-nav"/);
    assert.match(renderer, /data-codex-plus-page-nav-body="true"/);
    // 布局必须在 selectCodexPlusTab 之前建好，否则刷新左面板时找不到容器。
    assert.match(renderer, /installCodexPlusPageLayout\(overlay, initialTab\);[\s\S]{0,900}selectCodexPlusTab\(initialTab\);/);
    // 左面板分组导航走同一个选中函数。
    assert.match(renderer, /const pageNav = target\?\.closest\("\[data-codex-plus-page-nav\]"\)/);
    assert.match(renderer, /selectCodexPlusTab\(pageNav\.getAttribute\("data-codex-plus-page-nav"\)\)/);
  });

  it("把三个 rail 入口都接成独立页面，并跟随 Codex 的界面缩放", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    // 推荐内容从弹窗二级 tab 提成图标栏上的一级入口。
    assert.match(renderer, /const codexPlusRailSponsorId = "codex-plus-rail-sponsor"/);
    assert.match(renderer, /const codexPlusSponsorTab = "sponsor"/);
    assert.match(renderer, /function openCodexPlusSponsor\(\)/);
    assert.match(renderer, /id: codexPlusRailSponsorId, label: "推荐内容"/);
    // 三个入口都要参与选中态映射，否则「推荐内容」亮不起来。
    assert.match(renderer, /\[codexPlusRailSponsorId, "sponsor"\],/);
    assert.match(renderer, /if \(tab === codexPlusSponsorTab\) return "sponsor";/);

    // 弹窗里的 tab 条已随入口外移删除，不该再有残留。
    assert.doesNotMatch(renderer, /codex-plus-tab-button/);

    // 「拓展」入口用 VSCode 的扩展字形，和列表默认图标同一份 path。
    assert.match(renderer, /extensions: `<svg viewBox="0 0 16 16" fill="currentColor"><path d="\$\{codexPlusDefaultExtensionIconPath\}"/);

    // 界面缩放：读 Codex 的 zoom 变量，套到 overlay 上，尺寸用 calc 反向抵消。
    assert.match(renderer, /const codexPlusWindowZoomVar = "--codex-window-zoom"/);
    assert.match(renderer, /function codexPlusWindowZoom\(\)/);
    assert.match(renderer, /function applyCodexPlusZoom\(overlay\)/);
    assert.match(renderer, /overlay\.style\.setProperty\("zoom", String\(zoom\)\)/);
    assert.match(renderer, /width: calc\(100vw \/ var\(--codex-plus-zoom, 1\)\)/);
    // 页面模式的 left 偏移要按 zoom 折算回布局坐标，CSS 里的 width 用的是同一个空间，
    // 两边量纲不一致会把右边缘算短（曾出现 62 + 1652 = 1714，视口 1727）。
    assert.match(renderer, /const layoutLeft = zoom === 1 \? left : left \/ zoom;/);
    assert.match(renderer, /overlay\.style\.setProperty\("--codex-plus-page-left", `\$\{layoutLeft\}px`\)/);
  });

  it("颜色走 Codex 的语义令牌，不写死调色板", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    // applyCodexPlusTheme 读宿主算好的令牌，取不到才回落。
    assert.match(renderer, /function applyCodexPlusTheme\(overlay\)/);
    assert.match(renderer, /--color-token-text-primary/);
    assert.match(renderer, /--color-token-text-secondary/);
    assert.match(renderer, /--color-token-bg-primary/);
    // 早先写死的 zinc 调色板不能再出现在主题函数里。
    const themeBody = renderer.slice(
      renderer.indexOf("function applyCodexPlusTheme(overlay)"),
      renderer.indexOf("function codexPlusModalTab(tab)"),
    );
    assert.ok(themeBody.length > 0, "找不到 applyCodexPlusTheme 的函数体");
    assert.ok(!themeBody.includes('text: "#f3f4f6"'), "主题函数里残留硬编码的正文色");
    assert.ok(!themeBody.includes('textSecondary: "#a1a1aa"'), "主题函数里残留硬编码的次要色");
  });

  it("lists user scripts in the 拓展 page left panel", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function renderCodexPlusExtensionsNav\(\)/);
    // 点左面板的行 = 选中（右侧看详情），不直接切开关——
    // 开关移到详情页，避免点行就误触发启停。
    assert.match(renderer, /data-codex-extensions-select="installed:\$\{escapeHtml\(entry\.key\)\}"/);
    assert.match(renderer, /data-codex-extensions-select="market:\$\{escapeHtml\(entry\.key\)\}"/);
    assert.match(renderer, /codex-plus-page-nav-item-state/);
    // 每个条目都有图标，没有自带图标的用默认字形。
    assert.match(renderer, /codex-plus-page-nav-item-icon/);
    assert.match(renderer, /codexPlusDefaultExtensionIconPath/);
    // 脚本启停/状态变化要同步到左面板与详情。
    assert.match(renderer, /if \(codexPlusActiveEntry\(\) === "extensions"\) refreshCodexPlusExtensionsView\(\);/);
  });

  it("shows a VSCode-style detail pane for the selected 拓展", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /function renderCodexPlusExtensionsDetail\(\)/);
    assert.match(renderer, /data-codex-plus-extensions-detail="true"/);
    assert.match(renderer, /function codexPlusExtensionsSelectionDetail\(\)/);
    // 详情里给已安装的开关（复用 user-script-key 委托）与卸载入口。
    assert.match(renderer, /data-codex-extensions-uninstall=/);
    assert.match(renderer, /function uninstallUserScript\(key\)/);
    assert.match(renderer, /postJson\("\/user-scripts\/delete", \{ key \}\)/);
    // 内置脚本在只读目录里，只给用户脚本提供卸载。
    assert.match(renderer, /if \(local\?\.source === "user"\)/);
    // 市场条目在详情里也能直接安装。
    assert.match(renderer, /codex-plus-extensions-detail-primary/);
    // 选中项要高亮。
    assert.match(renderer, /data-active="\$\{String\(selected\)\}"/);
  });

  it("gives 拓展 a searchable 已安装 / 市场 分组视图", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /data-codex-extensions-search="true"/);
    assert.match(renderer, /function refreshCodexPlusExtensionsView\(\)/);
    assert.match(renderer, /function codexPlusExtensionsEntries\(\)/);
    assert.match(renderer, /function loadScriptMarket\(/);
    // 市场走 bridge 的两个新路由。
    assert.match(renderer, /postJson\("\/script-market\/list", \{\}\)/);
    assert.match(renderer, /postJson\("\/script-market\/install", \{ id \}\)/);
    // 空分组要留着显示占位（加载中/加载失败），只在搜索无匹配时才省略——
    // 否则「正在读取脚本市场…」和错误提示会被一起藏掉，面板全空。
    assert.match(renderer, /if \(!entries\.length && searching\) return "";/);
    assert.doesNotMatch(renderer, /if \(!entries\.length && \(searching \|\| !count\)\) return "";/);
    // 搜索重绘后要把焦点放回输入框，否则每敲一个字就断。
    assert.match(renderer, /next\.setSelectionRange\(next\.value\.length, next\.value\.length\)/);
  });

  it("does not install Codex++ UI in embedded browser documents", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /window\.top\s*!==\s*window/);
    assert.match(renderer, /!window\.electronBridge/);
    assert.ok(renderer.includes("/^app:\\\/\\\/\\-\\//i.test(window.location.href)"));
    assert.match(renderer, /codexPlusIsNodeTestHarness/);
  });

  it("initializes renderer styles without unresolved template identifiers", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    const appended = installRendererStyle(renderer);

    assert.equal(appended.length, 1);
    assert.match(appended[0].textContent ?? "", /#codex-plus-sidebar-nav/);
  });

  it("does not override the host document root typography or foreground", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const appended = installRendererStyle(renderer);
    const css = appended[0].textContent ?? "";
    const rootRule = css.match(/:root\s*\{([^}]*)\}/)?.[1] ?? "";

    assert.doesNotMatch(rootRule, /(?:^|;)\s*font(?:-family)?\s*:/);
    assert.doesNotMatch(rootRule, /(?:^|;)\s*color\s*:/);
    assert.match(css, /:where\([^)]*codex-plus-modal-overlay[^)]*\)\s*\{[^}]*font-family:\s*inherit;/s);
  });

  it("rewrites official usage status at the cache boundary instead of scanning alerts", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const scanStart = renderer.indexOf("  function scanLightweight()");
    const scanEnd = renderer.indexOf("  function officialUsagePolicy()", scanStart);
    assert.ok(scanStart >= 0 && scanEnd > scanStart);
    const scan = renderer.slice(scanStart, scanEnd);

    assert.doesNotMatch(scan, /syncOfficialUsagePolicy|refreshOfficialUsageAlert/);
    assert.match(renderer, /typeof nextStatus\.hideOfficialUsageAlert === "boolean"/);
    assert.match(renderer, /window\.__CODEX_PLUS_HIDE_OFFICIAL_USAGE_ALERT__ = nextStatus\.hideOfficialUsageAlert/);
    assert.match(renderer, /function syncOfficialUsagePolicy\(\)/);
    assert.match(renderer, /queryKey\[1\] !== "image-generation"/);
    assert.match(renderer, /image_generation_limit_reached/);
    assert.match(renderer, /if \(loaded\) syncOfficialUsagePolicy\(\);/);
    assert.doesNotMatch(renderer, /officialUsageAlertCards|refreshOfficialUsageAlertVisibility|codex-plus-hide-usage-alert/);
    assert.doesNotMatch(renderer, /mutationTouchesUsageAlert/);
    assert.match(renderer, /function isOfficialLowQuotaSidebarCard/);
    assert.match(renderer, /function isOfficialLowQuotaComposerBanner/);
    assert.match(renderer, /upsell-banner-title-/);
    assert.match(renderer, /function isOfficialLowQuotaComposerAside/);
    assert.match(renderer, /tagName !== "ASIDE"/);
    assert.match(renderer, /rounded-3xl/);
    assert.match(renderer, /function syncOfficialUsageWindowMode/);
    assert.match(renderer, /key === "official-hide"/);
    assert.match(renderer, /officialUsageWindowObserver\?\.disconnect\(\)/);
    assert.match(renderer, /officialUsageWindowMarker\}="hidden"/);
    assert.doesNotMatch(scan, /startOfficialUsageWindowBlock|hideOfficialUsageWindowsWithin/);
    const policyStart = renderer.indexOf("  function syncOfficialUsagePolicy()");
    const policyEnd = renderer.indexOf("  if (window.__CODEX_PLUS_TEST_RATE_LIMIT_UNLOCK__)", policyStart);
    assert.ok(policyStart >= 0 && policyEnd > policyStart);
    const policy = renderer.slice(policyStart, policyEnd);
    const sameKey = policy.slice(policy.indexOf("if (key === officialUsagePolicyApplied)"), policy.indexOf("const previous"));
    assert.match(sameKey, /rewriteCachedOfficialUsage/);
    assert.doesNotMatch(sameKey, /hideOfficialUsageWindowsWithin|querySelectorAll/);
    assert.match(renderer, /function codexPlusPublishUsageData/);
    assert.match(renderer, /unlockSend: mixed/);
    assert.match(renderer, /limit_reached: false/);
    assert.doesNotMatch(renderer, /__codexPlusApiQuotaGate/);
    assert.doesNotMatch(renderer, /installExternalApiQuotaGate|__codexPlusApiQuotaBreakpoint|refreshComposers/);
  });

  // issue #2169：HTTP 回落成功不得掩盖桥接通道故障。桥接失败计数独立于后端状态，
  // 连续失败时状态灯降级呈现；回落路径绝不能清零计数或刷新 bridge 健康时间戳，
  // 否则启动器侧健康检查失去修复动力，重注入风暴对用户完全静默。
  it("surfaces persistent bridge degradation while the http fallback keeps the backend reachable", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");

    assert.match(renderer, /const CODEX_PLUS_BRIDGE_FAILURE_THRESHOLD = 3;/);
    assert.match(
      renderer,
      /function recordCodexPlusBridgeFailure\(\) \{\s*codexPlusBridgeFailureCount \+= 1;\s*\}/,
    );
    // 降级渲染：桥接连续失败 + 后端 ok → degraded
    assert.match(renderer, /bridgeDegraded && rawStatus === "ok" \? "degraded" : rawStatus/);
    assert.match(renderer, /桥接降级，自动修复中/);
    // 回落分支只记失败，不得触碰成功路径
    const fallbackBlock = renderer.match(
      /const fallback = await fetchBackendStatusFromHelper\(path, payload\);[\s\S]*?return fallback;\s*\}/,
    );
    assert.ok(fallbackBlock, "http fallback block not found in postJson");
    assert.doesNotMatch(fallbackBlock[0], /recordCodexPlusBridgeSuccess\(\)/);
    // 降级状态有专属样式（指示灯与文本）
    assert.match(renderer, /\.codex-plus-backend-indicator\[data-status="degraded"\]/);
    assert.match(renderer, /\.codex-plus-backend-label\[data-status="degraded"\]/);
  });

  it("keeps Windows Dream Skin compatible with the modern Codex main surface", async () => {
    const dreamSkinRenderer = await readFile(
      new URL("../../../assets/inject/upstream/dream-skin/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );
    const cidalaRenderer = await readFile(
      new URL("../../../assets/inject/upstream/cidala-tiger/windows/renderer-inject.js", import.meta.url),
      "utf8",
    );

    assert.match(dreamSkinRenderer, /codex-dream-skin-selectors\/1/);
    assert.match(dreamSkinRenderer, /MainContentSurface/);
    assert.match(dreamSkinRenderer, /data-ds-part/);
    assert.match(cidalaRenderer, /MainContentSurface/);
    assert.match(cidalaRenderer, /data-codex-plus-dream-surface/);
    assert.match(cidalaRenderer, /ensureShellMain/);
  });
});

/** 从注入脚本里取出 `shouldScheduleScan`，配上可控的依赖来跑。 */
function shouldScheduleScanRuntime(renderer: string) {
  const start = renderer.indexOf("  function shouldScheduleScan(");
  const end = renderer.indexOf("\n  function runScheduledScan(", start);
  assert.ok(start >= 0 && end > start, "shouldScheduleScan not found in renderer-inject.js");
  const source = renderer.slice(start, end);
  const factory = new Function(
    "isChatContentMutation",
    "isExtensionUiNode",
    "nodeSelfOrAncestorMatchesScanRelevance",
    "isScanRelevantNode",
    `${source}\nreturn shouldScheduleScan;`,
  );
  return factory(
    () => false,
    (node: { extension?: boolean }) => Boolean(node?.extension),
    // Codex 的容器（header / 侧栏 nav）本身就是 scan-relevant，这是自喂循环的关键前提。
    (node: { relevant?: boolean }) => Boolean(node?.relevant),
    (node: { relevant?: boolean; extension?: boolean }) =>
      Boolean(node?.relevant) && !node?.extension,
  ) as (mutations: unknown[]) => boolean;
}

const codexContainer = { nodeType: 1, relevant: true };

function mutation(addedNodes: unknown[] = [], removedNodes: unknown[] = []) {
  return { target: codexContainer, addedNodes, removedNodes };
}

describe("renderer injection scan scheduling", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：我们把自己的节点挂进 Codex 的容器，容器是 scan-relevant，
  // 于是每次写入都会再排一次 scan，scan 又重新写入，空闲时 CPU 被吃满。
  it("ignores mutations that only move the extension's own nodes", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([ownNode])]), false);
    // appendChild 一个已经在位的子节点会同时报 removed + added。
    assert.equal(shouldScheduleScan([mutation([ownNode], [ownNode])]), false);
  });

  it("still scans when Codex itself changes the same container", async () => {
    const shouldScheduleScan = shouldScheduleScanRuntime(await readFile(rendererPath, "utf8"));
    const codexNode = { nodeType: 1, relevant: true };
    const ownNode = { nodeType: 1, extension: true };

    assert.equal(shouldScheduleScan([mutation([codexNode])]), true);
    // 混合变更里只要有一个不是我们的，就不能跳过。
    assert.equal(shouldScheduleScan([mutation([ownNode, codexNode])]), true);
    // 属性变更没有 added/removed 节点，仍按容器相关性判定。
    assert.equal(shouldScheduleScan([mutation()]), true);
  });
});

interface MarketplacePatchHarness {
  install: () => void;
  sweeps: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function marketplacePatchRuntime(renderer: string, patchSucceeds: boolean): MarketplacePatchHarness {
  const start = renderer.indexOf("  const pluginMarketplaceRequestPatchMaxMisses = ");
  const end = renderer.indexOf("\n  function pluginPatchDisabledInRelayMode(", start);
  assert.ok(start >= 0 && end > start, "marketplace patch block not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexPluginMarketplaceUnlockVersion",
    "pluginPatchDisabledInRelayMode",
    "codexPlusSettings",
    "loadAppServerRequestCandidates",
    "patchPluginMarketplaceRequestClient",
    "sendCodexPlusDiagnostic",
    "__note",
    `${source}\nreturn installPluginMarketplaceRequestPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    () => false,
    () => ({ pluginMarketplaceUnlock: true }),
    // 每轮 sweep 在真实实现里会 fetch 全部 app asset，这里只计数并挂起，
    // 好让测试能在「上一轮尚未结束」的时刻再次调用 install。
    () =>
      new Promise((resolve) => {
        sweeps += 1;
        pending.push(() => resolve({ modules: [{}], candidates: [{}], sources: [], discovery: "fallback" }));
      }),
    () => patchSucceeds,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    // 放行所有挂起的 sweep，并把微任务队列排空。
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, sweeps: () => sweeps, diagnostics: () => diagnostics, settle };
}

/** 取出共用的 module loader，用假时钟驱动它的失败冷却。 */
function moduleLoaderRuntime(renderer: string) {
  const start = renderer.indexOf("  // issue #1960：失败必须被记住。");
  const end = renderer.indexOf("\n  async function loadOptionalCodexAppModule(", start);
  assert.ok(start >= 0 && end > start, "loadCodexAppModule not found in renderer-inject.js");
  const source = renderer.slice(start, end);

  let sweeps = 0;
  let clock = 1_000_000;
  const factory = new Function(
    "codexServiceTierModulePromises",
    "codexAppModuleFailures",
    "codexAppModuleRetryCooldownMs",
    "codexAppModuleMaxAttempts",
    "codexAppAssetUrl",
    "codexAppAssetUrlFromScriptText",
    "Date",
    `${source}\nreturn loadCodexAppModule;`,
  );
  const load = factory(
    new Map(),
    new Map(),
    30000,
    8,
    () => "",
    // 真实实现在这里会把全部 app asset 拉一遍；这里只计数并同样返回“没找到”。
    async () => {
      sweeps += 1;
      return "";
    },
    { now: () => clock },
  ) as (namePart: string) => Promise<unknown>;

  const attempt = async (namePart = "vscode-api-") => {
    try {
      await load(namePart);
    } catch {
      /* 预期失败 */
    }
  };
  return { attempt, sweeps: () => sweeps, advance: (ms: number) => { clock += ms; } };
}

describe("renderer injection codex app module loader", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：失败以前只是把 promise 删掉，等于没有负缓存，
  // 调用方一重试就重新 fetch 全部 app asset（实测 301 次请求/秒）。
  it("does not re-sweep every asset while the failure is still in cooldown", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    for (let i = 0; i < 20; i += 1) await loader.attempt();

    assert.equal(loader.sweeps(), 1);
  });

  it("retries once per cooldown window, then gives up for good", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    // 冷却期满就允许再试一次，避免 Codex 更新后 asset 回来了却永远发现不了。
    for (let i = 0; i < 30; i += 1) {
      await loader.attempt();
      loader.advance(30001);
    }

    // 连续失败达到上限(8)后彻底停手，而不是每个冷却窗口都再扫一遍。
    assert.equal(loader.sweeps(), 8);
  });

  it("keeps failures separate per asset prefix", async () => {
    const loader = moduleLoaderRuntime(await readFile(rendererPath, "utf8"));

    await loader.attempt("vscode-api-");
    await loader.attempt("app-initial-");
    await loader.attempt("vscode-api-");

    // 两个前缀各自试了一次；第三次命中 vscode-api- 自己的冷却。
    assert.equal(loader.sweeps(), 2);
  });
});

interface DispatcherPatchHarness {
  install: () => void;
  attempts: () => number;
  diagnostics: () => string[];
  settle: () => Promise<void>;
}

function dispatcherPatchRuntime(renderer: string, dispatcherFound: boolean): DispatcherPatchHarness {
  const start = renderer.indexOf("  const serviceTierDispatcherPatchMaxMisses = ");
  const end = renderer.indexOf("\n  async function loadBackendSettingsState(", start);
  assert.ok(start >= 0 && end > start, "service tier dispatcher patch block not found");
  const source = renderer.slice(start, end);

  let attempts = 0;
  let pending: Array<() => void> = [];
  const diagnostics: string[] = [];
  const fakeWindow: Record<string, unknown> = {};

  const factory = new Function(
    "window",
    "codexServiceTierRequestOverrideVersion",
    "loadCodexAppModule",
    "codexServiceTierDispatcherFromModule",
    "dispatchCodexPlusMessage",
    "installCodexRemoteSessionDispatcherSubscription",
    "sendCodexPlusDiagnostic",
    `${source}\nreturn installCodexServiceTierDispatcherPatch;`,
  );

  const install = factory(
    fakeWindow,
    1,
    // 真实实现每轮会依次试三个前缀，每个 miss 都触发一轮全量 asset 扫描。
    () =>
      new Promise((resolve, reject) => {
        attempts += 1;
        pending.push(() => (dispatcherFound ? resolve({}) : reject(new Error("未找到 Codex App asset"))));
      }),
    () => (dispatcherFound ? { dispatchMessage() {}, subscribe() {} } : null),
    () => undefined,
    () => undefined,
    (event: string) => diagnostics.push(event),
  ) as () => void;

  const settle = async () => {
    while (pending.length) {
      const flush = pending;
      pending = [];
      flush.forEach((resolve) => resolve());
      await Promise.resolve();
      await Promise.resolve();
      await Promise.resolve();
    }
  };

  return { install, attempts: () => attempts, diagnostics: () => diagnostics, settle };
}

describe("renderer injection service tier dispatcher patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：这个补丁挂在 scanLightweight() 里每轮都跑，是 #1324 同一缺陷的第三个实例。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    await harness.settle();
  });

  it("stops retrying and stops re-reporting once the dispatcher is clearly gone", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 三个前缀里第一个就抛，loadDispatcher 会继续试下一个，所以每轮不止一次尝试；
    // 关键是达到 maxMisses(8) 之后彻底停手。
    assert.equal(harness.diagnostics().filter((e) => e === "service_tier_dispatcher_patch_failed").length, 1);
    assert.deepEqual(harness.diagnostics().at(-1), "service_tier_dispatcher_patch_skipped");
    const settled = harness.attempts();
    harness.install();
    await harness.settle();
    assert.equal(harness.attempts(), settled);
  });

  it("keeps working normally when the dispatcher is found", async () => {
    const harness = dispatcherPatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.attempts(), 1);
    assert.deepEqual(harness.diagnostics(), ["service_tier_dispatcher_patch_installed"]);
  });
});

describe("renderer injection plugin marketplace patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  // issue #1960：scanDeferred() 每轮都调用这个补丁，而早退守卫只在打上补丁后才写入。
  // Codex 侧 asset 改名后这层永远成功不了，过去既不去重也不放弃，
  // 于是每轮 scan 都把全部 app asset 重新 fetch 一遍（实测 530 次 fetch/秒）。
  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 模拟连续多轮 scan：上一轮还挂着，后续调用必须被 in-flight 守卫挡掉。
    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    await harness.settle();
  });

  it("stops retrying once the asset is clearly unavailable", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 每次都跑完再发起下一轮，模拟长时间运行中的反复 scan。
    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 达到 maxMisses(8) 之后必须彻底停掉，而不是无限重试。
    assert.equal(harness.sweeps(), 8);
    // 首次 miss 上报一次，停用时再报一次，中间保持噤声。
    assert.deepEqual(harness.diagnostics(), [
      "plugin_marketplace_request_patch_not_found",
      "plugin_marketplace_request_patch_skipped",
    ]);
  });

  it("keeps working normally when the patch actually lands", async () => {
    const harness = marketplacePatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    // 打上补丁后守卫生效，后续 scan 不再重复扫描。
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    assert.deepEqual(harness.diagnostics(), ["plugin_marketplace_request_patch_installed"]);
  });
});

describe("relay pureApi provider resolution", () => {
  function providerRuntime(
    renderer: string,
    backendSettings: Record<string, unknown>,
    catalog: Record<string, unknown>,
    profile: Record<string, unknown>,
  ) {
    const start = renderer.indexOf("function codexRelayConfigModelProvider(");
    const codeStart = renderer.indexOf("function codexRemoteSessionTargetProvider(");
    const end = renderer.indexOf("\n  function codexRemoteSessionProviderRequestMethod", codeStart);
    assert.ok(start >= 0 && codeStart >= 0 && end > codeStart);
    const source = renderer.slice(start, end);
    const create = new Function(
      "codexPlusBackendSettings",
      "codexModelCatalog",
      "codexRemoteSessionActiveProfile",
      `${source}\nreturn { codexRelayConfigModelProvider, codexRemoteSessionTargetProvider };`,
    ) as (
      backend: Record<string, unknown>,
      cat: Record<string, unknown>,
      activeProfile: () => Record<string, unknown>,
    ) => {
      codexRelayConfigModelProvider: (configContents: string) => string;
      codexRemoteSessionTargetProvider: () => string;
    };
    return create(backendSettings, catalog, () => profile);
  }

  it("resolves the real model_provider from a pureApi relay profile instead of hardcoding custom", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      {},
      { codex_model_provider: "deepseek" },
      { relayMode: "pureApi", configContents: 'model = "deepseek-v4-flash-vision-exp"\nmodel_provider = "deepseek"' },
    );

    assert.equal(runtime.codexRelayConfigModelProvider('model_provider = "deepseek"'), "deepseek");
    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });

  it("still returns custom for pureApi relays that genuinely declare the custom provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      {},
      { codex_model_provider: "custom" },
      { relayMode: "pureApi", configContents: 'model_provider = "custom"\n[model_providers.custom]' },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  it("falls back to custom when a pureApi relay declares no provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(renderer, {}, { codex_model_provider: "" }, { relayMode: "pureApi", configContents: "" });

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  // activeRelayCodexProvider 是全局缓存，切换供应商后可能还留着上一个的值。
  // pureApi 时优先信 profile 自己的 configContents；profile 没声明就回到
  // "custom"，不采信这个缓存——cdp_bridge.rs 的 refreshedPureApiResumeProvider
  // 正是钉这个：陈旧缓存是 stale_custom_provider 时必须仍解析成 custom。
  it("ignores a possibly stale activeRelayCodexProvider for pureApi relays", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "stale_custom_provider" },
      { codex_model_provider: "" },
      { relayMode: "pureApi", configContents: "" },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "custom");
  });

  // 非 pureApi 才拿 activeRelayCodexProvider 兜底。
  it("still falls back to activeRelayCodexProvider outside pureApi", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "deepseek" },
      { codex_model_provider: "" },
      { relayMode: "mixedApi", configContents: "" },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });

  // profile 自己声明了供应方时，优先级高于全局缓存。
  it("prefers the profile's own configContents over the cached provider", async () => {
    const renderer = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
    const runtime = providerRuntime(
      renderer,
      { activeRelayCodexProvider: "stale_custom_provider" },
      { codex_model_provider: "" },
      { relayMode: "pureApi", configContents: 'model_provider = "deepseek"' },
    );

    assert.equal(runtime.codexRemoteSessionTargetProvider(), "deepseek");
  });
});

describe("Stepwise generation mode contracts", () => {
  it("exposes automatic and manual generation in manager settings", async () => {
    const app = await readFile(new URL("./App.tsx", import.meta.url), "utf8");
    const renderer = await readFile(
      new URL("../../../assets/inject/renderer-inject.js", import.meta.url),
      "utf8",
    );

    assert.match(app, /type StepwiseGenerationMode = "auto" \| "manual";/);
    assert.match(app, /type StepwiseProtocol = "auto" \| "chat_completions" \| "responses" \| "anthropic_messages";/);
    assert.match(app, /codexAppStepwiseProtocol: "chat_completions",/);
    assert.match(app, /codexAppStepwiseGenerationMode: "auto",/);
    assert.match(app, /codexAppAnswerOutlineEnabled: false,/);
    assert.match(renderer, /answerOutline: false,/);
    assert.match(app, /<Field label=\{t\("模式"\)\}>/);
    assert.match(app, /\{ value: "auto", label: t\("自动生成"\) \}/);
    assert.match(app, /\{ value: "manual", label: t\("手动刷新"\) \}/);
    assert.match(app, /\{ value: "auto", label: t\("自动兼容"\) \}/);
    assert.match(app, /\{ value: "anthropic_messages", label: "Anthropic Messages" \}/);
    assert.match(app, /function normalizeStepwiseProtocol\(/);
    assert.match(app, /return value === "manual" \? "manual" : "auto";/);
  });

  it("defers manual generation until refresh and rejects stale mode results", async () => {
    const stepwise = await readStepwiseSource();

    assert.match(stepwise, /const manualRequestPending = generationMode === "manual"/);
    assert.match(stepwise, /if \(generationMode === "manual" && !manualResultVisible && !manualRequestPending\)/);
    assert.match(stepwise, /\} else if \(manualRequestPending\) \{/);
    assert.match(stepwise, /state\.bridgeStatus = "manual-ready";/);
    assert.match(
      stepwise,
      /requestBridgeStepwise\(bridgeKey, userText, assistantText, generationMode, \{ userInitiated: true \}\)/,
    );
    assert.match(stepwise, /requestBridgeStepwise\(bridgeKey, userText, assistantText, "auto"\)/);
    assert.match(stepwise, /normalizedMode === "manual" && options\.userInitiated !== true/);
    assert.match(stepwise, /stepwiseGenerationMode\(\) === normalizedMode/);
    assert.match(stepwise, /state\.bridgePendingMode === normalizedMode/);
    assert.match(stepwise, /Object\.prototype\.hasOwnProperty\.call\(normalizedPatch, "generationMode"\)/);
    assert.match(stepwise, /if \(!Object\.prototype\.hasOwnProperty\.call\(nextSettings, "generationMode"\)\)/);
    assert.match(stepwise, /nextSettings\.generationMode = stepwiseGenerationMode\(\);/);
    const appearanceStart = stepwise.indexOf("function appearanceSettingsHtml()");
    const settingsStart = stepwise.indexOf("function settingsHtml()", appearanceStart);
    const appearanceMarkup = stepwise.slice(appearanceStart, settingsStart);
    assert.doesNotMatch(appearanceMarkup, /data-action="generation-mode"/);
    const footerStart = stepwise.indexOf('<div class="csw-runtime-grid"', settingsStart);
    const generationModeControl = stepwise.indexOf('data-action="generation-mode"', footerStart);
    const promptClickControl = stepwise.indexOf('data-action="prompt-click-mode"', footerStart);
    assert.ok(footerStart >= 0 && generationModeControl > footerStart && promptClickControl > generationModeControl);
    assert.match(stepwise, /<span class="csw-metric-label">模式<\/span>/);
    assert.match(stepwise, /return normalizeGenerationMode\(value\) === "manual" \? "手动刷新" : "自动生成";/);
    assert.match(stepwise, /return setGenerationMode\(nextGenerationMode\(\)\);/);
    assert.match(stepwise, /return writePromptClickMode\(nextPromptClickMode\(\)\);/);
    assert.match(stepwise, /button\.csw-metric-action\s*\{[^}]*padding:\s*0;/s);
    assert.match(
      stepwise,
      /\.csw-click-mode,[\s\S]*?\.csw-generation-mode\s*\{[^}]*min-width:\s*0;/,
    );
    assert.match(stepwise, /\.csw-generation-mode\s*\{[^}]*white-space:\s*nowrap;/s);
    assert.match(
      stepwise,
      /\.csw-metric-value,[\s\S]*?\.csw-metric-action\s*\{[^}]*overflow:\s*visible;[^}]*text-overflow:\s*clip;/,
    );
    assert.match(
      stepwise,
      /\.csw-metric-value,[\s\S]*?\.csw-generation-mode \.csw-metric-action\s*\{[^}]*white-space:\s*nowrap;/,
    );
    assert.match(
      stepwise,
      /\.csw-click-mode \.csw-metric-action\s*\{[^}]*overflow-wrap:\s*anywhere;[^}]*white-space:\s*normal;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-settings-footer\s*\{[^}]*display:\s*grid;[^}]*grid-template-columns:\s*minmax\(max-content, 1fr\) auto;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-runtime-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0, max-content\) minmax\(0, 1fr\);[^}]*width:\s*100%;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-command-button\s*\{[^}]*flex:\s*0 0 30px;[^}]*height:\s*30px;[^}]*padding:\s*0;[^}]*width:\s*30px;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 440px\)[\s\S]*?\.csw-command-label\s*\{[^}]*display:\s*none;/,
    );
    assert.match(
      stepwise,
      /class="csw-command-button"[^>]*title="\$\{escapeAttr\(title\)\}"[^>]*aria-label="\$\{escapeAttr\(title\)\}"/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 360px\)[\s\S]*?\.csw-runtime-grid\s*\{[^}]*grid-template-columns:\s*minmax\(0, 1fr\);/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 360px\)[\s\S]*?\.csw-generation-mode,[\s\S]*?\.csw-click-mode\s*\{[^}]*width:\s*100%;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 320px\)[\s\S]*?\.csw-metric\s*\{[^}]*white-space:\s*nowrap;/,
    );
    assert.match(
      stepwise,
      /@container csw-panel \(max-width: 320px\)[\s\S]*?\.csw-command-button\s*\{[^}]*flex:\s*0 0 28px;[^}]*height:\s*28px;[^}]*width:\s*28px;/,
    );
    const toggleStart = stepwise.indexOf("async function setGenerationMode(value)");
    const immediateCancel = stepwise.indexOf(
      "applyRuntimeSettings({ ...(state.settings || {}), generationMode: nextMode });",
      toggleStart,
    );
    const settingsSave = stepwise.indexOf('bridgeCall("/settings/set", {', toggleStart);
    assert.ok(toggleStart >= 0 && immediateCancel > toggleStart && settingsSave > immediateCancel);

    const progressStart = stepwise.indexOf("function nextProgressState()");
    const manualProgressGuard = stepwise.indexOf('if (stepwiseGenerationMode() === "manual") return null;', progressStart);
    const localScanProgress = stepwise.indexOf('state.scanStatus === "assistant-changed"', progressStart);
    assert.ok(progressStart >= 0 && manualProgressGuard > progressStart && localScanProgress > manualProgressGuard);
    assert.match(stepwise, /title: "当前为手动模式"/);
    assert.doesNotMatch(stepwise, /title: "待生成"/);

    const outlineExpressionStart = stepwise.indexOf("function usesOutlineExpression(");
    const outlineExpressionEnd = stepwise.indexOf("function resolveFabExpression(", outlineExpressionStart);
    const outlineExpression = stepwise.slice(outlineExpressionStart, outlineExpressionEnd);
    assert.match(outlineExpression, /stepwiseWaitingForManualRefresh\(\)/);

    const runtimePresentationStart = stepwise.indexOf("function settingsRuntimePresentation(");
    const runtimePresentationEnd = stepwise.indexOf("function settingsCommandHtml(", runtimePresentationStart);
    const runtimePresentation = stepwise.slice(runtimePresentationStart, runtimePresentationEnd);
    assert.match(runtimePresentation, /!outlineExpression && stepwiseWaitingForManualRefresh\(settings\)/);

    const scanStart = stepwise.indexOf("function scan(");
    const outlineRefresh = stepwise.indexOf("void refreshOutline({ message, assistantHash: hash });", scanStart);
    const manualScanBranch = stepwise.indexOf('if (generationMode === "manual" && !manualResultVisible && !manualRequestPending)', scanStart);
    const cachedScanBranch = stepwise.indexOf('else if (hasSuccessfulCache)', scanStart);
    const automaticGenerate = stepwise.indexOf('requestBridgeStepwise(bridgeKey, userText, assistantText, "auto")', scanStart);
    assert.ok(scanStart >= 0 && outlineRefresh > scanStart && manualScanBranch > outlineRefresh);
    assert.ok(cachedScanBranch > manualScanBranch);
    assert.ok(automaticGenerate > cachedScanBranch);
  });

  it("keeps feature tab order draggable and persistent", async () => {
    const stepwise = await readStepwiseSource();

    assert.match(stepwise, /const VIEW_ORDER_KEY = "codex-stepwise-view-order-v1";/);
    assert.match(stepwise, /function normalizeViewOrder\(value\)/);
    assert.match(stepwise, /function persistViewOrder\(order\)/);
    assert.match(stepwise, /function installViewTabReorder\(\)/);
    assert.match(stepwise, /data-reorderable="true"/);
    assert.match(stepwise, /state\.viewReorderCleanup\?\.\(\)/);
    assert.match(stepwise, /syncViewTabSelection\(state\.activeTab, true\);/);
    assert.match(stepwise, /dataset\.codexStepwiseStyleVersion === SCRIPT_VERSION/);
    assert.match(stepwise, /style\.dataset\.codexStepwiseStyleVersion = SCRIPT_VERSION/);
    assert.match(stepwise, /VIEW_SLIDE_MS = 240/);
    assert.match(stepwise, /VIEW_INDICATOR_MS = 220/);
  });

  it("keeps Stepwise Manager controls at one height", async () => {
    const styles = await readFile(new URL("./styles.css", import.meta.url), "utf8");

    assert.match(styles, /\.stepwise-settings-block\s*\{[\s\S]*?--stepwise-control-height:\s*40px;/);
    assert.match(
      styles,
      /\.stepwise-settings-block input[^,]*,[\s\S]*?\.stepwise-settings-block \.app-select-trigger,[\s\S]*?\.stepwise-settings-block \.field-select,[\s\S]*?\.stepwise-settings-block \.select-input\s*\{[\s\S]*?height:\s*var\(--stepwise-control-height\);[\s\S]*?min-height:\s*var\(--stepwise-control-height\);/,
    );
  });
});

// issue #2256/#2255：app-server model request patch 的 miss 熔断以前被 provider
// 重试路径提前 return 绕过，失败变成 250ms 无限重试（每轮全量 fetch 全部 app asset）。
describe("renderer injection app-server model request patch", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  interface AppServerPatchHarness {
    install: () => void;
    sweeps: () => number;
    diagnostics: () => string[];
    settle: () => Promise<void>;
  }

  function appServerPatchRuntime(renderer: string, patchSucceeds: boolean): AppServerPatchHarness {
    const start = renderer.indexOf("  const appServerModelRequestPatchMaxMisses = ");
    const end = renderer.indexOf("\n  function ensureCodexModelWhitelistInstalls(", start);
    assert.ok(start >= 0 && end > start, "app-server model request patch block not found");
    const source = renderer.slice(start, end);

    let sweeps = 0;
    let pending: Array<() => void> = [];
    const diagnostics: string[] = [];
    const timers: Array<number> = [];
    const fakeWindow: Record<string, unknown> = {
      setTimeout: ((fn: () => void) => {
        timers.push(0);
        pending.push(fn);
        return 0;
      }) as unknown,
      clearTimeout: () => {},
    };

    const factory = new Function(
      "window",
      "codexAppServerModelRequestPatchVersion",
      "codexRemoteSessionProviderPatchEnabled",
      "loadAppServerRequestCandidates",
      "patchAppServerModelRequestClient",
      "sendCodexPlusDiagnostic",
      "Date",
      `${source}\nreturn installAppServerModelRequestPatch;`,
    );

    const install = factory(
      fakeWindow,
      1,
      // provider patch 开关两态都要测：以前 enabled 时走提前 return 绕过熔断。
      () => true,
      () =>
        new Promise((resolve) => {
          sweeps += 1;
          pending.push(() => resolve({ modules: [{}], candidates: [{}], sources: [], discovery: "fallback" }));
        }),
      () => patchSucceeds,
      (event: string) => diagnostics.push(event),
      Date,
    ) as () => void;

    const settle = async () => {
      // 重试定时器是挂起的回调：排空 sweep 再触发到期的 retry，直到没有新定时器。
      for (let round = 0; round < 32; round += 1) {
        if (!pending.length) break;
        const flushSweeps = pending;
        pending = [];
        flushSweeps.forEach((resolve) => resolve());
        await Promise.resolve();
        await Promise.resolve();
        await Promise.resolve();
      }
    };

    return { install, sweeps: () => sweeps, diagnostics: () => diagnostics, settle };
  }

  it("does not start a new sweep while the previous one is still running", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), false);

    for (let i = 0; i < 20; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    await harness.settle();
  });

  it("stops retrying via the provider path once maxMisses is reached", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), false);

    // 反复 install + settle，让每轮 miss 走完 provider 重试调度。
    for (let i = 0; i < 40; i += 1) {
      harness.install();
      await harness.settle();
    }

    // 关键回归断言：以前 provider 路径无限重试（40 轮 = 40 次 sweep），
    // 现在到 maxMisses(8) 就熔断停手。
    assert.equal(harness.sweeps(), 8);
    assert.equal(harness.diagnostics().filter((e) => e === "model_app_server_request_patch_not_found").length, 1);
    assert.deepEqual(harness.diagnostics().at(-1), "model_app_server_request_patch_skipped");
    const settled = harness.sweeps();
    harness.install();
    await harness.settle();
    assert.equal(harness.sweeps(), settled);
  });

  it("keeps working normally when the patch actually lands", async () => {
    const harness = appServerPatchRuntime(await readFile(rendererPath, "utf8"), true);

    harness.install();
    await harness.settle();
    for (let i = 0; i < 10; i += 1) harness.install();

    assert.equal(harness.sweeps(), 1);
    assert.deepEqual(harness.diagnostics(), ["model_app_server_request_patch_installed"]);
  });
});

// renderer-inject.js 现在是「分片源码 + 生成产物」：真正的源码在
// assets/inject/renderer-inject/*.js，产物是拼回去的单体文件，供 assets.rs 的
// include_str! 和本文件的大量按路径断言消费。
// 这组用例防止有人只改产物、不回改分片，让两边悄悄分叉。
describe("renderer inject 分片与产物一致", () => {
  const fragmentDir = new URL("../../../assets/inject/renderer-inject/", import.meta.url);
  const artifactPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  async function assembleFromFragments(): Promise<{ source: string; names: string[] }> {
    const manifest = JSON.parse(
      await readFile(new URL("manifest.json", fragmentDir), "utf8"),
    ) as { fragments: Array<{ name: string; description?: string }> };
    assert.ok(
      Array.isArray(manifest.fragments) && manifest.fragments.length > 0,
      "manifest.json 必须有 fragments",
    );
    const bodies = await Promise.all(
      manifest.fragments.map(async (fragment) => {
        assert.ok(fragment.name, "分片必须带 name");
        const body = await readFile(new URL(fragment.name, fragmentDir), "utf8");
        // 每片以换行收尾，直接相接才能还原原始行结构；漏了会让两处声明粘连。
        assert.ok(body.endsWith("\n"), `分片 ${fragment.name} 未以换行结尾`);
        return body;
      }),
    );
    return { source: bodies.join(""), names: manifest.fragments.map((f) => f.name) };
  }

  it("分片按 manifest 顺序拼接后与产物逐字节一致", async () => {
    const { source, names } = await assembleFromFragments();
    const artifact = await readFile(artifactPath, "utf8");
    assert.equal(
      source,
      artifact,
      `产物与分片不一致（分片 ${names.length} 个）。` +
        "改分片后请跑 node scripts/assemble-renderer-inject.mjs 重新组装。",
    );
  });

  it("拼接结果是完整可解析的单个 IIFE", async () => {
    const { source } = await assembleFromFragments();
    assert.ok(source.startsWith("(() => {\n"), "必须以 IIFE 开头");
    assert.ok(source.includes("\n})();\n"), "必须包含主 IIFE 的收尾");
    // 语法解析（不执行）能抓出分片切坏造成的括号不匹配。
    assert.doesNotThrow(() => new Function(source));
  });

  it("测试辅助依赖的锚点仍落在同一个分片里且顺序不变", async () => {
    // installRendererStyle() 用 indexOf 划区间，靠这两个标记定位。
    // 它们一旦被拆到不同分片、或前后顺序反转，helper 会静默取到空区间。
    const { source } = await assembleFromFragments();
    const start = source.indexOf("  function installStyle()");
    const end = source.indexOf("\n  function defaultCodexPlusSettings", start);
    assert.ok(start >= 0, "找不到 installStyle 锚点");
    assert.ok(end > start, "defaultCodexPlusSettings 必须排在 installStyle 之后");
  });

  it("拓展列表渲染不裸取 entry.item", async () => {
    // 已安装条目的形状是 { kind, key, name, meta, enabled, script }，没有 item；
    // 只有市场条目才有。裸取 entry.item.description 会在列表里只要有任意一个
    // 已安装脚本时就抛 TypeError，整块渲染中断，左面板停在「正在读取…」占位。
    // 已安装条目要走 codexPlusExtensionMarketItem(entry) 按 market_id 回查。
    const { source } = await assembleFromFragments();
    const openingTag = source.indexOf("  function renderCodexPlusExtensionsNav()");
    const closingTag = source.indexOf("\n  /** 左面板内容变了就整块重绘", openingTag);
    assert.ok(openingTag >= 0, "找不到 renderCodexPlusExtensionsNav");
    assert.ok(closingTag > openingTag, "找不到函数结束位置");
    const body = source.slice(openingTag, closingTag);
    assert.ok(
      !/entry\.item\s*\./.test(body),
      "itemHtml 里出现了不带可选链的 entry.item.*，会在已安装条目上抛 TypeError",
    );
    assert.ok(
      body.includes("codexPlusExtensionMarketItem(entry)"),
      "itemHtml 应该用 codexPlusExtensionMarketItem(entry) 解析图标与简介来源",
    );
  });
});

/**
 * 拓展接口层的契约测试。
 *
 * 这些测试把 01-registry.js 与 91-extension-api.js 的定义抽出来单独执行，
 * 不依赖真实 DOM——注册中心本身刻意不读 DOM，正是为了让它可以这样被验证。
 */
describe("拓展注册中心", () => {
  const rendererPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  interface RegistryHarness {
    registry: { rowActions: Map<string, unknown>; navEntries: Map<string, unknown>; pages: Map<string, unknown>; menuItems: Map<string, unknown> };
    register: (kind: string, reg: Map<string, unknown>, id: string, def: unknown, key: string) => () => void;
    registerSelector: (selector: string) => boolean;
    extensionSelector: () => string;
    isExtensionNode: (node: unknown) => boolean;
    items: (reg: Map<string, unknown>) => Array<Record<string, unknown>>;
    runCallback: (key: string, label: string, cb: () => unknown) => { ok: boolean; value?: unknown; error?: string };
    failures: () => Array<{ script_key: string; message: string }>;
    registryLog: () => unknown[];
  }

  async function registryRuntime(): Promise<RegistryHarness> {
    // 直接读注册中心分片本身：它刻意不读 DOM、不依赖 prelude 常量，所以能独立执行。
    // 不要从产物里 indexOf 切片——注册中心与接口层之间隔着十来个分片，区间会失控。
    const source = await readFile(
      new URL("../../../assets/inject/renderer-inject/01-registry.js", import.meta.url),
      "utf8",
    );

    const windowValue: Record<string, unknown> = {};
    const documentValue = {
      // registerCodexPlusExtensionSelector 用空 fragment 验证选择器语法，
      // 非法选择器会抛错，模拟这个方法就足以覆盖合法性判定。
      createDocumentFragment: () => ({
        querySelector: (selector: string) => {
          if (/[<>]/.test(selector)) throw new Error("invalid selector");
          return null;
        },
      }),
    };
    const factory = new Function(
      "window",
      "document",
      `${source}
return { codexPlusRegistry, registerCodexPlusExtension, registerCodexPlusExtensionSelector,
  codexPlusExtensionSelector, isCodexPlusExtensionNode, codexPlusExtensionItems,
  runCodexPlusExtensionCallback };`,
    );
    const api = factory(windowValue, documentValue) as Record<string, unknown>;
    return {
      registry: api.codexPlusRegistry as RegistryHarness["registry"],
      register: api.registerCodexPlusExtension as RegistryHarness["register"],
      registerSelector: api.registerCodexPlusExtensionSelector as RegistryHarness["registerSelector"],
      extensionSelector: api.codexPlusExtensionSelector as RegistryHarness["extensionSelector"],
      isExtensionNode: api.isCodexPlusExtensionNode as RegistryHarness["isExtensionNode"],
      items: api.codexPlusExtensionItems as RegistryHarness["items"],
      runCallback: api.runCodexPlusExtensionCallback as RegistryHarness["runCallback"],
      failures: () => (windowValue.__codexPlusExtensionFailures || []) as Array<{ script_key: string; message: string }>,
      registryLog: () => (windowValue.__codexPlusRegistryLog || []) as unknown[],
    };
  }

  it("第三方项从 1000 起排，不会插到内置项前面", async () => {
    const runtime = await registryRuntime();
    runtime.register("rowAction", runtime.registry.rowActions, "builtin:a", { order: 10, onActivate() {} }, "builtin");
    runtime.register("rowAction", runtime.registry.rowActions, "ext:b", { order: 5, onActivate() {} }, "user:x.js");
    const items = runtime.items(runtime.registry.rowActions);
    // 传入的 5 被抬到 1000：内置项永远排在拓展项之前。
    assert.deepEqual(items.map((item) => item.id), ["builtin:a", "ext:b"]);
    assert.equal(items[1].order, 1000);
  });

  it("按脚本配额限制注册数量，超出时抛错", async () => {
    const runtime = await registryRuntime();
    for (let index = 0; index < 16; index += 1) {
      runtime.register("navEntry", runtime.registry.navEntries, `ext:${index}`, { onActivate() {} }, "user:noisy.js");
    }
    assert.throws(
      () => runtime.register("navEntry", runtime.registry.navEntries, "ext:overflow", { onActivate() {} }, "user:noisy.js"),
      /最多注册 16 项/,
    );
    // 另一个脚本不受影响。
    assert.doesNotThrow(
      () => runtime.register("navEntry", runtime.registry.navEntries, "ext:other", { onActivate() {} }, "user:other.js"),
    );
  });

  it("拒绝重复 id 与缺少回调的定义", async () => {
    const runtime = await registryRuntime();
    runtime.register("page", runtime.registry.pages, "ext:p", { render() {} }, "user:x.js");
    assert.throws(
      () => runtime.register("page", runtime.registry.pages, "ext:p", { render() {} }, "user:x.js"),
      /已被占用/,
    );
    assert.throws(
      () => runtime.register("page", runtime.registry.pages, "ext:empty", {}, "user:x.js"),
      /必须提供 render \/ onActivate/,
    );
  });

  it("菜单项的开关形态（只给 onChange）不被校验误拒", async () => {
    const runtime = await registryRuntime();
    // 开关形态没有 render / onActivate，早期实现会把它当成非法定义拒掉。
    assert.doesNotThrow(
      () => runtime.register("menuItem", runtime.registry.menuItems, "ext:toggle", { onChange() {} }, "user:x.js"),
    );
    assert.doesNotThrow(
      () => runtime.register("page", runtime.registry.pages, "ext:cleanup-only", { onCleanup() {} }, "user:x.js"),
    );
  });

  it("dispose 只移除自己注册的那一项", async () => {
    const runtime = await registryRuntime();
    const dispose = runtime.register("page", runtime.registry.pages, "ext:first", { render() {} }, "user:x.js");
    runtime.register("page", runtime.registry.pages, "ext:second", { render() {} }, "user:y.js");
    dispose();
    assert.deepEqual([...runtime.registry.pages.keys()], ["ext:second"]);
    // 重复 dispose 不应误删后来者。
    dispose();
    assert.deepEqual([...runtime.registry.pages.keys()], ["ext:second"]);
  });

  it("选择器登记拒绝非法语法，避免 closest() 在每次 mutation 上抛错", async () => {
    const runtime = await registryRuntime();
    assert.equal(runtime.registerSelector('[data-codex-plus-ext="a"]'), true);
    assert.match(runtime.extensionSelector(), /data-codex-plus-ext/);
    assert.equal(runtime.registerSelector("div << p"), false);
    assert.equal(runtime.registerSelector(""), false);
  });

  it("回调抛错时记进该脚本的失败通道，不向调用方抛出", async () => {
    const runtime = await registryRuntime();
    const outcome = runtime.runCallback("user:broken.js", "rowAction.onActivate", () => {
      throw new Error("boom");
    });
    assert.equal(outcome.ok, false);
    assert.match(String(outcome.error), /boom/);
    assert.equal(runtime.failures().length, 1);
    assert.equal(runtime.failures()[0].script_key, "user:broken.js");
  });
});

/**
 * 对外接口层的护栏测试。
 *
 * 接口层挂在 window.codexPlus 上、面向第三方脚本，所以「哪些能力被开放」必须
 * 有测试守着——新增路由时默认不开放，漏网才是 bug。
 */
describe("拓展接口层", () => {
  const apiFragmentPath = new URL(
    "../../../assets/inject/renderer-inject/91-extension-api.js",
    import.meta.url,
  );
  const artifactPath = new URL("../../../assets/inject/renderer-inject.js", import.meta.url);

  async function readRouteWhitelist(): Promise<string[]> {
    const source = await readFile(apiFragmentPath, "utf8");
    const start = source.indexOf("const codexPlusExtensionRoutes = new Set([");
    const end = source.indexOf("]);", start);
    assert.ok(start >= 0 && end > start, "找不到路由白名单");
    const body = source.slice(start, end);
    return Array.from(body.matchAll(/"([^"]+)"/g), ([, route]) => route);
  }

  it("路由白名单只放只读能力，不含设置写入与远端控制", async () => {
    const routes = await readRouteWhitelist();
    assert.ok(routes.length > 0, "白名单不能为空");
    // 这些路由一旦开放，第三方脚本就能改用户配置或操作远端，必须显式决策后才能加。
    for (const forbidden of ["/settings/set", "/settings/get", "/delete", "/undo", "/share/create"]) {
      assert.ok(!routes.includes(forbidden), `${forbidden} 不应默认开放给拓展`);
    }
    assert.ok(routes.includes("/diagnostics/log"), "诊断上报应保持开放");
  });

  it("接口对象在 IIFE 收尾前挂载", async () => {
    const renderer = await readFile(artifactPath, "utf8");
    const mount = renderer.indexOf("window.codexPlus = buildCodexPlusExtensionApi();");
    const tail = renderer.indexOf("\n})();\n");
    assert.ok(mount >= 0, "找不到 window.codexPlus 的挂载点");
    // 挂载必须在主 IIFE 结束之前，否则闭包里的函数已经不可达。
    assert.ok(mount < tail, "挂载点必须位于 IIFE 收尾之前");
  });
});

/**
 * 菜单项的接入方式护栏。
 *
 * 菜单的接入点是「在 home 面板模板末尾追加一块」，不是把内置的一百多行模板
 * 拆成数据结构——后者会动到内置 UI 主干。这些断言守住这个决定，以及点击
 * 委托的分支顺序。
 */
describe("拓展菜单项", () => {
  const settingsPath = new URL(
    "../../../assets/inject/renderer-inject/40-backend-settings.js",
    import.meta.url,
  );
  const hostPath = new URL(
    "../../../assets/inject/renderer-inject/92-extension-host.js",
    import.meta.url,
  );

  it("内置菜单模板保持原样，只追加一个拓展挂载点", async () => {
    const source = await readFile(settingsPath, "utf8");
    // 挂载点在 home 面板里，且位于「提出问题」之后（即内置项末尾）。
    const openIdx = source.indexOf("overlay.innerHTML = `");
    const mountIdx = source.indexOf("${renderCodexPlusExtensionMenuRows()}");
    const issueIdx = source.indexOf("提出问题");
    assert.ok(mountIdx > 0, "找不到拓展菜单挂载点");
    assert.ok(mountIdx > issueIdx, "挂载点应位于内置项之后");
    // 关键：内置的行仍然是内联模板，没有被拆成数组。
    assert.ok(
      source.slice(openIdx, mountIdx).includes('class="codex-plus-row"'),
      "内置菜单行应当仍是内联模板",
    );
    assert.ok(
      !/const codexPlusBuiltinMenuRows\s*=/.test(source),
      "不应把内置菜单行抽成数组",
    );
  });

  it("点击委托里拓展分支排在内置分支之前", async () => {
    const source = await readFile(settingsPath, "utf8");
    const handler = source.indexOf('overlay.addEventListener("click"');
    assert.ok(handler > 0, "找不到点击委托");
    const body = source.slice(handler, handler + 600);
    const extIdx = body.indexOf("handleCodexPlusExtensionMenuClick(target)");
    const devtoolsIdx = body.indexOf("data-codex-open-devtools");
    assert.ok(extIdx > 0, "点击委托应当调用拓展菜单处理器");
    assert.ok(extIdx < devtoolsIdx, "拓展分支应排在内置分支之前");
  });

  it("开关与按钮分别渲染成不同的控件", async () => {
    const source = await readFile(hostPath, "utf8");
    const start = source.indexOf("function renderCodexPlusExtensionMenuRows()");
    const end = source.indexOf("function handleCodexPlusExtensionMenuClick(", start);
    assert.ok(start > 0 && end > start, "找不到菜单渲染函数");
    const body = source.slice(start, end);
    assert.match(body, /data-codex-plus-ext-setting/, "开关应有自己的 data 属性");
    assert.match(body, /data-codex-plus-ext-action/, "按钮应有自己的 data 属性");
    // 是追加到 home 面板，不是替换它。
    assert.ok(!/panel\.innerHTML\s*=/.test(body), "不应整体替换面板内容");
  });

  it("菜单项失败经由脚本状态上报，不向调用方抛出", async () => {
    const source = await readFile(hostPath, "utf8");
    const start = source.indexOf("function handleCodexPlusExtensionMenuClick(");
    const body = source.slice(start, start + 1400);
    assert.match(body, /runCodexPlusExtensionCallback/, "回调必须经失败隔离包装");
    assert.ok(!/try\s*{[\s\S]*item\.onActivate\(/.test(body), "不应裸调 onActivate");
  });
});
