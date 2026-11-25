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
    "@tauri-apps/api/core": { invoke: () => assert.fail("collapsed quota details must not read history") },
    "lucide-react": { RefreshCw: "RefreshCw" },
    "./components/ui/button": { Button: "button" },
    "./i18n": { getLanguage: () => "zh", t: (s: string) => s, tf: (s: string, args: unknown[]) => s.replace(/\{(\d+)\}/g, (_, n) => String(args[Number(n)])) },
    "./gateway-model": gateway,
    "./quota-timeline": timeline,
    "./QuotaCalendar": { QuotaCalendar: "calendar" },
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

test("供应商详情按实际时长显示窗口，用量页日历由账号卡片挂载", () => {
  const entry = account();
  entry.quota!.windows = [
    { windowMinutes: 180, usedPercent: 20, resetsAtMs: null },
    { windowMinutes: 2880, usedPercent: 30, resetsAtMs: null },
  ];
  const tree = harness().render("AccountQuotaDetails", { entry });
  assert.match(text(tree), /3 小时/);
  assert.match(text(tree), /2 天/);
  assert.doesNotMatch(text(tree), /5 小时|每周/);
  const providers = readFileSync(new URL("./GatewayProviders.tsx", import.meta.url), "utf8");
  const usage = readFileSync(new URL("./GatewayUsage.tsx", import.meta.url), "utf8");
  assert.doesNotMatch(providers, /QuotaCards|QuotaCalendar|ProviderQuotaTimeline/);
  assert.match(usage, /<QuotaCards entries=\{quotas\}/);
  assert.doesNotMatch(usage, /QuotaCalendar/);
  const panel = readFileSync(new URL("./ProviderQuotaPanel.tsx", import.meta.url), "utf8");
  assert.match(panel, /<QuotaCalendar\s[^>]+entry=\{entry\}\s+minutes=\{calendarWindow.minutes\}/);
});
