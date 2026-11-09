import assert from "node:assert/strict";
import { it } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * #2902：使用模式在页面本地存储之外再存进设置文件，本地存储丢了（例如管理员身份启动时 WebView 数据目录不同）
 * 时启动用它补回，问卷不再每次都弹。这里钉住三处接线，行为由 agent-focus-store 的单测覆盖。
 */
// Windows 检出是 CRLF：先统一成 LF，下面的正则才能按 \n 匹配。
const read = async (path: string) => (await readFile(new URL(path, import.meta.url), "utf8")).replace(/\r\n/g, "\n");

it("选择模式时同时备份进设置文件，启动时本地缺失就从设置文件补回", async () => {
  const store = await read("./agent-focus-store.ts");
  assert.match(store, /invoke\("set_manager_agent_focus", \{ mode \}\)/);
  assert.match(store, /export function saveAgentFocus\(mode: FocusMode\) \{\n\s*writeLocal\(mode\);\n\s*backupAgentFocus\(mode\);/);
  // 只补不覆盖：本地已有有效选择时以本地为准。
  assert.match(store, /if \(typeof window === "undefined" \|\| !shouldAskAgentFocus\(readStored\(\)\)\) return false;/);
  const app = await read("./App.tsx");
  assert.match(app, /if \(restoreAgentFocusFromSettings\(savedAgentFocus\)\) \{\n\s*setAgentFocusOpen\(false\);/);
  assert.match(app, /backupAgentFocus\(agentFocus\);/);
  const lib = await read("../src-tauri/src/lib.rs");
  assert.match(lib, /commands::set_manager_agent_focus,/);
});

it("启动时先读设置文件再决定弹不弹问卷，「稍后设置」也记住（#2936）", async () => {
  const app = await read("./App.tsx");
  // 初始不弹，交给 resolveAgentFocusPrompt 读完设置文件再定；判定期间入门清单也不出。
  assert.match(app, /const \[agentFocusOpen, setAgentFocusOpen\] = useState\(false\);/);
  assert.match(app, /const \[agentFocusChecking, setAgentFocusChecking\] = useState\(needAgentFocusPrompt\);/);
  assert.match(app, /void resolveAgentFocusPrompt\(async \(\) => \{\n\s*\/\/[^\n]*\n\s*const result = await invoke<SettingsResult>\("load_settings"\);\n\s*return isSuccessStatus\(result\.status\) \? result\.settings\.managerAgentFocus : undefined;/);
  assert.match(app, /if \(ask\) setAgentFocusOpen\(true\);\n\s*setAgentFocusChecking\(false\);/);
  assert.match(app, /setupOpen && !agentFocusOpen && !agentFocusChecking \?/);
  // 第一次问就点「稍后设置」：按全部功能记下（本地 + 设置文件），下次不再追问。
  assert.match(app, /if \(!agentFocus\) \{\n\s*\/\/[^\n]*\n\s*saveAgentFocus\("all"\);\n\s*setSetupOpen\(false\);/);
});
