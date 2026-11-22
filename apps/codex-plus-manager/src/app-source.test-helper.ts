import { readFileSync } from "node:fs";

/**
 * 管理器主界面原来全在 App.tsx 里，现在按页面拆成了多个文件（App 只管外壳和路由分发）。
 * 按源码断言主界面的测试读这些文件拼起来的全文，不用关心某段代码具体落在哪个文件。
 * 新拆出页面文件时登记到这里。
 */
export const APP_SOURCE_FILES = [
  "App.tsx",
  "app-types.ts",
  "app-shared.tsx",
  "routes.ts",
  "RelayEditor.tsx",
  "RequestLogScreen.tsx",
  "MultiLaunchScreen.tsx",
  "ClaudeDesktopPage.tsx",
  "WeixinPage.tsx",
  "ExtensionsPage.tsx",
] as const;

export function readAppSource(): string {
  return APP_SOURCE_FILES.map((name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8")).join("\n");
}
