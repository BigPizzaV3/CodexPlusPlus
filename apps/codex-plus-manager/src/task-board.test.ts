import assert from "node:assert/strict";
import { test } from "node:test";
import { readFile } from "node:fs/promises";
import { boardSummary, groupByStatus, matchesQuery, projectName, TASK_STATUSES, taskKey, type Task } from "./task-board-model.ts";

/**
 * 任务面板：前端只调一条 `task_board_request`，按 action 分发到 core 的 `task_board::dispatch`。
 * 这里钉死状态值与后端一致、路由登记、命令注册和英文文案，以及分组 / 搜索的纯函数。
 */
const read = (path: string) => readFile(new URL(path, import.meta.url), "utf8");

const task = (id: number, patch: Partial<Task> = {}): Task => ({
  id, title: `任务 ${id}`, status: "backlog", project: null, body: "", createdAt: id, updatedAt: id, statusAt: id, ...patch,
});

test("状态值与看板列顺序和 core 的 TaskStatus 一致", async () => {
  const core = await read("../../../crates/codex-plus-core/src/task_board.rs");
  const ids = TASK_STATUSES.map((entry) => entry.id);
  assert.deepEqual(ids, ["backlog", "todo", "in_progress", "in_review", "blocked", "done", "cancelled"]);
  const coreValues = [...core.matchAll(/TaskStatus::\w+ => "(\w+)"/g)].map((match) => match[1]);
  assert.deepEqual(coreValues, ids);
});

test("按状态分列，每列最近改过状态的在前；状态栏只数未了结的", () => {
  const tasks = [task(1, { statusAt: 10 }), task(2, { statusAt: 30 }), task(3, { status: "in_review", statusAt: 5 }), task(4, { status: "done" }), task(5, { status: "blocked" })];
  const groups = groupByStatus(tasks);
  assert.deepEqual(groups.backlog.map((item) => item.id), [2, 1]);
  assert.deepEqual(groups.in_review.map((item) => item.id), [3]);
  assert.deepEqual(groups.cancelled, []);
  assert.deepEqual(boardSummary(tasks), { open: 4, inProgress: 0, inReview: 1, blocked: 1 });
});

test("搜索命中编号、标题、正文、项目，多个词都要命中", () => {
  const item = task(12, { title: "修登录报错", body: "Token 过期", project: "D:\\work\\Codex++" });
  assert.ok(matchesQuery(item, ""));
  assert.ok(matchesQuery(item, "t-12"));
  assert.ok(matchesQuery(item, "登录 token"));
  assert.ok(matchesQuery(item, "codex++"));
  assert.ok(!matchesQuery(item, "登录 支付"));
  assert.equal(taskKey(12), "T-12");
  assert.equal(projectName("D:\\work\\Codex++\\"), "Codex++");
  assert.equal(projectName("/Users/me/repo"), "repo");
  assert.equal(projectName(null), "");
});

test("任务面板在侧栏第一组和命令面板里，并在路由表中渲染", async () => {
  const app = await read("./App.tsx");
  assert.match(app, /\| "taskBoard" \|/);
  assert.match(app, /\{ id: "taskBoard", get label\(\) \{ return t\("任务面板"\); \}, icon: SquareKanban \}/);
  assert.match(app, /const toolNavigationRoutes: Route\[\] = \[[^\]]*"taskBoard"[^\]]*\];/, "命令面板能搜到");
  assert.match(app, /\{ key: "tasks", route: "taskBoard", label: t\("任务面板"\), icon: SquareKanban, match: \["taskBoard"\] \},/, "侧栏第一组的入口");
  assert.match(app, /const globalNavigationRoutes: Route\[\] = \[[^\]]*"taskBoard"[^\]]*\];/);
  assert.match(app, /\{route === "taskBoard" \? <TaskBoardPage \/> : null\}/);
  assert.match(app, /taskBoard: t\("/, "页面要有副标题");
  assert.match(app, /route === "taskBoard" \? "task-board-screen" : ""/, "看板用满宽度");
});

test("前端调用的命令名与 Tauri 注册一致，动作与后端分发一致", async () => {
  const page = await read("./TaskBoardPage.tsx");
  const commands = await read("../src-tauri/src/task_board_commands.rs");
  const lib = await read("../src-tauri/src/lib.rs");
  const core = await read("../../../crates/codex-plus-core/src/task_board.rs");
  assert.match(page, /invoke<TaskBoard>\("task_board_request", \{ action, args \}\)/);
  assert.match(commands, /task_board::dispatch\(&action, &args\.unwrap_or\(Value::Null\)\)/);
  assert.match(lib, /task_board_commands::task_board_request,/);
  for (const action of ["list", "create", "update", "delete"]) {
    assert.ok(page.includes(`"${action}"`), `前端缺少动作 ${action}`);
    assert.ok(core.includes(`"${action}" =>`), `后端缺少动作 ${action}`);
  }
  assert.match(page, /run\("create", \{ task: \{ title, status \} \}\)/);
  assert.match(page, /run\("update", \{ id, patch \}\)/);
  assert.match(page, /await appConfirm\(tf\("删除 \{0\}/, "删除前确认");
});

test("任务面板的文案都有英文", async () => {
  const page = await read("./TaskBoardPage.tsx");
  const model = await read("./task-board-model.ts");
  const en = await read("./i18n-en.ts");
  const plain = new Set([...page.matchAll(/\bt\("([^"]+)"\)/g)].map((match) => match[1]));
  for (const match of model.matchAll(/(?:label|hint): "([^"]+)"/g)) plain.add(match[1]);
  const templates = new Set([...page.matchAll(/\btf\("([^"]+)"/g)].map((match) => match[1]));
  assert.ok(plain.size > 20 && templates.size > 5);
  const missing = [...plain, ...templates].filter((key) => !en.includes(`${JSON.stringify(key)}:`));
  assert.deepEqual(missing, []);
});
