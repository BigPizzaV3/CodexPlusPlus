import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";

const appSource = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");
const settingsSource = await readFile(
  new URL("../web/src/components/BoardSettingsMenu.tsx", import.meta.url),
  "utf8",
);
const menuSource = await readFile(
  new URL("../web/src/components/ProjectAutomationMenu.tsx", import.meta.url),
  "utf8",
);
const iconSource = await readFile(
  new URL("../web/src/components/LinearIcon.tsx", import.meta.url),
  "utf8",
);
const styles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");

test("project automation state is device-local and scoped by taskboard project", () => {
  assert.match(appSource, /const PROJECT_AUTOMATIONS_KEY = "taskboard\.projectAutomations\.v1"/);
  assert.match(appSource, /type ProjectAutomationStatus = "ACTIVE" \| "PAUSED"/);
  assert.match(appSource, /automationId\?: string/);
  assert.match(appSource, /codexProjectId: string/);
  assert.match(appSource, /type AutomationIntervalMinutes = 5 \| 10 \| 15 \| 30 \| 60/);
  assert.match(appSource, /DEFAULT_AUTOMATION_OPTIONS[\s\S]*?model: "gpt-5\.5"[\s\S]*?reasoningEffort: "high"/);
  assert.match(appSource, /localStorage\.getItem\(PROJECT_AUTOMATIONS_KEY\)/);
  assert.match(appSource, /localStorage\.setItem\(PROJECT_AUTOMATIONS_KEY, JSON\.stringify\(next\)\)/);
  assert.match(appSource, /projectAutomations\[selectedProjectId\]/);
});

test("automation requests use the exact Codex host message contract", () => {
  assert.match(appSource, /type: "taskboard:automation-request"/);
  assert.match(appSource, /operation: "ensure-active" \| "pause" \| "list"/);
  assert.match(appSource, /taskboardProjectId: selectedProjectId/);
  assert.match(appSource, /codexProjectId/);
  assert.match(appSource, /projectName: selectedProject\.name/);
  assert.match(appSource, /workspacePath/);
  assert.match(appSource, /skillPath: manageTaskboardSkillPath/);
  assert.match(appSource, /intervalMinutes: options\.intervalMinutes/);
  assert.match(appSource, /model: options\.model/);
  assert.match(appSource, /reasoningEffort: options\.reasoningEffort/);
  assert.match(appSource, /message\.type === "taskboard:automation-response"/);
  assert.match(appSource, /pendingAutomationRequestsRef/);
  assert.match(appSource, /requestId/);
  assert.match(appSource, /window\.setTimeout/);
});

test("project mapping is based on exact ids and workspace paths, never project names", () => {
  assert.match(appSource, /hostContext\?\.projects\?\.some\([\s\S]*?project\.id === selectedProject\.id/);
  assert.match(appSource, /deviceWorkspacePaths\[project\.id\] === workspacePath/);
  assert.match(appSource, /请先在 Codex 中添加并映射该项目目录/);
  assert.doesNotMatch(appSource, /project\.name === selectedProject\.name/);
});

test("the project navigation automation menu owns the icon, fields, and accessible popover", () => {
  assert.doesNotMatch(settingsSource, /自动认领待办|automationEnabled|automationPending/);
  assert.match(menuSource, /status === "ACTIVE" \? "play" : "pause"/);
  assert.doesNotMatch(menuSource, /statusStarted|statusTodo/);
  assert.match(menuSource, /aria-busy=\{pending/);
  assert.match(menuSource, /自动认领/);
  assert.match(menuSource, /无自动化/);
  assert.doesNotMatch(menuSource, /已开启自动认领|自动认领未开启/);
  assert.match(menuSource, /自动认领开关/);
  assert.match(menuSource, /5, 10, 15, 30, 60/);
  assert.match(menuSource, /AUTOMATION_MODELS\.map/);
  assert.match(menuSource, /EFFORT_LABELS\[effort\]/);
  assert.match(menuSource, /createPortal/);
  assert.match(menuSource, /window\.addEventListener\("resize"/);
  assert.match(menuSource, /window\.addEventListener\("scroll", closeFromViewportChange, true\)/);
  assert.match(menuSource, /no-drag/);
  assert.doesNotMatch(menuSource, /event\.key === "Tab"/);
  assert.match(appSource, /<ProjectAutomationMenu/);
  assert.match(appSource, /<ProjectAutomationMenu[\s\S]*?<button[\s\S]*?header-create-button/);
  assert.doesNotMatch(appSource, /toolbar-connection/);
  assert.match(appSource, /仅本地任务面板可用/);
});

test("automation status uses the verified Linear play and pause icon assets", () => {
  assert.match(
    iconSource,
    /play:\s*\{\s*content: <path d="m5\.604 2\.41 7\.23 4\.502a1\.375 1\.375 0 0 1-.02 2\.345L5\.585 13\.6a1\.375 1\.375 0 0 1-2\.083-1\.18V3\.576A1\.375 1\.375 0 0 1 5\.604 2\.41Z" \/>,\s*\}/s,
  );
  assert.match(
    iconSource,
    /pause:\s*\{\s*content: <path d="M3\.5 3\.5a1 1 0 0 1 1-1H6a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H4\.5a1 1 0 0 1-1-1v-9ZM9 3\.5a1 1 0 0 1 1-1h1\.5a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1H10a1 1 0 0 1-1-1v-9Z" \/>,\s*\}/s,
  );
});

test("automation play and pause retain Linear's 16px filled presentation", () => {
  const playAndPause = iconSource.slice(
    iconSource.indexOf("pause:"),
    iconSource.indexOf("priority:"),
  );
  assert.doesNotMatch(playAndPause, /viewBox: "0 0 24 24"|strokeWidth|strokeLinecap|strokeLinejoin/);
});

test("the automation menu reuses the Linear switch and keeps form focus chrome suppressed", () => {
  assert.match(menuSource, /className=\{`board-setting-switch\$\{draft\.status === "ACTIVE" \? " is-on" : ""\}`\}/);
  assert.match(menuSource, /role="switch"/);
  assert.match(menuSource, /aria-checked=\{draft\.status === "ACTIVE"\}/);
  assert.doesNotMatch(menuSource, /type="checkbox"/);
  assert.match(styles, /\.project-automation-field select:focus-visible\s*\{[^}]*outline:\s*0;[^}]*box-shadow:\s*none;/s);
  assert.doesNotMatch(styles, /\.project-automation-switch input:focus-visible/);
});

test("unavailable automation state has one notice, clears stale errors, and cannot change", () => {
  assert.match(menuSource, /error && error !== unavailableReason/);
  assert.match(menuSource, /const disabled = pending \|\| Boolean\(unavailableReason\)/);
  assert.equal(menuSource.match(/disabled=\{disabled\}/g)?.length, 4);
  const reconcileSource = appSource.slice(
    appSource.indexOf("const reconcileProjectAutomation"),
    appSource.indexOf("const saveProjectAutomation"),
  );
  assert.match(
    reconcileSource,
    /automationProjectContext\.unavailableReason[\s\S]*?\) \{\s*setAutomationError\(null\);\s*return;/,
  );
  assert.doesNotMatch(reconcileSource, /setAutomationError\(automationProjectContext\.unavailableReason/);
});

test("automation changes submit immediately with model-specific effort normalization", () => {
  assert.match(menuSource, /onChange: \(options: AutomationOptions\) => void/);
  assert.match(menuSource, /const disabled = pending \|\| Boolean\(unavailableReason\)/);
  assert.match(menuSource, /const submitChange = \(next: AutomationOptions\) => \{[\s\S]*?setDraft\(next\);[\s\S]*?onChange\(next\);[\s\S]*?\}/);
  assert.match(menuSource, /submitChange\(withAutomationModel\(draft, event\.target\.value as AutomationModel\)\)/);
  assert.match(menuSource, /getAutomationModel\(draft\.model\)\.efforts\.map/);
  assert.match(menuSource, /<option key=\{effort\} value=\{effort\}>\{EFFORT_LABELS\[effort\]\}<\/option>/);
  assert.match(menuSource, /low: "轻度"/);
  assert.match(menuSource, /xhigh: "极高 \(xhigh\)"/);
  assert.match(menuSource, /max: "最高"/);
  assert.match(menuSource, /ultra: "极高 \(ultra\)"/);
  assert.doesNotMatch(menuSource, />取消</);
  assert.doesNotMatch(menuSource, />保存</);
  assert.doesNotMatch(menuSource, /project-automation-actions/);
  assert.doesNotMatch(menuSource, /onSave/);
  assert.doesNotMatch(styles, /\.project-automation-actions/);
});

test("pending completion reconciles the optimistic draft to confirmed host state", () => {
  assert.match(menuSource, /const wasPendingRef = useRef\(pending\)/);
  assert.match(
    menuSource,
    /if \(wasPendingRef\.current && !pending\) \{\s*setDraft\(\{ \.\.\.DEFAULT_OPTIONS, \.\.\.automation \}\);\s*\}/,
  );
  assert.match(menuSource, /wasPendingRef\.current = pending/);
  assert.match(menuSource, /disabled=\{disabled\}/);
});

test("opening settings and changing projects reconcile with the host list", () => {
  assert.match(appSource, /sendAutomationRequest\("list", options, stored\?\.automationId\)/);
  assert.match(appSource, /items\.find\(\(item\) => item\.id === stored\?\.automationId\)/);
  assert.match(appSource, /items\.length === 1 \? items\[0\] : undefined/);
  assert.match(appSource, /status: item\.status/);
  assert.match(appSource, /automationId: undefined/);
  assert.match(appSource, /options\.status === "PAUSED" && !stored\?\.automationId/);
  assert.match(appSource, /writeProjectAutomation\(selectedProjectId, previousRecord\)/);
});
