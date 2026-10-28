import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import ts from "typescript";
import { UpdateNotifications } from "./update-notification.ts";

const source = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
const file = ts.createSourceFile("App.tsx", source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);

function handler(name: string, dependencies: Record<string, unknown>): (...args: any[]) => Promise<void> {
  let expression = "";
  const visit = (node: ts.Node) => {
    if (ts.isVariableDeclaration(node) && node.name.getText(file) === name && node.initializer) expression = node.initializer.getText(file);
    ts.forEachChild(node, visit);
  };
  visit(file);
  assert.ok(expression, `missing ${name}`);
  const javascript = ts.transpileModule(`const handler = ${expression};`, { compilerOptions: { target: ts.ScriptTarget.ES2022 } }).outputText;
  return new Function(...Object.keys(dependencies), `${javascript}; return handler;`)(...Object.values(dependencies));
}

const release = {
  status: "ok", message: "发现可用更新。", currentVersion: "1.10.0", latestVersion: "v1.11.0",
  releaseSummary: "### 新增\n- **更新弹窗**", updateAvailable: true, assetName: "update.dmg", assetUrl: "https://example.test/update.dmg",
};

test("actual update check keeps About data, honors mute, and allows manual prompts", async () => {
  const notices: unknown[] = [];
  const updates: unknown[] = [];
  const prompts: unknown[] = [];
  const notifications = new UpdateNotifications(() => ({ getItem: () => null, setItem: () => {} }));
  notifications.setPreferences({ autoCheck: true, skippedVersion: "" });
  const pending = { current: null };
  let response: any = release;
  const check = handler("checkUpdate", {
    updateCheckPending: pending, updateInstallPending: { current: false },
    call: async () => { if (response instanceof Error) throw response; return response; },
    setUpdate: (value: unknown) => updates.push(value), setUpdatePrompt: (value: unknown) => prompts.push(value),
    updateNotifications: { current: notifications }, showNotice: (...args: unknown[]) => notices.push(args),
    t: (value: string) => value, stringifyError: String, ensureUpdatePreferences: async () => {},
  });
  await check(true);
  assert.deepEqual(prompts, [release]);
  notifications.muteToday();
  await check(true);
  assert.equal(prompts.length, 1);
  assert.equal(updates.length, 2, "About remains usable while automatic reminders are muted");
  await check(false);
  assert.equal(prompts.length, 2);
  response = { ...release, status: "failed" };
  await check(true);
  assert.equal(updates.length, 3, "background failures preserve the previous usable release");
  assert.equal(notices.length, 0);
  response = new Error("offline");
  await check(true);
  assert.equal(notices.length, 0);
  await check(false);
  assert.equal(notices.length, 1);
  assert.equal(pending.current, null);
});

test("a manual check reuses an in-flight background request and overrides today's mute", async () => {
  const notifications = new UpdateNotifications(() => ({ getItem: () => null, setItem: () => {} }));
  notifications.setPreferences({ autoCheck: true, skippedVersion: "" });
  notifications.muteToday();
  const prompts: unknown[] = [];
  let requests = 0;
  let finish!: (value: unknown) => void;
  const response = new Promise((resolve) => { finish = resolve; });
  const check = handler("checkUpdate", {
    updateCheckPending: { current: null }, updateInstallPending: { current: false },
    call: async () => { requests++; return response; }, setUpdate: () => {},
    setUpdatePrompt: (value: unknown) => prompts.push(value), updateNotifications: { current: notifications },
    showNotice: () => {}, t: (value: string) => value, stringifyError: String, ensureUpdatePreferences: async () => {},
  });
  const background = check(true);
  await check(false);
  finish(release);
  await background;
  assert.equal(requests, 1);
  assert.deepEqual(prompts, [release]);
});

test("background checks load saved preferences first, honor auto-check off and skipped versions, manual checks still prompt", async () => {
  const notifications = new UpdateNotifications(() => ({ getItem: () => null, setItem: () => {} }));
  const prompts: unknown[] = [];
  const notices: unknown[] = [];
  let requests = 0;
  let loads = 0;
  let saved = { autoCheck: false, skippedVersion: "" };
  let response: any = release;
  const check = handler("checkUpdate", {
    updateCheckPending: { current: null }, updateInstallPending: { current: false },
    call: async () => { requests++; return response; }, setUpdate: () => {},
    setUpdatePrompt: (value: unknown) => prompts.push(value), updateNotifications: { current: notifications },
    showNotice: (...args: unknown[]) => notices.push(args), t: (value: string) => value, stringifyError: String,
    // 模拟启动时设置尚未加载：后台检查先读一次已保存的偏好。
    ensureUpdatePreferences: async () => { loads++; notifications.setPreferences(saved); },
  });
  await check(true);
  assert.equal(loads, 1);
  assert.equal(requests, 0, "auto-check off skips the network request entirely");
  await check(true);
  assert.equal(loads, 1, "preferences are read once and then kept in sync by settings updates");
  await check(false);
  assert.equal(requests, 1, "manual checks ignore the auto-check switch");
  assert.deepEqual(prompts, [release]);

  saved = { autoCheck: true, skippedVersion: "v1.11.0" };
  notifications.setPreferences(saved);
  await check(true);
  assert.equal(requests, 2);
  assert.equal(prompts.length, 1, "a skipped version is not prompted again in the background");
  assert.equal(notices.length, 0);
  await check(false);
  assert.equal(prompts.length, 2, "manual checks still show the skipped version");
  response = { ...release, latestVersion: "v1.12.0" };
  await check(true);
  assert.equal(prompts.length, 3, "a newer release than the skipped one prompts again");
});

test("installing the prompted release prevents duplicate downloads and preserves retry data on failure", async () => {
  const states: any[] = [];
  const updates: any[] = [];
  const calls: any[] = [];
  const pending = { current: false };
  let callback: (event: any) => void = () => {};
  let finish!: (result: unknown) => void;
  const result = new Promise((resolve) => { finish = resolve; });
  let unlistened = 0;
  const install = handler("performUpdate", {
    update: { ...release, latestVersion: "v1.12.0" }, updateInstallPending: pending,
    setUpdateInstallProgress: (state: unknown) => states.push(state), setUpdate: (state: unknown) => updates.push(state),
    t: (value: string) => value, stringifyError: String, showNotice: () => {},
    window: { setTimeout: () => 1, clearTimeout: () => {} },
    listen: async (_event: string, listener: typeof callback) => { callback = listener; return () => unlistened++; },
    call: async (command: string, args: unknown) => { calls.push({ command, args }); return result; },
    run: async (task: () => unknown) => task(),
  });
  const task = install(release);
  await Promise.resolve();
  await install(release);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].args.release.version, release.latestVersion);
  callback({ payload: { stage: "downloading", percent: 68 } });
  assert.equal(states.at(-1).percent, 68);
  finish({ status: "failed", message: "network failure", progress: 0 });
  await task;
  assert.equal(pending.current, false);
  assert.equal(unlistened, 1);
  assert.equal(states.at(-1).failed, true);
  assert.equal(updates[0].assetUrl, release.assetUrl, "failed installs retain their original asset for retry from About");
  await install(release);
  assert.equal(calls.length, 2);
});

test("a progress listener failure clears the installation lock and shows the error", async () => {
  const pending = { current: false };
  const states: any[] = [];
  const install = handler("performUpdate", {
    update: release, updateInstallPending: pending, t: (value: string) => value, stringifyError: String,
    setUpdateInstallProgress: (state: unknown) => states.push(state), showNotice: () => {},
    window: { setTimeout: () => 1, clearTimeout: () => {} },
    listen: async () => { throw new Error("listener unavailable"); },
  });
  await install();
  assert.equal(pending.current, false);
  assert.equal(states.at(-1).failed, true);
  assert.match(states.at(-1).message, /listener unavailable/);
});
