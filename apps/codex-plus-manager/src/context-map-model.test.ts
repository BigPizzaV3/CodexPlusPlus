import assert from "node:assert/strict";
import test from "node:test";
import { contextCurve, contextGrid, contextMap } from "./context-map-model.ts";
import type { ContextPart } from "./request-flow-model.ts";

const parts: ContextPart[] = [
  { kind: "tools", estimatedTokens: 200, items: 3 },
  { kind: "user", estimatedTokens: 100, items: 2 },
];

test("low utilization leaves mostly free cells and does not mix categories inside a cell", () => {
  const { slices, usedShare, mapShare } = contextMap(parts, 3570, 268430);
  assert.equal(usedShare, 3570 / 272000);
  assert.equal(mapShare, .34);
  const cells = contextGrid(slices, 100);
  assert.equal(cells.length, 100);
  assert.equal(cells[0], "tools");
  assert.equal(cells.filter(cell => cell === "free").length, 99);
});

test("whole cells allocate fractional category shares without losing or duplicating cells", () => {
  const { slices } = contextMap(parts, 40, 60);
  assert.deepEqual(contextGrid(slices, 10), ["tools", "tools", "tools", "user", ...Array(6).fill("free")]);
  assert.ok(contextGrid(contextMap(parts, 999, 1).slices, 100).includes("free"));
});

test("history curve fills to the baseline and distinguishes unknown gaps from measured neighbors", () => {
  const curve = contextCurve([.2, null, .4, .8], 1, 800, 100, 0);
  assert.equal(curve.gaps, "M100,80C300,80 300,60 500,60");
  assert.equal(curve.observed, "M500,60C600,60 600,20 700,20");
  assert.ok(curve.area.endsWith("L700,100H100Z"));
  assert.equal(contextCurve([null, .2, null], 1, 800, 100, 0).area, "");
});

test("empty or full windows do not fabricate occupied or free parts", () => {
  assert.deepEqual(contextMap(parts, 0, 100), { slices: [{ kind: "free", share: 1 }], usedShare: 0, mapShare: 0 });
  const full = contextMap(parts, 150, 0);
  assert.equal(full.usedShare, 1);
  assert.equal(full.mapShare, 1);
  assert.ok(full.slices.every(slice => slice.kind !== "free"));
});

test("unknown capacity omits free space, and older logs keep a neutral used slice", () => {
  const composition = contextMap(parts, 300, null);
  assert.equal(composition.usedShare, null);
  assert.ok(composition.slices.every(slice => slice.kind !== "free"));
  assert.deepEqual(contextMap([], 200, 800).slices, [{ kind: "unclassified", share: .2 }, { kind: "free", share: .8 }]);
  assert.deepEqual(contextMap([], null, null).slices, []);
});
