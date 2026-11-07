import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import * as policies from "./subscription-policy.ts";

type Element = { type: string; props: Record<string, any> };
const jsx = (type: string, props: Record<string, any>) => ({ type, props });
const modules: Record<string, unknown> = {
  "react/jsx-runtime": { jsx, jsxs: jsx },
  "./i18n": {
    t: (value: string) => value,
    tf: (value: string, args: unknown[]) => value.replace(/\{(\d+)\}/g, (_, index) => String(args[Number(index)])),
  },
  "./gateway-ui": { QuotaWindows: "QuotaWindows" },
  "./subscription-policy": policies,
  "./AppSelect": { AppSelect: "AppSelect" },
  "./NativeAuthControls.css": {},
};
const exports: Record<string, any> = {};
vm.runInNewContext(ts.transpileModule(
  readFileSync(new URL("./SubscriptionPolicyFields.tsx", import.meta.url), "utf8"),
  { compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX, target: ts.ScriptTarget.ES2022 } },
).outputText, {
  exports,
  require: (name: string) => {
    assert.ok(name in modules, name);
    return modules[name];
  },
});

function children(value: any): Element[] {
  if (value == null || typeof value !== "object") return [];
  if (Array.isArray(value)) return value.flatMap(children);
  return [value, ...children(value.props?.children)];
}

test("真实额度字段组件可编辑省略、null和空窗口表，不白屏且没有额外授权开关", () => {
  for (const windowUsedPercentLimits of [undefined, null, {}]) {
    const updates: policies.SubscriptionPolicy[] = [];
    const value = { permitted: false, ...(windowUsedPercentLimits === undefined ? {} : { windowUsedPercentLimits }) };
    const tree = exports.SubscriptionPolicyFields({ value, quota: {
      windowsComplete: true, updatedAtMs: Date.now(),
      windows: [{ windowMinutes: 300, usedPercent: 12, resetsAtMs: null }, { windowMinutes: 10080, usedPercent: 20, resetsAtMs: null }],
    }, onChange: (policy: policies.SubscriptionPolicy) => updates.push(policy) });
    const inputs = children(tree).filter((element) => element.type === "input");
    const permission = inputs.find((element) => element.props.type === "checkbox");
    assert.equal(permission, undefined);
    const selectors = children(tree).filter((element) => element.type === "AppSelect");
    assert.equal(selectors.find((element) => element.props.ariaLabel === "用量未知或过期时")?.props.value, "allow");
    const next = selectors.find((element) => element.props.ariaLabel === "达到上限后")?.props.options.find((option: { value: string }) => option.value === "nextPermitted");
    assert.equal(next?.label, "下一个订阅账号");
    const fiveHours = inputs.find((element) => element.props["aria-label"] === "300 分钟已用上限（%）");
    assert.ok(fiveHours);
    assert.equal(fiveHours.props.value, "");
    fiveHours.props.onChange({ currentTarget: { value: "80" } });
    assert.deepEqual(updates.at(-1)?.windowUsedPercentLimits, { "300": 80 });
    assert.equal("permitted" in updates.at(-1)!, false);
    assert.equal(value.permitted, false);
  }
});

test("官方只返回7d时不造5h字段，也不删除已存但当前不适用的阈值", () => {
  const value = { windowUsedPercentLimits: { "300": 0, "10080": 80 } };
  let saved: policies.SubscriptionPolicy | undefined;
  const tree = exports.SubscriptionPolicyFields({ value, quota: {
    windowsComplete: true, updatedAtMs: Date.now(),
    windows: [{ windowMinutes: 10080, usedPercent: 20, resetsAtMs: null }],
  }, onChange: (policy: policies.SubscriptionPolicy) => { saved = policy; } });
  const inputs = children(tree).filter(element => element.type === "input");
  assert.equal(inputs.some(element => element.props["aria-label"] === "300 分钟已用上限（%）"), false);
  const weekly = inputs.find(element => element.props["aria-label"] === "10080 分钟已用上限（%）");
  assert.ok(weekly);
  weekly.props.onChange({ currentTarget: { value: "90" } });
  assert.deepEqual(saved?.windowUsedPercentLimits, { "300": 0, "10080": 90 });
});

test("编辑账号的窗口阈值时保留其他窗口和失败策略", () => {
  const value = {
    ...policies.defaultSubscriptionPolicy(), windowUsedPercentLimits: { "43200": 91 },
    onLimit: "apiFallback" as const, unknownUsage: "allow" as const, maxAgeSeconds: 120,
  };
  let saved: policies.SubscriptionPolicy | undefined;
  const tree = exports.SubscriptionPolicyFields({ value, onChange: (policy: policies.SubscriptionPolicy) => { saved = policy; } });
  const input = children(tree).find((element) => element.props?.["aria-label"] === "43200 分钟已用上限（%）");
  assert.ok(input);
  input.props.onChange({ currentTarget: { value: "85" } });
  assert.deepEqual(structuredClone(saved), { ...value, windowUsedPercentLimits: { "43200": 85 } });
});
