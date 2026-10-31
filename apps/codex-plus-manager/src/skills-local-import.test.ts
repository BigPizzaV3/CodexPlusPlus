import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";

// 本地和 cc-switch 导入先预览重复项，再由用户显式选择覆盖。
const extra = readFileSync(new URL("./ManagementExtra.tsx", import.meta.url), "utf8");
const lib = readFileSync(new URL("../src-tauri/src/lib.rs", import.meta.url), "utf8");
const commands = readFileSync(new URL("../src-tauri/src/commands.rs", import.meta.url), "utf8");

test("skills page previews a picked local folder before importing selected entries", () => {
  assert.match(extra, /import \{ open as openDialog \} from "@tauri-apps\/plugin-dialog";/);
  assert.match(extra, /openDialog\(\{ directory: true, multiple: false,/);
  assert.match(extra, /await previewImport\("local", path\)/);
  assert.match(extra, /previewImport\("cc-switch", null\)/);
  assert.match(extra, /invoke<SkillImportPreview>\("preview_skill_import"/);
  assert.match(extra, /run\("apply_skill_import"/);
  assert.match(extra, /onClick=\{\(\) => void importLocalSkills\(\)\} disabled=\{busy\}>.*\{t\("从本地目录导入"\)\}/);
  assert.match(lib, /commands::import_local_skills,/);
  assert.match(lib, /commands::preview_skill_import,/);
  assert.match(lib, /commands::apply_skill_import,/);
  assert.match(commands, /pub async fn import_local_skills\(path: String\) -> CommandResult<SkillsPayload>/);
});

test("import preview shows per-item errors and never selects those entries", () => {
  const drawer = extra.slice(extra.indexOf("function SkillImportDrawer"), extra.indexOf("function SortableSkillGroup"));
  assert.match(drawer, /candidates\.filter\(\(item\) => !item\.error && !item\.matches\.length/);
  assert.match(drawer, /candidates\.filter\(\(item\) => !item\.error && selected\.has\(item\.id\)\)/);
  assert.match(drawer, /candidates\.filter\(\(item\) => !item\.error\)\.map\(\(item\) => item\.id\)/);
  assert.match(drawer, /disabled=\{busy \|\| !!item\.error\}/);
  assert.match(drawer, /item\.error \? <small className="mg-skill-import-error" role="alert">\{item\.error\}<\/small>/);
});

test("manager resumes Codex only after Skill recovery succeeds", () => {
  const start = lib.indexOf("let startup_app = app.handle().clone();");
  const end = lib.indexOf("// 网关模式", start);
  assert.ok(start >= 0 && end > start);
  const startup = lib.slice(start, end);
  assert.match(startup, /spawn_blocking\(commands::prepare_skills_on_startup\)/);
  assert.match(startup, /Ok\(Ok\(\(\)\)\) => commands::resume_existing_codex_background\(/);
  const failure = startup.slice(startup.indexOf("result =>"));
  assert.match(failure, /manager\.skills_startup_recovery_failed/);
  assert.doesNotMatch(failure, /resume_existing_codex_background|multi_instance_resume/);
});

test("locally imported skills get their own source label and no GitHub link", () => {
  assert.match(extra, /const LOCAL_IMPORT_REPO_KEY = "local";/);
  assert.match(extra, /if \(skill\.repoKey === LOCAL_IMPORT_REPO_KEY\) return t\("本地导入"\);/);
  assert.match(extra, /skill\.repoKey === "cc-switch" \|\| skill\.repoKey === LOCAL_IMPORT_REPO_KEY\) return "";/);
});
