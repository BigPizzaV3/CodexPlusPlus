/**
 * macOS 主窗口去掉系统标题栏（lib.rs 的 build_main_window：TitleBarStyle::Overlay + hidden_title），
 * 红黄绿按钮浮在侧栏左上角，页面标题和右侧按钮占据原来标题栏那一行（参照 Cursor / magpie）。
 *
 * 只在 Tauri 的 macOS 主窗口里给 <html> 加 `mac-titlebar`，样式据此留出按钮位置、把标题行做成可拖动区域。
 * Windows / Linux 仍用系统标题栏，浏览器预览可用 `?macTitlebar=1` 模拟。
 */
export const MAC_TITLEBAR_CLASS = "mac-titlebar";

export function usesMacOverlayTitlebar(win: Window = window): boolean {
  try {
    if (new URLSearchParams(win.location.search).get("macTitlebar") === "1") return true;
  } catch {
    // location 读不到时按非 macOS 处理。
  }
  const isTauri = "__TAURI_INTERNALS__" in win;
  const platform = `${win.navigator.platform ?? ""} ${win.navigator.userAgent ?? ""}`;
  return isTauri && /Mac/i.test(platform);
}

export function applyMacTitlebarClass(doc: Document = document, win: Window = window): void {
  if (usesMacOverlayTitlebar(win)) doc.documentElement.classList.add(MAC_TITLEBAR_CLASS);
}
