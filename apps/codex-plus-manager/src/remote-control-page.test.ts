import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「远程控制」页：前端只调一条 `remote_control_request`，按 action 分发；
 * 钉死路由登记、命令名与后端注册一致、文案都有英文。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");

test("远程控制页挂在工具分组里，并在路由表中渲染", async () => {
  const app = await read("./App.tsx");
  assert.match(app, /\| "remoteControl" \|/, "Route 类型里要有 remoteControl");
  assert.match(app, /\{ id: "remoteControl", get label\(\) \{ return t\("远程控制"\); \}, icon: Smartphone, badge: "实验" \}/);
  assert.match(app, /const toolNavigationRoutes: Route\[\] = \[[^\]]*"remoteControl"[^\]]*\];/);
  assert.match(app, /const globalNavigationRoutes: Route\[\] = \[[^\]]*"remoteControl"[^\]]*\];/);
  assert.match(app, /\{route === "remoteControl" \? <RemoteControlPage \/> : null\}/);
  assert.match(app, /remoteControl: t\("/, "页面要有副标题");
});

test("前端调用的命令名与 Tauri 注册一致，动作与后端分发一致", async () => {
  const page = await read("./RemoteControlPage.tsx");
  const commands = await read("../src-tauri/src/remote_control_commands.rs");
  const lib = await read("../src-tauri/src/lib.rs");
  assert.match(page, /invoke<RemoteControlStatus>\("remote_control_request", \{ action \}\)/);
  assert.match(lib, /remote_control_commands::remote_control_request,/);
  assert.match(lib, /remote_control_commands::start_from_saved_state_in_background\(\);/, "管理器启动时按保存的开关拉起");
  assert.match(lib, /codex_plus_core::remote_control::stop\(\);/, "管理器退出时停掉服务");
  for (const action of ["status", "enable", "disable", "resetToken"]) {
    assert.ok(page.includes(`"${action}"`), `前端缺少动作 ${action}`);
    assert.ok(commands.includes(`"${action}" =>`), `后端缺少动作 ${action}`);
  }
});

test("远程控制页的文案都有英文", async () => {
  const page = await read("./RemoteControlPage.tsx");
  const en = await read("./i18n-en.ts");
  const keys = [...page.matchAll(/\bt\("([^"]+)"\)/g)].map((match) => match[1]);
  assert.ok(keys.length > 10);
  const missing = [...new Set(keys)].filter((key) => !en.includes(`${JSON.stringify(key)}:`));
  assert.deepEqual(missing, []);
});

test("访问链接显示时把令牌打码", async () => {
  const page = await read("./RemoteControlPage.tsx");
  assert.match(page, /url\.replace\(\/#token=\.\*\/, "#token=…"\)/);
});
