import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

const source = readFileSync(new URL("../../../assets/inject/renderer-inject/82-rate-limit-auto-resume.js", import.meta.url), "utf8");
const threadId = "019f4e36-490e-7ae0-8e78-a8b3ab33a429";

type TestTurn = { id: string; status: string; error: { codexErrorInfo: Record<string, { httpStatusCode: number }>; message: string } };
type ReadResult = { thread: { id: string; turns: TestTurn[] } };

function failedTurn(id = "turn-1", status = 429): TestTurn {
  return { id, status: "failed", error: { codexErrorInfo: { responseTooManyFailedAttempts: { httpStatusCode: status } }, message: `HTTP ${status}` } };
}

function harness(settings: { enabled?: boolean } = { enabled: true }) {
  const data = {
    enabled: settings.enabled, now: 100000, threadId, hostId: "local", active: true, visible: true,
    label: "Resume", draft: "", disabled: false, composerCount: 1, clicks: 0,
    turn: failedTurn(), reads: 0, clientAvailable: true, href: `/thread/${threadId}`,
  };
  const chip = { style: { display: "" }, textContent: "", id: "", type: "", listeners: {} as Record<string, () => void>,
    setAttribute() {}, addEventListener(name: string, fn: () => void) { this.listeners[name] = fn; } };
  let hasChip = false;
  const button = { get disabled() { return data.disabled; }, getAttribute(name: string) { return name === "aria-label" ? data.label : null; }, click() { data.clicks++; } };
  const root = { closest() { return null; }, querySelectorAll(selector: string) { return selector === "button" ? [button] : [{ value: data.draft }]; } };
  const document = {
    querySelectorAll() { return Array.from({ length: data.composerCount }, () => root); },
    getElementById() { return hasChip ? chip : null; }, createElement() { return chip; },
    body: { appendChild() { hasChip = true; } },
  };
  const intervals = new Set();
  const window = { location: { get href() { return data.href; } }, setInterval(fn: () => void) { intervals.add(fn); return fn; }, clearInterval(fn: () => void) { intervals.delete(fn); } };
  const client = {
    async sendRequest(method: string, params: unknown) {
      assert.equal(method, "thread/read");
      assert.deepEqual(params, { threadId: data.threadId, includeTurns: true });
      data.reads++;
      return { thread: { id: data.threadId, turns: [failedTurn("historical-429"), data.turn] } };
    },
  };
  const api = new Function("document", "window", "Date", "codexPlusSettings", "setCodexPlusSetting", "locationThreadId", "sessionRows", "sessionRefFromRow", "isCurrentSessionRow", "visibleElement", "discoverNativeHostClient", "nativeHostClient",
    `${source}\nreturn { tick: rateLimitAutoResumeTick, sync: syncCodexPlusRateLimitAutoResume, state: rateLimitAutoResumeState, is429: rateLimitAutoResumeIs429 };`)(
      document, window, { now: () => data.now }, () => ({ rateLimitAutoResume: data.enabled }),
      (_key: string, value: boolean) => { data.enabled = value; }, () => data.threadId, () => [{}],
      () => ({ session_id: data.threadId, host_id: data.hostId }), () => data.active, () => data.visible,
      async () => data.clientAvailable ? client : null, () => null,
    );
  return { data, api, chip, intervals, client, async tick(ms = 0) { data.now += ms; await api.tick(); } };
}

test("429 waits 30 seconds, rechecks latest turn, and clicks the native Resume once", async () => {
  const h = harness();
  await h.tick();
  assert.equal(h.data.clicks, 0);
  await h.tick(29999);
  assert.equal(h.data.reads, 1);
  await h.tick(1);
  assert.equal(h.data.clicks, 1);
  assert.equal(h.data.reads, 2);
  await h.tick(29999);
  assert.equal(h.data.clicks, 1);
  assert.equal(h.data.reads, 2, "unchanged Resume must not repeatedly read full history");
  await h.tick(1);
  assert.equal(h.data.clicks, 1);
  assert.equal(h.data.reads, 3, "recheck the turn after the cooldown even if Resume never changed");
});

test("missing and false settings default off without a timer or native reads", async () => {
  for (const settings of [{}, { enabled: false }]) {
    const h = harness(settings);
    h.api.sync(); await h.tick(120000);
    assert.equal(h.intervals.size, 0);
    assert.equal(h.data.reads, 0);
    assert.equal(h.data.clicks, 0);
  }
});

test("a fast second 429 is detected even without observing a button transition", async () => {
  const h = harness(); await h.tick(); await h.tick(30000);
  h.data.turn = failedTurn("fast-failure");
  await h.tick(29999);
  assert.equal(h.data.clicks, 1);
  await h.tick(1);
  assert.equal(h.data.clicks, 1, "a new failed turn needs its own 30-second wait");
  await h.tick(29999);
  assert.equal(h.data.clicks, 1);
  await h.tick(1);
  assert.equal(h.data.clicks, 2);
});

test("concurrent ticks and a repeated tick at the deadline cannot double click", async () => {
  const h = harness(); await h.tick();
  await Promise.all([h.tick(30000), h.tick(), h.tick()]);
  assert.equal(h.data.clicks, 1);
  assert.equal(h.data.reads, 2);
  await h.tick();
  assert.equal(h.data.clicks, 1);
});

test("turn changes while the verification read is pending restart the cooldown", async () => {
  const h = harness(); await h.tick();
  let resolve: (value: ReadResult) => void = () => {};
  h.client.sendRequest = () => new Promise((done) => { resolve = done; }) as ReturnType<typeof h.client.sendRequest>;
  const pending = h.tick(30000);
  await Promise.resolve(); await Promise.resolve();
  resolve({ thread: { id: threadId, turns: [failedTurn("new-turn")] } });
  await pending;
  assert.equal(h.data.clicks, 0);
  assert.equal(h.api.state.nextAttemptAt, h.data.now + 30000);
});

test("structured status takes precedence; plain conversation numbers cannot match", () => {
  const { api } = harness();
  assert.equal(api.is429(failedTurn().error), true);
  assert.equal(api.is429({ ...failedTurn("a", 503).error, message: "HTTP 429" }), false);
  for (const message of ["HTTP 429", "unexpected status 429", "status code: 429", "429 Too Many Requests"]) assert.equal(api.is429({ message }), true);
  for (const message of ["line 429", "request 1429", "HTTP 503", "rate limit", "quota exceeded"]) assert.equal(api.is429({ message }), false);
});

test("other failures, completed turns and old 429 failures cannot trigger", async () => {
  for (const turn of [failedTurn("a", 503), { ...failedTurn(), status: "completed" }, { ...failedTurn(), error: { codexErrorInfo: {}, message: "line 429" } }]) {
    const h = harness(); h.data.turn = turn;
    await h.tick(); await h.tick(60000);
    assert.equal(h.data.clicks, 0);
  }
});

test("disabled switch, nonempty draft, hidden/disabled/Send button, ambiguous composer and unknown host do not trigger", async () => {
  const cases = [{ enabled: false }, { draft: "unsent text" }, { visible: false }, { disabled: true }, { label: "Send" }, { composerCount: 2 }, { hostId: "" }, { active: false }, { clientAvailable: false }];
  for (const values of cases) {
    const h = harness(); Object.assign(h.data, values);
    await h.tick(); await h.tick(60000);
    assert.equal(h.data.clicks, 0, JSON.stringify(values));
  }
});

test("draft typed or a new turn started during countdown cancels the click", async () => {
  const h = harness(); await h.tick();
  h.data.draft = "editing";
  await h.tick(30000);
  assert.equal(h.data.clicks, 0);
  h.data.draft = "";
  h.data.turn = { ...failedTurn(), status: "inProgress" };
  await h.tick(30000);
  assert.equal(h.data.clicks, 0);
});

test("a different failed turn at deadline gets its own full countdown", async () => {
  const h = harness(); await h.tick();
  h.data.turn = failedTurn("turn-2");
  await h.tick(30000);
  assert.equal(h.data.clicks, 0);
  await h.tick(30000);
  assert.equal(h.data.clicks, 1);
});

test("one click per failed turn survives disappearing and returning Resume", async () => {
  const h = harness(); await h.tick(); await h.tick(30000);
  h.data.label = "Stop"; await h.tick();
  h.data.label = "Resume"; await h.tick(60000); await h.tick(60000);
  assert.equal(h.data.clicks, 1);
});

test("five separate failed turns exhaust the visit budget", async () => {
  const h = harness();
  for (let i = 0; i < 6; i++) {
    h.data.label = "Stop"; await h.tick(5000);
    h.data.label = "Resume"; h.data.turn = failedTurn(`turn-${i}`);
    await h.tick(5000); await h.tick(30000);
  }
  assert.equal(h.data.clicks, 5);
  assert.match(h.chip.textContent, /已达上限/);
});

test("changing chat or host discards countdown and visit counter", async () => {
  for (const values of [{ threadId: "019f4e36-490e-7ae0-8e78-a8b3ab33a430", href: "/thread/other" }, { hostId: "remote" }]) {
    const h = harness(); await h.tick(); Object.assign(h.data, values);
    await h.tick(30000);
    assert.equal(h.data.clicks, 0);
    assert.equal(h.api.state.attempts, 0);
    await h.tick(30000);
    assert.equal(h.data.clicks, 1);
  }
});

test("clicking status stops immediately, including an in-flight read", async () => {
  const h = harness(); await h.tick();
  let resolve: (value: ReadResult) => void = () => {};
  h.client.sendRequest = () => new Promise((done) => { resolve = done; }) as ReturnType<typeof h.client.sendRequest>;
  const pending = h.tick(30000);
  await Promise.resolve(); await Promise.resolve();
  h.chip.listeners.click();
  resolve({ thread: { id: threadId, turns: [failedTurn()] } });
  await pending;
  assert.equal(h.data.clicks, 0);
  assert.equal(h.data.enabled, false);
  h.api.sync(); assert.equal(h.intervals.size, 0);
});

test("a late read after navigation cannot click another chat", async () => {
  const h = harness(); await h.tick();
  let resolve: (value: ReadResult) => void = () => {};
  h.client.sendRequest = () => new Promise((done) => { resolve = done; }) as ReturnType<typeof h.client.sendRequest>;
  const pending = h.tick(30000);
  await Promise.resolve(); await Promise.resolve();
  h.data.href = "/thread/another";
  resolve({ thread: { id: threadId, turns: [failedTurn()] } });
  await pending;
  assert.equal(h.data.clicks, 0);
});

test("RPC errors do not click, and only one timer is installed", async () => {
  const h = harness();
  h.api.sync(); h.api.sync(); assert.equal(h.intervals.size, 1);
  h.client.sendRequest = async () => { throw new Error("unavailable"); };
  await h.tick(); await h.tick(30000);
  assert.equal(h.data.clicks, 0);
});
