import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as gateway from "./gateway-model.ts";
import * as timeline from "./quota-timeline.ts";

type Element = { type: string; props: Record<string, any> };
const text = (node: any): string => node == null || typeof node === "boolean" ? "" : Array.isArray(node) ? node.map(text).join("") : typeof node === "object" ? text(node.props?.children) : String(node);
const elements = (node: any): Element[] => node == null || typeof node !== "object" ? [] : Array.isArray(node) ? node.flatMap(elements) : [node, ...elements(node.props?.children)];

function harness() {
  const hooks: any[] = [];
  let cursor = 0;
  const jsx = (type: unknown, props: unknown) => ({ type, props });
  const modules: Record<string, unknown> = {
    react: {
      useState: (initial: any) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = typeof initial === "function" ? initial() : initial;
        return [hooks[index], (value: any) => { hooks[index] = typeof value === "function" ? value(hooks[index]) : value; }];
      }, useEffect: () => {},
    },
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "lucide-react": { ChevronLeft: "ChevronLeft", ChevronRight: "ChevronRight" },
    "./components/ui/button": { Button: "button" },
    "./i18n": { getLanguage: () => "zh", t: (s: string) => s, tf: (s: string, args: unknown[]) => s.replace(/\{(\d+)\}/g, (_, n) => String(args[Number(n)])) },
    "./gateway-model": gateway,
    "./quota-timeline": timeline,
    "./ProviderQuotaPanel.css": {},
  };
  const exports: Record<string, any> = {};
  vm.runInNewContext(ts.transpileModule(readFileSync(new URL("./ProviderQuotaPanel.tsx", import.meta.url), "utf8"), {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
  }).outputText, { exports, require: (name: string) => { assert.ok(name in modules, `unexpected import ${name}`); return modules[name]; } });
  return { render: (name: string, props: unknown) => { cursor = 0; return exports[name](props) as Element; } };
}

function account(): gateway.GatewayPoolEntry {
  const now = Date.now();
  return { id: "a", name: "GPT account", account: "chatgpt", baseUrl: "", protocol: "responses", official: true, models: [], keyCount: 1, plan: "Plus", quota: {
    updatedAtMs: now, windows: [
      { windowMinutes: 300, usedPercent: 0, resetsAtMs: now + 18_000_000 },
      { windowMinutes: 10080, usedPercent: 100, resetsAtMs: now + 86_400_000 },
    ],
  } };
}

test("原有账号详情显示剩余比例和具体重置日期，未知 Credit 不显示为零", () => {
  const entry = account();
  const ui = harness();
  let tree = ui.render("AccountQuotaDetails", { entry });
  assert.match(text(tree), /剩余 100%/);
  assert.match(text(tree), /剩余 0%/);
  assert.match(text(tree), /Credit 状态：未知/);
  assert.equal(elements(tree).filter(e => e.type === "time").length, 2);
  entry.quota!.updatedAtMs = 0;
  tree = ui.render("AccountQuotaDetails", { entry });
  assert.doesNotMatch(text(tree), /剩余 \d+%/);
  assert.equal(elements(tree).find(e => e.props?.role === "progressbar")!.props["aria-valuenow"], undefined);
});

test("时间轴支持切换窗口、前后翻页和回到今天；未来窗口明确标记推算", () => {
  const ui = harness();
  const entry = account();
  const render = () => ui.render("ProviderQuotaTimeline", { entries: [entry] });
  let tree = render();
  const initial = text(tree);
  assert.match(initial, /后续窗口（推算）/);
  assert.match(initial, /剩余 0%/);
  const click = (label: string) => {
    const button = elements(tree).find(e => e.type === "button" && (e.props["aria-label"] === label || text(e) === label));
    assert.ok(button, label); button.props.onClick(); tree = render();
  };
  click("下一时段");
  assert.notEqual(text(tree), initial);
  click("今天");
  assert.equal(text(tree), initial);
  click("5 小时");
  assert.match(text(tree), /24 小时/);
  assert.match(text(tree), /剩余 100%/);
  assert.equal(elements(tree).find(e => e.type === "button" && text(e) === "5 小时")!.props["aria-pressed"], true);
  assert.equal(elements(tree).find(e => e.type === "tr" && e.props["data-off"] !== undefined)!.props["data-off"], false, "旧账号没写 enabled 时仍视为开启");
});

test("空池隐藏时间轴，缺少窗口时不给出虚假比例", () => {
  assert.equal(harness().render("ProviderQuotaTimeline", { entries: [] }), null);
  const entry = account(); entry.quota = null;
  const tree = harness().render("ProviderQuotaTimeline", { entries: [entry] });
  assert.match(text(tree), /未提供重置时间/);
  assert.doesNotMatch(text(tree), /剩余 \d+%/);
});

test("普通订阅刷新后立即显示额度，不需要查询或持有重置卡", () => {
  const ui = harness();
  const entry = account();
  const quota = { ...entry.quota!, windows: [entry.quota!.windows[0]] };
  const apiEntry = { ...entry, id: "api", account: null, name: "API only" };
  assert.equal(ui.render("ProviderQuotaTimeline", { entries: [apiEntry] }), null);
  const before = ui.render("ProviderQuotaTimeline", { entries: [{ ...entry, quota: null }, apiEntry] });
  assert.match(text(before), /额度待刷新/);

  const after = ui.render("ProviderQuotaTimeline", { entries: [{ ...entry, quota, resetCredits: null }, apiEntry] });
  assert.match(text(after), /24 小时/);
  assert.match(text(after), /剩余 100%/);
  assert.doesNotMatch(text(after), /API only|重置卡到期/);
  assert.ok(elements(after).some(element => element.props.title?.includes("自然重置")));
});
