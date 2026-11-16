import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";

/**
 * 「任务面板」在 Codex App 左侧图标栏里（47-task-board-page.js），不在管理器里。
 * 页面经 bridge 路由 `/task-board` 调 core `task_board::dispatch`；这里钉死状态值与后端一致、
 * 图标栏入口登记进所有写死内置 id 的地方、路由不开放给拓展，以及分组 / 搜索的纯函数。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");
const shardPath = "../../../assets/inject/renderer-inject/47-task-board-page.js";

type Task = { id: number; title: string; status: string; project?: string | null; body: string; createdAt: number; updatedAt: number; statusAt: number };
const task = (id: number, patch: Partial<Task> = {}): Task => ({
  id, title: `任务 ${id}`, status: "backlog", project: null, body: "", createdAt: id, updatedAt: id, statusAt: id, ...patch,
});

/** 分片顶层只有常量和函数声明，直接求值拿出纯函数。 */
async function loadShard() {
  const source = await read(shardPath);
  return new Function(`${source}\nreturn { codexPlusTaskStatuses, codexPlusTaskGroups, codexPlusTaskMatches, codexPlusTaskProjectName, codexPlusTaskKey };`)() as {
    codexPlusTaskStatuses: Array<{ id: string; label: string }>;
    codexPlusTaskGroups: (tasks: Task[]) => Record<string, Task[]>;
    codexPlusTaskMatches: (task: Task, query: string) => boolean;
    codexPlusTaskProjectName: (project?: string | null) => string;
    codexPlusTaskKey: (id: number) => string;
  };
}

test("状态值与看板列顺序和 core 的 TaskStatus 一致", async () => {
  const shard = await loadShard();
  const core = await read("../../../crates/codex-plus-core/src/task_board.rs");
  const ids = shard.codexPlusTaskStatuses.map((entry) => entry.id);
  assert.deepEqual(ids, ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "cancelled"]);
  assert.deepEqual([...core.matchAll(/TaskStatus::\w+ => "(\w+)"/g)].map((match) => match[1]), ids);
});

test("按状态分列，每列最近改过状态的在前；搜索命中编号、标题、正文、项目", async () => {
  const shard = await loadShard();
  const groups = shard.codexPlusTaskGroups([task(1, { statusAt: 10 }), task(2, { statusAt: 30 }), task(3, { status: "in_review" }), task(4, { status: "nope" })]);
  // statusAt：2 → 30、1 → 10、4 → 4；不认识的状态（4）归到收集。
  assert.deepEqual(groups.backlog.map((item) => item.id), [2, 1, 4]);
  assert.deepEqual(groups.in_review.map((item) => item.id), [3]);
  assert.deepEqual(groups.cancelled, []);

  const item = task(12, { title: "修登录报错", body: "Token 过期", project: "D:\\work\\Codex++" });
  assert.ok(shard.codexPlusTaskMatches(item, ""));
  assert.ok(shard.codexPlusTaskMatches(item, "t-12"));
  assert.ok(shard.codexPlusTaskMatches(item, "登录 token"));
  assert.ok(!shard.codexPlusTaskMatches(item, "登录 支付"));
  assert.equal(shard.codexPlusTaskKey(12), "T-12");
  assert.equal(shard.codexPlusTaskProjectName("D:\\work\\Codex++\\"), "Codex++");
  assert.equal(shard.codexPlusTaskProjectName("/Users/me/repo"), "repo");
  assert.equal(shard.codexPlusTaskProjectName(null), "");
});

test("Codex 左侧图标栏的「任务面板」入口：登记进所有写死内置 id 的地方", async () => {
  const renderer = await read("../../../assets/inject/renderer-inject.js");
  const manifest = JSON.parse(await read("../../../assets/inject/renderer-inject/manifest.json"));
  assert.ok(manifest.fragments.some((fragment: { name: string }) => fragment.name === "47-task-board-page.js"), "分片登记进清单");
  assert.match(renderer, /codexPlusRailTasksId = "codex-plus-rail-tasks"/);
  assert.match(renderer, /codexPlusTaskBoardTab = "task-board"/);
  assert.match(renderer, /\{ id: codexPlusRailTasksId, label: "任务面板", iconMarkup: icons\.tasks, withStatus: false, onActivate: openCodexPlusTaskBoardPage \}/);
  assert.match(renderer, /\[codexPlusRailTasksId, "tasks"\],/, "选中态");
  assert.match(renderer, /if \(tab === codexPlusTaskBoardTab\) return "tasks";/);
  // 两处图标栏点击监听都要排除新入口，否则点它会把刚打开的页面关掉。
  assert.equal(renderer.split("#${codexPlusRailRemoteId}, #${codexPlusRailTasksId}`)) return;").length - 1, 2);
  assert.match(renderer, /#\$\{codexPlusRailTasksId\} > button/, "扫描调度排除自己的节点");
  assert.match(renderer, /data-codex-plus-active-tab="\$\{codexPlusTaskBoardTab\}"/);
  assert.match(renderer, /markCodexPlusExtensionNode\(overlay, "builtin-task-board"\)/);
});

test("页面经 /task-board 调 core 的同一份分发；路由不开放给拓展；删除前确认；关页时停掉轮询", async () => {
  const page = await read(shardPath);
  const routes = await read("../../../crates/codex-plus-core/src/routes.rs");
  const core = await read("../../../crates/codex-plus-core/src/task_board.rs");
  const extensionApi = await read("../../../assets/inject/renderer-inject/91-extension-api.js");
  assert.match(page, /postJson\("\/task-board", \{ action, \.\.\.args \}\)/);
  assert.match(routes, /"\/task-board" => \{\n[^}]*crate::task_board::bridge_route\(&payload\)/);
  assert.ok(!extensionApi.includes(`"/task-board"`), "不开放给拓展");
  for (const action of ["list", "create", "update", "delete"]) {
    assert.ok(page.includes(`codexPlusTaskRun("${action}"`) || page.includes(`codexPlusTaskRequest("${action}"`), `Codex 页面缺少动作 ${action}`);
    assert.ok(core.includes(`"${action}" =>`), `后端缺少动作 ${action}`);
  }
  assert.match(page, /codexPlusTaskRun\("create", \{ task: \{ title, status \} \}\)/);
  assert.match(page, /codexPlusTaskRun\("update", \{ id, patch \}\)/);
  assert.match(page, /await codexPlusAsk\(\{\n\s*title: `删除 /, "删除前确认");
  assert.match(page, /window\.__codexPlusExtensionPageCleanup = \(\) => \{\n\s*clearInterval\(state\.pollTimer\);/);
  assert.match(page, /window\.addEventListener\("keydown", codexPlusTaskCaptureKeydown, true\);/, "在 window 捕获阶段接管按键，排在 Codex 的 document 捕获监听之前");
  assert.match(page, /window\.removeEventListener\("keydown", codexPlusTaskCaptureKeydown, true\);\n\s*window\.addEventListener/, "重复打开不叠加监听");
  assert.match(page, /if \(typing \|\| !\(event\.metaKey \|\| event\.ctrlKey \|\| event\.altKey\)\) event\.stopPropagation\(\);/, "打字不落进 Codex 的输入框，组合键照常交给 Codex");
  assert.match(page, /function codexPlusTaskParkFocus\(selector\)/, "Esc 之后把焦点留在本页");
  // 状态栏和详情里的状态按钮不能共用 data-codex-task-status：querySelector 会先命中按钮，把计数写进按钮里。
  assert.equal(page.split('data-codex-task-status="').length - 1, 1, "data-codex-task-status 只给状态按钮用");
  assert.match(page, /data-codex-task-summary="true"/);
});

test("管理器里不再有任务面板页面", async () => {
  const app = await read("./App.tsx");
  assert.doesNotMatch(app, /taskBoard|TaskBoardPage/);
  const lib = await read("../src-tauri/src/lib.rs");
  assert.doesNotMatch(lib, /task_board/);
});
