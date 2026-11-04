import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

async function fixture() {
  const source = await readFile(new URL("../../../assets/inject/renderer-inject/90-action-groups.js", import.meta.url), "utf8");
  const start = source.indexOf("  function onConversationViewCompositionStart(");
  const end = source.indexOf("  function cleanupConversationView(", start);
  assert.ok(start >= 0 && end > start);
  const state = { settleFramesLeft: 0, rafId: 0, composing: false };
  const frames = new Map<number, () => void>();
  let calls = 0, id = 0;
  const api = new Function("conversationViewState", "requestAnimationFrame", "cancelAnimationFrame", "conversationViewAlignNow",
    source.slice(start, end) + "\nreturn {schedule:scheduleConversationViewAlign,start:onConversationViewCompositionStart,end:onConversationViewCompositionEnd};",
  )(state, (fn: () => void) => { frames.set(++id, fn); return id; }, (id: number) => frames.delete(id), () => calls++);
  const frame = () => { const pending = [...frames.values()]; frames.clear(); pending.forEach(fn => fn()); };
  return { state, frames, ...api, frame, calls: () => calls };
}

it("coalesces repeated requests into a bounded two-frame alignment pass", async () => {
  const f = await fixture();
  f.schedule(); f.frame();
  for (let i = 0; i < 100; i++) f.schedule();
  f.frame();
  assert.equal(f.calls(), 2);
  assert.equal(f.frames.size, 0, "updates must not continually extend the alignment pass");
});

it("does not measure layout while IME composition is active", async () => {
  const f = await fixture();
  f.state.composing = true;
  f.schedule(); f.frame();
  assert.equal(f.calls(), 0);
  f.state.composing = false;
  f.schedule(); f.frame(); f.frame();
  assert.equal(f.calls(), 2);
});

it("does not run an already queued alignment after composition begins", async () => {
  const f = await fixture();
  f.schedule();
  f.state.composing = true;
  f.frame();
  assert.equal(f.calls(), 0);
  assert.equal(f.frames.size, 0);
});

it("preserves an explicitly requested settling budget", async () => {
  const f = await fixture();
  f.schedule(4);
  for (let i=0;i<4;i++) f.frame();
  assert.equal(f.calls(), 4);
  assert.equal(f.frames.size, 0);
});

it("cancels pending work on composition start and realigns once it ends", async () => {
  const f = await fixture();
  f.schedule();
  f.start({target:{closest: () => null}});
  assert.equal(f.state.composing, false);
  f.start({target:{closest: () => ({})}});
  assert.equal(f.state.composing, true);
  assert.equal(f.frames.size, 0);
  f.end(); f.frame(); f.frame();
  assert.equal(f.state.composing, false);
  assert.equal(f.calls(), 2);
  f.end();
  assert.equal(f.frames.size, 0);
});
