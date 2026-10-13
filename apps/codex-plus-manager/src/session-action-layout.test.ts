import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const source = await readFile(new URL("../../../assets/inject/renderer-inject.js", import.meta.url), "utf8");
const extract = (name: string) => {
  const match = source.match(new RegExp(`^  function ${name}\\([^]*?^  \\}`, "m"));
  assert.ok(match, name);
  return match[0];
};

it("keeps the single-row entry point and batches global action layout", () => {
  assert.match(extract("syncActionGroupLayout"), /syncActionGroupLayouts\(\[\{ row, group \}\]\)/);
  const all = extract("syncActionGroupsLayout");
  assert.match(all, /force = false/);
  assert.match(all, /syncActionGroupLayouts\(sessionRows\(\)\.map/);
  assert.doesNotMatch(all, /forEach|syncActionGroupLayout\(/);
});

it("snapshots native colors and separates each action layout measurement phase", () => {
  const layout = extract("syncActionGroupLayouts");
  assert.match(layout, /color: nativeStyle\?\.color, background: nativeStyle\?\.backgroundColor/);
  assert.match(layout, /force \|\| group\.dataset\.codexActionLayoutStable !== "true"/);
  assert.match(layout, /layout\.group\.getBoundingClientRect\(\)\.right/);
  assert.match(layout, /layout\.group\.getBoundingClientRect\(\)\.left/);
  assert.match(layout, /group\.dataset\.codexActionLayoutStable = "true"/);
  assert.doesNotMatch(layout, /setTimeout|setInterval|requestAnimationFrame/);
});

it("forces one batched layout from the existing resize frame", async () => {
  const startup = await readFile(new URL("../../../assets/inject/renderer-inject/99-startup.js", import.meta.url), "utf8");
  assert.match(startup, /requestAnimationFrame\(/);
  assert.match(startup, /syncActionGroupsLayout\(true\)/);
  assert.doesNotMatch(startup, /sessionRows\(\)\.forEach|delete group\.dataset\.codexActionLayoutStable/);
});
