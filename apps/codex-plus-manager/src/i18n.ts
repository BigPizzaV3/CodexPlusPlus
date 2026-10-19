// Lightweight source-text-keyed i18n for the Codex++ manager UI.
//
// The app is authored in Chinese. Every user-facing Chinese literal is wrapped
// with `t("中文")` (plain strings) or `tf("前缀 {0}", [expr])` (interpolated
// strings) by tools/i18n-codemod.mjs. When the active language is English we
// look the source text up in the English dictionary; otherwise we return the
// original Chinese, so Chinese stays the zero-overhead default.
//
// 语言通过外部 store 通知 React 原地更新；模块级标签使用 getter，保留页面和草稿。

import { EN_BACKEND, EN_BACKEND_PATTERNS, EN_PLAIN, EN_TEMPLATE } from "@/i18n-en";

export type Language = "zh" | "en";

const STORAGE_KEY = "codex-plus-lang";

function resolveInitialLanguage(): Language {
  try {
    return window.localStorage.getItem(STORAGE_KEY) === "en" ? "en" : "zh";
  } catch {
    return "zh";
  }
}

let LANG: Language = resolveInitialLanguage();
const listeners = new Set<() => void>();

export function subscribeLanguage(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function getLanguage(): Language {
  return LANG;
}

/** Translate a plain Chinese literal. Falls back to the source text. */
export function t(zh: string): string {
  if (LANG !== "en") return zh;
  const plain = EN_PLAIN[zh] ?? EN_BACKEND[zh];
  if (plain) return plain;
  for (const [re, replacement] of EN_BACKEND_PATTERNS) {
    if (re.test(zh)) return zh.replace(re, replacement);
  }
  return zh;
}

/**
 * Translate an interpolated literal. `key` carries `{0}`,`{1}`… placeholders in
 * the original (Chinese) order; `args` are the runtime values for each. In
 * Chinese we substitute into the key itself; in English we substitute into the
 * looked-up template (also falling back to the key).
 */
export function tf(key: string, args: Array<string | number>): string {
  const template = LANG === "en" ? EN_TEMPLATE[key] ?? key : key;
  return template.replace(/\{(\d+)\}/g, (match, index) => {
    const value = args[Number(index)];
    return value === undefined || value === null ? match : String(value);
  });
}

/** 原地切换，绝不把包含凭据的表单草稿写入浏览器存储。 */
export function setLanguage(language: Language): void {
  if (LANG === language) return;
  try {
    window.localStorage.setItem(STORAGE_KEY, language);
  } catch {
    // 存储不可用时仍允许本窗口切换语言。
  }
  LANG = language;
  for (const listener of listeners) listener();
}

/** Flip between Chinese and English. */
export function toggleLanguage(): void {
  setLanguage(LANG === "en" ? "zh" : "en");
}
