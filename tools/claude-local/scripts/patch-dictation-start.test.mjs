import assert from "node:assert/strict";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import vm from "node:vm";
import {
  applyStartupPatches,
  extractElectronStartupAsset,
  isPatchedStartupAsset,
  patchFacadeBundle,
  patchRecorderBundle,
  restoreStartupPatches,
  startupAssets,
} from "./patch-dictation-start.mjs";
import { facadeFixture, recorderFixture } from "./fixtures/dictation-start.mjs";

async function settle() {
  for (let i = 0; i < 12; i += 1) await Promise.resolve();
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

function clockContext(extra = {}) {
  let now = 0;
  let next = 1;
  const timers = new Map();
  const logs = [];
  const context = vm.createContext({
    performance: { now: () => now },
    setTimeout: (fn, delay) => {
      const id = next++;
      timers.set(id, { fn, at: now + delay });
      return id;
    },
    clearTimeout: (id) => timers.delete(id),
    console: { info: (...args) => logs.push(args) },
    ...extra,
  });
  return {
    context, logs, timers,
    async advance(ms) {
      const until = now + ms;
      for (;;) {
        const due = [...timers.entries()].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
        if (!due) break;
        const [id, timer] = due;
        now = timer.at;
        timers.delete(id);
        timer.fn();
        await settle();
      }
      now = until;
      await settle();
    },
  };
}

function stateStore() {
  const listeners = new Set();
  const state = {
    status: "idle", startIsWaiting: false, drainPending: false, finishingOwnerId: null,
    connectedAt: null, ownerId: null, isMouseHolding: false, isPushToTalk: false,
    transcript: "private-transcript", error: null,
    setOwnerId(ownerId) { update({ ownerId }); },
    setLastStopReason() {},
    setIsPushToTalk(isPushToTalk) { update({ isPushToTalk }); },
    setStatus(status) { update({ status, startIsWaiting: false }); },
    setError(error) { update({ error, status: "error", startIsWaiting: false }); },
  };
  function update(values) {
    Object.assign(state, values);
    for (const listener of listeners) listener(state);
  }
  return { getState: () => state, subscribe: (listener) => { listeners.add(listener); return () => listeners.delete(listener); }, update };
}

function facadeRuntime({ load, start, waitForMic } = {}) {
  const store = stateStore();
  let pending = null;
  let cancels = 0;
  let generation = 0;
  const session = {
    micStream: null,
    async start(...args) {
      if (start) return start(store, ...args);
      const current = ++generation;
      store.getState().setStatus("connecting");
      const stream = await (waitForMic?.promise ?? Promise.resolve({ stop() {} }));
      // 实际 machinery 的 await acquireMicStream 后 generation 不匹配分支。
      if (current !== generation) { stream.stop(); return; }
      this.micStream = stream;
      store.update({ connectedAt: 1 });
      store.getState().setStatus("recording");
    },
    cancel() {
      cancels += 1;
      generation += 1;
      this.micStream?.stop();
      this.micStream = null;
      store.getState().setStatus("idle");
    },
    stop() {
      generation += 1;
      const wasRecording = this.micStream !== null;
      this.micStream?.stop();
      this.micStream = null;
      store.update({ drainPending: wasRecording });
      store.getState().setStatus("idle");
    },
  };
  const runtime = clockContext({
    fD: store, session, XS: (owner) => { pending = owner; },
    WM() {}, Tl: load ?? (() => Promise.resolve({ dictationSession: session })), report() {},
  });
  vm.runInContext(`${patchFacadeBundle(facadeFixture)}\nglobalThis.facade=Vf;`, runtime.context);
  return { ...runtime, store, session, get pending() { return pending; }, get cancels() { return cancels; }, get generation() { return generation; } };
}

test("a suspended audio context times out after five seconds, closes, and rejects into native cleanup", async () => {
  let closed = 0;
  const audioContext = { state: "suspended", resume: () => new Promise(() => {}), close() { closed += 1; this.state = "closed"; } };
  const runtime = clockContext({ audioContext });
  vm.runInContext(`${patchRecorderBundle(recorderFixture)}\nglobalThis.recorder=new AudioRecorder(audioContext);`, runtime.context);
  const started = vm.runInContext("recorder.startWithStream()", runtime.context);
  const rejected = assert.rejects(started, { name: "DictationAudioContextTimeout" });
  await settle();
  await runtime.advance(4999);
  assert.equal(closed, 0);
  await runtime.advance(1);
  await rejected;
  assert.equal(closed, 1);
  assert.equal(runtime.context.recorder.state, "error");
  assert.equal(runtime.timers.size, 0);
});

test("successful resume keeps its context and clears the startup deadline", async () => {
  let closed = 0;
  const audioContext = { state: "suspended", resume() { this.state = "running"; }, close() { closed += 1; } };
  const runtime = clockContext({ audioContext });
  vm.runInContext(`${patchRecorderBundle(recorderFixture)}\nglobalThis.recorder=new AudioRecorder(audioContext);`, runtime.context);
  await vm.runInContext("recorder.startWithStream()", runtime.context);
  await runtime.advance(6000);
  assert.equal(closed, 0);
  assert.equal(runtime.context.recorder.state, "recording");
  assert.equal(runtime.timers.size, 0);
});

test("a machinery import that never settles exits pending after fifteen seconds", async () => {
  const runtime = facadeRuntime({ load: () => new Promise(() => {}) });
  const started = vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  await settle();
  assert.equal(runtime.pending, "private-owner");
  await runtime.advance(15000);
  await started;
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "error");
  assert.equal(runtime.cancels, 0);
  assert.equal(runtime.timers.size, 0);
});

test("a timed out native start cancels its generation and releases a late synthetic stream", async () => {
  const waitForMic = deferred();
  const runtime = facadeRuntime({ waitForMic });
  const started = vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  await settle();
  assert.equal(runtime.store.getState().status, "connecting");
  const before = runtime.generation;
  await runtime.advance(20000);
  await started;
  assert.equal(runtime.cancels, 1);
  assert.equal(runtime.generation, before + 1);
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "error");
  assert.equal(runtime.store.getState().error, "无法启动听写，请重试。");
  let stopped = 0;
  waitForMic.resolve({ stop() { stopped += 1; } });
  await settle();
  assert.equal(stopped, 1);
  assert.equal(runtime.session.micStream, null);
  assert.equal(runtime.store.getState().status, "error");
  assert.equal(runtime.timers.size, 0);
});

test("stopping a pending native start prevents a later startup deadline from replacing idle with an error", async () => {
  const waitForMic = deferred();
  const runtime = facadeRuntime({ waitForMic });
  const started = vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  await settle();
  vm.runInContext("facade.stop()", runtime.context);
  await runtime.advance(20000);
  await started;
  assert.equal(runtime.cancels, 0);
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "idle");
  let stopped = 0;
  waitForMic.resolve({ stop() { stopped += 1; } });
  await settle();
  assert.equal(stopped, 1);
  assert.equal(runtime.store.getState().status, "idle");
  assert.equal(runtime.timers.size, 0);
});

test("recorder resume timeout propagates through the patched facade and clears connecting and pending", async () => {
  const runtime = facadeRuntime();
  let closed = 0;
  runtime.context.audioContext = {
    state: "suspended", resume: () => new Promise(() => {}),
    close() { closed += 1; this.state = "closed"; },
  };
  vm.runInContext(`${patchRecorderBundle(recorderFixture)}
session.start=async()=>{
  fD.getState().setStatus("connecting");
  const recorder=new AudioRecorder(audioContext);
  await recorder.startWithStream();
  fD.getState().setStatus("recording");
};`, runtime.context);
  const started = vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  await settle();
  await runtime.advance(5000);
  await started;
  assert.equal(closed, 1);
  assert.equal(runtime.cancels, 1);
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "error");
  assert.equal(runtime.timers.size, 0);
});

for (const synchronous of [true, false]) {
  test(`a ${synchronous ? "synchronous throw" : "rejected start"} after connecting exits through cancellation`, async () => {
    const runtime = facadeRuntime({ start(store) {
      store.getState().setStatus("connecting");
      if (synchronous) throw new Error("private-error-detail");
      return Promise.reject(new Error("private-error-detail"));
    } });
    await vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
    assert.equal(runtime.cancels, 1);
    assert.equal(runtime.pending, null);
    assert.equal(runtime.store.getState().status, "error");
    assert.equal(runtime.store.getState().error, "无法启动听写，请重试。");
    assert.equal(runtime.timers.size, 0);
    assert.doesNotMatch(JSON.stringify(runtime.logs), /private-error-detail|private-owner|private-transcript/);
  });
}

test("successful native start is not cancelled and safe diagnostics distinguish recording and finishing", async () => {
  const runtime = facadeRuntime();
  await vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  await runtime.advance(25000);
  assert.equal(runtime.cancels, 0);
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "recording");
  assert.equal(runtime.context.__codexPlusLocalDictationStatus.phase, "recording");
  vm.runInContext("facade.stop()", runtime.context);
  assert.equal(runtime.context.__codexPlusLocalDictationStatus.phase, "finishing");
  runtime.store.update({ drainPending: false });
  assert.equal(runtime.context.__codexPlusLocalDictationStatus.phase, "idle");
  const phases = runtime.logs.map(([, snapshot]) => snapshot.phase);
  assert.deepEqual(phases, ["idle", "pending", "connecting", "recording", "finishing", "idle"]);
  assert.doesNotMatch(JSON.stringify(runtime.logs), /private-owner|private-transcript/);
  assert.equal(runtime.timers.size, 0);
});

test("startup patch is exact, idempotent, and refuses ambiguous or corrupt versions", () => {
  for (const [patch, fixture, asset] of [[patchRecorderBundle, recorderFixture, startupAssets[0]], [patchFacadeBundle, facadeFixture, startupAssets[1]]]) {
    const patched = patch(fixture);
    assert.deepEqual(patch(patched), patched);
    assert.equal(isPatchedStartupAsset(asset.pathname, patched), true);
    assert.equal(isPatchedStartupAsset(asset.pathname, Buffer.from(`${patched}corrupt`)), false);
    assert.throws(() => patch(`${fixture}${fixture}`), /does not match|ambiguous/);
    assert.throws(() => patch("unknown version"), /does not match/);
  }
  assert.equal(isPatchedStartupAsset("/unlisted.js", patchRecorderBundle(recorderFixture)), false);
});

test("a failed native cancellation still clears the visible connecting state", async () => {
  const runtime = facadeRuntime({ start(store) {
    store.getState().setStatus("connecting");
    throw new Error("private-error-detail");
  } });
  runtime.session.cancel = () => { throw new Error("cleanup failed"); };
  await vm.runInContext('facade.start(null,{}, "private-owner")', runtime.context);
  assert.equal(runtime.pending, null);
  assert.equal(runtime.store.getState().status, "error");
  assert.equal(runtime.timers.size, 0);
});

function removeFixtureTree(directory) {
  for (const entry of fs.readdirSync(directory, { withFileTypes: true })) {
    const target = path.join(directory, entry.name);
    if (entry.isDirectory()) removeFixtureTree(target);
    else fs.unlinkSync(target);
  }
  fs.rmdirSync(directory);
}

function electronEntry(url, input, { version = 5, contentLength } = {}) {
  const body = Buffer.from(input);
  const key = Buffer.from(`1/0/${url}`);
  const header = Buffer.alloc(24);
  Buffer.from("305c72a71b6dfbfc", "hex").copy(header);
  header.writeUInt32LE(version, 8);
  header.writeUInt32LE(key.length, 12);
  const footer = (flags, streamSize = 0) => {
    const bytes = Buffer.alloc(24);
    Buffer.from("d8410d97456ffaf4", "hex").copy(bytes);
    bytes.writeUInt32LE(flags, 8);
    bytes.writeBigUInt64LE(BigInt(streamSize), 16);
    return bytes;
  };
  const metadata = Buffer.concat([
    Buffer.alloc(40),
    Buffer.from(`HTTP/1.1 200 OK\0content-length: ${contentLength ?? body.length}\0`),
  ]);
  return Buffer.concat([header, key, body, footer(1), metadata, Buffer.alloc(32), footer(3, metadata.length)]);
}

test("Electron Simple Cache v5 matches only the declared key and validates the stream body boundary", () => {
  const asset = startupAssets[1];
  const source = `${facadeFixture}\n// ${asset.url}\n// ${asset.url}\n`;
  const bytes = electronEntry(asset.url, source);
  assert.equal(extractElectronStartupAsset(bytes, asset).toString(), source);
  const unrelated = electronEntry("https://claude.ai/new", source);
  assert.equal(extractElectronStartupAsset(unrelated, asset), null);
  assert.equal(extractElectronStartupAsset(Buffer.from(source), asset), null);
  assert.throws(() => extractElectronStartupAsset(electronEntry(asset.url, source, { version: 6 }), asset), /Unsupported/);
  assert.throws(() => extractElectronStartupAsset(electronEntry(asset.url, source, { contentLength: 1 }), asset), /body bounds/);
  const keyEnd = 24 + bytes.readUInt32LE(12);
  assert.throws(() => extractElectronStartupAsset(bytes.subarray(0, keyEnd + Buffer.byteLength(source)), asset), /body bounds/);
});

test("both startup assets are prepared before writes and can be restored without changing Electron cache", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "claude-startup-patch-"));
  try {
    const cacheDir = path.join(root, "proxy");
    const electronCacheDir = path.join(root, "electron");
    fs.mkdirSync(electronCacheDir);
    const fixtures = [recorderFixture, facadeFixture];
    const unrelated = electronEntry("https://claude.ai/new", `reference ${startupAssets[1].url} reference ${startupAssets[1].url}`);
    fs.writeFileSync(path.join(electronCacheDir, "000-unrelated_0"), unrelated);
    for (const [index, asset] of startupAssets.entries()) {
      fs.writeFileSync(path.join(electronCacheDir, `${index}_0`), electronEntry(asset.url, fixtures[index]));
    }
    assert.deepEqual(applyStartupPatches({ cacheDir, electronCacheDir }).map(({ status }) => status), ["patched", "patched"]);
    assert.deepEqual(applyStartupPatches({ cacheDir, electronCacheDir }).map(({ status }) => status), ["already-patched", "already-patched"]);
    assert.equal(restoreStartupPatches({ cacheDir }), "restored");
    for (const [index, asset] of startupAssets.entries()) {
      const key = asset.url.replace(/^https:\/\//, "");
      const name = crypto.createHash("sha1").update(key).digest("hex");
      assert.equal(fs.readFileSync(path.join(cacheDir, name), "utf8"), fixtures[index]);
      assert.equal(fs.readFileSync(path.join(electronCacheDir, `${index}_0`)).includes(Buffer.from("codex-local-dictation-start-v1")), false);
    }
    assert.deepEqual(fs.readFileSync(path.join(electronCacheDir, "000-unrelated_0")), unrelated);
  } finally {
    removeFixtureTree(root);
  }
});

test("an unsupported second asset is refused before writing any local patched asset", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "claude-startup-refused-"));
  try {
    const cacheDir = path.join(root, "proxy");
    const electronCacheDir = path.join(root, "electron");
    fs.mkdirSync(electronCacheDir);
    for (const [index, asset] of startupAssets.entries()) {
      fs.writeFileSync(path.join(electronCacheDir, `${index}_0`), electronEntry(asset.url, index === 0 ? recorderFixture : "unsupported version"));
    }
    assert.throws(() => applyStartupPatches({ cacheDir, electronCacheDir }), /does not match/);
    assert.equal(fs.existsSync(cacheDir), false);
  } finally {
    removeFixtureTree(root);
  }
});
