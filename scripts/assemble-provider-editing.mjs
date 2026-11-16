/**
 * 把管理器的供应商编辑纯函数（apps/codex-plus-manager/src/relay-profile-editing.ts）打包成注入分片
 * assets/inject/renderer-inject/42-provider-editing.generated.js，供 Codex 页面里的「供应商」页使用。
 *
 * 管理器与注入页共用同一份函数，字段 ↔ config.toml / auth.json 的联动不会两边各写一套。
 * `@/i18n` 换成中文直出（注入页只有中文界面），其余依赖原样打进来。
 *
 * 用法：
 *   node scripts/assemble-provider-editing.mjs           # 写回分片
 *   node scripts/assemble-provider-editing.mjs --check   # 只校验，不写（测试 / CI 用）
 *
 * 写回分片后还要跑 node scripts/assemble-renderer-inject.mjs 重新组装单体产物。
 */
import { createRequire } from "node:module";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const managerDir = path.join(root, "apps/codex-plus-manager");
const require = createRequire(path.join(managerDir, "package.json"));
// 锁定版本的 esbuild（见 apps/codex-plus-manager/package.json），产物可复现。
const { build } = require("esbuild");

// 直连供应商的编辑函数挂在顶层；网关供应商池的纯函数（gateway-model.ts）挂在 `.gateway` 下。
const entryContents = [
  "export * from \"./relay-profile-editing.ts\";",
  "export * as gateway from \"./gateway-model.ts\";",
  "export * as gatewaySafety from \"./gateway-agent-safety.ts\";",
].join("\n");
const shim = path.join(root, "scripts/provider-editing-i18n-shim.mjs");
const output = path.join(root, "assets/inject/renderer-inject/42-provider-editing.generated.js");

const result = await build({
  stdin: { contents: entryContents, resolveDir: path.join(managerDir, "src"), sourcefile: "provider-editing-entry.ts", loader: "ts" },
  // 产物里的来源注释是相对工作目录的路径：固定到仓库根，从哪个目录运行结果都一样。
  absWorkingDir: root,
  bundle: true,
  format: "iife",
  globalName: "codexPlusProviderEditing",
  platform: "browser",
  target: "es2022",
  charset: "utf8",
  legalComments: "none",
  write: false,
  plugins: [{
    name: "codex-plus-i18n-shim",
    setup(pluginBuild) {
      pluginBuild.onResolve({ filter: /^@\/i18n$|(^|\/)i18n(\.ts)?$/ }, () => ({ path: shim }));
      pluginBuild.onResolve({ filter: /^@\// }, (args) => ({
        path: path.join(managerDir, "src", `${args.path.slice(2)}.ts`),
      }));
    },
  }],
});

// esbuild 的 iife 产物是顶层 `var codexPlusProviderEditing = (() => {…})();`，
// 拼进主 IIFE 后就是一个局部变量，不会挂到 window 上。
const body = result.outputFiles[0].text.replace(/^"use strict";\n/, "");
if (!body.trimEnd().endsWith("})();")) throw new Error("esbuild iife 产物的收尾不是 })();");
// 行首的 `})();` 会被当成主 IIFE 的收尾（renderer-inject.test.ts 按它定位），这里加注释区分开。
const tagged = body.trimEnd().replace(/\}\)\(\);$/, "})(); // 结束 codexPlusProviderEditing");
const fragment = [
  "  // 生成文件，勿手改：node scripts/assemble-provider-editing.mjs",
  "  // 来源：apps/codex-plus-manager/src/relay-profile-editing.ts（与管理器共用的供应商编辑纯函数）",
  // 不缩进：产物里有多行模板字符串（TOML 片段），加缩进会改变生成的文件内容。
  tagged,
  "",
].join("\n");

if (process.argv.includes("--check")) {
  const current = await readFile(output, "utf8").catch(() => "");
  // Windows runner 默认 core.autocrlf=true，*.js / *.mjs 没在 .gitattributes 里钉 LF，
  // 检出的分片和 shim 都是 CRLF，esbuild 产物却是 LF；只比内容，不比换行符。
  const lf = (text) => text.replace(/\r\n/g, "\n");
  if (lf(current) !== lf(fragment)) {
    throw new Error("42-provider-editing.generated.js 与 relay-profile-editing.ts 不一致，请运行 node scripts/assemble-provider-editing.mjs");
  }
  console.log("provider editing fragment up to date");
} else {
  await writeFile(output, fragment, "utf8");
  console.log(`wrote ${path.relative(root, output)} (${fragment.split("\n").length} lines)`);
}
