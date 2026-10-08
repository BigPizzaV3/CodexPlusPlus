import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { createContext, runInContext } from "node:vm";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/96-layout-physics.js", import.meta.url), "utf8");
type Rect = { left: number; top: number; width: number; height: number };
type Panel = Rect & { minWidth?: number; minHeight?: number; canResizeHeight?: boolean };
type Panels = Record<string, Panel>;
type SolveResult = { rects: Record<string, Rect>; changedIds: string[]; blocked: boolean };
type SpringState = { position: number; velocity: number; settled: boolean };
type SpringOptions = { stiffness?: number; damping?: number; mass?: number; min?: number; max?: number; reducedMotion?: boolean };
const window = {} as {
  physics: {
    solve(panels: Panels, activeId: string, target: Rect, bounds: Rect, origin?: Rect | null): SolveResult;
    spring(position: number, velocity: number, target: number, dtSeconds: number, options?: SpringOptions): SpringState;
  };
};
runInContext(`(function() { ${source}\nwindow.physics = { solve: codexPlusLayoutSolve, spring: codexPlusLayoutSpringStep }; })();`, createContext({ window }));
const { solve, spring } = window.physics;
const plain = <T>(value: T): T => JSON.parse(JSON.stringify(value));

function assertPacking(rects: Record<string, Rect>, bounds: Rect) {
  const entries = Object.entries(rects);
  for (const [id, rect] of entries) {
    assert.ok(Object.values(rect).every(Number.isFinite), `${id} must have finite geometry`);
    assert.ok(rect.width > 0 && rect.height > 0, `${id} must remain visible`);
    assert.ok(rect.left >= bounds.left - .001 && rect.top >= bounds.top - .001, `${id} must stay inside the top/left boundaries`);
    assert.ok(rect.left + rect.width <= bounds.left + bounds.width + .001, `${id} must stay inside the right boundary`);
    assert.ok(rect.top + rect.height <= bounds.top + bounds.height + .001, `${id} must stay inside the bottom boundary`);
  }
  for (let first = 0; first < entries.length; first++) {
    for (let second = first + 1; second < entries.length; second++) {
      const [aId, a] = entries[first]; const [bId, b] = entries[second];
      const overlapWidth = Math.min(a.left + a.width, b.left + b.width) - Math.max(a.left, b.left);
      const overlapHeight = Math.min(a.top + a.height, b.top + b.height) - Math.max(a.top, b.top);
      assert.ok(overlapWidth < .001 || overlapHeight < .001, `${aId} overlaps ${bId}: ${JSON.stringify({ a, b })}`);
    }
  }
}

test("dragged panel keeps its requested position and pushes a horizontal chain without overlaps", () => {
  const bounds = { left: 0, top: 0, width: 1000, height: 160 };
  const panels: Panels = {
    active: { left: 0, top: 0, width: 200, height: 160 },
    second: { left: 210, top: 0, width: 200, height: 160 },
    third: { left: 420, top: 0, width: 200, height: 160 },
    fourth: { left: 630, top: 0, width: 200, height: 160 },
  };
  const original = plain(panels);
  const target = { ...panels.active, left: 120 };
  const result = solve(panels, "active", target, bounds);
  assert.equal(result.blocked, false);
  assert.equal(result.rects.active.left, target.left);
  for (const id of ["second", "third", "fourth"]) {
    assert.ok(result.rects[id].left > panels[id].left, `${id} should yield to the moving chain`);
    assert.ok(result.changedIds.includes(id));
    assert.equal(result.rects[id].width, panels[id].width);
  }
  assertPacking(result.rects, bounds);
  assert.deepEqual(panels, original, "solving a pointer move must not mutate its fixed baseline");
});

test("an unchanged baseline makes repeated pointer targets stable and returning to the origin drift-free", () => {
  const bounds = { left: 8, top: 52, width: 984, height: 160 };
  const panels: Panels = {
    active: { left: 8, top: 52, width: 200, height: 160 },
    second: { left: 218, top: 52, width: 200, height: 160 },
    third: { left: 428, top: 52, width: 200, height: 160 },
  };
  const target = { ...panels.active, left: 128 };
  const first = solve(panels, "active", target, bounds, panels.active);
  for (const left of [88, 148, 48, 128, 128]) {
    const result = solve(panels, "active", { ...target, left }, bounds, panels.active);
    assertPacking(result.rects, bounds);
  }
  assert.deepEqual(plain(solve(panels, "active", target, bounds, panels.active)), plain(first));
  const returned = solve(panels, "active", panels.active, bounds, panels.active);
  assert.deepEqual(plain(returned.rects), panels);
  assert.equal(returned.blocked, false);
});

test("an impossible corner movement stops at a stable feasible position while preserving usable minimum sizes", () => {
  const bounds = { left: 0, top: 0, width: 616, height: 100 };
  const panels: Panels = {
    active: { left: 0, top: 0, width: 300, height: 100, minWidth: 280 },
    obstacle: { left: 308, top: 0, width: 300, height: 100, minWidth: 280 },
  };
  const target = { ...panels.active, left: 150 };
  const result = solve(panels, "active", target, bounds);
  assert.equal(result.blocked, true);
  assert.ok(result.rects.active.left >= 0 && result.rects.active.left < 150);
  assert.equal(result.rects.active.width, 300);
  assert.ok(result.rects.obstacle.width >= 280);
  assertPacking(result.rects, bounds);
  for (let attempt = 0; attempt < 50; attempt++) assert.deepEqual(plain(solve(panels, "active", target, bounds)), plain(result));
});

test("resize respects usable minimum sizes and keeps feasible layouts inside the available screen", () => {
  const bounds = { left: 8, top: 52, width: 700, height: 400 };
  const panels: Panels = {
    active: { left: 20, top: 70, width: 300, height: 160, minWidth: 280, minHeight: 80, canResizeHeight: true },
    other: { left: 470, top: 70, width: 220, height: 160, minWidth: 220 },
  };
  const small = solve(panels, "active", { left: 20, top: 70, width: 10, height: 5 }, bounds);
  assert.ok(small.rects.active.width >= 280 && small.rects.active.height >= 80);
  assertPacking(small.rects, bounds);
  const moved = solve(panels, "active", { ...panels.active, left: -900, top: -500 }, bounds);
  assertPacking(moved.rects, bounds);
  assert.equal(moved.rects.active.left, bounds.left); assert.equal(moved.rects.active.top, bounds.top);
});

test("moving the full-height sidebar uses the neighboring rail edge to narrow the composer without moving the rail", () => {
  const bounds = { left: 8, top: 48, width: 1264, height: 744 };
  const panels: Panels = {
    rail: { left: 8, top: 48, width: 52, height: 744, minWidth: 44, minHeight: 80, canResizeHeight: true },
    sidebar: { left: 60, top: 48, width: 239, height: 744, minWidth: 180, minHeight: 80, canResizeHeight: true },
    summary: { left: 1012, top: 267, width: 250, height: 235, minWidth: 220, minHeight: 235, canResizeHeight: false },
    composer: { left: 580.406, top: 168.5, width: 691.594, height: 98.5, minWidth: 280, minHeight: 98.5, canResizeHeight: false },
  };
  const result = solve(panels, "sidebar", { ...panels.sidebar, left: 720 }, bounds);
  assert.equal(result.blocked, false);
  assert.equal(result.rects.sidebar.left, 720);
  assert.deepEqual(plain(result.rects.rail), { left: 8, top: 48, width: 52, height: 744 }, "the full-height rail should keep its original position and dimensions");
  assert.ok(result.rects.composer.width >= 280 && result.rects.composer.width <= 660);
  assert.ok(result.rects.composer.left >= 60 && result.rects.composer.left + result.rects.composer.width <= 720);
  assert.equal(result.rects.composer.height, 98.5, "narrowing the input box must not flatten its natural height");
  assert.equal(result.changedIds.includes("rail"), false);
  assertPacking(result.rects, bounds);
});

test("elastic motion overshoots gently, converges exactly, and stops scheduling once settled", () => {
  let state: SpringState = { position: 0, velocity: 0, settled: false };
  const positions: number[] = [];
  for (let frame = 0; frame < 240 && !state.settled; frame++) {
    state = spring(state.position, state.velocity, 100, 1 / 60);
    assert.ok(Number.isFinite(state.position) && Number.isFinite(state.velocity));
    positions.push(state.position);
  }
  assert.ok(positions.some((position) => position > 100), "the requested elastic feedback should have a damped overshoot");
  assert.equal(state.settled, true);
  assert.equal(state.position, 100); assert.equal(state.velocity, 0);
  assert.deepEqual(plain(spring(state.position, state.velocity, 100, 1 / 60)), plain(state));
});

test("spring motion uses elapsed seconds rather than frame count", () => {
  const elapsed = (steps: number, dt: number) => {
    let state = { position: 0, velocity: 0, settled: false };
    for (let step = 0; step < steps; step++) state = spring(state.position, state.velocity, 100, dt);
    return state;
  };
  const normal = elapsed(12, .01); const fastDisplay = elapsed(24, .005);
  assert.ok(Math.abs(normal.position - fastDisplay.position) < .000001);
  assert.ok(Math.abs(normal.velocity - fastDisplay.velocity) < .000001);
  assert.ok(elapsed(12, .02).position > normal.position, "twice the elapsed time advances farther even at the same frame count");
});

test("reduced motion is immediate and bounds remain finite during large or invalid time intervals", () => {
  const instant = spring(0, 900, 100, 1 / 60, { reducedMotion: true });
  assert.deepEqual(plain(instant), { position: 100, velocity: 0, settled: true });
  for (const dt of [0, NaN, Infinity, 10]) {
    const result = spring(0, 900, 100, dt, { min: 0, max: 100 });
    assert.ok(Number.isFinite(result.position) && Number.isFinite(result.velocity));
    assert.ok(result.position >= 0 && result.position <= 100);
  }
  const outside = spring(0, 0, 800, 1 / 60, { min: 0, max: 100, reducedMotion: true });
  assert.equal(outside.position, 100); assert.equal(outside.velocity, 0);
});
