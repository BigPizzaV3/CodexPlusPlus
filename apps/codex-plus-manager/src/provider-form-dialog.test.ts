import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as gateway from "./gateway-model.ts";
import * as forms from "./provider-form-model.ts";
import * as sync from "./provider-sync-model.ts";
import * as policies from "./subscription-policy.ts";

type Element = { type: string; props: Record<string, any> };

function children(value: any): Element[] {
  if (value == null || typeof value !== "object") return [];
  if (Array.isArray(value)) return value.flatMap(children);
  return [value, ...children(value.props?.children)];
}

function text(value: any): string {
  if (value == null || typeof value === "boolean") return "";
  if (Array.isArray(value)) return value.map(text).join("");
  return typeof value === "object" ? text(value.props?.children) : String(value);
}

function find(tree: Element, predicate: (element: Element) => boolean): Element {
  const found = children(tree).find(predicate);
  assert.ok(found);
  return found;
}

function harness(official = true, configured = true) {
  const secret = "synthetic-refresh-token-not-for-the-form";
  const entry = Object.assign({
    id: "official-account", name: "Account A", baseUrl: official ? "" : "https://api.example.com/v1", protocol: "responses" as const,
    official, account: official ? "chatgpt" : "", models: ["model-a"], keyCount: 0,
    proxy: "", subscriptionPolicy: configured ? { windowUsedPercentLimits: { "43200": 91 } } : undefined,
    quota: { windowsComplete: true, updatedAtMs: Date.now(),
      windows: [{ windowMinutes: 10080, usedPercent: 20, resetsAtMs: null }] },
  }, { authContents: secret, officialAccountRef: "canonical-account" });
  const original = structuredClone(entry);
  const saved: gateway.ProviderDraft[] = [];
  const hooks: any[] = [];
  let cursor = 0;
  const jsx = (type: any, props: Record<string, any>) => typeof type === "function" ? type(props) : { type, props };
  const modules: Record<string, unknown> = {
    react: {
      useState: (initial: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = typeof initial === "function" ? initial() : initial;
        return [hooks[index], (value: unknown) => {
          hooks[index] = typeof value === "function" ? value(hooks[index]) : value;
        }];
      },
      useRef: (initial: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = { current: initial };
        return hooks[index];
      },
      useEffect: () => { cursor += 1; },
      useMemo: (compute: () => unknown) => compute(),
    },
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-dom": { createPortal: (child: unknown) => child },
    "@tauri-apps/api/core": { invoke: () => { assert.fail("editing must not request credentials or perform network calls"); } },
    "lucide-react": Object.fromEntries(["Pencil", "Plus", "RefreshCw", "Search", "Star", "X"].map(name => [name, name])),
    "@/components/ui/button": { Button: "Button" },
    "@/components/ui/input": { Input: "Input" },
    "./i18n": {
      t: (value: string) => value,
      tf: (value: string, args: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
    },
    "./gateway-model": gateway,
    "./gateway-ui": { Avatar: "Avatar", QuotaWindows: "QuotaWindows" },
    "./use-escape-layer": { useEscapeLayer: () => {} },
    "./ProviderExtrasDialog": { lookupPublicPrices: () => { assert.fail("editing must not fetch prices"); } },
    "./provider-form-model": forms,
    "./ProviderWindowsDialog": { windowSummary: () => "" },
    "./provider-sync-model": sync,
    "./subscription-policy": policies,
    "./AppSelect": { AppSelect: "AppSelect" },
    "./ProviderFormDialog.css": {},
    "./NativeAuthControls.css": {},
  };
  const load = (filename: string) => {
    const exports: Record<string, any> = {};
    vm.runInNewContext(ts.transpileModule(readFileSync(new URL(filename, import.meta.url), "utf8"), {
      compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
    }).outputText, {
      exports, document: { body: {} },
      require: (name: string) => {
        assert.ok(name in modules, `unexpected import ${name}`);
        return modules[name];
      },
    });
    return exports;
  };
  modules["./SubscriptionPolicyFields"] = load("./SubscriptionPolicyFields.tsx");
  const component = load("./ProviderFormDialog.tsx");
  return {
    entry, original, saved, secret,
    render: () => {
      cursor = 0;
      return component.ProviderFormDialog({
        form: { mode: "edit", entry }, busy: null,
        onSubmit: (draft: gateway.ProviderDraft) => saved.push(draft), onCancel: () => {},
      }) as Element;
    },
  };
}

function field(tree: Element, label: string): Element {
  const container = find(tree, element => element.type === "label" && text(element).startsWith(label));
  return find(container, element => element.type === "Input");
}

function submit(tree: Element) {
  find(tree, element => element.type === "form").props.onSubmit({ preventDefault() {} });
}

test("官方账号真实表单保留名称模型代理和额度编辑，不暴露API连接或登录凭据", () => {
  const f = harness();
  let tree = f.render();
  assert.equal(find(tree, element => element.type === "Button" && element.props.type === "submit").props.disabled, false, "官方空URL不妨碍保存");
  assert.doesNotMatch(text(tree), /API Key|授权|synthetic-refresh-token/);
  assert.equal(children(tree).some(element => element.props?.role === "radiogroup"), false);
  assert.equal(children(tree).some(element => element.props?.type === "password" || element.props?.placeholder === "https://api.example.com/v1"), false);
  const quotaSection = find(tree, element => element.type === "section" && text(element).startsWith("ChatGPT 订阅额度"));
  assert.equal(children(quotaSection).some(element => element.props?.type === "checkbox"), false);
  assert.equal(children(tree).some(element => element.props?.value === f.secret), false);

  field(tree, "名称").props.onChange({ currentTarget: { value: "Renamed account" } });
  tree = f.render();
  field(tree, "HTTP 代理").props.onChange({ currentTarget: { value: "socks5://127.0.0.1:1080" } });
  tree = f.render();
  find(tree, element => element.props?.["aria-label"] === "添加模型").props.onChange({ currentTarget: { value: "model-b" } });
  tree = f.render();
  find(tree, element => element.props?.["aria-label"] === "添加模型").props.onKeyDown({ key: "Enter", preventDefault() {} });
  tree = f.render();
  find(tree, element => element.props?.["aria-label"] === "设 model-b 为默认模型").props.onClick();
  tree = f.render();
  find(tree, element => element.props?.["aria-label"] === "10080 分钟已用上限（%）").props.onChange({ currentTarget: { value: "80" } });
  tree = f.render();
  find(tree, element => element.type === "AppSelect" && element.props.ariaLabel === "达到上限后").props.onChange("nextPermitted");
  tree = f.render();
  find(tree, element => element.type === "AppSelect" && element.props.ariaLabel === "用量未知或过期时").props.onChange("block");
  submit(f.render());

  assert.equal(f.saved.length, 1);
  const input = gateway.providerInput(f.saved[0]);
  assert.equal(input.name, "Renamed account");
  assert.equal(input.model, "model-b");
  assert.equal(input.modelList, "model-a\nmodel-b");
  assert.equal(input.proxy, "socks5://127.0.0.1:1080");
  assert.deepEqual(input.keys, []);
  assert.deepEqual(structuredClone(input.subscriptionPolicy), {
    ...policies.defaultSubscriptionPolicy(), windowUsedPercentLimits: { "43200": 91, "10080": 80 },
    onLimit: "nextPermitted", unknownUsage: "block",
  });
  assert.equal("permitted" in input.subscriptionPolicy!, false);
  assert.equal("authContents" in input, false);
  assert.equal("officialAccountRef" in input, false);
  assert.doesNotMatch(JSON.stringify(input), /synthetic-refresh-token/);
  assert.deepEqual(f.entry, f.original, "编辑草稿不改变原账号快照或凭据引用");
});

test("官方账号真实表单拒绝空名称和无协议代理，程序化提交同样不会保存", () => {
  const f = harness();
  let tree = f.render();
  field(tree, "名称").props.onChange({ currentTarget: { value: " " } });
  tree = f.render();
  assert.equal(find(tree, element => element.type === "Button" && element.props.type === "submit").props.disabled, true);
  submit(tree);
  assert.equal(f.saved.length, 0);
  field(tree, "名称").props.onChange({ currentTarget: { value: "Account A" } });
  tree = f.render();
  field(tree, "HTTP 代理").props.onChange({ currentTarget: { value: "127.0.0.1:7890" } });
  tree = f.render();
  assert.equal(find(tree, element => element.type === "Button" && element.props.type === "submit").props.disabled, true);
  submit(tree);
  assert.equal(f.saved.length, 0);
  field(tree, "HTTP 代理").props.onChange({ currentTarget: { value: "direct" } });
  tree = f.render();
  assert.equal(find(tree, element => element.type === "Button" && element.props.type === "submit").props.disabled, false);
  submit(tree);
  assert.equal(f.saved.length, 1);
});

test("旧官方账号仅改名不会自动创建订阅拦截策略", () => {
  const f = harness(true, false);
  let tree = f.render();
  field(tree, "名称").props.onChange({ currentTarget: { value: "Renamed legacy account" } });
  tree = f.render();
  submit(tree);
  assert.equal(f.saved.length, 1);
  assert.equal("subscriptionPolicy" in gateway.providerInput(f.saved[0]), false);
});

test("普通API供应商真实表单保持连接和Key编辑，不加入代理或订阅额度字段", () => {
  const f = harness(false);
  const tree = f.render();
  assert.match(text(tree), /API Key/);
  assert.doesNotMatch(text(tree), /HTTP 代理|ChatGPT 订阅额度/);
  assert.equal(children(tree).some(element => element.props?.role === "radiogroup"), true);
  assert.equal(children(tree).some(element => element.props?.type === "password"), true);
  assert.equal(children(tree).some(element => element.props?.placeholder === "https://api.example.com/v1"), true);
  submit(tree);
  assert.equal(f.saved.length, 1);
});
