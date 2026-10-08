const assert = require("node:assert/strict");
const { readFileSync } = require("node:fs");
const vm = require("node:vm");
const { webcrypto } = require("node:crypto");
const source = readFileSync(`${__dirname}/renderer-inject/93-dictation.js`, "utf8");
const flush = () => new Promise(resolve => setImmediate(resolve));

function fixture() {
  const requests = [], events = [], frames = [];
  let resolveResponse, getMedia;
  class Textarea {
    constructor() { this._value = "existing draft"; this.selectionStart = this.selectionEnd = this._value.length; this.dataset = {}; this.isConnected = true; }
    get value() { return this._value; }
    set value(value) { this._value = value; }
    setSelectionRange(from, to) { this.selectionStart = from; this.selectionEnd = to; }
    focus() { document.activeElement = this; }
    dispatchEvent(event) { events.push(event.type); }
    closest(selector) { return selector.includes("inert") ? null : root; }
  }
  const editor = new Textarea();
  const send = { disabled: false, getAttribute: key => key === "aria-label" ? "Send" : null, click: () => events.push("send") };
  const root = { dataset: {}, contains: node => node === editor, querySelectorAll: selector => selector === "button" ? [send] : [editor], hasAttribute: name => name === "data-codex-composer-root" };
  editor.parentElement = root;
  const footer = { parentElement: root, getAttribute: () => null };
  const document = {
    body: {}, activeElement: editor,
    querySelectorAll: selector => selector.includes("dictation-target") ? [editor] : [],
    getElementById: () => null,
    addEventListener() {}, removeEventListener() {},
    execCommand: () => { throw Error("must use native transaction or controlled textarea"); },
  };
  const track = { stop: () => events.push("track-stop") };
  class Recorder {
    static isTypeSupported(type) { return type === "audio/webm"; }
    constructor(stream, options) { this.mimeType = options.mimeType; this.state = "inactive"; }
    start() { this.state = "recording"; }
    stop() {
      this.state = "inactive";
      queueMicrotask(() => { this.ondataavailable?.({ data: new Blob(["audio"], { type: this.mimeType }) }); this.onstop?.(); });
    }
  }
  const window = { location: { href: "app://-/thread/one" }, addEventListener() {}, removeEventListener() {} };
  const context = vm.createContext({
    window, document, navigator: { mediaDevices: { getUserMedia: () => getMedia ? getMedia() : Promise.resolve({ getTracks: () => [track] }) } },
    MediaRecorder: Recorder, HTMLTextAreaElement: Textarea, Blob, AbortController, Event, DOMException,
    crypto: webcrypto, console, btoa, setInterval, clearInterval, setTimeout,
    requestAnimationFrame: fn => { frames.push(fn); },
    registerCodexPlusExtensionSelector() {}, visibleElement: () => true, isExtensionUiNode: () => false,
    codexPlusBackendSettingsLoaded: true, codexPlusBackendSettings: { dictation: { enabled: true } },
    helperBase: "http://127.0.0.1:57321", showToast: message => events.push(message),
    loadOptionalCodexAppModule: async () => ({}),
    postJson: async (path, payload) => {
      if (path === "/dictation/status") return { enabled: true, configured: true, helperToken: "fake-capability" };
      if (path === "/dictation/cancel") { events.push({ cancel: payload.requestId }); return { status: "cancelled" }; }
      assert.equal(path, "/dictation/transcribe");
      requests.push({ path, payload });
      return new Promise(resolve => { resolveResponse = text => resolve({ text }); });
    },
    fetch: () => { throw Error("native app CSP blocks renderer localhost fetch"); },
  });
  vm.runInContext(`${source}\n globalThis.testApi = { state: codexPlusDictation, start: startCodexPlusDictation, stop: stopCodexPlusDictation, cancel: cancelCodexPlusDictation, retry: transcribeCodexPlusDictation, insert: codexPlusInsertDictation, finish: finishCodexPlusDictation, key: codexPlusDictationKeyHandler, encode: codexPlusDictationAudioBase64 };`, context);
  // State-machine contracts do not depend on visual layout; DOM mounting is separately guarded.
  context.renderCodexPlusDictation = () => {};
  context.installCodexPlusDictation = () => {};
  return {
    ...context.testApi, editor, root, footer, requests, events, frames, context,
    resolve: text => resolveResponse(text), setMedia: value => { getMedia = value; },
    respondFailure: () => { resolveResponse = null; },
  };
}

(async () => {
  const backendLoaderSource = readFileSync(`${__dirname}/renderer-inject/30-service-tier.js`, "utf8");
  const loaderStart = backendLoaderSource.indexOf("  async function loadBackendSettingsState(");
  assert.ok(loaderStart >= 0);
  const loaderEnd = backendLoaderSource.indexOf("\n  }\n", loaderStart);
  assert.ok(loaderEnd > loaderStart);
  const loaderSource = backendLoaderSource.slice(loaderStart, loaderEnd + "\n  }\n".length);
  const scanSource = readFileSync(`${__dirname}/renderer-inject/95-conversation-view.js`, "utf8");
  const scanStart = scanSource.indexOf("  function runScanStep(step)");
  const scanEnd = scanSource.indexOf("  function scan()", scanStart);
  assert.ok(scanStart >= 0 && scanEnd > scanStart);
  for (const testMode of [false, true]) {
    let installations = 0;
    const context = vm.createContext({
      window: { __CODEX_PLUS_TEST_SERVICE_TIER__: testMode },
      codexPlusBackendSettingsSeq: 3, codexPlusBackendSettingsLoaded: false,
      codexPlusBackendSettings: { activeRelayCodexProvider: "stale" },
      postJson: async () => ({ enhancementsEnabled: true, activeRelayCodexProvider: "deepseek", dictation: { enabled: false } }),
      installCodexPlusDictation: () => { installations += 1; throw Error("dictation UI unavailable"); },
    });
    vm.runInContext(`${scanSource.slice(scanStart, scanEnd)}\n${loaderSource}`, context);
    assert.equal(await context.loadBackendSettingsState(), true, "UI installation failure must not invalidate successfully loaded provider settings");
    assert.equal(context.codexPlusBackendSettingsLoaded, true);
    assert.equal(context.codexPlusBackendSettings.activeRelayCodexProvider, "deepseek");
    assert.equal(installations, testMode ? 0 : 1, "service-tier-only harness must skip uninitialized recording UI");
    if (!testMode) assert.match(context.window.__codexSessionDeleteScanFailures[0], /dictation UI unavailable/);
    context.postJson = async () => ({});
    assert.equal(await context.loadBackendSettingsState(), false, "invalid backend responses still fail the settings load");
    assert.equal(installations, testMode ? 0 : 1);
  }

  const encoding = fixture();
  const binary = Uint8Array.from({ length: 524291 }, (_, index) => index % 256);
  const encoded = await encoding.encode(new Blob([binary]), new AbortController().signal);
  assert.deepEqual(Buffer.from(encoded, "base64"), Buffer.from(binary), "chunked base64 preserves non-text audio across chunk boundaries");

  let releaseAudio;
  encoding.state.blob = { size: 5, type: "audio/webm", arrayBuffer: () => new Promise(resolve => { releaseAudio = resolve; }) };
  const encodingPending = encoding.retry(); await flush();
  encoding.cancel(); releaseAudio(new Uint8Array([1, 2, 3, 4, 5]).buffer); await encodingPending;
  assert.equal(encoding.requests.length, 0, "cancel during encoding prevents the bridge upload");
  assert.equal(encoding.events.some(event => typeof event === "object" && event.cancel), false, "cancel during encoding must not create an unused backend cancellation");

  const insert = fixture();
  assert.equal(insert.insert(insert.editor, "  hello  ", null, null), true);
  assert.equal(insert.editor.value, "existing draft hello");
  assert.deepEqual(insert.events, ["input"]);
  insert.editor.selectionStart = 0; insert.editor.selectionEnd = 8;
  assert.equal(insert.insert(insert.editor, "replace", null, null), true);
  assert.equal(insert.editor.value, "replace draft hello", "selection replacement preserves the rest of the draft");

  const native = fixture();
  const nativeCalls = [];
  const api = { context: () => ({ composerId: "target", root: native.root }), append: (id, text) => { nativeCalls.push([id, text]); return true; } };
  assert.equal(native.insert(native.editor, "你好", api, "target"), true);
  assert.deepEqual(nativeCalls, [["target", "你好"]]);
  assert.equal(native.editor.value, "existing draft", "native composer owns its document and attachments");
  assert.equal(native.insert(native.editor, "wrong", api, "other"), false);
  assert.equal(native.insert(native.editor, "busy", { ...api, append: () => false }, "target"), false, "do not bypass native dictation/readonly refusal");
  native.editor.matches = () => true;
  assert.equal(native.insert(native.editor, "readonly", null, null), false);
  assert.equal(native.editor.value, "existing draft", "compatibility insertion must not bypass readonly");

  const registry = fixture();
  registry.editor.matches = () => false;
  vm.runInContext(`
    var RM = new Map([[document.activeElement, {composerId: "native-one", appendPromptText: text => { document.activeElement.value += " " + text; }, isDictationInProgress: () => false}]]);
    function CVn(e,t,n){for(let[r,i]of RM)if(i.composerId===e&&r.isConnected&&!r.matches(":disabled, [readonly], [contenteditable='false']"))return i.isDictationInProgress?.()===true||i.appendPromptText==null?false:(i.appendPromptText(t,n),r.focus(),true);return false}
    function IVn(){let e=document.activeElement?.closest("[data-codex-composer-root]");return e?{composerId:"native-one",root:e}:null}
    loadOptionalCodexAppModule = async () => ({changedExportName: CVn, anotherChangedExportName: IVn});
  `, registry.context);
  await registry.start(registry.footer);
  assert.equal(registry.state.target.composerId, "native-one", "discover native APIs by capabilities rather than minified export names");
  registry.state.startedAt -= 1000; registry.stop("insert"); await flush(); registry.resolve("native result"); await flush();
  assert.equal(registry.editor.value, "existing draft native result");
  assert.equal(registry.state.phase, "idle");

  const success = fixture();
  await success.start(success.footer);
  assert.equal(success.state.phase, "recording");
  success.state.startedAt -= 1000;
  success.stop("insert"); await flush();
  assert.equal(success.requests.length, 1);
  assert.equal(success.requests[0].path, "/dictation/transcribe");
  assert.equal(success.requests[0].payload.helperToken, "fake-capability");
  assert.equal(success.requests[0].payload.apiKey, undefined, "never attach coding or ASR credentials in renderer");
  assert.equal(success.requests[0].payload.filename, "dictation.webm");
  assert.equal(success.requests[0].payload.mimeType, "audio/webm");
  assert.equal(Buffer.from(success.requests[0].payload.audioBase64, "base64").toString(), "audio", "audio uses the privileged bridge even when renderer fetch is forbidden");
  success.resolve("hello"); await flush();
  assert.equal(success.editor.value, "existing draft hello");
  assert.equal(success.state.phase, "idle");
  assert.ok(success.events.includes("track-stop"));
  assert.equal(success.events.some(event => typeof event === "object" && event.cancel), false, "success cleanup must not send cancellation");

  const cancelled = fixture();
  await cancelled.start(cancelled.footer); cancelled.state.startedAt -= 1000;
  cancelled.stop("insert"); await flush();
  cancelled.cancel(); cancelled.resolve("late result"); await flush();
  assert.equal(cancelled.editor.value, "existing draft", "late results after cancellation must never insert");
  assert.equal(cancelled.state.phase, "idle");
  assert.ok(cancelled.events.some(event => event.cancel === cancelled.requests[0].payload.requestId), "cancel interrupts the matching Rust request");

  const permission = fixture(); let grantPermission;
  permission.setMedia(() => new Promise(resolve => { grantPermission = resolve; }));
  const pending = permission.start(permission.footer); await flush();
  permission.cancel();
  grantPermission({ getTracks: () => [{ stop: () => permission.events.push("late-track-stop") }] });
  await pending;
  assert.ok(permission.events.includes("late-track-stop"), "cancel while permission is pending releases subsequently granted mic");
  assert.equal(permission.state.phase, "idle");

  const changed = fixture();
  await changed.start(changed.footer); changed.state.startedAt -= 1000;
  changed.stop("send"); await flush();
  changed.context.window.location.href = "app://-/thread/two";
  changed.resolve("other chat"); await flush();
  assert.equal(changed.editor.value, "existing draft");
  assert.equal(changed.state.phase, "result");
  assert.equal(changed.state.text, "other chat", "retain copyable text instead of inserting in another chat");
  changed.cancel();

  const sending = fixture();
  await sending.start(sending.footer); sending.state.startedAt -= 1000;
  sending.stop("send"); await flush(); sending.resolve("send this"); await flush();
  assert.equal(sending.editor.value, "existing draft send this");
  assert.equal(sending.events.includes("send"), false, "wait for native state update");
  sending.frames.shift()(); sending.frames.shift()(); await flush();
  assert.equal(sending.events.includes("send"), true);

  const switched = fixture();
  await switched.start(switched.footer); switched.state.startedAt -= 1000;
  switched.stop("send"); await flush(); switched.resolve("draft"); await flush();
  switched.context.window.location.href = "app://-/thread/two";
  switched.frames.shift()(); switched.frames.shift()(); await flush();
  assert.equal(switched.events.includes("send"), false, "navigation during send scheduling must never submit another chat");

  const cancelSending = fixture();
  await cancelSending.start(cancelSending.footer); cancelSending.state.startedAt -= 1000;
  cancelSending.stop("send"); await flush(); cancelSending.resolve("draft"); await flush();
  cancelSending.cancel(); cancelSending.frames.shift()(); cancelSending.frames.shift()(); await flush();
  assert.equal(cancelSending.events.includes("send"), false, "cancel while native rendering is pending must suppress send");

  const keyboard = fixture();
  await keyboard.start(keyboard.footer); keyboard.state.startedAt -= 1000;
  const keyboardEvents = [];
  const enter = { key: "Enter", preventDefault: () => keyboardEvents.push("prevent"), stopImmediatePropagation: () => keyboardEvents.push("stop") };
  keyboard.key({ ...enter, shiftKey: true });
  assert.equal(keyboard.state.phase, "recording", "Shift+Enter still inserts a native newline");
  keyboard.key({ ...enter, isComposing: true });
  assert.equal(keyboard.state.phase, "recording", "IME confirmation must not stop recording or send");
  keyboard.key(enter); await flush();
  assert.equal(keyboard.state.phase, "transcribing");
  assert.equal(keyboard.state.action, "send");
  assert.deepEqual(keyboardEvents, ["prevent", "stop"], "Enter suppresses native submission of the old draft until transcription completes");
  keyboard.key({ ...enter, key: "Escape" }); keyboard.resolve("cancelled by Escape"); await flush();
  assert.equal(keyboard.state.phase, "idle");
  assert.equal(keyboard.editor.value, "existing draft");

  const failed = fixture();
  const originalPost = failed.context.postJson;
  failed.context.postJson = async (path, payload) => path === "/dictation/transcribe" ? { status: "failed", message: "upstream unavailable" } : originalPost(path, payload);
  await failed.start(failed.footer); failed.state.startedAt -= 1000;
  failed.stop("insert"); await flush();
  assert.equal(failed.state.phase, "error");
  const blob = failed.state.blob;
  assert.ok(blob, "failed transcription retains recorded audio for retry");
  let retryPayload;
  failed.context.postJson = async (path, payload) => {
    if (path !== "/dictation/transcribe") return originalPost(path, payload);
    retryPayload = payload; return { text: "retry" };
  };
  await failed.retry();
  assert.equal(Buffer.from(retryPayload.audioBase64, "base64").toString(), await blob.text(), "retry transmits the retained recording");
  assert.equal(failed.editor.value, "existing draft retry");
  assert.equal(failed.state.phase, "idle");
  assert.equal(failed.state.blob, null, "success releases recorded audio");

  const disabled = fixture();
  disabled.context.postJson = async () => ({ enabled: false, configured: false });
  await disabled.start(disabled.footer);
  assert.equal(disabled.state.phase, "error");
  assert.equal(disabled.requests.length, 0);
  disabled.cancel();

  const optIn = fixture();
  optIn.context.codexPlusBackendSettings.dictation.enabled = false;
  await optIn.start(optIn.footer);
  assert.equal(optIn.state.phase, "idle", "disabled settings never unlock the recording entry");
  assert.equal(optIn.requests.length, 0);

  const masterOff = fixture();
  masterOff.context.codexPlusBackendSettings.enhancementsEnabled = false;
  let masterOffMediaCalls = 0;
  masterOff.setMedia(() => { masterOffMediaCalls += 1; throw Error("disabled enhancements must not request microphone"); });
  await masterOff.start(masterOff.footer);
  masterOff.state.blob = new Blob(["audio"], { type: "audio/webm" });
  await masterOff.retry();
  assert.equal(masterOffMediaCalls, 0);
  assert.equal(masterOff.state.phase, "idle", "master enhancement switch disables recording and retained-audio retry");
  assert.equal(masterOff.requests.length, 0);
  assert.equal(masterOff.state.blob, null);

  const masterOffPermission = fixture(); let masterOffGrant;
  masterOffPermission.setMedia(() => new Promise(resolve => { masterOffGrant = resolve; }));
  const masterOffPending = masterOffPermission.start(masterOffPermission.footer); await flush();
  masterOffPermission.context.codexPlusBackendSettings.enhancementsEnabled = false;
  masterOffGrant({ getTracks: () => [{ stop: () => masterOffPermission.events.push("disabled-track-stop") }] });
  await masterOffPending;
  assert.ok(masterOffPermission.events.includes("disabled-track-stop"), "closing the master switch releases a late microphone grant");
  assert.equal(masterOffPermission.state.phase, "idle");
  assert.equal(masterOffPermission.requests.length, 0);

  const masterOffResult = fixture();
  await masterOffResult.start(masterOffResult.footer); masterOffResult.state.startedAt -= 1000;
  masterOffResult.stop("insert"); await flush();
  masterOffResult.context.codexPlusBackendSettings.enhancementsEnabled = false;
  masterOffResult.resolve("disabled result"); await flush();
  assert.equal(masterOffResult.editor.value, "existing draft", "closing the master switch suppresses an in-flight transcript insertion");
  assert.equal(masterOffResult.state.phase, "idle");

  const masterOffSend = fixture();
  await masterOffSend.start(masterOffSend.footer); masterOffSend.state.startedAt -= 1000;
  masterOffSend.stop("send"); await flush(); masterOffSend.resolve("draft"); await flush();
  masterOffSend.context.codexPlusBackendSettings.enhancementsEnabled = false;
  masterOffSend.frames.shift()(); masterOffSend.frames.shift()(); await flush();
  assert.equal(masterOffSend.events.includes("send"), false, "closing the master switch cancels a scheduled send");

  const denied = fixture();
  denied.setMedia(() => Promise.reject(Object.assign(Error("denied"), { name: "NotAllowedError" })));
  await denied.start(denied.footer);
  assert.equal(denied.state.phase, "error");
  assert.match(denied.state.error, /麦克风权限/);
  assert.equal(denied.requests.length, 0);
  denied.cancel();

  const short = fixture();
  await short.start(short.footer); short.stop("insert"); await flush();
  assert.equal(short.requests.length, 0, "accidental very short recordings do not incur ASR requests");
  assert.equal(short.state.phase, "idle");

  const large = fixture();
  await large.start(large.footer); large.state.startedAt -= 1000;
  large.state.recorder.ondataavailable({ data: new Blob([new Uint8Array(25 * 1024 * 1024 + 1)]) });
  await flush();
  assert.equal(large.state.phase, "error");
  assert.match(large.state.error, /25 MiB/);
  assert.equal(large.requests.length, 0, "oversized recordings are stopped without an upstream upload");
  large.cancel();

  const clock = fixture();
  clock.state.phase = "recording"; clock.state.target = { id: "clock", href: clock.context.window.location.href };
  clock.state.startedAt = Date.now() - 5000; clock.editor.dataset.codexPlusDictationTarget = "clock";
  const label = { textContent: "" };
  const controls = { dataset: { dictationTarget: "clock", renderKey: "recording:insert::" }, querySelector: () => label, replaceChildren: () => { throw Error("timer must not recreate controls"); } };
  clock.context.document.querySelectorAll = selector => selector.includes("data-dictation-controls") ? [controls] : selector.includes("dictation-target") ? [clock.editor] : [];
  vm.runInContext(source.slice(source.indexOf("  function renderCodexPlusDictation()"), source.indexOf("  function createCodexPlusDictationControls()")), clock.context);
  clock.context.renderCodexPlusDictation();
  assert.match(label.textContent, /录音 0:0[56]/, "recording timer updates the label while preserving button identity and focus");
  console.log("dictation recording, insertion, cancellation, retry and send contracts passed");
})().catch(error => { console.error(error); process.exitCode = 1; });
