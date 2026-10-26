import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// Windows 检出（core.autocrlf）下源码是 CRLF，先归一化换行。
const app = readFileSync(new URL("./App.tsx", import.meta.url), "utf8").replace(/\r\n/g, "\n");

/** RelayScreen 里渲染 ProviderCatalogScreen 的那段 JSX。 */
function catalogUsage() {
  const start = app.indexOf("<ProviderCatalogScreen");
  assert.ok(start > 0, "找不到 ProviderCatalogScreen 的使用处");
  return app.slice(start, app.indexOf("/>", app.indexOf("onSelect=", start)) + 2);
}

test("从目录页选模板进编辑页，返回时回到目录页而不是供应商列表", () => {
  const usage = catalogUsage();
  const onSelect = usage.slice(usage.indexOf("onSelect="));
  assert.match(onSelect, /setNewProfileDraft\(profile\)/);
  // 选模板时关掉目录页，就是修复前的形态：编辑页返回只能回到最外层。
  assert.doesNotMatch(onSelect, /setProviderCatalogOpen\(false\)/);
  // 编辑页必须先于目录页渲染，否则目录页开着时选了模板也看不到编辑页。
  assert.ok(app.indexOf("if (detailProfile) {") < app.indexOf("if (providerCatalogOpen) {"));
});

test("保存新供应商后回到列表，目录页一并关闭", () => {
  const detail = app.slice(app.indexOf("<RelayProfileDetail"), app.indexOf("if (providerCatalogOpen) {"));
  const onSaved = detail.slice(detail.indexOf("onSaved="));
  assert.match(onSaved, /setProviderCatalogOpen\(false\)/);
});

test("目录页搜索词由外层保存，返回目录页时不丢", () => {
  const usage = catalogUsage();
  assert.match(usage, /query=\{providerCatalogQuery\}/);
  assert.match(usage, /onQueryChange=\{setProviderCatalogQuery\}/);
});
