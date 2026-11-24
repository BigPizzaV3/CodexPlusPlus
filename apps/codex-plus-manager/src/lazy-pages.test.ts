import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import test from "node:test";

// 不常用的页面按需加载：主界面（App 及其静态依赖）里不能再静态引入它们，否则 Vite 会把它们打回主包。
const LAZY_MODULES = [
  "GatewayPage", "HubOverview", "SkillsPage", "ComputerUsePage", "AgentCollaborationPage", "PluginMarketScreen",
  "RemoteControlPage", "ClaudeDesktopPage", "WeixinPage", "ExtensionsPage", "ExtensionManagerPanel",
  "SettingsScreens", "agent-cache", "OnboardingTour", "RequestLogScreen",
  // 只被上面这些页面用到的大模块，也要跟着按需加载。
  "GatewayProviders", "GatewayAgents", "GatewayUsage", "ComputerUsePreview", "DshPluginsPanel",
  "CommunityExtensionsPanel", "ConfigDoctorPanel",
];

const src = new URL("./", import.meta.url);

function resolve(from: URL, spec: string): URL | null {
  let base: URL;
  if (spec.startsWith("@/")) base = new URL(spec.slice(2), src);
  else if (spec.startsWith(".")) base = new URL(spec, from);
  else return null;
  for (const ext of ["", ".tsx", ".ts"]) {
    const candidate = new URL(base.href + ext);
    if (existsSync(candidate) && statSync(candidate).isFile()) return candidate;
  }
  return null;
}

/** 从入口出发、只沿静态 import（不含 import() 与纯类型引入）能走到的本地模块。 */
function staticClosure(entry: string): Set<string> {
  const seen = new Set<string>();
  const walk = (file: URL) => {
    const name = file.pathname.slice(src.pathname.length).replace(/\.tsx?$/, "");
    if (seen.has(name)) return;
    seen.add(name);
    const text = readFileSync(file, "utf8");
    for (const match of text.matchAll(/^\s*(?:import|export)\s+(?!type\b)(?:[^"';]*?\s+from\s+)?["']([^"']+)["']/gm)) {
      if (match[1].endsWith(".css") || match[1].endsWith(".json")) continue;
      const target = resolve(file, match[1]);
      if (target) walk(target);
    }
  };
  walk(new URL(entry, src));
  return seen;
}

test("按需加载的页面不在主界面的静态依赖里", () => {
  const eager = staticClosure("App.tsx");
  for (const name of LAZY_MODULES) assert.ok(!eager.has(name), `${name} 被主界面静态引入，会打回主包`);
  // 首页和侧栏仍随主界面一起加载。
  for (const name of ["HomeScreen", "AppSidebar", "RelayScreen", "routes"]) assert.ok(eager.has(name), name);
});

test("App 用 React.lazy 引入这些页面，内容区有 Suspense 兜底", () => {
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  for (const [component, module] of [
    ["GatewayPage", "GatewayPage"], ["SkillsPage", "SkillsPage"], ["ClaudeLocalizationScreen", "ClaudeDesktopPage"],
    ["WeixinConnectScreen", "WeixinPage"], ["UserScriptsScreen", "ExtensionsPage"], ["GlobalSettingsScreen", "SettingsScreens"],
    ["OnboardingTour", "OnboardingTour"], ["RequestLogScreen", "RequestLogScreen"],
  ]) {
    assert.ok(app.includes(`const ${component} = lazyNamed(() => import("./${module}"), "${component}");`), component);
  }
  assert.match(app, /<Suspense fallback=\{<PageFallback \/>\}>\n\s*\{extensionPageView \?\? \(<>/);
});

test("托盘面板不会因为拆分带上主界面", () => {
  const tray = staticClosure("TrayPanel.tsx");
  for (const name of ["App", "app-shared", "RelayEditor", "SettingsScreens", "SkillsPage"]) assert.ok(!tray.has(name), name);
});

test("按需加载的页面启动后空闲时预加载；代码已到手的直接渲染，不再挂起一拍", () => {
  const lazy = readFileSync(new URL("./lazy-page.tsx", import.meta.url), "utf8");
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  // 每个 lazyNamed 登记自己的加载函数，App 挂载后启动预加载，卸载时取消。
  assert.match(lazy, /loaders\.push\(factory\);/);
  assert.match(app, /useEffect\(\(\) => preloadLazyPages\(\), \[\]\);/);
  // 一次读一个、中间留空隙；某个失败不影响后面的。
  assert.match(lazy, /void loaders\[index\]\(\)\n\s*\.catch\(\(\) => \{\}\)\n\s*\.then\(\(\) => \{\n\s*if \(!cancelled\) timer = window\.setTimeout\(\(\) => next\(index \+ 1\), PRELOAD_GAP_MS\);/);
  assert.match(lazy, /timer = window\.setTimeout\(\(\) => next\(0\), PRELOAD_START_MS\);/);
  assert.match(lazy, /cancelled = true;\n\s*window\.clearTimeout\(timer\);/);
  // 读失败不记一辈子：下次渲染或下次预加载再试。
  assert.match(lazy, /\(error: unknown\) => \{[\s\S]*?pending = null;\n\s*throw error;/);
  // 用真正的组件还是 React.lazy 在挂载时定下来，之后不换（页面不会被重新挂载）。
  assert.match(lazy, /const \[Component\] = useState<ComponentType<any>>\(\(\) => loaded \?\? Lazy\);/);
});

test("页面代码或数据还没到时用和内容形状接近的占位，不是一行「读取中…」", () => {
  const lazy = readFileSync(new URL("./lazy-page.tsx", import.meta.url), "utf8");
  const layout = readFileSync(new URL("./cx-layout.tsx", import.meta.url), "utf8");
  const css = readFileSync(new URL("./cursor-ui.css", import.meta.url), "utf8");
  // 整页占位：前 150ms 不画（本地加载通常更快），再慢才显示。
  assert.match(lazy, /return shown \? <CxPageSkeleton \/> : null;/);
  assert.doesNotMatch(lazy, /className="empty"/);
  // 占位行就是一行 CxRow 的形状（同一个行高），文字只给读屏。
  assert.match(layout, /<div aria-hidden="true" className="cx-row cx-loading-row" key=\{index\}>/);
  assert.match(layout, /<div aria-busy="true" aria-label=\{label \?\? t\("读取中…"\)\} className="cx-loading" role="status">/);
  assert.match(layout, /<div aria-busy="true" aria-label=\{t\("读取中…"\)\} className="cx-page cx-page-skeleton" role="status">/);
  assert.match(css, /:root \.shell \.cx-skeleton,\n:root \.shell \.cx-row-icon\.cx-skeleton \{[^}]*animation: cx-skeleton-shift 1\.4s ease-in-out infinite;/);
  // 各页面数据没到时都用它，不再拿空状态（CxEmpty）写一行「正在读取…」。
  for (const file of ["AgentSessions.tsx", "ClaudePageExtensions.tsx", "ClaudeProviders.tsx", "GatewayAgents.tsx", "GatewayPage.tsx", "ManagementPages.tsx", "CodexConnectionMode.tsx", "RequestLogScreen.tsx", "SkillsPage.tsx"]) {
    const text = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
    assert.match(text, /<CxLoading\b/, file);
    assert.doesNotMatch(text, /<CxEmpty title=\{t\("正在读取[^"]*"\)\} \/>/, file);
  }
});
