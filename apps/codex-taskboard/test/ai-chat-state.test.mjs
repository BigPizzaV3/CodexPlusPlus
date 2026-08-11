import assert from "node:assert/strict";
import { test } from "node:test";

import {
  aiChatEventStatus,
  buildThreadCreateInput,
  buildTurnInput,
  chatPrimaryAction,
  createAiSnapshotRefreshQueue,
  filterVisibleAiEvents,
  insertSkillMention,
  isAiChatCapabilityAvailable,
  needsDangerConfirmation,
  normalizeChatSelection,
  patchAiChatSnapshot,
  readSkillMention,
  routeChatState,
  settingsForNewAiThread,
  shouldRefreshAiSnapshot,
} from "../web/src/aiChatState.ts";

const models = [
  {
    slug: "codex-real-model",
    displayName: "Codex Real Model",
    description: "Host model",
    defaultReasoningEffort: "high",
    supportedReasoningEfforts: ["medium", "high"],
    serviceTiers: [{ id: "priority", name: "Priority" }],
  },
  {
    slug: "codex-fast-model",
    displayName: "Codex Fast Model",
    description: "Fast host model",
    defaultReasoningEffort: "low",
    supportedReasoningEfforts: ["low"],
    serviceTiers: [],
  },
];

test("AI chat is exposed only when the local capability is explicit", () => {
  assert.equal(isAiChatCapabilityAvailable({ localAiChat: true }), true);
  assert.equal(isAiChatCapabilityAvailable({ localAiChat: false }), false);
  assert.equal(isAiChatCapabilityAvailable(undefined), false);
});

test("new threads freeze the current project and optional issue as server identifiers", () => {
  assert.deepEqual(buildThreadCreateInput("project-1", "issue-1"), {
    projectId: "project-1",
    issueId: "issue-1",
  });
  assert.deepEqual(buildThreadCreateInput("project-1", null), {
    projectId: "project-1",
  });
  assert.equal(buildThreadCreateInput("", null), null);
});

test("route changes update only the next origin and preserve the selected global thread", () => {
  assert.deepEqual(
    routeChatState(
      { selectedThreadId: "thread-a", pendingProjectId: "project-a", pendingIssueId: "issue-a" },
      "project-b",
      "issue-b",
    ),
    {
      selectedThreadId: "thread-a",
      pendingProjectId: "project-b",
      pendingIssueId: "issue-b",
    },
  );
});

test("new-thread settings are reused only when they belong to the current project catalog", () => {
  const settings = {
    model: "codex-real-model",
    reasoningEffort: "high",
    sandbox: "workspace-write",
  };
  assert.deepEqual(settingsForNewAiThread("project-a", "project-a", settings), settings);
  assert.deepEqual(settingsForNewAiThread("project-b", "project-a", settings), {});
  assert.deepEqual(settingsForNewAiThread("project-b", null, settings), {});
});

test("PATCH results can update only the snapshot for the thread that started the request", () => {
  const threadA = {
    id: "thread-a",
    title: "A",
    status: "idle",
    origin: { projectId: "project-a" },
  };
  const threadB = {
    id: "thread-b",
    title: "B",
    status: "idle",
    origin: { projectId: "project-b" },
  };
  const current = { thread: threadB, events: [], runs: [] };

  assert.equal(patchAiChatSnapshot(current, "thread-a", threadA), current);
  assert.deepEqual(patchAiChatSnapshot(current, "thread-b", {
    ...threadB,
    title: "B updated",
  }), {
    ...current,
    thread: { ...threadB, title: "B updated" },
  });
});

test("model and effort selections are restricted to the real catalog", () => {
  assert.deepEqual(normalizeChatSelection(models, "codex-real-model", "medium"), {
    model: "codex-real-model",
    reasoningEffort: "medium",
  });
  assert.deepEqual(normalizeChatSelection(models, "codex-real-model", "fake-effort"), {
    model: "codex-real-model",
    reasoningEffort: "high",
  });
  assert.deepEqual(normalizeChatSelection(models, "missing-model", "high"), {
    model: "codex-real-model",
    reasoningEffort: "high",
  });
  assert.equal(normalizeChatSelection([], "missing-model", "high"), null);
});

test("@ skill mentions keep a visible label while sending only the selected real id", () => {
  assert.deepEqual(readSkillMention("请用 @cl", 6), {
    start: 3,
    end: 6,
    query: "cl",
  });
  assert.deepEqual(insertSkillMention("请用 @cl 检查", 3, 6, {
    id: "cloudflare",
    label: "Cloudflare",
    scope: "user",
  }), {
    value: "请用 @Cloudflare 检查",
    caret: 14,
    skillId: "cloudflare",
  });
});

test("turn input cannot contain cwd, hidden context, model overrides or arbitrary args", () => {
  const input = buildTurnInput("检查 LOCAL-103", ["cloudflare"], false);
  assert.deepEqual(input, {
    message: "检查 LOCAL-103",
    skillIds: ["cloudflare"],
  });
  assert.equal(JSON.stringify(input).includes("workspacePath"), false);
  assert.equal(JSON.stringify(input).includes("manage-taskboard"), false);
  assert.equal(JSON.stringify(input).includes("model"), false);
  assert.deepEqual(buildTurnInput("执行", [], true), {
    message: "执行",
    dangerFullAccessConfirmed: true,
  });
});

test("runtime controls distinguish send, stop, danger confirmation and SSE refresh hints", () => {
  assert.equal(chatPrimaryAction("running", "hello"), "stop");
  assert.equal(chatPrimaryAction("idle", "hello"), "send");
  assert.equal(chatPrimaryAction("idle", "  "), "disabled");
  assert.equal(chatPrimaryAction("idle", "hello", true), "disabled");
  assert.equal(chatPrimaryAction("running", "hello", true), "disabled");
  assert.equal(needsDangerConfirmation("danger-full-access", false), true);
  assert.equal(needsDangerConfirmation("danger-full-access", true), false);
  assert.equal(needsDangerConfirmation("workspace-write", false), false);
  assert.equal(shouldRefreshAiSnapshot("ai.event"), true);
  assert.equal(shouldRefreshAiSnapshot("ai.run"), true);
  assert.equal(shouldRefreshAiSnapshot("unrelated"), false);
});

test("visible activity keeps only the latest lifecycle item without merging messages", () => {
  const events = filterVisibleAiEvents([
    {
      id: "1",
      type: "agent_message",
      role: "assistant",
      content: "公开回复一",
      data: { itemId: "shared-message" },
    },
    {
      id: "2",
      type: "agent_message",
      role: "assistant",
      content: "公开回复二",
      data: { itemId: "shared-message" },
    },
    {
      id: "3",
      type: "command_execution",
      role: "activity",
      content: "npm test",
      data: { itemId: "command-1", status: "started" },
    },
    {
      id: "4",
      type: "command_execution",
      role: "activity",
      content: "npm test",
      data: { itemId: "command-1", status: "in_progress" },
    },
    {
      id: "5",
      type: "command_execution",
      role: "activity",
      content: "npm test",
      data: { itemId: "command-1", status: "completed" },
    },
    { id: "6", type: "todo_list", role: "activity", content: "完成测试" },
    { id: "7", type: "turn.failed", role: "error", content: "执行失败" },
    { id: "8", type: "user_message", role: "user", content: "第一条", data: { itemId: "user-1" } },
    { id: "9", type: "user_message", role: "user", content: "第二条", data: { itemId: "user-1" } },
    { id: "10", type: "reasoning", role: "activity", content: "private chain of thought" },
    { id: "11", type: "raw_jsonl", role: "activity", content: "{\"secret\":true}" },
  ]);
  assert.deepEqual(events.map((event) => event.id), ["1", "2", "5", "6", "7", "8", "9"]);
});

test("activity status treats started, running and in_progress as active and failures as failed", () => {
  assert.equal(aiChatEventStatus({
    id: "1",
    type: "command_execution",
    role: "activity",
    content: "",
    data: { status: "started" },
  }), "running");
  assert.equal(aiChatEventStatus({
    id: "2",
    type: "todo_list",
    role: "activity",
    content: "",
    data: { status: "in_progress" },
  }), "running");
  assert.equal(aiChatEventStatus({
    id: "3",
    type: "turn.failed",
    role: "error",
    content: "failed",
  }), "failed");
  assert.equal(aiChatEventStatus({
    id: "4",
    type: "file_change",
    role: "activity",
    content: "",
    data: { status: "completed" },
  }), "completed");
});

test("snapshot hint refreshes allow one in-flight request and one queued request", async () => {
  const calls = [];
  const releases = [];
  const queue = createAiSnapshotRefreshQueue(async (threadId) => {
    calls.push(threadId);
    await new Promise((resolve) => releases.push(resolve));
  });

  const first = queue.request("thread-1");
  const queued = [
    queue.request("thread-1"),
    queue.request("thread-1"),
    queue.request("thread-1"),
  ];
  assert.deepEqual(calls, ["thread-1"]);

  releases.shift()();
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["thread-1", "thread-1"]);

  releases.shift()();
  await Promise.all([first, ...queued]);
  assert.deepEqual(calls, ["thread-1", "thread-1"]);
  queue.clear();
});

test("reasoning and raw JSONL events are excluded from the visible timeline", () => {
  const events = filterVisibleAiEvents([
    { id: "1", type: "agent_message", role: "assistant", content: "公开回复" },
    { id: "2", type: "reasoning", role: "activity", content: "private chain of thought" },
    { id: "3", type: "raw_jsonl", role: "activity", content: "{\"secret\":true}" },
    { id: "4", type: "command", role: "activity", content: "npm test" },
  ]);
  assert.deepEqual(events.map((event) => event.id), ["1", "4"]);
});
