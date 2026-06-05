import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { CircleArrowUp, Moon, RefreshCw, Rocket, Sun } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { AboutScreen, ContextScreen, EnhanceScreen, MaintenanceScreen, NoticeDialog, OverviewScreen, RecommendationsScreen, RelayScreen, SessionsScreen, SettingsScreen, UserScriptsScreen } from "./components/screens";
import { DEFAULT_DEBUG_PORT, DEFAULT_HELPER_PORT, DEFAULT_LOG_LINE_COUNT, PROVIDER_SYNC_PROGRESS, STORAGE_KEYS, defaultSettings, providerSyncProgressMessage, routes, syncMarketInstalledState, type AdsResult, type BackendSettings, type CodexContextEntries, type CommandResult, type ContextEntriesResult, type ContextKind, type DeleteLocalSessionResult, type DiagnosticsResult, type ExtractRelayCommonConfigResult, type InstallResult, type LaunchMode, type LiveContextEntriesResult, type LocalSession, type LocalSessionsResult, type LogsResult, type OverviewResult, type ProviderSyncPayload, type ProviderSyncProgress, type ProviderSyncTargetsResult, type RelayFilesResult, type RelayProfile, type RelayProfileModelsResult, type RelayProfileTestResult, type RelayResult, type Route, type ScriptMarketResult, type SettingsBackfillResult, type SettingsResult, type StartupResult, type Status, type Theme, type UpdateResult, type WatcherResult } from "./components/screens/model";
import { activeRelayProfile, isSuccessStatus, loadInitialRoute, loadInitialTheme, mergeLiveLinkedRelayProfiles, normalizeSettings, numberOrDefault, relayProfileModeSwitchedText, relayProfileReadinessText, relayProfileSwitchCommand, relayProfileSwitchValidation, routeSubtitleKey, routeTitleKey, stringifyError, syncLegacyRelayFields } from "./components/screens/utils";
export function App() {
    const { t, i18n: i18nInstance } = useTranslation();
    const [theme, setTheme] = useState<Theme>(() => loadInitialTheme());
    const [route, setRoute] = useState<Route>(() => loadInitialRoute());
    const [notice, setNotice] = useState<{
        title: string;
        message: string;
        status?: Status;
    } | null>(null);
    const [overview, setOverview] = useState<OverviewResult | null>(null);
    const [settings, setSettings] = useState<SettingsResult | null>(null);
    const [relayFiles, setRelayFiles] = useState<RelayFilesResult | null>(null);
    const [localSessions, setLocalSessions] = useState<LocalSessionsResult | null>(null);
    const [liveContextEntries, setLiveContextEntries] = useState<CodexContextEntries | null>(null);
    const [logs, setLogs] = useState<LogsResult | null>(null);
    const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
    const [watcher, setWatcher] = useState<WatcherResult | null>(null);
    const [update, setUpdate] = useState<UpdateResult | null>(null);
    const [ads, setAds] = useState<AdsResult | null>(null);
    const [scriptMarket, setScriptMarket] = useState<ScriptMarketResult | null>(null);
    const [launchForm, setLaunchForm] = useState({
        appPath: "",
        debugPort: String(DEFAULT_DEBUG_PORT),
        helperPort: String(DEFAULT_HELPER_PORT),
    });
    const [settingsForm, setSettingsForm] = useState<BackendSettings>({ ...defaultSettings });
    const [providerSyncProgress, setProviderSyncProgress] = useState<ProviderSyncProgress>({
        active: false,
        percent: 0,
        message: t("notify.noProviderSync"),
        result: null,
    });
    const [providerSyncTargets, setProviderSyncTargets] = useState<ProviderSyncTargetsResult | null>(null);
    const [selectedProviderSyncTarget, setSelectedProviderSyncTarget] = useState("");
    const [removeOwnedData, setRemoveOwnedData] = useState(false);
    const call = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);
    const logDiagnostic = (event: string, detail: Record<string, unknown> = {}) => {
        void invoke("write_diagnostic_event", { event, detail }).catch(() => { });
    };
    const run = async <T,>(task: () => Promise<T>): Promise<T | null> => {
        try {
            return await task();
        }
        catch (error) {
            showNotice(t("notify.callFailed"), stringifyError(error), "failed");
            return null;
        }
    };
    const refreshOverview = async (silent = false) => {
        const result = await run(() => call<OverviewResult>("load_overview"));
        if (result) {
            setOverview(result);
            if (!silent)
                showResultNotice(t("notify.overviewChecked"), result, { silentSuccess: true });
        }
    };
    const refreshSettings = async (silent = false) => {
        const result = await run(() => call<SettingsResult>("load_settings"));
        if (result) {
            setSettings(result);
            const normalized = normalizeSettings(result.settings);
            setSettingsForm(normalized);
            setLaunchForm((current) => ({
                ...current,
                appPath: current.appPath || result.settings.codexAppPath || "",
            }));
            if (!silent)
                showResultNotice(t("notify.settingsLoaded"), result, { silentSuccess: true });
            return normalized;
        }
        return null;
    };
    const refreshScriptMarket = async (silent = false) => {
        const result = await run(() => call<ScriptMarketResult>("refresh_script_market"));
        if (result) {
            setScriptMarket(result);
            setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
            if (!silent || !isSuccessStatus(result.status))
                showResultNotice(t("notify.scriptMarket"), result, { silentSuccess: true });
        }
    };
    const installMarketScript = async (id: string) => {
        const result = await run(() => call<ScriptMarketResult>("install_market_script", { id }));
        if (result) {
            setScriptMarket(result);
            setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
            showResultNotice(t("notify.scriptMarket"), result);
        }
    };
    const setUserScriptEnabled = async (key: string, enabled: boolean) => {
        const result = await run(() => call<SettingsResult>("set_user_script_enabled", { key, enabled }));
        if (result) {
            setSettings(result);
            setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
            showResultNotice(t("notify.localScripts"), result);
        }
    };
    const deleteUserScript = async (key: string) => {
        const script = settings?.user_scripts?.scripts?.find((item) => item.key === key);
        const name = script?.name || key;
        if (!window.confirm(t("confirm.deleteScript", { name })))
            return;
        const result = await run(() => call<SettingsResult>("delete_user_script", { key }));
        if (result) {
            setSettings(result);
            setScriptMarket((current) => syncMarketInstalledState(current, result.user_scripts));
            showResultNotice(t("notify.localScripts"), result);
        }
    };
    const refreshRelay = async (silent = false) => {
        const result = await run(() => call<RelayResult>("relay_status"));
        if (result) {
            if (!silent)
                showResultNotice(t("notify.loginStatus"), result, { silentSuccess: true });
        }
    };
    const refreshRelayFiles = async (silent = false) => {
        const result = await run(() => call<RelayFilesResult>("read_relay_files"));
        if (result) {
            setRelayFiles(result);
            if (!silent)
                showResultNotice(t("notify.configFiles"), result, { silentSuccess: true });
        }
        return result;
    };
    const refreshLocalSessions = async (silent = false) => {
        const result = await run(() => call<LocalSessionsResult>("list_local_sessions"));
        if (result) {
            setLocalSessions(result);
            if (!silent || !isSuccessStatus(result.status))
                showResultNotice(t("notify.sessionManagement"), result, { silentSuccess: true });
        }
        return result;
    };
    const deleteLocalSession = async (session: LocalSession) => {
        const title = session.title || session.id;
        if (!window.confirm(t("confirm.deleteSession", { title })))
            return;
        const result = await run(() => call<DeleteLocalSessionResult>("delete_local_session", {
            request: { sessionId: session.id, title: session.title },
        }));
        if (result) {
            showResultNotice(t("notify.sessionDelete"), result);
            await refreshLocalSessions(true);
        }
    };
    const refreshLiveContextEntries = async (silent = false) => {
        const result = await run(() => call<LiveContextEntriesResult>("read_live_context_entries"));
        if (result) {
            setLiveContextEntries(result.entries);
            if (!silent || !isSuccessStatus(result.status))
                showResultNotice(t("notify.toolsAndPlugins"), result, { silentSuccess: true });
        }
        return result;
    };
    const syncLiveContextEntries = async (next: BackendSettings, silent = false) => {
        const result = await run(() => call<LiveContextEntriesResult>("sync_live_context_entries", { request: { settings: next } }));
        if (result) {
            setLiveContextEntries(result.entries);
            if (!silent || !isSuccessStatus(result.status))
                showResultNotice(t("notify.toolsAndPlugins"), result, { silentSuccess: true });
        }
        return result;
    };
    const refreshLogs = async (silent = false) => {
        const result = await run(() => call<LogsResult>("read_latest_logs", { request: { lines: DEFAULT_LOG_LINE_COUNT } }));
        if (result) {
            setLogs(result);
            if (!silent)
                showResultNotice(t("notify.logsRefreshed"), result, { silentSuccess: true });
        }
    };
    const refreshDiagnostics = async (silent = false) => {
        const result = await run(() => call<DiagnosticsResult>("copy_diagnostics"));
        if (result) {
            setDiagnostics(result);
            if (!silent)
                showResultNotice(t("notify.diagnosticsGenerated"), result, { silentSuccess: true });
        }
    };
    const refreshWatcher = async (silent = false) => {
        const result = await run(() => call<WatcherResult>("load_watcher_state"));
        if (result) {
            setWatcher(result);
            if (!silent)
                showResultNotice(t("notify.watcherStatus"), result, { silentSuccess: true });
        }
    };
    const navigate = async (next: Route) => {
        setRoute(next);
        if (next === "overview")
            await refreshOverview(true);
        if (next === "relay") {
            await refreshSettings(true);
            await refreshRelay(true);
            await refreshRelayFiles(true);
        }
        if (next === "sessions") {
            await refreshSettings(true);
            await refreshLocalSessions(true);
            await refreshProviderSyncTargets(true);
        }
        if (next === "context") {
            await refreshSettings(true);
            await refreshRelayFiles(true);
            await refreshLiveContextEntries(true);
        }
        if (next === "settings")
            await refreshSettings(true);
        if (next === "userScripts") {
            await refreshSettings(true);
            await refreshScriptMarket(true);
        }
        if (next === "recommendations")
            await refreshAds(true);
        if (next === "about") {
            await refreshOverview(true);
            await refreshLogs(true);
            await refreshDiagnostics(true);
        }
        if (next === "maintenance") {
            await refreshOverview(true);
            await refreshWatcher(true);
        }
    };
    const launch = async () => {
        const result = await launchCommand("launch_codex_plus");
        if (result) {
            showNotice(t("notify.launchTask"), result.message, result.status);
            await refreshOverview(true);
        }
    };
    const restart = async () => {
        const result = await launchCommand("restart_codex_plus");
        if (result) {
            showNotice(t("notify.restart"), result.message, result.status);
            await refreshOverview(true);
        }
    };
    const launchCommand = async (command: "launch_codex_plus" | "restart_codex_plus") => {
        const result = await run(() => call<CommandResult<Record<string, unknown>>>(command, {
            request: {
                appPath: launchForm.appPath,
                debugPort: numberOrDefault(launchForm.debugPort, DEFAULT_DEBUG_PORT),
                helperPort: numberOrDefault(launchForm.helperPort, DEFAULT_HELPER_PORT),
            },
        }));
        return result;
    };
    const repairBackend = async () => {
        const result = await run(() => call<SettingsResult>("repair_backend"));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            showNotice(t("notify.backendRepair"), result.message, result.status);
        }
    };
    const installEntrypoints = async () => {
        const result = await run(() => call<InstallResult>("install_entrypoints"));
        if (result) {
            showNotice(t("notify.entryInstall"), result.message, result.status);
            await refreshOverview(true);
        }
    };
    const uninstallEntrypoints = async () => {
        const result = await run(() => call<InstallResult>("uninstall_entrypoints", {
            options: { removeOwnedData },
        }));
        if (result) {
            showNotice(t("notify.entryUninstall"), result.message, result.status);
            await refreshOverview(true);
        }
    };
    const repairShortcuts = async () => {
        const result = await run(() => call<InstallResult>("repair_shortcuts"));
        if (result) {
            showNotice(t("notify.shortcutRepair"), result.message, result.status);
            await refreshOverview(true);
        }
    };
    const watcherAction = async (command: string) => {
        const result = await run(() => call<WatcherResult>(command));
        if (result) {
            setWatcher(result);
            showNotice(t("notify.watcherAction"), result.message, result.status);
        }
    };
    const checkUpdate = async (silent = false) => {
        const result = await run(() => call<UpdateResult>("check_update"));
        if (result) {
            setUpdate(result);
            if (!silent || result.updateAvailable) {
                showNotice(t("notify.githubReleaseCheck"), result.message, result.status);
            }
        }
    };
    const performUpdate = async () => {
        const release = update?.latestVersion && update.assetName && update.assetUrl
            ? {
                version: update.latestVersion,
                url: "",
                body: update.releaseSummary ?? "",
                asset_name: update.assetName,
                asset_url: update.assetUrl,
            }
            : null;
        const result = await run(() => call<UpdateResult>("perform_update", { release }));
        if (result) {
            setUpdate(result);
            showNotice(t("notify.updateInstall"), result.message, result.status);
        }
    };
    const saveSettings = async () => {
        const next = await settingsForSave(settingsForm, false);
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            showNotice(t("notify.settingsSave"), result.message, result.status);
        }
    };
    const saveSettingsValue = async (next: BackendSettings, silent = true, preserveLinkedProfiles = false) => {
        const normalized = normalizeSettings(next);
        setSettingsForm(normalized);
        const settingsToSave = await settingsForSave(normalized, preserveLinkedProfiles);
        const result = await run(() => call<SettingsResult>("save_settings", { settings: settingsToSave }));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            if (!silent || !isSuccessStatus(result.status))
                showNotice(t("notify.settingsSave"), result.message, result.status);
        }
    };
    const settingsForSave = async (next: BackendSettings, preserveLinkedProfiles: boolean) => {
        const normalized = normalizeSettings(next);
        if (!normalized.ccsLinkEnabled || preserveLinkedProfiles)
            return normalized;
        const refreshed = await refreshSettings(true);
        if (!refreshed)
            return normalized;
        return mergeLiveLinkedRelayProfiles(normalized, normalizeSettings(refreshed));
    };
    const importCcsProviders = async () => {
        const result = await run(() => call<SettingsResult>("import_ccs_providers"));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            showResultNotice(t("notify.ccsLink"), result);
        }
    };
    const resetSettings = async () => {
        const result = await run(() => call<SettingsResult>("reset_settings"));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            showNotice(t("notify.settingsReset"), result.message, result.status);
        }
    };
    const refreshAds = async (silent = false) => {
        const result = await run(() => call<AdsResult>("load_ads"));
        if (result) {
            setAds(result);
            if (!silent)
                showResultNotice(t("notify.recommendations"), result, { silentSuccess: true });
        }
    };
    const refreshProviderSyncTargets = async (silent = false) => {
        const result = await run(() => call<ProviderSyncTargetsResult>("load_provider_sync_targets"));
        if (result) {
            setProviderSyncTargets(result);
            const targets = result.targets ?? [];
            const saved = settingsForm.providerSyncLastSelectedProvider;
            const preferred = targets.find((target) => target.id === saved)?.id ||
                targets.find((target) => target.isCurrentProvider)?.id ||
                targets[0]?.id ||
                "openai";
            setSelectedProviderSyncTarget((current) => (targets.some((target) => target.id === current) ? current : preferred));
            if (!silent && !isSuccessStatus(result.status))
                showNotice(t("notify.providerSyncTarget"), result.message, result.status);
        }
        return result;
    };
    const syncProvidersNow = async () => {
        if (providerSyncProgress.active)
            return;
        setProviderSyncProgress({
            active: true,
            percent: PROVIDER_SYNC_PROGRESS.initialPercent,
            message: selectedProviderSyncTarget ? t("notify.syncingTo", { target: selectedProviderSyncTarget }) : t("notify.scanningSessions"),
            result: null,
        });
        const progressTimer = window.setInterval(() => {
            setProviderSyncProgress((current) => {
                if (!current.active)
                    return current;
                return {
                    ...current,
                    percent: Math.min(PROVIDER_SYNC_PROGRESS.maxPercent, current.percent + PROVIDER_SYNC_PROGRESS.stepPercent),
                    message: current.percent < PROVIDER_SYNC_PROGRESS.markerCheckThreshold ? t("notify.checkingMarkers") : t("notify.writingBackup"),
                };
            });
        }, PROVIDER_SYNC_PROGRESS.tickMs);
        try {
            const targetProvider = selectedProviderSyncTarget || undefined;
            const result = await run(() => call<CommandResult<ProviderSyncPayload>>("sync_providers_now", { targetProvider }));
            if (result) {
                setProviderSyncProgress({
                    active: false,
                    percent: 100,
                    message: providerSyncProgressMessage(result),
                    result,
                });
                if (targetProvider) {
                    const next = {
                        ...settingsForm,
                        providerSyncLastSelectedProvider: targetProvider,
                        providerSyncSavedProviders: Array.from(new Set([...(settingsForm.providerSyncSavedProviders ?? []), targetProvider])).sort(),
                    };
                    setSettingsForm(next);
                }
                await refreshProviderSyncTargets(true);
                showNotice(t("notify.historySessionRepair"), result.message, result.status);
            }
            else {
                setProviderSyncProgress({
                    active: false,
                    percent: 100,
                    message: t("notify.repairFailed"),
                    result: null,
                });
            }
        }
        finally {
            window.clearInterval(progressTimer);
        }
    };
    const applyRelayInjection = async (silent = false) => {
        const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
        if (settingsResult) {
            setSettings(settingsResult);
            setSettingsForm(normalizeSettings(settingsResult.settings));
            if (!isSuccessStatus(settingsResult.status)) {
                showNotice(t("notify.settingsSave"), settingsResult.message, settingsResult.status);
                return false;
            }
        }
        else {
            return false;
        }
        const result = await run(() => call<RelayResult>("apply_relay_injection"));
        if (result) {
            await refreshRelayFiles(true);
            if (!silent || !isSuccessStatus(result.status))
                showNotice(t("notify.officialMixApiKey"), result.message, result.status);
        }
        return !!result && isSuccessStatus(result.status) && result.configured;
    };
    const saveLaunchMode = async (launchMode: LaunchMode, silent = false, baseSettings: BackendSettings = settingsForm) => {
        const next = { ...baseSettings, launchMode };
        setSettingsForm(next);
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
            setSettings(result);
            setSettingsForm(normalizeSettings(result.settings));
            if (!silent)
                showNotice(t("notify.enhancementMode"), result.message, result.status);
        }
        return result;
    };
    const applyPureApiInjection = async (silent = false) => {
        const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
        if (settingsResult) {
            setSettings(settingsResult);
            setSettingsForm(normalizeSettings(settingsResult.settings));
            if (!isSuccessStatus(settingsResult.status)) {
                showNotice(t("notify.settingsSave"), settingsResult.message, settingsResult.status);
                return false;
            }
        }
        else {
            return false;
        }
        const result = await run(() => call<RelayResult>("apply_pure_api_injection"));
        if (result) {
            await refreshRelayFiles(true);
            if (!silent || !isSuccessStatus(result.status))
                showNotice(t("notify.pureApiMode"), result.message, result.status);
        }
        return !!result && isSuccessStatus(result.status) && result.configured;
    };
    const clearRelayInjection = async (silent = false) => {
        const result = await run(() => call<RelayResult>("clear_relay_injection"));
        if (result) {
            await refreshRelayFiles(true);
            if (!silent || !isSuccessStatus(result.status))
                showNotice(t("notify.officialLoginMode"), result.message, result.status);
        }
        return !!result && isSuccessStatus(result.status) && !result.configured;
    };
    const saveRelayFile = async (kind: "config" | "auth", contents: string, silent = false) => {
        const result = await run(() => call<RelayFilesResult>("save_relay_file", { request: { kind, contents } }));
        if (result) {
            setRelayFiles(result);
            if (!silent || !isSuccessStatus(result.status)) {
                showNotice(kind === "config" ? t("configPreview.title") : t("authJson.title"), result.message, result.status);
            }
            await refreshRelay(true);
        }
    };
    const upsertContextEntry = async (next: BackendSettings, kind: ContextKind, id: string, tomlBody: string) => {
        const result = await run(() => call<ContextEntriesResult>("upsert_context_entry", {
            request: { settings: next, kind, id, tomlBody },
        }));
        if (!result)
            return null;
        let normalized = normalizeSettings(result.settings);
        const saveResult = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
        if (saveResult) {
            setSettings(saveResult);
            normalized = normalizeSettings(saveResult.settings);
        }
        setSettingsForm(normalized);
        if (!isSuccessStatus(result.status))
            showResultNotice(t("notify.toolsAndPlugins"), result);
        return normalized;
    };
    const deleteContextEntry = async (next: BackendSettings, kind: ContextKind, id: string) => {
        const result = await run(() => call<ContextEntriesResult>("delete_context_entry", {
            request: { settings: next, kind, id },
        }));
        if (!result)
            return null;
        let normalized = normalizeSettings(result.settings);
        const saveResult = await run(() => call<SettingsResult>("save_settings", { settings: normalized }));
        if (saveResult) {
            setSettings(saveResult);
            normalized = normalizeSettings(saveResult.settings);
        }
        setSettingsForm(normalized);
        if (!isSuccessStatus(result.status))
            showResultNotice(t("notify.toolsAndPlugins"), result);
        return normalized;
    };
    const extractRelayCommonConfig = async (configContents: string) => {
        const result = await run(() => call<ExtractRelayCommonConfigResult>("extract_relay_common_config", {
            request: { configContents },
        }));
        if (result)
            showResultNotice(t("notify.commonConfig"), result);
        return result && isSuccessStatus(result.status) ? result : null;
    };
    const testRelayProfile = async (profile: RelayProfile) => {
        const result = await run(() => call<RelayProfileTestResult>("test_relay_profile", { profile }));
        if (result)
            showNotice(t("notify.providerTest"), result.message, result.status);
    };
    const fetchRelayProfileModels = async (profile: RelayProfile) => {
        const result = await run(() => call<RelayProfileModelsResult>("fetch_relay_profile_models", { profile }));
        if (result)
            showNotice(t("notify.modelList"), result.message, result.status);
        return result && isSuccessStatus(result.status) ? result.models : null;
    };
    const switchOfficialMode = async () => {
        const switched = await clearRelayInjection(true);
        if (!switched)
            return;
        const result = await saveLaunchMode("relay", true);
        if (result)
            showNotice(t("notify.officialLoginMode"), t("notify.switchedToOfficial"), result.status);
    };
    const switchPureApiMode = async () => {
        const switched = await applyPureApiInjection(true);
        if (!switched)
            return;
        const result = await saveLaunchMode("patch", true);
        if (result)
            showNotice(t("notify.pureApiMode"), t("notify.switchedToPureApi"), result.status);
    };
    const switchRelayProfile = async (next: BackendSettings, previousActiveRelayId = settingsForm.activeRelayId) => {
        let switchSettings = normalizeSettings(next);
        if (switchSettings.ccsLinkEnabled) {
            const targetRelayId = switchSettings.activeRelayId;
            const refreshed = await refreshSettings(true);
            if (!refreshed)
                return;
            const latest = normalizeSettings(refreshed);
            if (!latest.relayProfiles.some((profile) => profile.id === targetRelayId)) {
                showNotice(t("notify.providerSwitch"), t("notify.targetProviderMissing"), "failed");
                return;
            }
            switchSettings = syncLegacyRelayFields({ ...latest, activeRelayId: targetRelayId });
        }
        if (!switchSettings.relayProfilesEnabled) {
            showNotice(t("notify.providerConfigDisabled"), t("notify.cannotWriteConfig"), "failed");
            return;
        }
        const targetBeforeSnapshot = activeRelayProfile(switchSettings);
        logDiagnostic("switchRelayProfile.start", {
            currentRelayId: settingsForm.activeRelayId,
            targetRelayId: switchSettings.activeRelayId,
            targetRelayName: targetBeforeSnapshot.name,
            targetRelayMode: targetBeforeSnapshot.relayMode,
            ccsLinkEnabled: switchSettings.ccsLinkEnabled,
        });
        const nextWithSnapshot = await snapshotActiveRelayFilesBeforeSwitch(switchSettings, previousActiveRelayId);
        if (!nextWithSnapshot) {
            logDiagnostic("switchRelayProfile.snapshot_failed", {
                currentRelayId: settingsForm.activeRelayId,
                targetRelayId: switchSettings.activeRelayId,
            });
            return;
        }
        const selectedBeforeSave = activeRelayProfile(nextWithSnapshot);
        const validationError = relayProfileSwitchValidation(selectedBeforeSave);
        if (validationError) {
            logDiagnostic("switchRelayProfile.validation_failed", {
                targetRelayId: selectedBeforeSave.id,
                targetRelayName: selectedBeforeSave.name,
                error: validationError,
            });
            showNotice(t("notify.providerConfigIncorrect"), validationError, "failed");
            return;
        }
        let selectedSettings = nextWithSnapshot;
        logDiagnostic("switchRelayProfile.save_settings_start", {
            targetRelayId: selectedBeforeSave.id,
            targetRelayName: selectedBeforeSave.name,
        });
        const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: nextWithSnapshot }));
        if (settingsResult) {
            selectedSettings = normalizeSettings(settingsResult.settings);
            setSettings(settingsResult);
            setSettingsForm(selectedSettings);
            if (!isSuccessStatus(settingsResult.status)) {
                logDiagnostic("switchRelayProfile.save_settings_failed", {
                    targetRelayId: selectedBeforeSave.id,
                    status: settingsResult.status,
                    message: settingsResult.message,
                });
                showNotice(t("notify.providerSwitch"), settingsResult.message, settingsResult.status);
                return;
            }
        }
        else {
            logDiagnostic("switchRelayProfile.save_settings_no_result", {
                targetRelayId: selectedBeforeSave.id,
            });
            return;
        }
        const selectedAfterSave = activeRelayProfile(selectedSettings);
        const command = relayProfileSwitchCommand(selectedAfterSave);
        logDiagnostic("switchRelayProfile.apply_start", {
            targetRelayId: selectedAfterSave.id,
            targetRelayName: selectedAfterSave.name,
            command,
        });
        const result = await run(() => call<RelayResult>(command));
        if (!result) {
            logDiagnostic("switchRelayProfile.apply_no_result", {
                targetRelayId: selectedAfterSave.id,
                command,
            });
            return;
        }
        await refreshRelayFiles(true);
        if (!isSuccessStatus(result.status) || (selectedAfterSave.relayMode === "pureApi" && !result.configured)) {
            logDiagnostic("switchRelayProfile.apply_failed", {
                targetRelayId: selectedAfterSave.id,
                command,
                status: result.status,
                message: result.message,
                configured: result.configured,
            });
            showNotice(t("notify.providerSwitch"), relayProfileReadinessText(selectedAfterSave, result), result.status);
            return;
        }
        const currentSelected = activeRelayProfile(selectedSettings);
        const launchMode = currentSelected.relayMode === "pureApi" ? "patch" : "relay";
        logDiagnostic("switchRelayProfile.launch_mode_start", {
            targetRelayId: currentSelected.id,
            launchMode,
        });
        const modeResult = await saveLaunchMode(launchMode, true, selectedSettings);
        if (modeResult) {
            logDiagnostic("switchRelayProfile.ok", {
                targetRelayId: currentSelected.id,
                launchMode,
                status: modeResult.status,
            });
            showNotice(t("notify.providerSwitch"), relayProfileModeSwitchedText(currentSelected), modeResult.status);
        }
        else {
            logDiagnostic("switchRelayProfile.launch_mode_no_result", {
                targetRelayId: currentSelected.id,
                launchMode,
            });
        }
    };
    const snapshotActiveRelayFilesBeforeSwitch = async (next: BackendSettings, previousActiveRelayId: string): Promise<BackendSettings | null> => {
        const current = settingsForm.relayProfiles.find((profile) => profile.id === previousActiveRelayId) || activeRelayProfile(settingsForm);
        const selected = activeRelayProfile(next);
        if (current.id === selected.id)
            return next;
        logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.start", {
            currentRelayId: current.id,
            currentRelayName: current.name,
            selectedRelayId: selected.id,
            selectedRelayName: selected.name,
        });
        const result = await run(() => call<SettingsBackfillResult>("backfill_relay_profile_from_live", {
            request: { settings: next, profileId: current.id },
        }));
        if (!result || !isSuccessStatus(result.status)) {
            logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.failed", {
                currentRelayId: current.id,
                selectedRelayId: selected.id,
                status: result?.status,
                message: result?.message,
            });
            showNotice(t("notify.providerSwitch"), result?.message ?? t("notify.targetProviderMissing"), result?.status ?? "failed");
            return null;
        }
        logDiagnostic("snapshotActiveRelayFilesBeforeSwitch.ok", {
            currentRelayId: current.id,
            selectedRelayId: selected.id,
        });
        return syncLegacyRelayFields(normalizeSettings(result.settings));
    };
    const copyText = async (text: string, message: string) => {
        try {
            await navigator.clipboard.writeText(text);
        }
        catch (error) {
            showNotice(t("notify.copyFailed"), stringifyError(error), "failed");
        }
    };
    const openExternalUrl = async (url: string) => {
        const result = await run(() => call<CommandResult<Record<string, unknown>>>("open_external_url", { url }));
        if (result) {
            showResultNotice(t("notify.openLink"), result, { silentSuccess: true });
        }
    };
    const showNotice = (title: string, message: string, status?: Status) => {
        setNotice({ title, message, status });
    };
    const showResultNotice = (title: string, result: Pick<CommandResult<unknown>, "message" | "status">, options: {
        silentSuccess?: boolean;
    } = {}) => {
        if (options.silentSuccess && isSuccessStatus(result.status))
            return;
        showNotice(title, result.message, result.status);
    };
    useEffect(() => {
        void (async () => {
            const startup = await run(() => call<StartupResult>("startup_options"));
            if (startup?.showUpdate) {
                setRoute("about");
                void checkUpdate(false);
            }
            else {
                void checkUpdate(true);
            }
            await refreshOverview(true);
            await refreshSettings(true);
            await refreshRelay(true);
            await refreshProviderSyncTargets(true);
        })();
    }, []);
    useEffect(() => {
        document.documentElement.classList.toggle("dark", theme === "dark");
        document.documentElement.classList.toggle("light", theme === "light");
        window.localStorage.setItem(STORAGE_KEYS.theme, theme);
    }, [theme]);
    const saveCodexAppPath = async (appPath: string) => {
        const next = { ...settingsForm, codexAppPath: appPath };
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
            setSettings(result);
            const normalized = normalizeSettings(result.settings);
            setSettingsForm(normalized);
            setLaunchForm((current) => ({ ...current, appPath: normalized.codexAppPath }));
            await refreshOverview(true);
        }
        return result;
    };
    const actions = useMemo(() => ({
        refreshCurrent: () => navigate(route),
        launch,
        restart,
        repairBackend,
        installEntrypoints,
        uninstallEntrypoints,
        repairShortcuts,
        checkUpdate,
        performUpdate,
        saveSettings,
        saveSettingsValue,
        refreshSettings,
        resetSettings,
        chooseCodexAppPath: async (mode: "folder" | "file") => {
            let selected: unknown;
            try {
                selected = await open(mode === "folder"
                    ? { directory: true, multiple: false, title: t("editor.placeholderSelectorTitle") }
                    : {
                        directory: false,
                        multiple: false,
                        title: t("editor.placeholderSelectorTitleFile"),
                        filters: [{ name: t("editor.placeholderSelectorFilter"), extensions: ["exe", "app"] }],
                    });
            }
            catch (error) {
                // Surface plugin failures (e.g. missing capability permission) so the
                // buttons no longer appear unresponsive — see #345.
                const message = error instanceof Error ? error.message : String(error);
                showNotice(t("notify.codexAppPath"), t("notify.selectorFailed", { message }), "failed");
                return;
            }
            if (typeof selected === "string" && selected.trim()) {
                const result = await saveCodexAppPath(selected.trim());
                if (result) {
                    showNotice(t("notify.codexAppPath"), t("notify.pathSaved"), result.status);
                }
            }
        },
        clearCodexAppPath: async () => {
            const result: CommandResult<{}> | null = await run(() => call("clear_codex_app_path"));
            if (result) {
                showNotice(t("notify.codexAppPath"), t("notify.pathCleared"), result.status);
            }
        },
        saveManualCodexAppPath: async () => {
            if (!launchForm.appPath) {
                showNotice(t("notify.codexAppPath"), t("notify.pathRequired"), "failed");
                return;
            }
            const result: CommandResult<Record<string, unknown>> | null = await run(() => call("save_codex_app_path", { path: launchForm.appPath }));
            if (result) {
                showNotice(t("notify.codexAppPath"), t("notify.pathSaved"), result.status);
            }
        },
        syncProvidersNow,
        refreshProviderSyncTargets,
        setProviderSyncTarget: (provider: string) => {
            setSelectedProviderSyncTarget(provider);
            setSettingsForm((current) => ({ ...current, providerSyncLastSelectedProvider: provider }));
        },
        setLaunchMode: async (launchMode: LaunchMode) => {
            await saveLaunchMode(launchMode);
        },
        refreshRelay,
        refreshRelayFiles,
        refreshLiveContextEntries,
        syncLiveContextEntries,
        importCcsProviders,
        refreshAds,
        refreshScriptMarket,
        installMarketScript,
        setUserScriptEnabled,
        deleteUserScript,
        refreshLocalSessions,
        deleteLocalSession,
        openExternalUrl,
        applyRelayInjection,
        applyPureApiInjection,
        clearRelayInjection,
        saveRelayFile,
        upsertContextEntry,
        deleteContextEntry,
        extractRelayCommonConfig,
        testRelayProfile,
        fetchRelayProfileModels,
        switchRelayProfile,
        switchOfficialMode,
        switchPureApiMode,
        refreshLogs,
        refreshDiagnostics,
        showMessage: async (title: string, message: string, status?: Status) => showNotice(title, message, status),
        copyLogs: () => copyText(logs?.text ?? "", t("notify.logsCopied")),
        copyDiagnostics: () => copyText(diagnostics?.report ?? "", t("notify.diagnosticsCopied")),
        goLogs: () => navigate("about"),
        checkHealth: async () => {
            await refreshOverview(true);
            await refreshRelay(true);
            await refreshWatcher(true);
            showNotice(t("notify.checkComplete"), t("notify.checkCompleteMsg"), "ok");
        },
        installWatcher: () => watcherAction("install_watcher"),
        uninstallWatcher: () => watcherAction("uninstall_watcher"),
        enableWatcher: () => watcherAction("enable_watcher"),
        disableWatcher: () => watcherAction("disable_watcher"),
        toggleTheme: () => setTheme((current) => (current === "dark" ? "light" : "dark")),
    }), [route, launchForm, settingsForm, settings, removeOwnedData, update, logs, diagnostics, theme, relayFiles, localSessions, selectedProviderSyncTarget]);
    const hasUpdate = update?.updateAvailable === true;
    return (<div className={`shell ${theme}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">{t("brand.mark")}</div>
          <div className="brand-copy">
            <div className="brand-title-row">
              <div className="brand-title">{t("brand.title")}</div>
              {hasUpdate ? (<button className="update-dot" onClick={() => {
                setRoute("about");
                void checkUpdate(false);
            }} title={t("notify.newVersionFound", { version: update?.latestVersion ?? "" })} type="button">
                  <CircleArrowUp className="h-4 w-4" aria-hidden="true"/>
                </button>) : null}
            </div>
            <div className="brand-subtitle">{t("brand.subtitle")}</div>
          </div>
        </div>
        <nav className="nav">
          {routes.map((item) => {
            const Icon = item.icon;
            return (<button className={`nav-item ${route === item.id ? "active" : ""}`} key={item.id} onClick={() => void navigate(item.id)} title={t(item.labelKey)} type="button">
                <span className="nav-icon">
                  <Icon className="h-4 w-4" aria-hidden="true"/>
                </span>
                <span className="nav-label">{t(item.labelKey)}</span>
              </button>);
        })}
        </nav>
      </aside>
      <main className="workspace">
        <header className="topbar" key={`topbar-${route}`}>
          <div>
            <h1>{t(routeTitleKey(route))}</h1>
            <p>{t(routeSubtitleKey(route))}</p>
          </div>
          <div className="topbar-actions">
            <Button onClick={() => {
            const next = i18nInstance.language === "zh" ? "en" : "zh";
            i18nInstance.changeLanguage(next);
            window.localStorage.setItem(STORAGE_KEYS.lang, next);
        }} size="icon" title={i18nInstance.language === "zh" ? t("lang.en") : t("lang.zh")} variant="outline">
              <span className="h-4 w-4 flex items-center justify-center text-xs font-bold">
                {i18nInstance.language === "zh" ? t("lang.enShort") : t("lang.zhShort")}
              </span>
            </Button>
            <Button onClick={actions.toggleTheme} size="icon" title={theme === "dark" ? t("theme.switchDark") : t("theme.switchLight")} variant="outline">
              {theme === "dark" ? <Sun className="h-4 w-4"/> : <Moon className="h-4 w-4"/>}
            </Button>
            <Button onClick={() => void actions.restart()} title={t("topbar.restart")} variant="outline">
              <Rocket className="h-4 w-4"/>
              {t("topbar.restart")}
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} size="icon" title={t("topbar.refresh")} variant="outline">
              <RefreshCw className="h-4 w-4"/>
            </Button>
          </div>
        </header>
        <section className="screen" key={route}>
          {route === "overview" ? (<OverviewScreen overview={overview} actions={actions}/>) : null}
          {route === "relay" ? (<RelayScreen settings={settings} relayFiles={relayFiles} form={settingsForm} onFormChange={setSettingsForm} actions={actions}/>) : null}
          {route === "sessions" ? (<SessionsScreen settings={settings} form={settingsForm} sessions={localSessions} providerSyncProgress={providerSyncProgress} providerSyncTargets={providerSyncTargets} selectedProviderSyncTarget={selectedProviderSyncTarget} onFormChange={setSettingsForm} actions={actions}/>) : null}
          {route === "context" ? (<ContextScreen form={settingsForm} liveEntries={liveContextEntries} relayFiles={relayFiles} onFormChange={setSettingsForm} actions={actions}/>) : null}
          {route === "enhance" ? (<EnhanceScreen form={settingsForm} onFormChange={setSettingsForm} actions={actions}/>) : null}
          {route === "userScripts" ? <UserScriptsScreen settings={settings} market={scriptMarket} actions={actions}/> : null}
          {route === "recommendations" ? <RecommendationsScreen ads={ads} actions={actions}/> : null}
          {route === "maintenance" ? (<MaintenanceScreen overview={overview} watcher={watcher} settings={settings} launchForm={launchForm} onLaunchFormChange={setLaunchForm} removeOwnedData={removeOwnedData} onRemoveOwnedDataChange={setRemoveOwnedData} actions={actions}/>) : null}
          {route === "about" ? <AboutScreen overview={overview} update={update} logs={logs} diagnostics={diagnostics} actions={actions}/> : null}
          {route === "settings" ? (<SettingsScreen settings={settings} theme={theme} form={settingsForm} onFormChange={setSettingsForm} actions={actions}/>) : null}
        </section>
      </main>
      {notice ? (<NoticeDialog key={`${notice.title}-${notice.message}-${notice.status ?? ""}`} notice={notice} onClose={() => setNotice(null)}/>) : null}
    </div>);
}

