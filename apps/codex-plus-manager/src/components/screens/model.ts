import { ExternalLink, FileCode2, Hammer, Info, KeyRound, LayoutDashboard, MessageCircle, Network, Settings, Wrench, type LucideIcon } from "lucide-react";
import { APP_LINKS, DEFAULT_CLI_WRAPPER_API_KEY_ENV, DEFAULT_RELAY_PROFILE_ID, DEFAULT_RELAY_TEST_MODEL } from "../../appConfig";
import i18n from "../../i18n";
export * from "../../appConfig";
export type Status = "ok" | "failed" | "not_implemented" | "not_checked" | string;
export type CommandResult<T> = T & {
    status: Status;
    message: string;
};
export type PathState = {
    status: string;
    path: string | null;
};
export type LaunchStatus = {
    status: string;
    message: string;
    started_at_ms: number;
    debug_port: number | null;
    helper_port: number | null;
    codex_app: string | null;
};
export type OverviewResult = CommandResult<{
    codex_app: PathState;
    codex_version: string | null;
    silent_shortcut: PathState;
    management_shortcut: PathState;
    latest_launch: LaunchStatus | null;
    current_version: string;
    update_status: string;
    settings_path: string;
    logs_path: string;
}>;
export type BackendSettings = {
    codexAppPath: string;
    codexExtraArgs: string[];
    providerSyncEnabled: boolean;
    providerSyncSavedProviders: string[];
    providerSyncManualProviders: string[];
    providerSyncLastSelectedProvider: string;
    relayProfilesEnabled: boolean;
    ccsLinkEnabled: boolean;
    enhancementsEnabled: boolean;
    codexAppPluginEntryUnlock: boolean;
    codexAppPluginMarketplaceUnlock: boolean;
    codexAppForcePluginInstall: boolean;
    codexAppModelWhitelistUnlock: boolean;
    codexAppSessionDelete: boolean;
    codexAppMarkdownExport: boolean;
    codexAppProjectMove: boolean;
    codexAppConversationTimeline: boolean;
    codexAppConversationView: boolean;
    codexAppThreadScrollRestore: boolean;
    codexAppZedRemoteOpen: boolean;
    codexAppUpstreamWorktreeCreate: boolean;
    codexAppNativeMenuPlacement: boolean;
    codexAppServiceTierControls: boolean;
    codexGoalsEnabled: boolean;
    launchMode: LaunchMode;
    relayBaseUrl: string;
    relayApiKey: string;
    relayProfiles: RelayProfile[];
    relayCommonConfigContents: string;
    relayContextConfigContents: string;
    activeRelayId: string;
    relayTestModel: string;
    cliWrapperEnabled: boolean;
    cliWrapperBaseUrl: string;
    cliWrapperApiKey: string;
    cliWrapperApiKeyEnv: string;
};
export type LaunchMode = "patch" | "relay";
export type RelayProfile = {
    id: string;
    linkedCcsProviderId: string;
    name: string;
    model: string;
    baseUrl: string;
    upstreamBaseUrl: string;
    apiKey: string;
    protocol: RelayProtocol;
    relayMode: RelayMode;
    officialMixApiKey: boolean;
    testModel: string;
    configContents: string;
    authContents: string;
    useCommonConfig: boolean;
    contextSelection: RelayContextSelection;
    contextSelectionInitialized: boolean;
    contextWindow: string;
    autoCompactLimit: string;
    modelList: string;
    userAgent: string;
};
export type RelayContextSelection = {
    mcpServers: string[];
    skills: string[];
    plugins: string[];
};
export type ContextKind = "mcp" | "skill" | "plugin";
export type CodexContextEntry = {
    id: string;
    kind: ContextKind;
    title: string;
    summary: string;
    tomlBody: string;
    enabled: boolean;
};
export type CodexContextEntries = {
    mcpServers: CodexContextEntry[];
    skills: CodexContextEntry[];
    plugins: CodexContextEntry[];
};
export type RelayProtocol = "responses" | "chatCompletions";
export type RelayMode = "official" | "mixedApi" | "pureApi";
/** @deprecated Use APP_LINKS.scriptMarket */
export const SCRIPT_MARKET_REPOSITORY_URL = APP_LINKS.scriptMarket;
export const emptyContextSelection = (): RelayContextSelection => ({
    mcpServers: [],
    skills: [],
    plugins: [],
});
export type UserScriptInventory = {
    enabled?: boolean;
    scripts?: Array<{
        key: string;
        name: string;
        source: string;
        enabled: boolean;
        status: string;
        error: string;
        market_id?: string;
        version?: string;
        installed?: boolean;
        source_url?: string;
        homepage?: string;
    }>;
};
export type SettingsResult = CommandResult<{
    settings: BackendSettings;
    settings_path: string;
    user_scripts: UserScriptInventory;
}>;
export type RelayResult = CommandResult<{
    authenticated: boolean;
    authSource: string;
    accountLabel: string | null;
    configPath: string;
    configured: boolean;
    requiresOpenaiAuth: boolean;
    hasBearerToken: boolean;
    backupPath: string | null;
}>;
export type RelayFilesResult = CommandResult<{
    configPath: string;
    authPath: string;
    configContents: string;
    authContents: string;
}>;
export type LocalSession = {
    id: string;
    title: string;
    cwd: string;
    modelProvider: string;
    archived: boolean;
    updatedAtMs: number | null;
    rolloutPath: string;
};
export type LocalSessionsResult = CommandResult<{
    dbPath: string;
    sessions: LocalSession[];
}>;
export type DeleteLocalSessionResult = CommandResult<{
    status: string;
    session_id: string;
    message: string;
    undo_token: string | null;
    backup_path: string | null;
}>;
export type ContextEntriesResult = CommandResult<{
    settings: BackendSettings;
    entries: CodexContextEntries;
}>;
export type LiveContextEntriesResult = CommandResult<{
    entries: CodexContextEntries;
}>;
export type ExtractRelayCommonConfigResult = CommandResult<{
    commonConfigContents: string;
    profileConfigContents: string;
}>;
export type SettingsBackfillResult = CommandResult<{
    settings: BackendSettings;
}>;
export type RelayProfileTestResult = CommandResult<{
    httpStatus: number;
    endpoint: string;
    responsePreview: string;
}>;
export type RelayProfileModelsResult = CommandResult<{
    models: string[];
    endpoint: string;
}>;
export type CcsProviderImport = {
    sourceId: string;
    name: string;
    baseUrl: string;
    apiKey: string;
    protocol: RelayProtocol;
    configContents: string;
    authContents: string;
};
export type ProviderSyncPayload = {
    syncStatus?: string;
    targetProvider?: string;
    changedSessionFiles?: number;
    skippedLockedRolloutFiles?: string[];
    sqliteRowsUpdated?: number;
    sqliteProviderRowsUpdated?: number;
    sqliteUserEventRowsUpdated?: number;
    sqliteCwdRowsUpdated?: number;
    updatedWorkspaceRoots?: number;
    encryptedContentWarning?: string | null;
};
export type ProviderSyncTargetSource = "config" | "rollout" | "sqlite" | "manual";
export type ProviderSyncTargetOption = {
    id: string;
    sources: ProviderSyncTargetSource[];
    isCurrentProvider: boolean;
    isManual: boolean;
    isSaved: boolean;
};
export type ProviderSyncTargetsPayload = {
    currentProvider: string;
    targets: ProviderSyncTargetOption[];
};
export type ProviderSyncTargetsResult = CommandResult<ProviderSyncTargetsPayload>;
export type ProviderSyncProgress = {
    active: boolean;
    percent: number;
    message: string;
    result: CommandResult<ProviderSyncPayload> | null;
};
export type LogsResult = CommandResult<{
    path: string;
    text: string;
    lines: number;
}>;
export type DiagnosticsResult = CommandResult<{
    report: string;
}>;
export type WatcherResult = CommandResult<{
    enabled: boolean;
    disabled_flag: string;
}>;
export type InstallResult = CommandResult<{
    silent_shortcut: {
        installed: boolean;
        path: string | null;
    };
    management_shortcut: {
        installed: boolean;
        path: string | null;
    };
}>;
export type UpdateResult = CommandResult<{
    currentVersion: string;
    latestVersion?: string | null;
    releaseSummary?: string;
    assetName?: string | null;
    assetUrl?: string | null;
    updateAvailable?: boolean;
    installedPath?: string;
    progress?: number;
}>;
export type AdItem = {
    id?: string;
    type: "sponsor" | "normal" | string;
    title: string;
    description: string;
    url: string;
    highlights?: string[];
    expires_at?: string;
};
export type AdsResult = CommandResult<{
    version: number;
    ads: AdItem[];
}>;
export type ScriptMarketItem = {
    id: string;
    name: string;
    description: string;
    version: string;
    author: string;
    tags: string[];
    homepage: string;
    script_url: string;
    sha256: string;
    installed: boolean;
    installedVersion: string;
    updateAvailable: boolean;
};
export type ScriptMarketResult = CommandResult<{
    market: {
        status: string;
        message: string;
        indexUrl: string;
        updatedAt: string;
        scripts: ScriptMarketItem[];
    };
    user_scripts: UserScriptInventory;
}>;
export function providerSyncProgressMessage(result: CommandResult<ProviderSyncPayload>): string {
    const changed = result.changedSessionFiles ?? 0;
    const rows = result.sqliteRowsUpdated ?? 0;
    const target = result.targetProvider || i18n.t("notify.noProviderSync");
    const skipped = result.skippedLockedRolloutFiles?.length ?? 0;
    const skippedText = skipped ? i18n.t("notify.skippedFiles", { count: skipped }) : "";
    return i18n.t("notify.syncResult", { target, changed, rows, skippedText });
}
function providerSyncSourceLabels(): Record<ProviderSyncTargetSource, string> {
    return {
        config: i18n.t("notify.sourceConfig"),
        rollout: i18n.t("notify.sourceRollout"),
        sqlite: i18n.t("notify.sourceSqlite"),
        manual: i18n.t("notify.sourceManual"),
    };
}
export function providerSyncTargetLabel(target: ProviderSyncTargetOption): string {
    const labels = target.sources.map((source) => providerSyncSourceLabels()[source]).filter(Boolean);
    const current = target.isCurrentProvider ? [i18n.t("notify.currentProvider")] : [];
    return [...labels, ...current].join(" / ") || i18n.t("notify.discovered");
}
export function syncMarketInstalledState(current: ScriptMarketResult | null, userScripts: UserScriptInventory): ScriptMarketResult | null {
    if (!current)
        return current;
    const installed = new Map((userScripts.scripts ?? [])
        .filter((script) => script.market_id)
        .map((script) => [script.market_id || "", script.version || ""]));
    return {
        ...current,
        user_scripts: userScripts,
        market: {
            ...current.market,
            scripts: current.market.scripts.map((script) => {
                const installedVersion = installed.get(script.id) || "";
                return {
                    ...script,
                    installed: Boolean(installedVersion),
                    installedVersion,
                    updateAvailable: Boolean(installedVersion) && installedVersion !== script.version,
                };
            }),
        },
    };
}
export type StartupResult = CommandResult<{
    showUpdate: boolean;
}>;
export type Route = "overview" | "relay" | "sessions" | "context" | "enhance" | "userScripts" | "recommendations" | "maintenance" | "about" | "settings";
export type Theme = "dark" | "light";
export const routes: Array<{
    id: Route;
    labelKey: string;
    icon: LucideIcon;
}> = [
    { id: "overview", labelKey: "nav.overview", icon: LayoutDashboard },
    { id: "relay", labelKey: "nav.relay", icon: KeyRound },
    { id: "sessions", labelKey: "nav.sessions", icon: MessageCircle },
    { id: "context", labelKey: "nav.context", icon: Network },
    { id: "enhance", labelKey: "nav.enhance", icon: Hammer },
    { id: "userScripts", labelKey: "nav.userScripts", icon: FileCode2 },
    { id: "recommendations", labelKey: "nav.recommendations", icon: ExternalLink },
    { id: "maintenance", labelKey: "nav.maintenance", icon: Wrench },
    { id: "about", labelKey: "nav.about", icon: Info },
    { id: "settings", labelKey: "nav.settings", icon: Settings },
];
export const defaultSettings: BackendSettings = {
    codexAppPath: "",
    codexExtraArgs: [],
    providerSyncEnabled: false,
    providerSyncSavedProviders: [],
    providerSyncManualProviders: [],
    providerSyncLastSelectedProvider: "",
    relayProfilesEnabled: true,
    ccsLinkEnabled: false,
    enhancementsEnabled: true,
    codexAppPluginEntryUnlock: true,
    codexAppPluginMarketplaceUnlock: true,
    codexAppForcePluginInstall: true,
    codexAppModelWhitelistUnlock: true,
    codexAppSessionDelete: true,
    codexAppMarkdownExport: true,
    codexAppProjectMove: true,
    codexAppConversationTimeline: true,
    codexAppConversationView: false,
    codexAppThreadScrollRestore: true,
    codexAppZedRemoteOpen: true,
    codexAppUpstreamWorktreeCreate: true,
    codexAppNativeMenuPlacement: true,
    codexAppServiceTierControls: false,
    codexGoalsEnabled: false,
    launchMode: "patch",
    relayBaseUrl: "",
    relayApiKey: "",
    relayProfiles: [
        {
            id: DEFAULT_RELAY_PROFILE_ID,
            linkedCcsProviderId: "",
            name: i18n.t("createRelayProfile.defaultProfileName"),
            model: "",
            baseUrl: "",
            upstreamBaseUrl: "",
            apiKey: "",
            protocol: "responses",
            relayMode: "official",
            officialMixApiKey: false,
            testModel: "",
            configContents: "",
            authContents: "",
            useCommonConfig: true,
            contextSelection: emptyContextSelection(),
            contextSelectionInitialized: true,
            contextWindow: "",
            autoCompactLimit: "",
            modelList: "",
            userAgent: "",
        },
    ],
    relayCommonConfigContents: "",
    relayContextConfigContents: "",
    activeRelayId: DEFAULT_RELAY_PROFILE_ID,
    relayTestModel: DEFAULT_RELAY_TEST_MODEL,
    cliWrapperEnabled: false,
    cliWrapperBaseUrl: "",
    cliWrapperApiKey: "",
    cliWrapperApiKeyEnv: DEFAULT_CLI_WRAPPER_API_KEY_ENV,
};
