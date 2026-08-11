import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

import {
  buildThreadCreateInput,
  buildTurnInput,
  chatPrimaryAction,
  filterVisibleAiEvents,
  insertSkillMention,
  isAiChatCapabilityAvailable,
  needsDangerConfirmation,
  normalizeChatSelection,
  routeChatState,
  shouldRefreshAiSnapshot,
} from "../web/src/aiChatState.ts";

const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const chatSource = await readFile(
  new URL("../web/src/components/AiChat.tsx", import.meta.url),
  "utf8",
);
const apiSource = await readFile(new URL("../web/src/api.ts", import.meta.url), "utf8");
const styles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");

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

test("AI chat capability is local-only and thread creation freezes the current origin", () => {
  assert.equal(isAiChatCapabilityAvailable({ localAiChat: true }), true);
  assert.equal(isAiChatCapabilityAvailable({ localAiChat: false }), false);
  assert.equal(isAiChatCapabilityAvailable(undefined), false);
  assert.deepEqual(buildThreadCreateInput("project-1", "task-1"), {
    projectId: "project-1",
    issueId: "task-1",
  });
  assert.deepEqual(buildThreadCreateInput("project-1", null), {
    projectId: "project-1",
  });
  assert.equal(buildThreadCreateInput("", null), null);
});

test("navigation updates only the pending new-thread origin and preserves the selected global thread", () => {
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

test("model and effort selections are normalized exclusively against the real catalog", () => {
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

test("@ skill insertion uses the selected real skill id while keeping the mention visible", () => {
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

test("turn input contains only visible user content, real skill ids and one-time confirmation", () => {
  assert.deepEqual(buildTurnInput("检查 LOCAL-103", ["cloudflare"], false), {
    message: "检查 LOCAL-103",
    skillIds: ["cloudflare"],
  });
  assert.deepEqual(buildTurnInput("执行", [], true), {
    message: "执行",
    dangerFullAccessConfirmed: true,
  });
  assert.equal(JSON.stringify(buildTurnInput("hello", [], false)).includes("workspacePath"), false);
  assert.equal(JSON.stringify(buildTurnInput("hello", [], false)).includes("manage-taskboard"), false);
});

test("running threads expose stop, danger-full-access requires confirmation, and SSE is a refresh hint", () => {
  assert.equal(chatPrimaryAction("running", "hello"), "stop");
  assert.equal(chatPrimaryAction("idle", "hello"), "send");
  assert.equal(chatPrimaryAction("idle", "  "), "disabled");
  assert.equal(needsDangerConfirmation("danger-full-access", false), true);
  assert.equal(needsDangerConfirmation("danger-full-access", true), false);
  assert.equal(needsDangerConfirmation("workspace-write", false), false);
  assert.equal(shouldRefreshAiSnapshot("ai.event"), true);
  assert.equal(shouldRefreshAiSnapshot("ai.run"), true);
  assert.equal(shouldRefreshAiSnapshot("unrelated"), false);
});

test("reasoning and raw JSONL events never enter the visible activity timeline", () => {
  const events = filterVisibleAiEvents([
    { id: "1", type: "agent_message", role: "assistant", content: "公开回复" },
    { id: "2", type: "reasoning", role: "activity", content: "private chain of thought" },
    { id: "3", type: "raw_jsonl", role: "activity", content: "{\"secret\":true}" },
    { id: "4", type: "command", role: "activity", content: "npm test" },
  ]);
  assert.deepEqual(events.map((event) => event.id), ["1", "4"]);
});

test("App wires the global panel outside project/detail branches and hides it without local capability", () => {
  assert.match(appSource, /<AiChat/);
  assert.match(appSource, /projectId=\{selectedProjectId \|\| null\}/);
  assert.match(appSource, /issueId=\{detailTaskId\}/);
  assert.match(appSource, /localAiChat/);
  assert.match(chatSource, /if \(!available\) return null/);
  assert.match(chatSource, /className="ai-chat-launcher/);
  assert.match(chatSource, /className="ai-chat-panel/);
});

test("AI chat API uses the stable local contract and never sends cwd or hidden prompt fields", () => {
  assert.match(apiSource, /\/api\/local\/ai\/catalog\?projectId=/);
  assert.match(apiSource, /\/api\/local\/ai\/threads/);
  assert.match(apiSource, /\/turns/);
  assert.match(apiSource, /\/interrupt/);
  assert.match(apiSource, /new EventSource\(`\/api\/local\/ai\/threads\//);
  assert.doesNotMatch(apiSource, /hiddenPrompt|workspacePath:\s*input|argv|cwd/);
});

test("panel follows the measured Codex-like layout and responsive boundary", () => {
  assert.match(styles, /\.ai-chat-launcher\s*\{[\s\S]*?position:\s*fixed;[\s\S]*?width:\s*40px;[\s\S]*?height:\s*40px;/);
  assert.match(styles, /\.ai-chat-panel\s*\{[\s\S]*?position:\s*fixed;[\s\S]*?width:\s*min\(500px,/);
  assert.match(styles, /\.ai-chat-panel\s*\{[\s\S]*?height:\s*min\(680px,\s*calc\(100vh - 80px\)\);/);
  assert.match(styles, /\.ai-chat-panel-header\s*\{[\s\S]*?height:\s*48px;/);
  assert.match(styles, /\.ai-chat-composer\s*\{[\s\S]*?min-height:\s*116px;[\s\S]*?max-height:\s*240px;/);
  assert.match(styles, /@media \(max-width:\s*719px\)/);
  assert.doesNotMatch(chatSource, /<select/);
});

test("chat renders Markdown, public activity cards and never renders host-only fields", () => {
  assert.match(chatSource, /ReactMarkdown/);
  assert.match(chatSource, /remarkPlugins=\{\[remarkGfm\]\}/);
  assert.match(chatSource, /ai-chat-activity/);
  assert.match(chatSource, /aria-label="停止生成"/);
  assert.match(chatSource, /aria-label="发送消息"/);
  assert.doesNotMatch(chatSource, /origin\.workspacePath/);
  assert.doesNotMatch(chatSource, /codexThreadId/);
  assert.doesNotMatch(chatSource, /manageTaskboardSkillPath/);
});

test("composer does not submit during IME composition and background runs keep launcher state fresh", () => {
  const composingGuard = chatSource.indexOf("event.nativeEvent.isComposing");
  const skillSelection = chatSource.indexOf('event.key === "Enter" && skillMention');
  const messageSubmission = chatSource.indexOf('event.key === "Enter" && !event.shiftKey');
  assert.ok(composingGuard > 0);
  assert.ok(composingGuard < skillSelection);
  assert.ok(composingGuard < messageSubmission);
  assert.match(chatSource, /backgroundRunningThreadIds/);
  assert.match(chatSource, /subscribeAiChatThread\(threadId/);
  assert.match(chatSource, /observedRunStatusesRef/);
});

test("composer and Enter submission stay disabled while a snapshot is loading", () => {
  assert.match(chatSource, /disabled=\{[\s\S]*?loading[\s\S]*?\}/);
  assert.match(chatSource, /const composerBlocked = loading[\s\S]*?\|\| settingsSaving/);
  assert.match(chatSource, /if \(composerBlocked\) return;/);
  assert.match(chatSource, /chatPrimaryAction\([\s\S]*?composerBlocked/);
});

test("new threads cannot inherit settings from a selected thread in another project", () => {
  assert.match(chatSource, /settingsForNewAiThread\(/);
  assert.match(chatSource, /catalogProjectId/);
  assert.match(chatSource, /catalogLoadedProjectId/);
  assert.match(chatSource, /createAiChatThread\(\{[\s\S]*?\.\.\.settings/);
});

test("quiet refreshes preserve action errors and PATCH results are guarded by their starting thread", () => {
  assert.match(chatSource, /if \(!quiet\) setError\(null\);/);
  assert.match(chatSource, /patchAiChatSnapshot\(current,\s*threadId,\s*thread\)/);
  assert.match(chatSource, /selectedThreadRef\.current === threadId/);
  assert.match(chatSource, /restoreDraftSettings\(previousThread\)/);
});

test("danger confirmation sends the bound pending retry instead of the current draft", () => {
  assert.match(chatSource, /pendingDangerInput/);
  assert.match(chatSource, /setPendingDangerInput\(\{[\s\S]*?message:\s*trimmed/);
  assert.match(chatSource, /startMessage\(\s*pendingDangerInput\.message,\s*true,\s*pendingDangerInput\.skillIds/);
  assert.doesNotMatch(chatSource, /onClick=\{\(\) => void startMessage\(draft,\s*true\)\}/);
});

test("SSE hints are coalesced and the panel remains resizable without clipping narrow menus", () => {
  assert.match(chatSource, /createAiSnapshotRefreshQueue/);
  assert.match(chatSource, /selectedHintRefreshQueue\.request\(selectedThreadId\)/);
  assert.match(styles, /\.ai-chat-panel\s*\{[\s\S]*?resize:\s*both;/);
  assert.match(styles, /@media \(max-width:\s*719px\)[\s\S]*?\.ai-chat-panel\s*\{[\s\S]*?resize:\s*none;/);
  assert.match(styles, /@media \(max-width:\s*719px\)[\s\S]*?\.ai-chat-menu-wrap\s*\{[\s\S]*?position:\s*static;/);
  assert.match(styles, /@media \(max-width:\s*719px\)[\s\S]*?\.ai-chat-option-menu\s*\{[\s\S]*?right:\s*0;[\s\S]*?left:\s*0;/);
});

test("history exposes deletion of local records without adding rename controls", () => {
  assert.match(chatSource, /deleteAiChatThread\(/);
  assert.match(chatSource, /aria-label=\{`删除对话 \$\{thread\.title\}`\}/);
  assert.match(chatSource, /window\.confirm\(`删除本地对话“\$\{thread\.title\}”\？`\)/);
  assert.doesNotMatch(chatSource, /重命名对话|renameAiChatThread/);
});
