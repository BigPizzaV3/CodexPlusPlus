import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { claudeParts, extensionEnabledForHost, extensionStateForHost } from "./extension-host-model.ts";
import { managerExtensionPages, managerPageOverrideConflicts } from "./extension-manager-pages.ts";
import { EN_TEMPLATE } from "./i18n-en.ts";

// 拓展的管理器页面在界面上的三件事：本地拓展行的「打开面板」与侧栏入口同一个条件、
// 顶替被别的拓展抢先时行上有警告标签、被顶替的页面仍显示所在分组的页签条。

const read = (name: string) => readFileSync(new URL(name, import.meta.url), "utf8");
const appText = read("./App.tsx");
const pageText = read("./ExtensionsPage.tsx");
const page = ts.createSourceFile("ExtensionsPage.tsx", pageText, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function functionSource(name: string) {
  const node = page.statements.find(
    (statement): statement is ts.FunctionDeclaration => ts.isFunctionDeclaration(statement) && statement.name?.text === name,
  );
  assert.ok(node, name);
  return node.getText(page).replace(/^export /, "");
}

type Element = { type: string; props: Record<string, any> };
function elements(node: any): Element[] {
  if (!node || typeof node !== "object") return [];
  if (Array.isArray(node)) return node.flatMap(elements);
  return [node, ...elements(node.props?.children), ...elements(node.props?.badges), ...elements(node.props?.detail)];
}
const textOf = (node: any): string => (node == null || typeof node === "boolean" ? "" : typeof node !== "object" ? String(node) : Array.isArray(node) ? node.map(textOf).join("") : textOf(node.props?.children));

/** 路由表在这里只取页面名：和 routes.ts 里这几页的 label 一致。 */
const routeLabels: Record<string, string> = { gatewayUsage: "用量", gatewaySettings: "设置", hub: "本地服务总览", sessions: "会话" };

/** 执行真实的 ScriptRow（React 换成只记录元素树的桩），返回它渲染出来的元素。 */
function renderRow(props: Record<string, unknown>) {
  const source = ["ScriptRow", "localScriptStateLabel", "localizedPackageText", "managerRouteLabel"].map(functionSource).join("\n");
  const componentNames = [...source.matchAll(/<([A-Z]\w*)\b/g)].map((match) => match[1]);
  const states: unknown[] = [];
  let cursor = 0;
  const context = vm.createContext({
    ...Object.fromEntries(componentNames.map((name) => [name, name])),
    React: { Fragment: "Fragment", createElement: (type: string, elementProps: object | null, ...children: unknown[]) => ({ type, props: { ...elementProps, children } }) },
    t: (value: string) => value,
    tf: (value: string, args: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    getLanguage: () => "zh",
    useState: (initial: unknown) => {
      const index = cursor++;
      if (!(index in states)) states[index] = initial;
      return [states[index], (next: unknown) => { states[index] = typeof next === "function" ? (next as (value: unknown) => unknown)(states[index]) : next; }];
    },
    claudeParts, extensionEnabledForHost, extensionStateForHost,
    isKnownRoute: (id: string) => Object.hasOwn(routeLabels, id),
    routeTitle: (id: string) => routeLabels[id],
    localServiceRoutes: ["gatewayProviders", "gatewayRouting", "gateway", "gatewaySettings"],
  });
  vm.runInContext(ts.transpileModule(source, { compilerOptions: { jsx: ts.JsxEmit.React, target: ts.ScriptTarget.ES2022 } }).outputText, context);
  const render = () => { cursor = 0; return elements(context.ScriptRow(props)); };
  return { render };
}

const extension = (extra: Record<string, unknown> = {}) => ({
  key: "user:board", name: "看板", source: "user", kind: "package", enabled: true, status: "loaded", error: "", manifestError: "",
  package: { id: "board", version: "1.0.0", description: "", engine: 1, routes: [], managerPage: { title: "看板页", replaces: ["gatewayUsage", "gatewaySettings"], data: [] } },
  ...extra,
});
const actions = new Proxy({}, { get: () => () => Promise.resolve() });
const openPanel = (nodes: Element[]) => nodes.find((node) => node.type === "MenuItem" && /打开面板|收起面板/.test(textOf(node)));

test("行菜单「打开面板」和侧栏入口同一个条件：运行出错不影响，清单有错或不可用时没有", () => {
  // 拓展在 Codex 页面里运行出错（status failed + error），清单没问题：页面可用，面板能打开。
  const crashed = extension({ status: "failed", error: "TypeError: boom" });
  const available = (script: ReturnType<typeof extension>, enabled = true) => managerExtensionPages({ enabled, scripts: [script] }).some((item) => item.key === script.key);
  assert.equal(available(crashed), true);
  const row = renderRow({ host: "codex", script: crashed, globalEnabled: true, actions, managerPageAvailable: available(crashed) });
  const item = openPanel(row.render());
  assert.ok(item, "运行出错的拓展仍能打开面板");
  item.props.onClick();
  assert.ok(row.render().some((node) => node.type === "ExtensionManagerPanel" && node.props.scriptKey === "user:board"));
  // 清单有错：侧栏没有它的入口，行菜单里也没有「打开面板」。
  const broken = extension({ status: "failed", error: "坏清单", manifestError: "坏清单" });
  assert.equal(available(broken), false);
  assert.equal(openPanel(renderRow({ host: "codex", script: broken, globalEnabled: true, actions, managerPageAvailable: available(broken) }).render()), undefined);
  // 总开关关着：侧栏入口没有，这里也没有。
  assert.equal(available(extension(), false), false);
  assert.equal(openPanel(renderRow({ host: "codex", script: extension(), globalEnabled: false, actions, managerPageAvailable: false }).render()), undefined);
  // Claude 宿主下本来就没有管理器面板。
  assert.equal(openPanel(renderRow({ host: "claude", script: extension(), globalEnabled: true, actions, managerPageAvailable: true }).render()), undefined);
  // 页面把 App 算好的那一份传给每一行。
  assert.match(pageText, /managerPageAvailable=\{codexHost && managerPages\.some\(\(page\) => page\.key === script\.key\)\}/);
  assert.match(pageText, /const managerPage = codexHost && enabled && managerPageAvailable \? script\.package\?\.managerPage \?\? null : null;/);
  assert.match(appText, /<UserScriptsScreen [^\n]*managerPages=\{extensionPages\} managerPageConflicts=\{pageOverrideConflicts\}/);
  assert.match(appText, /const pageOverrideConflicts = useMemo\(\(\) => managerPageOverrideConflicts\(extensionPages\), \[extensionPages\]\);/);
});

test("顶替被别的拓展抢先时，行上有警告标签：说清哪一页、被谁抢先", () => {
  const warnBadges = (nodes: Element[]) => nodes.filter((node) => node.type === "CxBadge" && node.props.tone === "warn").map((node) => textOf(node));
  // 没有冲突：只有来源标签；能力说明里的路由 id 换成了页面名，本地服务的页签带上分组名。
  let nodes = renderRow({ host: "codex", script: extension(), globalEnabled: true, actions, managerPageAvailable: true }).render();
  assert.deepEqual(warnBadges(nodes), []);
  const detail = nodes.find((node) => node.type === "span" && typeof node.props.title === "string" && node.props.title.includes("顶替管理器页面"));
  assert.ok(detail);
  assert.match(detail.props.title, /顶替管理器页面：用量、本地服务 · 设置/);
  // 有冲突：从清单一路算出来的结果交给这一行。
  const first = { ...extension(), key: "user:first", name: "先来的" };
  const second = extension();
  const conflicts = managerPageOverrideConflicts(managerExtensionPages({ enabled: true, scripts: [first, second] }));
  assert.deepEqual(conflicts["user:board"].map((conflict) => conflict.routes), [["gatewayUsage", "gatewaySettings"]]);
  nodes = renderRow({
    host: "codex", script: second, globalEnabled: true, actions, managerPageAvailable: true,
    overrideConflicts: [{ by: "先来的", routes: ["gatewayUsage"] }, { by: "另一个", routes: ["hub", "unknownRoute"] }],
  }).render();
  assert.deepEqual(warnBadges(nodes), [
    "顶替「用量」未生效：已被「先来的」顶替",
    "顶替「本地服务总览、unknownRoute」未生效：已被「另一个」顶替",
  ]);
  // 资源超限的标签照旧是短标签，完整内容在悬停里。
  nodes = renderRow({
    host: "codex", globalEnabled: true, actions, managerPageAvailable: true,
    script: extension({ package: { ...extension().package, skippedAssets: ["big.png"] } }),
  }).render();
  const skipped = nodes.find((node) => node.type === "CxBadge" && node.props.tone === "warn");
  assert.equal(textOf(skipped), "资源超限");
  assert.equal(skipped?.props.title, "big.png");
  // 抢先者按本地列表里显示的名字说；只在 Codex 宿主下提示。
  assert.match(pageText, /\(codexHost \? managerPageConflicts\[key\] \?\? \[\] : \[\]\)\.map\(/);
  assert.match(pageText, /by: winner \? localizedPackageText\(winner\.package\?\.locales, "title", winner\.name\) : conflict\.by\.title/);
  assert.equal(typeof EN_TEMPLATE["顶替「{0}」未生效：已被「{1}」顶替"], "string", "新文案有英文");
});

test("被顶替的页面仍显示所在分组的页签条", () => {
  const replaced = appText.indexOf("{extensionPageView ?? (<>");
  const head = appText.indexOf('<header className="cx-page-head"');
  assert.ok(head >= 0 && replaced > head);
  // 「本地服务」「用量」「MCP 与 Skills」的页签条都在「拓展页面 ?? 内置页面」那一块的外面、标题行的下面：
  // 页面被拓展顶替时照常渲染，同组的其它页还能直接切过去。
  for (const tabs of ["<CustomizeTabs ", "<UsageTabs ", "<LocalServiceTabs "]) {
    const at = appText.indexOf(tabs, head);
    assert.ok(at > head && at < replaced, `${tabs.trim()} 要在被顶替的那一块外面`);
    assert.equal(appText.indexOf(tabs, replaced), -1, `${tabs.trim()} 只渲染一处`);
  }
  // 页签条覆盖的路由不变：用量的两页、本地服务的四页加旧总览。
  assert.match(appText, /\{route === "gatewayUsage" \|\| route === "requestLog" \? <UsageTabs /);
  assert.match(appText, /\{isLocalServiceRoute\(route\) \? <LocalServiceTabs /);
  // 横幅跟着页面，在页签条下面：「使用拓展页面」在页签条之后，「查看内置页面」在拓展页面容器里。
  const restorable = appText.indexOf("{restorableOverride ? (", head);
  assert.ok(restorable > appText.indexOf("<LocalServiceTabs ", head) && restorable < replaced);
  assert.match(appText, /<div className="extension-page-host">\n\s*\{overridingPage \? \(\n\s*<div className="extension-page-banner">/);
  // 「请求」页的铺满布局（整页不滚动）只给内置页面，拓展页面照普通页面排，不会被裁掉。
  assert.match(appText, /route === "requestLog" && !shownExtensionPage \? "request-log-screen" : ""/);
  // 切回内置的「请求」页时多一条横幅：它单独占一行，日志卡仍占满剩余高度。
  const css = read("./cx-pages-gateway.css");
  assert.match(appText, /\$\{restorableOverride \? "has-override-banner" : ""\}/);
  assert.match(css, /\.screen\.request-log-screen\.has-override-banner \{\r?\n\s*grid-template-rows: auto auto auto minmax\(0, 1fr\);/);
});
