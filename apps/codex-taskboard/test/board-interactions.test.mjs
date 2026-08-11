import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const boardColumnSource = await readFile(new URL("../web/src/components/BoardColumn.tsx", import.meta.url), "utf8");
const apiSource = await readFile(new URL("../web/src/api.ts", import.meta.url), "utf8");
const styles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");
const detailSource = await readFile(new URL("../web/src/components/TaskDetail.tsx", import.meta.url), "utf8");
const editorSource = await readFile(new URL("../web/src/components/TaskEditor.tsx", import.meta.url), "utf8");
const labelPickerSource = await readFile(new URL("../web/src/components/LabelPicker.tsx", import.meta.url), "utf8");
const contextMenuSource = await readFile(new URL("../web/src/components/TaskContextMenu.tsx", import.meta.url), "utf8");
const cardSource = await readFile(new URL("../web/src/components/TaskCard.tsx", import.meta.url), "utf8");
const filterSource = await readFile(new URL("../web/src/taskFilters.ts", import.meta.url), "utf8");
const typesSource = await readFile(new URL("../web/src/types.ts", import.meta.url), "utf8");

function workflowStatuses() {
  const match = typesSource.match(/export const TASK_STATUSES = (\[[\s\S]*?\]) as const/);
  assert.ok(match);
  return [...match[1].matchAll(/"([^"]+)"/g)].map((entry) => entry[1]);
}

test("dragging previews the insertion rank before committing it", () => {
  assert.match(boardColumnSource, /function findDropBefore/);
  assert.match(boardColumnSource, /clientY < card\.getBoundingClientRect\(\)\.top \+ card\.offsetHeight \/ 2/);
  assert.match(boardColumnSource, /onDrop\(status, taskId, findDropBefore/);
  assert.match(boardColumnSource, /function getTaskDragShift/);
  assert.match(boardColumnSource, /shift -= dragDistance/);
  assert.match(boardColumnSource, /shift \+= dragDistance/);
  assert.match(boardColumnSource, /dragShift=\{dragShift\}/);
  assert.match(styles, /\.task-card\.is-dragging \{[\s\S]*?opacity: 0/);
  assert.doesNotMatch(styles, /\.task-card\.is-dragging \{[^}]*pointer-events: none/);
  assert.match(styles, /transform 160ms cubic-bezier/);
  assert.match(appSource, /beforeTaskId: string \| null = null/);
  assert.match(appSource, /\(previousTask\.sortOrder \+ nextTask\.sortOrder\) \/ 2/);
  assert.match(appSource, /currentOrder\.every\(\(candidate, index\) => candidate\.id === desiredOrder\[index\]\.id\)/);
  assert.match(appSource, /setTasks\(\(current\) => sortTasks\(current\.map/);
  assert.match(appSource, /setSettlingTaskId\(task\.id\)/);
  assert.match(styles, /\.task-card\.is-settling \{[\s\S]*?task-card-settle 200ms/);
});

test("text selection is reserved for editable fields", () => {
  assert.match(styles, /body \{[^}]*user-select: none/);
  assert.match(styles, /input,[\s\S]*?textarea,[\s\S]*?\[contenteditable="true"\][\s\S]*?user-select: text/);
});

test("issue cards omit redundant metadata and keep three compact, well-spaced rows", () => {
  assert.doesNotMatch(cardSource, /task\.createdAt|创建于|card-footer|created-at|projectName|project-chip/);
  assert.doesNotMatch(styles, /\.card-footer|\.created-at|\.project-chip/);
  assert.match(styles, /\.task-card \{[\s\S]*?min-height: 80px;[\s\S]*?gap: 6px;[\s\S]*?padding: 7px 8px/);
  assert.match(detailSource, /currentTask\.createdAt/);
});

test("scrollbars stay proportional while the workflow node library hides its bar", () => {
  assert.match(styles, /:root \{[\s\S]*?--scrollbar-thumb: rgba\(27, 27, 27, 0\.15\)/);
  assert.match(styles, /:root\[data-theme="dark"\] \{[\s\S]*?--scrollbar-thumb: rgba\(238, 238, 239, 0\.15\)/);
  assert.match(styles, /\* \{[\s\S]*?scrollbar-color: var\(--scrollbar-thumb\) transparent[\s\S]*?scrollbar-width: thin/);
  assert.match(styles, /\*::\-webkit-scrollbar-track,[\s\S]*?\*::\-webkit-scrollbar-track-piece,[\s\S]*?\*::\-webkit-scrollbar-corner \{[\s\S]*?background: transparent/);
  assert.match(styles, /\*::\-webkit-scrollbar-button \{[\s\S]*?display: none/);
  assert.match(styles, /\*::\-webkit-scrollbar-thumb \{[\s\S]*?min-height: 30px[\s\S]*?background: var\(--scrollbar-thumb\)[\s\S]*?background-clip: padding-box/);
  assert.doesNotMatch(styles, /scrollbar-color: var\(--border-strong\) transparent/);
  assert.doesNotMatch(styles, /\*::\-webkit-scrollbar-thumb:(?:vertical|horizontal)/);
  assert.match(styles, /\.workflow-node-groups \{[\s\S]*?overflow-y: auto;[\s\S]*?scrollbar-width: none/);
  assert.match(styles, /\.workflow-node-groups::\-webkit-scrollbar \{[\s\S]*?display: none;[\s\S]*?width: 0;[\s\S]*?height: 0/);
});

test("each status column remains a drop target for the full board height", () => {
  assert.match(styles, /\.board \{[\s\S]*?align-items: stretch;[\s\S]*?height: 100%/);
  assert.match(styles, /\.board-column \{[\s\S]*?display: flex;[\s\S]*?flex-direction: column;[\s\S]*?height: 100%/);
  assert.match(styles, /\.column-list \{[\s\S]*?flex: 1 0 auto;[\s\S]*?min-height: calc\(100% - 48px\)/);
});

test("the issue board has no shared vertical scroll and each status column scrolls below a sticky heading", () => {
  assert.match(styles, /\.board-scroll \{[\s\S]*?overflow-x: auto;[\s\S]*?overflow-y: hidden;[\s\S]*?overscroll-behavior-y: none/);
  assert.match(styles, /\.board-column \{[\s\S]*?overflow-x: hidden;[\s\S]*?overflow-y: auto;[\s\S]*?overscroll-behavior-y: contain/);
  assert.match(styles, /\.column-header \{[\s\S]*?position: sticky;[\s\S]*?top: 0;[\s\S]*?background: var\(--board-column-surface\)/);
});

test("the complete Linear-style workflow shares one ordered status source", () => {
  assert.deepEqual(workflowStatuses(), [
    "backlog",
    "todo",
    "in_progress",
    "in_review",
    "blocked",
    "done",
    "canceled",
  ]);
  assert.match(boardColumnSource, /in_review: \{ label: "审核中", tone: "review" \}/);
  assert.match(boardColumnSource, /blocked: \{ label: "已阻塞", tone: "blocked" \}/);
  assert.match(boardColumnSource, /canceled: \{ label: "已取消", tone: "canceled" \}/);
  assert.match(cardSource, /import \{ TASK_STATUSES,/);
  assert.doesNotMatch(cardSource, /STATUS_ORDER/);
  assert.match(detailSource, /TASK_STATUSES\.map\(\(status\) =>/);
  assert.match(editorSource, /TASK_STATUSES\.map\(\(value\) =>/);
  assert.match(contextMenuSource, /TASK_STATUSES\.map\(\(status, index\) =>/);
});

test("review, blocked and canceled statuses round-trip through filter URLs", () => {
  const statuses = workflowStatuses();
  const selected = ["in_review", "blocked", "canceled"];
  const url = new URL("http://taskboard.local/");
  url.searchParams.set("status", selected.join(","));
  const restored = url.searchParams.get("status").split(",").filter((status) => statuses.includes(status));

  assert.deepEqual(restored, selected);
  assert.match(filterSource, /filters\.statuses\.join\(","\)/);
  assert.match(filterSource, /\.split\(","\)\.filter\(isTaskStatus\)/);
  assert.match(filterSource, /TASK_STATUSES\.includes\(value as TaskStatus\)/);
});

test("the column surface wraps its heading and issue list", () => {
  assert.match(styles, /\.board-column \{[\s\S]*?--board-column-surface: var\(--column-header\)[\s\S]*?background: var\(--board-column-surface\)/);
  assert.match(styles, /\.column-header \{[\s\S]*?background: var\(--board-column-surface\)/);
  assert.match(styles, /\.column-list \{[\s\S]*?padding: 8px 8px 8px/);
});

test("common issue mutations enter a Linear-style undo queue", () => {
  assert.match(appSource, /const undoStackRef = useRef<UndoOperation\[]>/);
  assert.match(appSource, /event\.key\.toLowerCase\(\) === "z"/);
  assert.match(appSource, /event\.metaKey \|\| event\.ctrlKey/);
  assert.match(appSource, /function pushUndo/);
  assert.match(appSource, /setUndoNotice\(showNotice \? \{ id: operation\.id, message \} : null\)/);
  assert.match(appSource, /moveTask\(task, destination, beforeTaskId, true\)/);
  assert.doesNotMatch(appSource, /setAnnouncement\(`已撤回：/);
  assert.match(appSource, /className="toast undo-toast"/);
  assert.match(appSource, /restoreTaskRequest\(archived\)/);
  assert.match(apiSource, /export async function restoreTask/);
});

test("selected issues can be bulk moved to another project", () => {
  assert.match(appSource, /const selectedIssueTasks = useMemo/);
  assert.match(appSource, /async function moveSelectedTasksToProject/);
  assert.match(appSource, /className="compact-select bulk-project-move"/);
  assert.match(appSource, /moveTaskRequest\(task, task\.status, undefined, \{ projectId: targetProject\.id \}\)/);
  assert.match(appSource, /moveTaskRequest\(moved, previous\.status, previous\.sortOrder, \{ projectId: previous\.projectId \}\)/);
  assert.match(apiSource, /options: \{ threadId\?: string; projectId\?: string \} = \{\}/);
  assert.match(apiSource, /\.\.\.\(options\.projectId \? \{ projectId: options\.projectId \} : \{\}\)/);
});

test("issues expose processing conversations without manual binding", () => {
  assert.match(detailSource, /在对话中打开/);
  assert.match(detailSource, /onOpenInThread\(currentTask\)/);
  assert.doesNotMatch(appSource, /detail-thread-button/);
  assert.doesNotMatch(detailSource, /输入对话 ID|解除 Codex 对话绑定|>绑定</);
  assert.doesNotMatch(editorSource, /对话 ID|linkedThreadId/);
  assert.match(detailSource, /currentTask\.threadId/);
  assert.doesNotMatch(detailSource, /currentTask\.threadIds/);
  assert.match(detailSource, /<strong>查看对话<\/strong>/);
  assert.match(detailSource, /className="conversation-thread-id">\{threadId\}/);
  assert.doesNotMatch(detailSource, /shortThreadId/);
  assert.doesNotMatch(detailSource, /detail-property-label">Codex/);
  assert.match(detailSource, /comment\.threadId/);
  assert.match(detailSource, /threadId=\{comment\.threadId\}/);
  assert.doesNotMatch(detailSource, /compact/);
  assert.doesNotMatch(styles, /issue-conversation-link\.compact/);
  assert.match(detailSource, /代码分支/);
  assert.match(detailSource, /Worktree/);
  assert.match(detailSource, /developmentContext/);
  assert.doesNotMatch(detailSource, /placeholder="绑定分支/);
  assert.doesNotMatch(contextMenuSource, /打开关联 Codex 对话/);
  assert.match(contextMenuSource, /onOpenInThread/);
});

test("issues bind one workflow from the current project's workflow tabs", () => {
  assert.match(typesSource, /export interface Task \{[\s\S]*?workflowId: string \| null/);
  assert.match(typesSource, /export interface TaskDraft \{[\s\S]*?workflowId: string \| null/);
  assert.match(appSource, /function taskToDraft[\s\S]*?workflowId: task\.workflowId/);
  assert.match(appSource, /const \[workflowOptions, setWorkflowOptions\] = useState<WorkflowOption\[\]>/);
  assert.match(appSource, /workflowOptionsFromWorkspace\(record\.workspace\)/);
  assert.match(editorSource, /workflows: WorkflowOption\[\]/);
  assert.match(editorSource, /workflowId: workflowId \|\| null/);
  assert.match(editorSource, /<span className="sr-only">工作流<\/span>/);
  assert.match(detailSource, /<span className="detail-property-label">工作流<\/span>/);
  assert.match(detailSource, /workflowId: event\.target\.value \|\| null/);
  assert.match(detailSource, /当前设备未找到此流程/);
});

test("comments stage, upload, render and delete their own attachments", () => {
  assert.match(apiSource, /export async function uploadCommentAttachment/);
  assert.match(apiSource, /\/api\/comments\/\$\{encodeURIComponent\(commentId\)\}\/attachments/);
  assert.match(detailSource, /pendingCommentFiles/);
  assert.match(detailSource, /uploadCommentAttachment\(comment\.id, file\)/);
  assert.match(detailSource, /comment\.attachments\.map/);
  assert.match(detailSource, /setPendingAttachmentDelete\(attachment\)/);
});

test("issue creation and detail share one searchable, creatable label picker", () => {
  assert.match(editorSource, /<LabelPicker/);
  assert.match(detailSource, /<LabelPicker/);
  assert.match(appSource, /<TaskDetail[\s\S]*?availableLabels=\{availableLabels\}/);
  assert.match(detailSource, /selectedLabels=\{currentTask\.labels\}/);
  assert.match(detailSource, /saveTask\(\{ labels: nextLabels \}, "labels"\)/);
  assert.doesNotMatch(detailSource, /标签，以逗号分隔|function saveLabels|labels\.split/);
  assert.match(labelPickerSource, /availableLabels\.filter/);
  assert.match(labelPickerSource, /selectedLabels\.includes\(label\)/);
  assert.match(labelPickerSource, /创建 “\{normalizedSearch\}”/);
  assert.match(labelPickerSource, /labelColor\(normalizedSearch\)/);
  assert.match(labelPickerSource, /aria-multiselectable="true"/);
  assert.match(styles, /\.detail-label-picker \.label-popover/);
});
