import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { usesMacOverlayTitlebar } from "./mac-titlebar.ts";

function fakeWindow({ tauri, platform, search = "" }: { tauri: boolean; platform: string; search?: string }) {
  return {
    ...(tauri ? { __TAURI_INTERNALS__: {} } : {}),
    location: { search },
    navigator: { platform, userAgent: "" },
  } as unknown as Window;
}

test("只有 Tauri 的 macOS 窗口去掉系统标题栏；Windows 和浏览器预览不变，预览可用 ?macTitlebar=1 模拟", () => {
  assert.equal(usesMacOverlayTitlebar(fakeWindow({ tauri: true, platform: "MacIntel" })), true);
  assert.equal(usesMacOverlayTitlebar(fakeWindow({ tauri: true, platform: "Win32" })), false);
  assert.equal(usesMacOverlayTitlebar(fakeWindow({ tauri: false, platform: "MacIntel" })), false);
  assert.equal(usesMacOverlayTitlebar(fakeWindow({ tauri: false, platform: "Win32", search: "?macTitlebar=1" })), true);
});

test("macOS 主窗口用 Overlay 标题栏，页面标题行和侧栏顶部是拖动区域", () => {
  const rust = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
  assert.match(rust, /#\[cfg\(target_os = "macos"\)\]\s*\{\s*main_window_builder = main_window_builder\s*\.title_bar_style\(tauri::TitleBarStyle::Overlay\)\s*\.hidden_title\(true\)/);
  const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8");
  assert.match(app, /<header className="cx-page-head" data-tauri-drag-region="deep">/);
  assert.equal((app.match(/className="cx-titlebar-drag" data-tauri-drag-region \/>/g) ?? []).length, 2);
  // 托盘面板、悬浮预览本来没有标题栏，不加 mac-titlebar。
  const main = readFileSync(new URL("./main.tsx", import.meta.url), "utf8");
  assert.match(main, /if \(window\.location\.hash !== "#tray-panel" && window\.location\.hash !== "#cu-pip"\) applyMacTitlebarClass\(\);/);
  const css = readFileSync(new URL("./cursor-ui.css", import.meta.url), "utf8");
  assert.match(css, /:root\.mac-titlebar \.shell \.screen > header\.cx-page-head,[\s\S]*?position: sticky;/);
});
