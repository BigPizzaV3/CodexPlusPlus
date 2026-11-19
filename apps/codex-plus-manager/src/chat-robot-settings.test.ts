import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";

const source = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8");
const remote = source("../../../crates/codex-plus-core/assets/remote-control/index.html");
const form = source("./ChatRobotSettings.tsx");

test("机器人配置只在管理器主窗口使用，提示和按钮都有英文", () => {
  const en = source("./i18n-en.ts");
  for (const [, label] of form.matchAll(/\bt\("([^"]+)"\)/g)) assert.ok(en.includes(`"${label}"`), label);
  const commands = source("../src-tauri/src/remote_control_commands.rs");
  assert.match(commands, /window\.label\(\) != "main"/);
  assert.match(source("./RemoteControlPage.tsx"), /<ChatRobotSettings /);
  assert.match(form, /invoke<RobotStatus>\("chat_robot_request"/);
  assert.doesNotMatch(source("../../../assets/inject/renderer-inject/91-extension-api.js"), /chat_robot_request/);
});

test("通知链接只在设备认证后打开相应会话，聊天和桌面布局都可用", async () => {
  const sourceBoot = remote.slice(remote.indexOf("  async function boot()"), remote.indexOf("  if (pairCode && !token)"));
  const thread = "01992000-0000-7000-8000-000000000001";
  for (const layout of ["chat", "codex"]) {
    for (const paired of [true, false]) {
      const calls: string[] = [];
      const state = { me: null as null | { role: string } };
      const api = async (method: string, path: string) => {
        calls.push(`${method} ${path}`);
        if (!paired) throw new Error("配对请求");
        return { role: "view" };
      };
      const boot = new Function("state", "api", "$", "m", "applyRole", "loadThreads", "document", "linkedThread", "selectThread",
        `${sourceBoot}\nreturn boot;`)(state, api, () => ({ hidden: false }), {
        onOffline: () => calls.push("offline"), startJiajia: () => {}, openLinkedThread: (id: string) => calls.push(`chat ${id}`),
      }, () => {}, () => {}, { documentElement: { dataset: { style: layout } } }, thread,
      (id: string) => calls.push(`codex ${id}`));
      await boot();
      assert.deepEqual(calls, ["GET /me", paired ? `${layout} ${thread}` : "offline"]);
    }
  }
});
