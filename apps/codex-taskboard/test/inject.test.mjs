import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";

import { parseTaskboardAutomationHostRequest } from "../shared/taskboard-automation.mjs";

const sourceUrl = new URL("../inject/codex-taskboard.user.js", import.meta.url);
const source = await readFile(sourceUrl, "utf8");
const webStyles = await readFile(new URL("../web/src/styles.css", import.meta.url), "utf8");
const webApp = await readFile(new URL("../web/src/App.tsx", import.meta.url), "utf8");

test("injection is an idempotent IIFE guarded by its current source hash", () => {
  assert.match(source, /^\(\(\) => \{/);
  assert.match(source, /const VERSION = "0\.6\.8"/);
  assert.match(source, /const SOURCE_HASH = window\.__CODEX_TASKBOARD_SOURCE_HASH__/);
  assert.match(source, /const SENTINEL_KEY = "__codexTaskboardInjection__"/);
  assert.match(source, /previous\?\.sourceHash === SOURCE_HASH/);
  assert.match(source, /previous\.refresh\(\);\s*return;/);
  assert.match(source, /sourceHash: SOURCE_HASH/);
  assert.match(source, /window\[SENTINEL_KEY\] = api/);
});

test("embedded page uses the local taskboard URL and supports a runtime override", () => {
  assert.match(source, /http:\/\/127\.0\.0\.1:47823\/\?host=codex/);
  assert.match(source, /window\.__CODEX_TASKBOARD_URL__/);
  assert.match(source, /nextFrame\.src = taskboardUrl\.href/);
  assert.match(source, /frameOrigin = taskboardUrl\.origin/);
});

test("entry clones the native Plugins row and the page covers the complete Codex workspace", () => {
  assert.match(source, /const PLUGIN_LABELS = \["插件", "plugins"\]/);
  assert.match(source, /if \(siblings\.length >= 3\) return plugin;/);
  assert.match(source, /return directButtons\.length >= 3/);
  assert.match(source, /const button = reference\.cloneNode\(true\)/);
  assert.match(source, /reference\.after\(entry\)/);
  assert.match(source, /document\.querySelector\("\.app-shell-main-content-frame"\)/);
  assert.match(source, /const surface = viewport\?\.parentElement/);
  assert.match(source, /surface\.appendChild\(page\)/);
  assert.match(source, /#\$\{PAGE_ID\} \{[\s\S]*?top: 0;/);
  assert.doesNotMatch(source, /--codex-taskboard-top-offset/);
  assert.match(source, /child\.setAttribute\(HIDDEN_ATTRIBUTE, "true"\)/);
  assert.match(source, /page\.hidden = false/);
  assert.doesNotMatch(source, /codex-taskboard-overlay/);
  assert.doesNotMatch(source, /codex-taskboard-toolbar/);
  assert.doesNotMatch(source, /aria-modal/);
});

test("injected Codex controls reuse the app's icon style instead of text glyphs", () => {
  assert.match(source, /const CODEX_ICONS = \{/);
  assert.match(source, /function applyCodexIcon\(svg, name\)/);
  assert.match(source, /applyCodexIcon\(icon, "panel"\)/);
  const panelSource = source.slice(
    source.indexOf("function toggleNativeThreadPanelSize"),
    source.indexOf("\n\n  async function showNativeThreadPanel"),
  );
  assert.match(panelSource, /setIconButton\(button, nativeThreadPanelSize === "maximized" \? "chevronDown" : "expand"\)/);
  assert.match(panelSource, /setIconButton\(full, "openExternal"\)/);
  assert.match(panelSource, /setIconButton\(size, nativeThreadPanelSize === "maximized" \? "chevronDown" : "expand"\)/);
  assert.match(panelSource, /setIconButton\(close, "close"\)/);
  assert.doesNotMatch(panelSource, /full\.textContent/);
  assert.doesNotMatch(panelSource, /size\.textContent/);
  assert.doesNotMatch(panelSource, /close\.textContent/);
});

test("opening Taskboard suppresses native selection and contextual header until close", () => {
  assert.match(source, /aside nav\[role="navigation"\] \[aria-current\]/);
  assert.match(source, /const NATIVE_PAGE_LABELS = \[[\s\S]*"新对话"[\s\S]*"new conversation"/);
  assert.match(source, /node\.removeAttribute\("aria-current"\)/);
  assert.match(source, /NATIVE_SELECTED_ATTRIBUTE/);
  assert.match(source, /app-shell-header-context-menu-surface/);
  assert.match(source, /restoreNativeSelection\(\)/);
  assert.match(source, /function onDocumentClick[\s\S]*closeTaskboard\(false\);/);
  assert.doesNotMatch(source, /setTimeout\(\(\) => closeTaskboard\(false\), 0\)/);
});

test("the embedded header fills the native titlebar without clipping or a full-page no-drag region", () => {
  assert.match(source, /top: 0;/);
  assert.match(source, /z-index: 31 !important/);
  assert.doesNotMatch(source, /headerRightInset/);
  assert.doesNotMatch(source, /NATIVE_HEADER_RIGHT_INSET/);
  assert.doesNotMatch(source, /clip-path: polygon/);
  assert.doesNotMatch(source, /codex-taskboard-titlebar-fill/);
  assert.doesNotMatch(source, /#\$\{PAGE_ID\} \{[^}]*-webkit-app-region: no-drag !important;/);
  assert.doesNotMatch(source, /#\$\{FRAME_ID\} \{[^}]*-webkit-app-region: no-drag !important;/);
  assert.match(source, /const NO_DRAG_LEFT_ID = "codex-taskboard-no-drag-left"/);
  assert.match(source, /const NO_DRAG_RIGHT_ID = "codex-taskboard-no-drag-right"/);
  assert.match(source, /window\.addEventListener\("resize", scheduleRefresh\)/);
});

test("only the empty embedded header spacer is draggable", () => {
  assert.match(webApp, /<div ref=\{dragRegionRef\} className="workspace-drag-region" aria-hidden="true" \/>/);
  assert.match(webApp, /type: "taskboard:drag-region"/);
  assert.match(source, /const DRAG_REGION_ID = "codex-taskboard-drag-region"/);
  assert.match(source, /message\.type === "taskboard:drag-region"/);
  assert.match(source, /function updateDragRegion\(payload\)/);
  assert.match(source, /#\$\{DRAG_REGION_ID\} \{[\s\S]*?-webkit-app-region: drag;/);
  assert.doesNotMatch(webStyles, /\.app-shell\.embedded \.workspace-header \{\s*-webkit-app-region: no-drag;/);
  assert.match(
    webStyles,
    /\.app-shell\.embedded \.workspace-drag-region \{\s*-webkit-app-region: drag;/,
  );
  assert.match(
    webStyles,
    /\.app-shell\.embedded \.workspace-header \.header-actions,[\s\S]*?-webkit-app-region: no-drag;/,
  );
});

test("the embedded header clears the macOS window controls when the Codex sidebar is collapsed", () => {
  assert.match(source, /const MACOS_TITLEBAR_SAFE_LEFT = 80/);
  assert.match(source, /function titlebarLeftInset\(\)/);
  assert.match(source, /if \(nativeSidebarCollapsed\(\)\) return MACOS_TITLEBAR_SAFE_LEFT/);
  assert.match(source, /MACOS_TITLEBAR_SAFE_LEFT - surfaceLeft/);
  assert.match(source, /titlebarLeftInset: titlebarLeftInset\(\)/);
  assert.match(webApp, /--codex-titlebar-left-inset/);
  assert.match(webStyles, /padding-left: calc\(16px \+ var\(--codex-titlebar-left-inset, 0px\)\)/);
});

test("the embedded header exposes Codex's native sidebar expansion when collapsed", () => {
  assert.match(source, /\[data-app-shell-sidebar-trigger="true"\]/);
  assert.match(source, /function nativeSidebarCollapsed\(\)/);
  assert.match(source, /sidebarCollapsed: nativeSidebarCollapsed\(\)/);
  assert.match(source, /message\.type === "taskboard:expand-sidebar"/);
  assert.match(source, /function expandNativeSidebar\(\)[\s\S]*?trigger\.click\(\)/);
  assert.match(webApp, /embedded && hostContext\?\.sidebarCollapsed/);
  assert.match(webApp, /type: "taskboard:expand-sidebar"/);
  assert.match(webApp, /className="detail-back-button codex-sidebar-expand-button"/);
  assert.match(webApp, /<LinearIcon name="codexSidebarExpand" \/>/);
  assert.match(webStyles, /\.codex-sidebar-expand-button \{[\s\S]*?width: 28px;[\s\S]*?height: 28px;/);
});

test("opening asks the resident launcher to ensure the service and rebuilds failed frames", () => {
  assert.match(source, /const HOST_BINDING_NAME = "__codexTaskboardHostV1"/);
  assert.match(source, /return requestHost\("ensure"\)/);
  assert.match(source, /result\.restarted/);
  assert.match(source, /loadTaskboardFrame\(\)/);
  assert.match(source, /waitForFrameReady\(\)/);
  assert.match(source, /hostResponse: onHostResponse/);
  assert.match(source, /function hasLiveHostBinding/);
  assert.match(source, /HOST_HEARTBEAT_MAX_AGE_MS/);
});

test("the injected iframe can be cache-busted without reloading the Codex shell", () => {
  assert.match(source, /const FRAME_REFRESH_PARAM = "__codex_taskboard_refresh"/);
  assert.match(source, /function reloadFrame\(\)/);
  assert.match(source, /loadTaskboardFrame\(true\)/);
  assert.match(source, /reloadFrame,/);
});

test("reopening reuses a ready cache-busted iframe without showing the startup placeholder", () => {
  assert.match(source, /function frameMatchesTaskboardUrl\(taskboardUrl\)/);
  assert.match(source, /loadedUrl\.searchParams\.delete\(FRAME_REFRESH_PARAM\)/);
  assert.match(source, /expectedUrl\.searchParams\.delete\(FRAME_REFRESH_PARAM\)/);
  const prepareSource = source.slice(
    source.indexOf("async function prepareTaskboard"),
    source.indexOf("function restoreNativeContent"),
  );
  assert.match(prepareSource, /const canReuseFrame = Boolean\([\s\S]*frameMatchesTaskboardUrl\(taskboardUrl\)/);
  assert.match(prepareSource, /if \(canReuseFrame\) showFrame\(\);\s*else showLoading\(\);/);
  assert.match(
    prepareSource,
    /if \(!frameReady \|\| result\.restarted \|\| !frameMatchesTaskboardUrl\(taskboardUrl\)\) \{\s*showLoading\(\);/,
  );
  assert.doesNotMatch(prepareSource, /async function prepareTaskboard\(generation\) \{\s*showLoading\(\);/);
});

test("iframe messages require both the exact origin and source window", () => {
  assert.match(
    source,
    /event\.source !== frame\.contentWindow \|\| event\.origin !== frameOrigin/,
  );
  assert.match(source, /message\.type === "taskboard:open-thread"/);
  assert.match(source, /message\.type === "taskboard:create-thread"/);
  assert.match(source, /postMessage\(message, frameOrigin\)/);
});

test("the iframe automation contract is forwarded through the fixed host binding", () => {
  assert.match(source, /message\.type === "taskboard:automation-request"/);
  assert.match(source, /function handleAutomationRequest\(payload\)/);
  assert.match(source, /requestHost\(\s*"automation",\s*buildAutomationHostPayload\(payload\),\s*\)/);
  assert.match(source, /operation: payload\.operation/);
  assert.match(source, /taskboardProjectId: payload\.taskboardProjectId/);
  assert.match(source, /codexProjectId: payload\.codexProjectId/);
  assert.match(source, /workspacePath: payload\.workspacePath/);
  assert.match(source, /skillPath: payload\.skillPath/);
  assert.match(source, /model: payload\.model/);
  assert.match(source, /reasoningEffort: payload\.reasoningEffort/);
  assert.match(source, /type: "taskboard:automation-response"/);
  assert.match(source, /requestId,\s*ok: true,\s*item: response\.item/);
  assert.match(source, /items: response\.items/);
  assert.match(source, /requestId,\s*ok: false,\s*error:/);
  assert.match(source, /binding\(JSON\.stringify\(\{ \.\.\.payload, id, action \}\)\)/);
});

test("complete App automation payloads cross the injected forwarder into the current parser", () => {
  const functionSource = source.slice(
    source.indexOf("function buildAutomationHostPayload"),
    source.indexOf("\n\n  async function handleAutomationRequest"),
  );
  assert.ok(functionSource.startsWith("function buildAutomationHostPayload"));
  const buildAutomationHostPayload = vm.runInNewContext(`(${functionSource})`);
  const basePayload = {
    requestId: "request-1",
    taskboardProjectId: "local",
    codexProjectId: "codex-project",
    projectName: "Local",
    workspacePath: "/tmp/local-project",
    skillPath: "/tmp/manage-taskboard/SKILL.md",
    automationId: "automation-1",
    enabledByUser: true,
    quotaAware: false,
    intervalMinutes: 10,
    model: "gpt-5.6-sol",
    reasoningEffort: "ultra",
  };

  for (const operation of ["list", "pause", "ensure-active"]) {
    const forwarded = {
      id: `host-${operation}`,
      action: "automation",
      ...buildAutomationHostPayload({ ...basePayload, operation }),
    };
    assert.deepEqual(
      parseTaskboardAutomationHostRequest(forwarded),
      forwarded,
      `${operation} must retain model and reasoningEffort`,
    );
  }
});

test("only a loopback Taskboard iframe can request native automation", () => {
  assert.match(source, /function isLocalTaskboardOrigin\(origin\)/);
  assert.match(source, /hostname === "127\.0\.0\.1" \|\| hostname === "localhost"/);
  assert.match(
    source,
    /if \(!isLocalTaskboardOrigin\(frameOrigin\)\) \{\s*postToFrame\(\{\s*type: "taskboard:automation-response"/,
  );
});

test("issues open an unsent native Codex composer in the exact workspace with a Skill mention", () => {
  assert.match(source, /function createThreadForTask\(payload\)/);
  assert.match(source, /\[data-app-action-sidebar-select-project\]/);
  assert.match(source, /data-codex-composer/);
  assert.match(source, /type: "electron-set-active-workspace-root"/);
  assert.match(source, /root: workspacePath/);
  assert.doesNotMatch(source, /prefillPrompt: prompt/);
  assert.match(source, /requestHostTaskComposerPrefill\(\{/);
  assert.match(source, /requestHost\("prefill-task-composer"/);
  assert.match(source, /function waitForPreparedComposer\(identifier, skillPath\)/);
  assert.match(source, /\[skill-mention-name\]/);
  assert.match(source, /mention\.getAttribute\("skill-mention-path"\) === skillPath/);
  assert.doesNotMatch(source, /submit\.click\(\)/);
  assert.match(source, /type: "taskboard:thread-prepared"/);
  assert.doesNotMatch(source, /function waitForCreatedThread/);
  assert.doesNotMatch(source, /type: "taskboard:thread-created"/);
  assert.doesNotMatch(webApp, /taskboard:thread-created/);
  assert.match(
    webApp,
    /const instruction = `e-taskboard Addressing the tasks mentioned in \$\{task\.identifier\}`/,
  );
  assert.match(
    webApp,
    /const prompt = `\[\$manage-taskboard\]\(\$\{manageTaskboardSkillPath\}\) \$\{instruction\}`/,
  );
  assert.match(webApp, /skillName: "manage-taskboard"/);
  assert.match(webApp, /skillDisplayName: "Manage Taskboard"/);
  assert.match(webApp, /skillPath: manageTaskboardSkillPath/);
  assert.match(webApp, /instruction,/);
  assert.match(webApp, /type: "taskboard:create-thread"/);
  assert.match(webApp, /type: "taskboard:open-thread"[\s\S]*workspacePath: resolvedWorkspacePath/);
  assert.match(webApp, /getCodexThreadTitle\(codexThreadId\)/);
  assert.match(webApp, /const threadTitle = await resolveCodexThreadTitle\(codexThreadId, task\.title\)/);
});

test("the standalone web page opens linked Codex tasks through the app deep link", () => {
  assert.match(webApp, /window\.location\.assign\(`codex:\/\/threads\/\$\{encodeURIComponent\(codexThreadId\)\}`\)/);
});

test("the injected app opens an existing local Codex task instead of a new composer", () => {
  const openThreadSource = source.slice(
    source.indexOf("async function navigateNativeThread"),
    source.indexOf("function projectRowById"),
  );
  assert.match(openThreadSource, /if \(row\?\.isConnected\) \{[\s\S]*row\.click\?\.\(\);[\s\S]*return true;/);
  assert.match(source, /function openThreadDeepLink\(threadId\) \{[\s\S]*window\.location\.assign\(deepLink\);[\s\S]*\}/);
  assert.match(
    source,
    /async function openThread\(threadId, workspacePath = "", threadTitle = ""\) \{[\s\S]*nativeThreadPanelWorkspacePath[\s\S]*if \(!opened\) openThreadDeepLink\(threadId\);/,
  );
  assert.match(openThreadSource, /await dispatchHostMessage\(\{\s*type: "navigate-to-route",\s*path: routeForThread\(normalizedThreadId\)/);
  assert.match(openThreadSource, /return waitForNativeThread\(normalizedThreadId, normalizedThreadTitle\)/);
  assert.match(openThreadSource, /showNativeThreadFallback\(threadId\)/);
  assert.match(source, /return `\/local\/\$\{encodeURIComponent\(threadId\)\}`/);
  assert.doesNotMatch(source, /return `\/thread\/\$\{encodeURIComponent\(threadId\)\}`/);
  assert.doesNotMatch(openThreadSource, /focusComposerNonce/);
  assert.match(openThreadSource, /findThreadRow\(normalizedThreadId, normalizedThreadTitle\)/);
});

test("host navigation follows Codex's renderer message bus", () => {
  assert.match(source, /function dispatchHostMessage\(message\)/);
  assert.match(source, /window\.postMessage\(message, window\.location\.origin\)/);
  assert.doesNotMatch(source, /new CustomEvent\("codex-message-from-view"/);
});

test("the standalone web page opens unlinked issues as prefilled empty Codex tasks", () => {
  assert.match(webApp, /const query = new URLSearchParams\(\)/);
  assert.match(webApp, /query\.set\("path", workspacePath\)/);
  assert.match(webApp, /query\.set\("prompt", prompt\)/);
  assert.match(webApp, /window\.location\.assign\(`codex:\/\/new\?/);
});

test("host context captures all Codex projects even when the sidebar section is collapsed", () => {
  assert.match(source, /function readCodexProjects\(\)/);
  assert.match(source, /\[data-app-action-sidebar-project-row\]/);
  assert.match(source, /data-app-action-sidebar-project-id/);
  assert.match(source, /function findProjectsSection\(\)/);
  assert.match(source, /data-app-action-sidebar-section-collapsed/);
  assert.match(source, /async function captureHostContext\(\)/);
  assert.match(source, /while \(!section && Date\.now\(\) < sectionDeadline\)/);
  assert.match(source, /requestHostEnsure\(taskboardUrl\),\s*captureHostContext\(\),/);
  assert.match(source, /let lastNativeThreadId = ""/);
  assert.match(source, /clickedThreadId.*lastNativeThreadId/s);
  assert.match(source, /activeThreadId \|\| lastNativeThreadId \|\| normalizeThreadId\(threadIdFromLocation\(\)\)/);
  assert.match(source, /replace\(\/\^\(\?:local\|cloud\):\/i, ""\)/);
  assert.match(source, /function findTasksSection\(\)/);
});

test("cleanup removes observers, listeners, timers and owned DOM", () => {
  assert.match(source, /observer\?\.disconnect\(\)/);
  assert.match(source, /window\.removeEventListener\("message", onFrameMessage\)/);
  assert.match(source, /document\.removeEventListener\("click", onDocumentClick, true\)/);
  assert.match(source, /window\.removeEventListener\("popstate", onNativeRouteChange\)/);
  assert.match(source, /window\.clearTimeout\(reattachTimer\)/);
  assert.match(source, /data-codex-taskboard-owned/);
  assert.match(source, /delete window\[SENTINEL_KEY\]/);
});

test("host integration stays thin", () => {
  assert.match(source, /new MutationObserver\(scheduleRefresh\)/);
  assert.match(source, /type: "taskboard:host-context"/);
  assert.match(source, /type: "taskboard:theme"/);
  assert.match(source, /type: "navigate-to-route"/);
  assert.doesNotMatch(source, /__codexSessionDeleteBridge/);
  assert.doesNotMatch(source, /import\s*\(/);
  assert.doesNotMatch(source, /window\.fetch\s*=/);
});
