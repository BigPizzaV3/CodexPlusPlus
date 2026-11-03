import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import type { GatewayPoolEntry } from "./gateway-model.ts";
import * as probeModel from "./provider-test.ts";

const source = ts.transpileModule(readFileSync(new URL("./ProviderTestDialog.tsx", import.meta.url), "utf8"), {
  compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 },
}).outputText;

type Element = { type: string; props: Record<string, any> };

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (cause: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

function response(ok = true) {
  return {
    status: "ok", message: "fixture", ok, category: ok ? "" : "auth",
    httpStatus: ok ? 200 : 401, model: "custom-model", wireApi: "responses",
    firstTokenMs: 100, totalMs: 200, reply: ok ? "ok" : "", replyTruncated: false,
    error: ok ? "" : "fixture authentication error", keyId: "", keyName: "", keyTail: "",
  };
}

function harness(options: { initialModel?: string; lockModel?: boolean; models?: string[] } = {}) {
  const entry: GatewayPoolEntry = {
    id: "provider", name: "Fixture provider", baseUrl: "https://fixture.invalid/v1",
    protocol: "responses", official: false, models: options.models ?? ["listed-model"], keyCount: 0,
  };
  const hooks: any[] = [];
  const cleanups = new Map<number, () => void>();
  const requests: Array<{ args: Record<string, unknown>; result: ReturnType<typeof deferred<unknown>> }> = [];
  const results: probeModel.ProviderProbeResult[] = [];
  let cursor = 0;
  let starts = 0;
  let cancellations = 0;
  let closes = 0;
  let verified = false;
  let escape = () => {};
  const exports: Record<string, any> = {};
  const jsx = (type: string, props: Record<string, any>) => ({ type, props });
  const modules: Record<string, unknown> = {
    react: {
      useState: (initialValue: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = initialValue;
        return [hooks[index], (value: unknown) => {
          hooks[index] = typeof value === "function" ? (value as (current: unknown) => unknown)(hooks[index]) : value;
        }];
      },
      useRef: (initialValue: unknown) => {
        const index = cursor++;
        if (!(index in hooks)) hooks[index] = { current: initialValue };
        return hooks[index];
      },
      useEffect: (effect: () => void | (() => void), deps: unknown[]) => {
        const index = cursor++;
        if (hooks[index]?.length === deps.length && hooks[index].every((value: unknown, i: number) => Object.is(value, deps[i]))) return;
        cleanups.get(index)?.();
        cleanups.delete(index);
        hooks[index] = deps.slice();
        const cleanup = effect();
        if (typeof cleanup === "function") cleanups.set(index, cleanup);
      },
    },
    "react/jsx-runtime": { jsx, jsxs: jsx, Fragment: "Fragment" },
    "react-dom": { createPortal: (child: unknown) => child },
    "@tauri-apps/api/core": { invoke: (command: string, args: Record<string, unknown>) => {
      if (command === "cancel_gateway_provider_test") {
        cancellations += 1;
        return Promise.resolve({ status: "cancelled" });
      }
      assert.equal(command, "test_gateway_provider");
      assert.equal(verified, false, "发送新请求前必须清除父组件的旧验证结果");
      const result = deferred<unknown>();
      requests.push({ args, result });
      return result.promise;
    } },
    "lucide-react": Object.fromEntries(["CheckCircle2", "Loader2", "X", "XCircle", "Zap"].map((name) => [name, name])),
    "@/components/ui/button": { Button: "Button" },
    "./i18n": { t: (value: string) => value, tf: (value: string, args: string[]) => value.replace(/\{(\d+)\}/g, (_, index) => args[Number(index)]) },
    "./AppSelect": { AppSelect: "AppSelect" },
    "./gateway-model": { maskAccountName: (value: string) => value },
    "./gateway-ui": { Avatar: "Avatar" },
    "./use-escape-layer": { useEscapeLayer: (_active: boolean, close: () => void) => { escape = close; } },
    "./provider-test": probeModel,
    "./ProviderFormDialog.css": {},
  };
  vm.runInNewContext(source, {
    exports, document: { body: {} },
    require: (name: string) => {
      assert.ok(name in modules, `unexpected import ${name}`);
      return modules[name];
    },
  });
  return {
    requests, results,
    render: () => {
      cursor = 0;
      return exports.ProviderTestDialog({
        entry, keyId: null, initialModel: options.initialModel, lockModel: options.lockModel,
        onStart: () => { starts += 1; verified = false; },
        onResult: (result: probeModel.ProviderProbeResult) => { results.push(result); verified = result.ok === true; },
        onClose: () => { closes += 1; },
      }) as Element;
    },
    starts: () => starts,
    verified: () => verified,
    cancellations: () => cancellations,
    closes: () => closes,
    escape: () => escape(),
    unmount: () => { cleanups.forEach((cleanup) => cleanup()); cleanups.clear(); },
  };
}

function children(element: any): Element[] {
  if (element == null || typeof element !== "object") return [];
  if (Array.isArray(element)) return element.flatMap(children);
  return [element, ...children(element.props?.children)];
}

function text(element: any): string {
  if (element == null || typeof element === "boolean") return "";
  if (Array.isArray(element)) return element.map(text).join("");
  return typeof element === "object" ? text(element.props?.children) : String(element);
}

function find(tree: Element, predicate: (element: Element) => boolean): Element {
  const found = children(tree).find(predicate);
  assert.ok(found);
  return found;
}

function start(tree: Element) {
  find(tree, (element) => element.type === "form").props.onSubmit({ preventDefault() {} });
}

function cancel(tree: Element) {
  find(tree, (element) => element.type === "Button" && text(element) === "取消测试").props.onClick();
}

test("限定供应商的自定义模型即使不在已列模型中，显示与实际请求仍保持一致", async () => {
  for (const models of [["listed-model"], []]) {
    const f = harness({ initialModel: "custom-model", lockModel: true, models });
    const tree = f.render();
    const select = find(tree, (element) => element.type === "AppSelect" && element.props.ariaLabel === "测试用的模型");
    assert.equal(select.props.value, "custom-model");
    assert.equal(select.props.disabled, true, "锁定模型时不能通过下拉改成别的模型");
    assert.ok(select.props.options.some((item: { value: string; label: string }) => item.value === "custom-model" && item.label === "custom-model"));
    assert.doesNotMatch(text(tree), /还没有模型/);
    start(tree);
    assert.equal(f.starts(), 1);
    assert.equal(f.requests[0].args.model, "custom-model");
    assert.equal(f.requests[0].args.id, "provider");
    f.requests[0].result.resolve(response());
    await flush();
    assert.equal(f.verified(), true);
    f.unmount();
  }
});

test("普通供应商测试仍可选择模型，进行中的重复提交不会重复触发回调或后台请求", async () => {
  const f = harness({ models: ["first", "second"] });
  let tree = f.render();
  const select = find(tree, (element) => element.type === "AppSelect");
  assert.equal(select.props.disabled, false);
  select.props.onChange("second");
  tree = f.render();
  start(tree);
  start(tree);
  assert.equal(f.requests.length, 1);
  assert.equal(f.starts(), 1);
  assert.equal(f.requests[0].args.model, "second");
  assert.equal(find(f.render(), (element) => element.type === "AppSelect").props.disabled, true);
  f.requests[0].result.resolve(response());
  await flush();
  f.unmount();
});

test("成功后每次重测同步清除父级验证，失败状态、异常和后台取消均不会复用旧成功", async () => {
  for (const outcome of ["failed", "throw", "cancelled", "probe-failed"] as const) {
    const f = harness({ initialModel: "custom-model" });
    start(f.render());
    f.requests[0].result.resolve(response());
    await flush();
    assert.equal(f.verified(), true);
    start(f.render());
    assert.equal(f.starts(), 2);
    assert.equal(f.verified(), false, "重测的响应到达前即失效");
    if (outcome === "throw") f.requests[1].result.reject(new Error("fixture network error"));
    else if (outcome === "probe-failed") f.requests[1].result.resolve(response(false));
    else f.requests[1].result.resolve({ status: outcome, message: "fixture command failed" });
    await flush();
    assert.equal(f.verified(), false, outcome);
    assert.equal(f.results.length, outcome === "probe-failed" ? 2 : 1);
    f.unmount();
  }
});

test("手动取消重测后，旧成功响应迟到也不会恢复父级验证", async () => {
  const f = harness({ initialModel: "custom-model" });
  start(f.render());
  f.requests[0].result.resolve(response());
  await flush();
  start(f.render());
  cancel(f.render());
  assert.equal(f.cancellations(), 1);
  assert.equal(f.verified(), false);
  f.requests[1].result.resolve(response());
  await flush();
  assert.equal(f.results.length, 1);
  assert.equal(f.verified(), false);
  f.unmount();
});

test("关闭或卸载时取消进行中的请求，迟到响应不再触发结果回调", async () => {
  for (const action of ["close", "unmount"] as const) {
    const f = harness({ initialModel: "custom-model" });
    start(f.render());
    if (action === "close") {
      f.escape();
      assert.equal(f.closes(), 1);
    } else f.unmount();
    assert.equal(f.cancellations(), 1);
    f.requests[0].result.resolve(response());
    await flush();
    assert.equal(f.results.length, 0);
    assert.equal(f.verified(), false);
    if (action === "close") f.unmount();
  }
});
