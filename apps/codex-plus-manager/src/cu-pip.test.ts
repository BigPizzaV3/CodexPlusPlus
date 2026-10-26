import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// Windows 检出（core.autocrlf）下源码是 CRLF，先归一化换行。
const read = (path: string) => readFileSync(new URL(path, import.meta.url), "utf8").replace(/\r\n/g, "\n");

test("悬浮预览窗口用同一个前端，按 #cu-pip 渲染预览组件", () => {
  const main = read("./main.tsx");
  assert.match(main, /window\.location\.hash === "#cu-pip" \? <CuPipWindow \/> : <App \/>/);
  const rust = read("../src-tauri/src/cu_pip.rs");
  assert.match(rust, /WebviewUrl::App\("\/index\.html#cu-pip"\.into\(\)\)/);
});

test("预览窗口在权限配置里，否则它调不了命令、收不到帧", () => {
  const capability = JSON.parse(read("../src-tauri/capabilities/default.json")) as { windows: string[] };
  assert.ok(capability.windows.includes("cu-pip"));
});

test("预览窗口不抢焦点：创建时不聚焦，显示走不激活的置顶", () => {
  const rust = read("../src-tauri/src/cu_pip.rs");
  assert.match(rust, /\.focused\(false\)/);
  assert.match(rust, /show_floating_without_activation/);
  // 用 window.show() 会激活 Codex++，用户正在用的窗口失焦。
  assert.doesNotMatch(rust.slice(rust.indexOf("fn order_front"), rust.indexOf("#[cfg(not(target_os = \"macos\"))]", rust.indexOf("fn order_front"))), /window\.show\(\)/);
});

test("预览组件加载后主动取目标，并能关闭与切到应用", () => {
  const view = read("./CuPipWindow.tsx");
  for (const command of ["cu_pip_target", "cu_pip_dismiss", "cu_pip_activate"]) {
    assert.ok(view.includes(`"${command}"`), `缺少 ${command}`);
  }
  const lib = read("../src-tauri/src/lib.rs");
  for (const command of ["cu_pip::cu_pip_target", "cu_pip::cu_pip_dismiss", "cu_pip::cu_pip_activate", "cu_pip::start("]) {
    assert.ok(lib.includes(command), `lib.rs 缺少 ${command}`);
  }
});

test("预览窗口标明 Codex++ Computer Use", () => {
  assert.ok(read("./CuPipWindow.tsx").includes(">Codex++ Computer Use<"));
  assert.match(read("../src-tauri/src/cu_pip.rs"), /\.title\("Codex\+\+ Computer Use"\)/);
});

test("预览窗口能拖动：标题栏是拖动区，权限里有 start-dragging，非前台时第一下点击也生效", () => {
  assert.match(read("./CuPipWindow.tsx"), /className="cu-pip-head" data-tauri-drag-region/);
  const capability = JSON.parse(read("../src-tauri/capabilities/default.json")) as { permissions: string[] };
  assert.ok(capability.permissions.includes("core:window:allow-start-dragging"));
  assert.match(read("../src-tauri/src/cu_pip.rs"), /\.accept_first_mouse\(true\)/);
});
