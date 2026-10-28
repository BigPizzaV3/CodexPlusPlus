import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// #511：Skills 页「从本地目录导入」走系统目录选择 → 后端 import_local_skills → 用返回的列表刷新。
const extra = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const commands = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");

test("skills page imports a picked local folder through the registered backend command", () => {
  assert.match(extra, /import \{ open as openDialog \} from "@tauri-apps\/plugin-dialog";/);
  assert.match(extra, /openDialog\(\{ directory: true, multiple: false,/);
  // run() 会用返回的 payload 刷新列表，并把失败原文（例如缺少 SKILL.md）提示出来。
  assert.match(extra, /if \(await run\("import_local_skills", \{ path \}\)\) setTab\("installed"\);/);
  assert.match(extra, /onClick=\{\(\) => void importLocalSkills\(\)\} disabled=\{busy\}>.*\{t\("从本地目录导入"\)\}/);
  assert.match(lib, /commands::import_local_skills,/);
  assert.match(commands, /pub async fn import_local_skills\(path: String\) -> CommandResult<SkillsPayload>/);
});

test("locally imported skills get their own source label and no GitHub link", () => {
  assert.match(extra, /const LOCAL_IMPORT_REPO_KEY = "local";/);
  assert.match(extra, /if \(skill\.repoKey === LOCAL_IMPORT_REPO_KEY\) return t\("本地导入"\);/);
  assert.match(extra, /skill\.repoKey === "cc-switch" \|\| skill\.repoKey === LOCAL_IMPORT_REPO_KEY\) return "";/);
});
