import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import test from "node:test";
import { build } from "esbuild";
import { createElement, type ComponentType } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import type { GatewayPayload } from "./gateway-model.ts";

const require = createRequire(import.meta.url);
const bundled = await build({
  entryPoints: [fileURLToPath(new URL("./RequestFlow.tsx", import.meta.url))],
  bundle: true,
  write: false,
  platform: "node",
  format: "cjs",
  jsx: "automatic",
  external: ["react", "react/jsx-runtime"],
  loader: { ".css": "empty" },
  define: { "import.meta.url": JSON.stringify(import.meta.url) },
});
const component = { exports: {} as { FlowRequestRow: ComponentType<Record<string, unknown>> } };
new Function("require", "module", "exports", bundled.outputFiles[0].text)(require, component, component.exports);

test("实际请求行在多组再次改名后更新正文与悬停提示，并保留传入的路由标识", () => {
  const payload = {
    groups: [
      { id: "first-id", name: "Before A" },
      { id: "second-id", name: "Before B" },
    ],
  } as GatewayPayload;
  const entries = payload.groups.map((group, index) => ({
    id: String(index), model: `group/${group.id}`, upstreamModel: "upstream-model",
    startedAtMs: 1, timestampMs: 2, durationMs: 100, success: true,
  }));
  const originals = JSON.stringify(entries);
  for (const names of [["After A", "After B"], ["Before B", "After A"]]) {
    payload.groups.forEach((group, index) => { group.name = names[index]; });
    for (const [index, entry] of entries.entries()) {
      const html = renderToStaticMarkup(createElement(component.exports.FlowRequestRow, { entry, payload, now: 3 }));
      assert.ok(html.includes(names[index]));
      assert.ok(html.includes(`title="${names[index]} → upstream-model"`));
      assert.ok(!html.includes(entry.model));
    }
    assert.equal(JSON.stringify(entries), originals);
  }
  const ordinary = renderToStaticMarkup(createElement(component.exports.FlowRequestRow, {
    entry: { ...entries[0], model: "provider/raw-model" }, payload, now: 3,
  }));
  assert.ok(ordinary.includes("provider/raw-model"));
});
