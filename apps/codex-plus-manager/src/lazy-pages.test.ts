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
