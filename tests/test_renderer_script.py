import subprocess
from pathlib import Path


def test_renderer_script_exists_and_parses_with_node():
    script = Path("codex_session_delete/inject/renderer-inject.js")
    assert script.exists()
    result = subprocess.run(["node", "--check", str(script)], capture_output=True, text=True)
    assert result.returncode == 0, result.stderr


def test_renderer_script_contains_hover_delete_contract():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "codex-delete-button" in text
    assert "codex-session-actions" in text
    assert "MutationObserver" in text
    assert "confirmDelete" in text
    assert "/delete" in text
    assert "/undo" in text


def test_renderer_script_supports_codex_sidebar_thread_attributes():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function sessionRows")
    end = text.index("\n\n  function archivePageHintVisible", start)
    session_rows_code = text[start:end]
    assert "const selectors" in text
    assert "sidebarThread" in text
    assert "data-app-action-sidebar-thread-id" in text
    assert "threadTitle" in text
    assert "data-thread-title" in text
    assert "selectors.sidebarThread" in session_rows_code
    assert "a[href*='session']" not in session_rows_code
    assert "conversation" not in session_rows_code
    assert "thread" not in session_rows_code.replace("sidebarThread", "")
    assert "hasSessionHint" not in session_rows_code


def test_renderer_script_positions_delete_button_without_affecting_layout():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "position: absolute" in text
    assert "right: 28px" in text
    assert "top: 50%" in text
    assert "transform: translateY(-50%)" in text
    assert "display: inline-flex" in text




def test_renderer_script_keeps_sponsors_separate_from_author_support():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    sponsor_start = text.index('<div class="codex-plus-panel" data-codex-plus-panel="sponsor"')
    support_start = text.index('<div class="codex-plus-panel" data-codex-plus-panel="support"', sponsor_start)
    sponsor_panel = text[sponsor_start:support_start]
    support_panel = text[support_start:text.index('</div>\n        </div>\n      </div>', support_start)]

    assert 'data-codex-plus-tab="sponsor"' in text
    assert 'data-codex-plus-tab="support"' in text
    assert "codexPlusAdsUrl" in text
    assert "renderCodexPlusAds()" in sponsor_panel
    assert "请我喝杯咖啡" not in sponsor_panel
    assert "请作者喝咖啡" not in sponsor_panel
    assert "codex-plus-sponsor-grid" not in sponsor_panel
    assert "请我喝杯咖啡" in support_panel
    assert "codex-plus-sponsor-grid" in support_panel



def test_renderer_script_configures_sponsor_ad_and_coffee_tabs():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert 'data-codex-plus-tab="sponsor" data-active="false">推荐内容</button>' in text
    assert 'data-codex-plus-tab="support" data-active="false">请作者喝咖啡</button>' in text
    assert "赞助商推荐" in text[text.index('data-codex-plus-panel="sponsor"'):text.index('data-codex-plus-panel="support"')]
    assert "普通推荐" in text[text.index('data-codex-plus-panel="sponsor"'):text.index('data-codex-plus-panel="support"')]
    assert 'data-codex-plus-active-tab="sponsor"' not in text
    assert '.codex-plus-modal-content[data-codex-plus-active-tab="support"] { width: min(820px, calc(100vw - 48px)); }' in text
    assert "codex-plus-ad-image" not in text
    assert "rawchat-sponsor.jpg" not in text[text.index("function renderCodexPlusAds"):text.index("function selectCodexPlusTab")]



def test_renderer_script_uses_recommendation_copy_for_ad_page():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    sponsor_panel = text[text.index('data-codex-plus-panel="sponsor"'):text.index('data-codex-plus-panel="support"')]

    assert 'data-codex-plus-tab="sponsor" data-active="false">推荐内容</button>' in text
    assert "赞助商推荐" in sponsor_panel
    assert "普通推荐" in sponsor_panel
    assert "广告分为" not in sponsor_panel



def test_renderer_script_filters_expired_remote_ads():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "expires_at" in text
    assert "isCodexPlusAdExpired" in text
    assert "Date.parse(ad.expires_at)" in text
    assert "!isCodexPlusAdExpired(ad)" in text



def test_renderer_script_loads_ads_through_bridge():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("async function fetchCodexPlusAds")
    end = text.index("function selectCodexPlusTab", start)
    fetch_code = text[start:end]

    assert 'postJson("/ads", {})' in fetch_code
    assert "fetch(" not in fetch_code



def test_renderer_script_loads_ads_through_helper_origin():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "\"/ads\"" in text
    assert "raw.githubusercontent.com/BigPizzaV3/Ad-List" not in text



def test_renderer_script_loads_ads_from_remote_json_without_local_fallback():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "\"/ads\"" in text
    assert "fetchCodexPlusAds" in text
    assert "codexPlusAds" in text
    assert "RawChat｜Codex 中转站" not in text
    assert "0029.org" not in text
    assert "rawchat.cn" not in text[text.index("function renderCodexPlusAds"):text.index("function selectCodexPlusTab")]
    assert "请求不到就不显示" not in text



def test_renderer_script_renders_sponsor_and_normal_ad_groups():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    sponsor_panel = text[text.index('data-codex-plus-panel="sponsor"'):text.index('data-codex-plus-panel="support"')]

    assert "赞助商推荐" in sponsor_panel
    assert "普通推荐" in sponsor_panel
    assert "renderCodexPlusAdGroup(\"sponsor\"" in text
    assert "renderCodexPlusAdGroup(\"normal\"" in text
    assert "codex-plus-ad-empty" in text
    assert "codex-plus-ad-image" not in text



def test_renderer_script_contains_conversation_timeline_contract():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "codex-conversation-timeline" in text
    assert "codex-conversation-timeline-marker" in text
    assert "codex-conversation-timeline-tooltip" in text
    assert "codex-conversation-timeline-target" in text
    assert "codexConversationTimelineVersion" in text
    assert "refreshConversationTimeline" in text
    assert "truncateTimelineQuestion" in text
    assert "timelineQuestionLimit = 40" in text
    assert "timelineMinTopPercent" in text
    assert "timelineMaxTopPercent" in text



def test_renderer_script_detects_user_questions_for_timeline_without_sidebar_scan():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function conversationTimelineQuestions")
    end = text.index("\n\n  function refreshConversationTimeline", start)
    timeline_detection_code = text[start:end]

    assert "conversationTimelineRoot" in timeline_detection_code
    assert "conversationTimelineQuestionCandidates" in timeline_detection_code
    assert "data-message-author-role=\"user\"" in text
    assert "data-testid=\"conversation-turn\"" in text
    assert "thread-scroll-container" in text
    assert "bg-token-foreground/5" in text
    assert "items-end" in text
    assert "visibleTimelineNode" in timeline_detection_code
    assert "timelineNodeId" in timeline_detection_code
    assert "main" in timeline_detection_code
    assert "selectors.sidebarThread" not in timeline_detection_code
    assert "document.body.textContent" not in timeline_detection_code
    assert "extractTimelineQuestionText" in timeline_detection_code



def test_renderer_script_refreshes_conversation_timeline_from_scan_loop():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    deferred_start = text.index("function scanDeferred")
    deferred_end = text.index("\n\n  function runScanStep", deferred_start)
    scan_deferred_code = text[deferred_start:deferred_end]
    extension_start = text.index("function isExtensionUiNode")
    extension_end = text.index("\n\n  const scanRelevantSelector", extension_start)
    extension_code = text[extension_start:extension_end]
    relevant_start = text.index("const scanRelevantSelector")
    relevant_end = text.index("\n\n  function isScanRelevantNode", relevant_start)
    relevant_code = text[relevant_start:relevant_end]
    chat_start = text.index("function isChatContentMutation")
    chat_end = text.index("\n\n  function shouldScheduleScan", chat_start)
    chat_code = text[chat_start:chat_end]

    assert "refreshConversationTimeline()" in scan_deferred_code
    assert ".codex-conversation-timeline" in extension_code
    assert "[data-message-author-role]" in relevant_code
    assert "[data-testid=\"conversation-turn\"]" in relevant_code
    assert "[class*=\"user-message\"]" in relevant_code
    assert "nodeLooksLikeTimelineQuestion(node)" in text
    assert "main .prose" in chat_code
    assert "return false" in chat_code



def test_renderer_script_timeline_uses_stable_hover_and_scroll_behavior():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "top: calc(72px + 12px)" in text
    assert "bottom: calc(28px + 12px)" in text
    assert "width: max-content" in text
    assert "max-width: min(320px, calc(100vw - 72px))" in text
    assert "overflow: hidden" in text
    assert "text-overflow: ellipsis" in text
    assert "z-index: 2147482501" in text
    assert "pointer-events: none" in text
    assert "scrollTimelineTarget" in text
    assert "nearestTimelineScroller" in text
    assert "timelineScrollerViewportTop(scroller)" in text
    assert "scrollTo({" in text
    assert "behavior: \"smooth\"" in text
    assert "aria-describedby" in text
    assert "role\", \"tooltip\"" in text
    assert "keydown" in text[text.index("function createConversationTimelineMarker"):text.index("\n\n  function prepareTimelineQuestions")]
    assert "click" not in text[text.index("function createConversationTimelineMarker"):text.index("\n\n  function refreshConversationTimeline")]
    assert "pointerup" in text[text.index("function createConversationTimelineMarker"):text.index("\n\n  function refreshConversationTimeline")]



def test_renderer_script_timeline_positions_questions_by_scroll_location():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function timelineScrollerViewportTop")
    end = text.index("\n\n  function removeConversationTimeline", start)
    marker_top_code = text[start:end]

    assert "timelineRawMarkerTop" in marker_top_code
    assert "timelineMarkerTops" in marker_top_code
    assert "getBoundingClientRect" in marker_top_code
    assert "timelineScrollableHeight(scroller)" in marker_top_code
    assert "timelineMaxMarkerGapPercent" in marker_top_code
    assert "questions.indexOf(question)" not in marker_top_code



def test_renderer_script_timeline_has_codex_plus_menu_toggle():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "conversationTimeline: true" in text
    assert "对话 Timeline" in text
    assert "data-codex-plus-setting=\"conversationTimeline\"" in text
    assert "codexPlusSettings().conversationTimeline" in text
    assert "removeConversationTimeline()" in text[text.index("function refreshConversationTimeline"):text.index("\n\n  function scanLightweight")]



def test_renderer_script_enables_plugin_entry_for_api_key_users():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function pluginEntryButton")
    end = text.index("\n\n  function unblockPluginInstallButtons", start)
    plugin_entry_code = text[start:end]
    assert "enablePluginEntry" in plugin_entry_code
    assert "pluginEntryButton" in plugin_entry_code
    assert "nav[role=\"navigation\"] button.h-token-nav-row.w-full" in text
    assert "svg path[d^=\"M7.94562 14.0277\"]" in text
    assert "selectors.pluginNavButton" in plugin_entry_code
    assert "selectors.pluginSvgPath" in plugin_entry_code
    assert "document.querySelectorAll(\"button\")" not in plugin_entry_code
    assert "disabled = false" in plugin_entry_code
    assert "removeAttribute(\"disabled\")" in plugin_entry_code
    assert "setAuthMethod(\"chatgpt\")" in text
    assert "插件 - 已解锁" in plugin_entry_code
    assert "Plugins - Unlocked" in plugin_entry_code
    assert "labelUnlockedPluginEntry" in plugin_entry_code
    assert "childNodes" in plugin_entry_code
    assert "node.nodeType === 3" in plugin_entry_code
    assert "labelTextNode.nodeValue" in plugin_entry_code
    assert ".textContent = /^Plugins" not in plugin_entry_code
    assert "__reactFiber" in text
    assert "/skills/plugins" not in text
    assert "skillProps.onClick" not in text


def test_renderer_script_unblocks_connector_unavailable_plugin_install_buttons_without_full_body_text_scan():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function pluginInstallCandidates")
    end = text.index("\n  let cachedSessionRows", start)
    plugin_unlock_code = text[start:end]
    assert "unblockPluginInstallButtons" in plugin_unlock_code
    assert "pluginInstallCandidates" in plugin_unlock_code
    assert "button:disabled.w-full.justify-center" in text
    assert "[role=\"button\"][aria-disabled=\"true\"].cursor-not-allowed" in text
    assert "selectors.disabledInstallButton" in plugin_unlock_code
    assert "document.body.textContent" not in plugin_unlock_code
    assert "button.disabled = false" in plugin_unlock_code
    assert "removeAttribute(\"aria-disabled\")" in plugin_unlock_code
    assert "labelForcedInstallButton" in plugin_unlock_code
    assert "强制安装" in plugin_unlock_code


def test_renderer_script_debounces_mutation_observer_scan():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "scanLightweight" in text
    assert "scanDeferred" in text
    assert "runScanStep" in text
    assert "codexSessionDeleteScanFailures" in text
    assert "runScanStep(scanLightweight)" in text
    assert "requestAnimationFrame(() => runScanStep(scanDeferred))" in text
    assert "if (window.__codexSessionDeleteScanPending) return" in text
    assert "setTimeout(runScheduledScan, 200)" in text
    assert "setTimeout(() => runScanStep(scanDeferred), 50)" not in text
    assert "codexSessionDeleteAttachButtonFailures" in text
    assert "tryAttachButton" in text
    assert "sessionRows().forEach(tryAttachButton)" in text
    assert "sessionRows().forEach(attachButton)" not in text
    assert "new MutationObserver(scheduleScan)" in text
    assert "new MutationObserver(scan)" not in text
    assert "scan();" in text
    assert "window.__codexProjectMoveApplyProjection" in text
    assert "window.__codexSessionDeleteObserver" in text


def test_renderer_script_ignores_chat_content_mutations_before_scheduling_scan():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("function isExtensionUiNode")
    end = text.index("\n\n  function runScheduledScan", start)
    should_schedule_code = text[start:end]
    assert "isChatContentMutation" in should_schedule_code
    assert "data-message-author-role" in should_schedule_code
    assert "data-testid=\"conversation-turn\"" in should_schedule_code
    assert "main .prose" in should_schedule_code
    assert "if (isChatContentMutation(mutation)) return false" in should_schedule_code
    should_start = text.index("function shouldScheduleScan")
    should_end = text.index("\n\n  function runScheduledScan", should_start)
    should_schedule_only = text[should_start:should_end]
    assert "nodeSelfOrAncestorMatchesScanRelevance(target)" in should_schedule_only
    assert "const changedNodes = [...Array.from(mutation.addedNodes), ...Array.from(mutation.removedNodes)]" in should_schedule_only
    assert "changedNodes.some((node) => node.nodeType === 1 && isScanRelevantNode(node))" in should_schedule_only
    assert "Array.from(mutation.addedNodes).some((node) => node.nodeType === 1 && isScanRelevantNode(node))" in should_schedule_code
    assert "selectors.sidebarThread" in should_schedule_code
    assert "selectors.appHeader" in should_schedule_code


def test_renderer_script_chat_filter_keeps_relevant_node_escape_hatch():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    start = text.index("const scanRelevantSelector")
    end = text.index("\n\n  function isChatContentMutation", start)
    relevant_code = text[start:end]
    assert "node.matches?.(scanRelevantSelector)" in relevant_code
    assert "node.querySelector?.(scanRelevantSelector)" in relevant_code
    assert "nodeLooksLikeTimelineQuestion(node)" in relevant_code
    assert "selectors.archiveNav" in relevant_code
    assert "selectors.disabledInstallButton" in relevant_code
    assert "button[aria-label=\"已归档对话\"]" in text
    assert "button:disabled.w-full.justify-center" in text
    assert "[role=\"button\"][aria-disabled=\"true\"].cursor-not-allowed" in text


def test_renderer_script_clears_focus_and_removes_deleted_rows():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "removeDeletedRow(row, button, ref)" in text
    assert "function releaseDeleteFocus" in text
    assert "releaseDeleteFocus(row, button)" in text
    assert "button.blur()" in text
    assert "document.activeElement.blur()" in text
    assert "row.remove()" in text
    assert "row.style.display = \"none\"" not in text


def test_renderer_script_uses_in_page_confirm_and_stops_early_pointer_events():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "confirm(" not in text
    assert "codex-delete-confirm-overlay" in text
    assert "escapeHtml(title)" in text
    assert "stopImmediatePropagation" in text
    assert "\"pointerdown\", \"mousedown\", \"mouseup\", \"touchstart\"" in text


def test_renderer_script_reloads_after_deleting_current_session():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "isCurrentSessionRow" in text
    assert "window.location.href.includes(ref.session_id)" in text
    assert "window.location.reload()" in text


def test_renderer_script_toast_does_not_capture_page_interactions():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "z-index: 2147483000" in text
    assert "pointer-events: none" in text
    assert "pointer-events: auto" in text
def test_renderer_script_sidebar_delete_opens_on_pointerup_when_click_is_unreliable():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "openDeleteConfirm" in text
    assert "codexDeleteVersion = \"6\"" in text
    assert "actionGroupFromRow" in text
    assert "removeActionGroups(row)" in text
    assert "row.dataset.codexDeleteRow = \"false\"" in text
    assert "installDeleteButtonEventDelegation" in text
    assert "codexSessionDeleteDocumentDeleteHandler" in text
    assert "document.addEventListener(\"pointerup\", handler, true)" in text
    assert "document.addEventListener(\"click\", handler, true)" in text
    assert "deleteButton.dataset.codexDeleteVersion = codexDeleteVersion" in text


    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "updateDeleteButtonOffsets" in text
    assert "codexDeleteStyleVersion = \"8\"" in text
    assert "right: 66px" in text
    assert "确认" in text
    assert "归档对话" in text
    assert "button.getAttribute(\"aria-label\")" in text
    assert "label === \"归档对话\"" in text
    assert "button.classList.contains(exportButtonClass)" in text


    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    archive_visible_start = text.index("function archivedPageVisible")
    archive_visible_end = text.index("\n\n  function sessionRefFromRow", archive_visible_start)
    archive_visible_code = text[archive_visible_start:archive_visible_end]
    assert "archivePageHintVisible" in text
    assert "button[aria-label=\"已归档对话\"]" in text
    assert "button[aria-label=\"Archived conversations\"]" in text
    assert "bg-token-list-hover-background" in text
    assert "archivedPageVisible" in text
    assert "document.body.textContent" not in archive_visible_code
    assert "archivedSessionRows" in text
    assert "archivedPageRows" in text
    assert "installArchivedDeleteAllButton" in text
    assert "if (!archivePageHintVisible()) return []" in text
    assert "if (!archivePageHintVisible())" in text
    assert "删除全部归档" in text
    assert "deleteArchivedSessions" in text
    assert "attachArchivedPageDeleteButton" in text
    assert "resolveArchivedThread" in text
    assert "stopArchivedButtonEvent" in text
    assert "[\"pointerdown\", \"mousedown\", \"mouseup\", \"touchstart\"].forEach((eventName) => {\n      button.addEventListener(eventName, stopArchivedButtonEvent, true);" in text
    assert "pointerup" in text
    assert "button.addEventListener(\"pointerup\", openArchivedDeleteAllConfirm, true)" in text
    assert "archivedRefFromRow(row)" in text
    assert "reactArchivedThreadFromNode" in text
    assert "archivedThreadFromRow" in text
    assert "props.archivedThread?.id" in text
    assert "archivedThread.id || archivedThread.sessionId" in text
    assert "replace(/\\d{4}年\\d{1,2}月\\d{1,2}日.*$/, \"\")" in text
    assert "const titleMatches = sessionRows().map(sessionRefFromRow)" not in text
    assert "document.querySelectorAll(\"[data-codex-archive-delete-all]\").forEach((node) => node.remove())" not in text
    assert "const existingButton = document.querySelector(\"[data-codex-archive-delete-all]\")" in text
    assert "if (existingButton?.dataset.codexArchiveDeleteAllVersion === codexArchiveDeleteAllVersion) return" in text
    assert "existingButton?.remove()" in text
    assert "button.dataset.codexArchiveDeleteAllVersion = codexArchiveDeleteAllVersion" in text
    assert "data-codex-archive-delete-all" in text
    assert "codex-archive-action-bar" in text
    assert "codexDeleteStyleVersion" in text
    assert "style.dataset.codexDeleteStyleVersion" in text
    assert "position: fixed" in text
    assert "archiveTitleContainer" in text
    assert "element.getBoundingClientRect().x > 350" in text
    assert "已归档对话" in text
    assert "insertAdjacentElement(\"afterend\", button)" in text
    assert "maxWidth: \"fit-content\"" in text
    assert "alignSelf: \"flex-start\"" in text
    assert "Object.assign(button.style" in text
    assert "cursor: \"pointer\"" in text
    assert "position: \"static\"" in text
    assert "data-codex-archive-page-row" in text
    assert "data-app-action-sidebar-thread-id" in text
    assert "取消归档" in text
    assert "已归档对话" in text
    assert "archiveRowFromUnarchiveButton" in text
    assert "[role=\"listitem\"], [role=\"row\"]" in text
    assert "Archived conversations" in text
    assert "data-codex-archive-row-action" in text
    assert "textContent = \"导出\"" in text
    assert "textContent = \"删除\"" in text
    assert "insertAdjacentElement(\"afterend\", exportButton)" in text
    assert "insertAdjacentElement(\"afterend\", deleteButton)" in text


def test_renderer_script_uses_bridge_only_helper_calls():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    helper_code = text[:text.index("const codexPlusAdsUrl")]
    assert "window.__codexSessionDeleteBridge" in text
    assert "fetch(" not in helper_code
    assert "XMLHttpRequest" not in text
    assert "postJson(\"/delete\"" in text
    assert "postJson(\"/undo\"" in text
    assert "postJson(\"/archived-thread\"" in text
    assert "postJson(\"/export-markdown\"" in text
    assert "Blob([markdown]" in text


def test_renderer_script_uses_chinese_delete_toast_fallbacks():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "删除成功" in text
    assert "删除失败" in text
    assert "撤销完成" in text
    assert "Delete failed" not in text
    assert "Deleted\"" not in text
    assert "Undo finished" not in text


def test_renderer_script_does_not_include_fast_mode_patch():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")
    assert "codexFastModeUnlockVersion" not in text
    assert "enableFastModeFeatureFlags" not in text
    assert "patchFastModeGates" not in text
    assert "patchGeneralSettingsSpeedGate" not in text
    assert "patchCodexPostForFastMode" not in text
    assert "recordFastModeDiagnostic" not in text
    assert "additionalSpeedTiers" not in text
    assert "bodyJsonString" not in text
    assert "forceChatGPTAuthForFastMode" not in text
    assert "codex-fast-mode-row" not in text


def test_renderer_script_includes_user_script_manager_ui_contract():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "用户脚本" in text
    assert "启用用户脚本" in text
    assert "重新加载用户脚本" in text
    assert "禁用后需重载页面或重启 Codex++" in text
    assert "codexPlusUserScripts" in text
    assert "loadUserScripts" in text
    assert "renderUserScripts" in text
    assert "data-codex-user-scripts-enabled" in text
    assert "data-codex-user-script-key" in text
    assert "data-codex-user-scripts-reload" in text
    assert "/user-scripts/list" in text
    assert "/user-scripts/set-enabled" in text
    assert "/user-scripts/set-script-enabled" in text
    assert "/user-scripts/reload" in text
    assert "codex-plus-tab-button" in text
    assert "data-codex-plus-tab=\"home\"" in text
    assert "data-codex-plus-tab=\"userScripts\"" in text
    assert "data-codex-plus-panel=\"home\"" in text
    assert "data-codex-plus-panel=\"userScripts\"" in text
    assert "selectCodexPlusTab" in text
    assert "打开 DevTools" in text
    assert "data-codex-open-devtools" in text
    assert "/devtools/open" in text
    assert "后端连接" in text
    assert "data-codex-backend-status" in text
    assert "data-codex-backend-repair" in text
    assert "checkBackendStatus" in text
    assert "renderBackendStatus" in text
    assert "scheduleBackendHeartbeat" in text
    assert "setInterval(checkBackendStatus, 5000)" in text
    assert "scheduleBackendHeartbeat();\n    loadUserScripts();" not in text
    assert "installCodexPlusMenu();\n    scheduleBackendHeartbeat();" in text
    assert "withBackendTimeout" in text
    assert "setTimeout(() => resolve({ status: \"failed\", message: \"后端已断开\" }), 2000)" in text
    assert "data-codex-backend-indicator" in text
    assert "codex-plus-backend-indicator" in text
    assert "/backend/status" in text
    assert "/backend/repair" in text

    assert "setAuthMethod(\"chatgpt\")" in text
    assert "patchFastModeGateOnObject" not in text
    assert "Codex++" in text
    assert "codexPlusVersion = \"1.0.7\"" in text
    assert "Codex++ ${codexPlusVersion}" in text
    assert "提出问题" in text
    assert "https://github.com/BigPizzaV3/CodexPlusPlus/issues" in text
    assert "window.open(issueUrl, \"_blank\")" in text
    assert "插件选项解锁" in text
    assert "特殊插件强制安装" in text
    assert "会话删除" in text
    assert "Markdown 导出" in text
    assert "原生菜单栏位置" in text
    assert "nativeMenuPlacement: true" in text
    assert "关于 Codex++" in text
    assert "https://github.com/BigPizzaV3/CodexPlusPlus" in text
    assert "codexPlusSettings" in text
    assert "pluginEntryUnlock" in text
    assert "forcePluginInstall" in text
    assert "sessionDelete" in text
    assert "markdownExport" in text
    assert "projectMove" in text
    assert "会话项目移动" in text
    assert "移动按钮" in text
    assert "codex-plus-modal-overlay" in text
    assert "codex-plus-modal-content" in text
    assert "codex-plus-modal-header" in text
    assert "codex-dialog-overlay" not in text
    assert "bg-token-dropdown-background/90" not in text
    assert "backdrop-blur-xl" not in text
    assert "codex-plus-menu-floating" in text
    assert "findNativeMenuInsertionPoint" in text
    assert "if (!codexPlusSettings().nativeMenuPlacement) return null" in text
    assert "right: var(--codex-plus-menu-right, 140px)" in text
    assert "left: auto" in text
    assert "pointer-events: auto" in text
    assert "-webkit-app-region: no-drag" in text
    assert ".codex-plus-trigger" in text
    assert "app-header-tint" in text
    assert "flex items-center gap-0.5" in text
    assert "codex-plus-menu-floating" in text
    assert "nativeButtonClass" in text
    assert "removeDuplicateCodexPlusMenus" in text
    assert "data-codex-plus-menu" in text
    assert "textContent || \"\").trim() === `Codex++ ${codexPlusVersion}`" in text
    assert "codexPlusMenuVersion !== \"6\"" in text
    assert "codexPlusTriggerInstalled = \"5\"" in text
    assert ".codex-plus-trigger:hover" not in text
    assert "function headerTitleRegion" in text
    assert "function isHeaderToolbarButton" in text
    assert 'button.closest(".ms-auto.flex.shrink-0.items-center")' in text
    assert "const titleRegion = headerTitleRegion(header);" in text
    assert "if (titleRegion?.contains?.(button)) return false;" in text
    assert ".map((button) => ({ button, rect: button.getBoundingClientRect() }))" in text
    assert ".filter(({ button, rect }) => isHeaderToolbarButton(button, header, rect))" in text


def test_renderer_script_has_sponsor_tab():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "data-codex-plus-tab=\"sponsor\"" in text
    assert "data-codex-plus-tab=\"support\"" in text
    assert "推荐内容" in text
    assert "请作者喝咖啡" in text
    assert "请我喝杯咖啡" in text
    assert "data-codex-plus-panel=\"sponsor\"" in text
    assert "data-codex-plus-panel=\"support\"" in text
    assert "window.__CODEX_PLUS_SPONSOR_IMAGES__?.alipay" in text
    assert "window.__CODEX_PLUS_SPONSOR_IMAGES__?.wechat" in text
    assert "codex-plus-sponsor-grid" in text
    assert "codex-plus-modal-content[data-codex-plus-active-tab=\"support\"]" in text
    assert "codex-plus-modal-content[data-codex-plus-active-tab=\"sponsor\"]" not in text
    assert "width: min(820px, calc(100vw - 48px))" in text
    assert "grid-template-columns: repeat(2, minmax(0, 1fr))" in text
    assert "max-width: 340px" in text
    assert "codex-plus-sponsor-qr" in text


def test_renderer_script_has_backend_provider_sync_toggle():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "Provider 同步" in text
    assert "切换供应商（model_provider）时不丢任何历史会话" in text
    assert "避免历史对话因为供应商切换而消失" in text
    assert "data-codex-backend-setting=\"providerSyncEnabled\"" in text
    assert "/settings/get" in text
    assert "/settings/set" in text
    assert "loadBackendSettings" in text
    assert "setBackendSetting" in text


def test_renderer_script_can_move_sidebar_threads_between_projects():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "codex-project-move-button" in text
    assert "codex-project-move-overlay" in text
    assert "codexProjectMoveVersion = \"1\"" in text
    assert "function moveSessionToProjectless" in text
    assert "function moveSessionToProject" in text
    assert "function projectMoveTargets" in text
    assert "function nativeProjectTargets" in text
    assert "data-app-action-sidebar-project-row" in text
    assert "data-app-action-sidebar-project-id" in text
    assert "data-app-action-sidebar-project-label" in text
    assert "get-global-state" in text
    assert "set-global-state" in text
    assert "projectless-thread-ids" in text
    assert "thread-workspace-root-hints" in text
    assert "electron-saved-workspace-roots" not in text
    assert "active-workspace-roots" not in text
    assert "project-order" not in text
    assert "function threadIdVariants" in text
    assert '`local:${bareId}`' in text
    assert "uniqueValues([...ids, ...variants])" in text
    assert "const variantSet = new Set(variants)" in text
    assert "ids.filter((id) => !variantSet.has(id))" in text
    assert "/thread-workspaces" not in text
    assert "/move-thread-workspace" in text
    assert "/thread-sort-key" in text
    assert "/thread-sort-keys" in text
    assert "hintKeys.forEach((id) => delete hints[id])" in text
    assert "hints[id] = targetCwd" not in text
    assert "codexProjectMoveProjection" in text
    assert "legacyProjectMoveOverridesKey" in text
    assert "function applyProjectMoveProjection" in text
    assert "scheduleProjectMoveProjection" in text
    assert "saveProjectMoveProjection(ref, target, target.sortMs || rowSortMs(row, ref, target))" in text
    assert "clearProjectMoveProjection(ref)" in text
    assert "refresh-recent-conversations-for-host" in text
    assert "function refreshAfterProjectMove" in text
    assert "function insertRowItemByTime" in text
    assert "function sortStateFromMoveResult" in text
    assert "function timestampMsFromPayload" in text
    assert "function relativeTimeLabel" in text
    assert "function updateRowTimeLabel" in text
    assert "dataset.codexProjectMoveTime" in text
    assert "function rowTimeLabelCandidates" in text
    assert "function cleanupRowTimeLabels" in text
    assert "function cleanupManagedStatusIconTimeNodes" in text
    assert "function nodeInsideStatusIcon" in text
    assert "function nodeLooksLikeTimeLabel" in text
    assert "className.includes(\"animate-spin\")" in text
    assert "node.children.length > 0" in text
    assert "data-codex-project-move-time-wrapper" in text
    assert "node.dataset?.codexProjectMoveTime !== \"true\"" in text
    assert "function rowSortMs" in text
    assert "function uuidV7TimestampMs" in text
    assert "function projectThreadList" in text
    assert "function applyChatsSortCorrection" in text
    assert "function scheduleChatsSortCorrection" in text
    assert "function reorderChatsRows" in text
    assert "window.__codexProjectMoveSortChats" in text
    assert "window.__codexProjectMoveRuntimeId" in text
    assert "__codexProjectMoveChatsSortTimer" in text
    assert "sortMsTrusted" in text
    assert "chatsSortDbRefreshIntervalMs" in text
    assert "data-app-action-sidebar-section-heading=\"Chats\"" in text
    assert "data-app-action-sidebar-project-list-id" in text
    assert "codexProjectMoveSortMs" in text
    assert "data-codex-project-move-injected-list" in text
    assert "codex-project-move-hidden" in text
    assert "window.__codexProjectMoveApplyProjection" in text
    assert "window.__codexProjectMoveTargets" in text
    assert "projectMoveButtonClass" in text
    assert "openProjectMoveMenuForRow" in text
    assert "existingMoveButton" in text
    assert "普通对话" in text
