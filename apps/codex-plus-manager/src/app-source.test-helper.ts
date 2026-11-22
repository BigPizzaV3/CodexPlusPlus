import { readFileSync } from "node:fs";
import ts from "typescript";

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
  "SessionRepairPanel.tsx",
  "SettingsScreens.tsx",
  "HomeScreen.tsx",
  "OverviewScreen.tsx",
  "AppDialogs.tsx",
  "RelayScreen.tsx",
  "AppSidebar.tsx",
] as const;

export function readAppSource(): string {
  return APP_SOURCE_FILES.map((name) => readFileSync(new URL(`./${name}`, import.meta.url), "utf8")).join("\n");
}

/**
 * 按名字取主界面某个顶层声明（函数、变量、类型）的源码，不管它拆到了哪个文件；找不到时返回空串。
 * 函数 / 类型声明优先：按需加载的页面在 App.tsx 里还有一个同名的 `const X = lazyNamed(...)`，要取的是页面本身。
 */
export function appDeclarationSource(name: string): string {
  let variable = "";
  for (const file of APP_SOURCE_FILES) {
    const text = readFileSync(new URL(`./${file}`, import.meta.url), "utf8");
    const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX);
    for (const statement of source.statements) {
      if (ts.isVariableStatement(statement)) {
        if (!variable && statement.declarationList.declarations.some((declaration) => declaration.name.getText(source) === name)) variable = statement.getText(source);
      } else if ("name" in statement && statement.name && ts.isIdentifier(statement.name as ts.Node) && (statement.name as ts.Identifier).text === name) {
        return statement.getText(source);
      }
    }
  }
  return variable;
}
