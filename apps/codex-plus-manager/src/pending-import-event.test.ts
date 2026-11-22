import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { it } from "node:test";

const read = async (path: string) => (await readFile(new URL(path, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

it("网页导入 / 一键换肤待确认项靠事件和窗口唤起读取，不再定时轮询磁盘", async () => {
  const app = await read("./App.tsx");
  const lib = await read("../src-tauri/src/lib.rs");
  // 前后端事件名一致。
  const eventName = /const PENDING_IMPORT_CHANGED_EVENT: &str = "([^"]+)";/.exec(lib)?.[1];
  assert.ok(eventName, "后端要定义待确认项事件");
  assert.ok(app.includes(`const PENDING_IMPORT_CHANGED_EVENT = "${eventName}";`));
  // macOS「打开 URL」存好后发事件。
  assert.match(lib, /handle_provider_import_url\(url\.as_str\(\)\) \{\n\s*show_main_window\(app_handle\);\n\s*\/\/[^\n]*\n\s*let _ = app_handle\.emit\(PENDING_IMPORT_CHANGED_EVENT, \(\)\);/);
  assert.match(app, /listen\(PENDING_IMPORT_CHANGED_EVENT,/);
  // 窗口唤起 / 获得焦点（后端发导航事件）时也读一次。
  const navigationListener = app.slice(app.indexOf("void listen(MANAGER_NAVIGATION_EVENT"));
  assert.match(navigationListener.slice(0, 400), /refreshPendingProviderImport\(true\);\n\s*void refreshPendingDreamSkinCommunity\(\);/);
  // 旧的 1.2 秒轮询已删除。
  assert.doesNotMatch(app, /\}, 1200\);/);
});
