// 界面主题（#2825）：除了固定深色 / 浅色，还能跟随系统外观。
// 主窗口和托盘面板共用 localStorage 里的同一个键；老用户存的 "dark" / "light" 原样沿用，
// 没存过（新装）的默认跟随系统。

export type ThemePreference = "system" | "dark" | "light";
export type ResolvedTheme = "dark" | "light";

export const THEME_STORAGE_KEY = "codex-plus-theme";
const SYSTEM_DARK_QUERY = "(prefers-color-scheme: dark)";

export function parseThemePreference(raw: string | null | undefined): ThemePreference {
  return raw === "dark" || raw === "light" ? raw : "system";
}

export function resolveTheme(preference: ThemePreference, systemPrefersDark: boolean): ResolvedTheme {
  if (preference === "system") return systemPrefersDark ? "dark" : "light";
  return preference;
}

/** 侧栏 / 命令面板的「切换主题」：按当前看到的颜色切到另一种，并改为固定模式。 */
export function toggledThemePreference(current: ResolvedTheme): ThemePreference {
  return current === "dark" ? "light" : "dark";
}

export function loadThemePreference(): ThemePreference {
  try {
    return parseThemePreference(window.localStorage.getItem(THEME_STORAGE_KEY));
  } catch {
    return "system";
  }
}

export function storeThemePreference(preference: ThemePreference) {
  try {
    window.localStorage.setItem(THEME_STORAGE_KEY, preference);
  } catch {
    // 存不下就只在本次生效。
  }
}

export function systemPrefersDark(): boolean {
  try {
    return window.matchMedia?.(SYSTEM_DARK_QUERY).matches ?? true;
  } catch {
    return true;
  }
}

/** 订阅系统外观变化；返回取消订阅。 */
export function watchSystemTheme(onChange: (prefersDark: boolean) => void): () => void {
  let query: MediaQueryList | undefined;
  try {
    query = window.matchMedia?.(SYSTEM_DARK_QUERY);
  } catch {
    query = undefined;
  }
  if (!query) return () => undefined;
  const listener = (event: MediaQueryListEvent) => onChange(event.matches);
  query.addEventListener("change", listener);
  return () => query?.removeEventListener("change", listener);
}

export function applyResolvedTheme(theme: ResolvedTheme) {
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.classList.toggle("light", theme === "light");
}
