import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  ArrowLeft,
  Bell,
  CheckCircle2,
  CircleArrowUp,
  Copy,
  Download,
  Edit3,
  GripVertical,
  Info,
  ExternalLink,
  Hammer,
  KeyRound,
  LayoutDashboard,
  Link2,
  MessageCircle,
  FileCode2,
  Moon,
  Plus,
  RefreshCw,
  Rocket,
  Save,
  ScrollText,
  Settings,
  ShieldCheck,
  Sun,
  TestTube,
  Trash2,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useMemo, useState, type CSSProperties } from "react";

import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";

type Status = "ok" | "failed" | "not_implemented" | "not_checked" | string;

type CommandResult<T> = T & {
  status: Status;
  message: string;
};

type PathState = {
  status: string;
  path: string | null;
};

type LaunchStatus = {
  status: string;
  message: string;
  started_at_ms: number;
  debug_port: number | null;
  helper_port: number | null;
  codex_app: string | null;
};

type OverviewResult = CommandResult<{
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

type BackendSettings = {
  codexAppPath: string;
  codexExtraArgs: string[];
  providerSyncEnabled: boolean;
  enhancementsEnabled: boolean;
  launchMode: LaunchMode;
  relayBaseUrl: string;
  relayApiKey: string;
  relayProfiles: RelayProfile[];
  activeRelayId: string;
  relayTestModel: string;
  cliWrapperEnabled: boolean;
  cliWrapperBaseUrl: string;
  cliWrapperApiKey: string;
  cliWrapperApiKeyEnv: string;
};

type LaunchMode = "patch" | "relay";

type RelayProfile = {
  id: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  relayMode: RelayMode;
  officialMixApiKey: boolean;
  testModel: string;
  configContents: string;
  authContents: string;
};

type RelayProtocol = "responses" | "chatCompletions";
type RelayMode = "official" | "mixedApi" | "pureApi";
const PROTOCOL_PROXY_BASE_URL = "http://127.0.0.1:57321/v1";
const SCRIPT_MARKET_REPOSITORY_URL = "https://github.com/BigPizzaV3/CodexPlusPlusScriptMarket";

type UserScriptInventory = {
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

type SettingsResult = CommandResult<{
  settings: BackendSettings;
  settings_path: string;
  user_scripts: UserScriptInventory;
}>;

type RelayResult = CommandResult<{
  authenticated: boolean;
  authSource: string;
  accountLabel: string | null;
  configPath: string;
  configured: boolean;
  requiresOpenaiAuth: boolean;
  hasBearerToken: boolean;
  backupPath: string | null;
}>;

type RelayFilesResult = CommandResult<{
  configPath: string;
  authPath: string;
  configContents: string;
  authContents: string;
}>;

type RelayProfileTestResult = CommandResult<{
  httpStatus: number;
  endpoint: string;
  responsePreview: string;
}>;

type CcsProviderImport = {
  sourceId: string;
  name: string;
  baseUrl: string;
  apiKey: string;
  protocol: RelayProtocol;
  configContents: string;
  authContents: string;
};

type CcsProvidersResult = CommandResult<{
  dbPath: string;
  providers: CcsProviderImport[];
}>;

type LogsResult = CommandResult<{
  path: string;
  text: string;
  lines: number;
}>;

type DiagnosticsResult = CommandResult<{
  report: string;
}>;

type WatcherResult = CommandResult<{
  enabled: boolean;
  disabled_flag: string;
}>;

type InstallResult = CommandResult<{
  silent_shortcut: { installed: boolean; path: string | null };
  management_shortcut: { installed: boolean; path: string | null };
}>;

type UpdateResult = CommandResult<{
  currentVersion: string;
  latestVersion?: string | null;
  releaseSummary?: string;
  assetName?: string | null;
  assetUrl?: string | null;
  updateAvailable?: boolean;
  installedPath?: string;
  progress?: number;
}>;

type AdItem = {
  id?: string;
  type: "sponsor" | "normal" | string;
  title: string;
  description: string;
  url: string;
  highlights?: string[];
  expires_at?: string;
};

type AdsResult = CommandResult<{
  version: number;
  ads: AdItem[];
}>;

type ScriptMarketItem = {
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

type ScriptMarketResult = CommandResult<{
  market: {
    status: string;
    message: string;
    indexUrl: string;
    updatedAt: string;
    scripts: ScriptMarketItem[];
  };
  user_scripts: UserScriptInventory;
}>;

type StartupResult = CommandResult<{
  showUpdate: boolean;
}>;

type Route = "overview" | "relay" | "enhance" | "userScripts" | "providerSync" | "recommendations" | "maintenance" | "about" | "settings" | "logs" | "diagnostics";
type Theme = "dark" | "light";

const routes: Array<{ id: Route; label: string; icon: LucideIcon }> = [
  { id: "overview", label: "概览", icon: LayoutDashboard },
  { id: "relay", label: "供应商配置", icon: KeyRound },
  { id: "enhance", label: "页面增强", icon: Hammer },
  { id: "userScripts", label: "脚本市场", icon: FileCode2 },
  { id: "providerSync", label: "历史会话修复", icon: Link2 },
  { id: "recommendations", label: "推荐内容", icon: ExternalLink },
  { id: "maintenance", label: "安装维护", icon: Wrench },
  { id: "about", label: "关于", icon: Info },
  { id: "settings", label: "设置", icon: Settings },
  { id: "logs", label: "日志", icon: ScrollText },
  { id: "diagnostics", label: "诊断", icon: Activity },
];

const defaultSettings: BackendSettings = {
  codexAppPath: "",
  codexExtraArgs: [],
  providerSyncEnabled: false,
  enhancementsEnabled: true,
  launchMode: "patch",
  relayBaseUrl: "",
  relayApiKey: "",
  relayProfiles: [
    {
      id: "default",
      name: "默认中转",
      baseUrl: "",
      apiKey: "",
      protocol: "responses",
      relayMode: "official",
      officialMixApiKey: false,
      testModel: "",
      configContents: "",
      authContents: "",
    },
  ],
  activeRelayId: "default",
  relayTestModel: "gpt-5-mini",
  cliWrapperEnabled: false,
  cliWrapperBaseUrl: "",
  cliWrapperApiKey: "",
  cliWrapperApiKeyEnv: "CUSTOM_OPENAI_API_KEY",
};

export function App() {
  const [theme, setTheme] = useState<Theme>(() => loadInitialTheme());
  const [route, setRoute] = useState<Route>(() => loadInitialRoute());
  const [notice, setNotice] = useState<{ title: string; message: string; status?: Status } | null>(null);
  const [overview, setOverview] = useState<OverviewResult | null>(null);
  const [settings, setSettings] = useState<SettingsResult | null>(null);
  const [relay, setRelay] = useState<RelayResult | null>(null);
  const [relayFiles, setRelayFiles] = useState<RelayFilesResult | null>(null);
  const [ccsProviders, setCcsProviders] = useState<CcsProvidersResult | null>(null);
  const [logs, setLogs] = useState<LogsResult | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [watcher, setWatcher] = useState<WatcherResult | null>(null);
  const [update, setUpdate] = useState<UpdateResult | null>(null);
  const [ads, setAds] = useState<AdsResult | null>(null);
  const [scriptMarket, setScriptMarket] = useState<ScriptMarketResult | null>(null);
  const [launchForm, setLaunchForm] = useState({
    appPath: "",
    debugPort: "9229",
    helperPort: "57321",
  });
  const [settingsForm, setSettingsForm] = useState<BackendSettings>({ ...defaultSettings });
  const [removeOwnedData, setRemoveOwnedData] = useState(false);

  const call = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);

  const run = async <T,>(task: () => Promise<T>): Promise<T | null> => {
    try {
      return await task();
    } catch (error) {
      showNotice("调用失败", stringifyError(error), "failed");
      return null;
    }
  };

  const refreshOverview = async (silent = false) => {
    const result = await run(() => call<OverviewResult>("load_overview"));
    if (result) {
      setOverview(result);
      if (!silent) showResultNotice("概览已检查", result, { silentSuccess: true });
    }
  };

  const refreshSettings = async (silent = false) => {
    const result = await run(() => call<SettingsResult>("load_settings"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      setLaunchForm((current) => ({
        ...current,
        appPath: current.appPath || result.settings.codexAppPath || "",
      }));
      if (!silent) showResultNotice("设置已加载", result, { silentSuccess: true });
    }
  };

  const refreshScriptMarket = async (silent = false) => {
    const result = await run(() => call<ScriptMarketResult>("refresh_script_market"));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("脚本市场", result, { silentSuccess: true });
    }
  };

  const installMarketScript = async (id: string) => {
    const result = await run(() => call<ScriptMarketResult>("install_market_script", { id }));
    if (result) {
      setScriptMarket(result);
      setSettings((current) => (current ? { ...current, user_scripts: result.user_scripts } : current));
      showResultNotice("脚本市场", result);
    }
  };

  const refreshRelay = async (silent = false) => {
    const result = await run(() => call<RelayResult>("relay_status"));
    if (result) {
      setRelay(result);
      if (!silent) showResultNotice("登录状态", result, { silentSuccess: true });
    }
  };

  const refreshRelayFiles = async (silent = false) => {
    const result = await run(() => call<RelayFilesResult>("read_relay_files"));
    if (result) {
      setRelayFiles(result);
      if (!silent) showResultNotice("配置文件", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshCcsProviders = async (silent = false) => {
    const result = await run(() => call<CcsProvidersResult>("load_ccs_providers"));
    if (result) {
      setCcsProviders(result);
      if (!silent || !isSuccessStatus(result.status)) showResultNotice("CCS 供应商", result, { silentSuccess: true });
    }
    return result;
  };

  const refreshLogs = async (silent = false) => {
    const result = await run(() => call<LogsResult>("read_latest_logs", { request: { lines: 240 } }));
    if (result) {
      setLogs(result);
      if (!silent) showResultNotice("日志已刷新", result, { silentSuccess: true });
    }
  };

  const refreshDiagnostics = async (silent = false) => {
    const result = await run(() => call<DiagnosticsResult>("copy_diagnostics"));
    if (result) {
      setDiagnostics(result);
      if (!silent) showResultNotice("诊断已生成", result, { silentSuccess: true });
    }
  };

  const refreshWatcher = async (silent = false) => {
    const result = await run(() => call<WatcherResult>("load_watcher_state"));
    if (result) {
      setWatcher(result);
      if (!silent) showResultNotice("Watcher 状态", result, { silentSuccess: true });
    }
  };

  const navigate = async (next: Route) => {
    setRoute(next);
    if (next === "overview") await refreshOverview(true);
    if (next === "relay") {
      await refreshSettings(true);
      await refreshRelay(true);
      await refreshRelayFiles(true);
      await refreshCcsProviders(true);
    }
    if (next === "settings") await refreshSettings(true);
    if (next === "userScripts") {
      await refreshSettings(true);
      await refreshScriptMarket(true);
    }
    if (next === "providerSync") await refreshSettings(true);
    if (next === "recommendations") await refreshAds(true);
    if (next === "about") await refreshOverview(true);
    if (next === "logs") await refreshLogs(true);
    if (next === "diagnostics") await refreshDiagnostics(true);
    if (next === "maintenance") {
      await refreshOverview(true);
      await refreshWatcher(true);
    }
  };

  const launch = async () => {
    const result = await launchCommand("launch_codex_plus");
    if (result) {
      showNotice("启动任务", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const restart = async () => {
    const result = await launchCommand("restart_codex_plus");
    if (result) {
      showNotice("重启 Codex", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const launchCommand = async (command: "launch_codex_plus" | "restart_codex_plus") => {
    const result = await run(() =>
      call<CommandResult<Record<string, unknown>>>(command, {
        request: {
          appPath: launchForm.appPath,
          debugPort: numberOrDefault(launchForm.debugPort, 9229),
          helperPort: numberOrDefault(launchForm.helperPort, 57321),
        },
      }),
    );
    return result;
  };

  const repairBackend = async () => {
    const result = await run(() => call<SettingsResult>("repair_backend"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("后端修复", result.message, result.status);
    }
  };

  const installEntrypoints = async () => {
    const result = await run(() => call<InstallResult>("install_entrypoints"));
    if (result) {
      showNotice("入口安装", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const uninstallEntrypoints = async () => {
    const result = await run(() =>
      call<InstallResult>("uninstall_entrypoints", {
        options: { removeOwnedData },
      }),
    );
    if (result) {
      showNotice("入口卸载", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const repairShortcuts = async () => {
    const result = await run(() => call<InstallResult>("repair_shortcuts"));
    if (result) {
      showNotice("快捷方式修复", result.message, result.status);
      await refreshOverview(true);
    }
  };

  const watcherAction = async (command: string) => {
    const result = await run(() => call<WatcherResult>(command));
    if (result) {
      setWatcher(result);
      showNotice("Watcher 操作", result.message, result.status);
    }
  };

  const checkUpdate = async (silent = false) => {
    const result = await run(() => call<UpdateResult>("check_update"));
    if (result) {
      setUpdate(result);
      if (!silent || result.updateAvailable) {
        showNotice("GitHub Release 检查", result.message, result.status);
      }
    }
  };

  const performUpdate = async () => {
    const release =
      update?.latestVersion && update.assetName && update.assetUrl
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
      showNotice("更新安装", result.message, result.status);
    }
  };

  const saveSettings = async () => {
    const result = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("设置保存", result.message, result.status);
    }
  };

  const saveSettingsValue = async (next: BackendSettings, silent = true) => {
    setSettingsForm(next);
    const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      if (!silent || !isSuccessStatus(result.status)) showNotice("设置保存", result.message, result.status);
    }
  };

  const importCcsProviders = async () => {
    const result = await run(() => call<SettingsResult>("import_ccs_providers"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      await refreshCcsProviders(true);
      showResultNotice("导入 CCSwitch 配置", result);
    }
  };

  const resetSettings = async () => {
    const result = await run(() => call<SettingsResult>("reset_settings"));
    if (result) {
      setSettings(result);
      setSettingsForm(normalizeSettings(result.settings));
      showNotice("设置重置", result.message, result.status);
    }
  };

  const refreshAds = async (silent = false) => {
    const result = await run(() => call<AdsResult>("load_ads"));
    if (result) {
      setAds(result);
      if (!silent) showResultNotice("推荐内容", result, { silentSuccess: true });
    }
  };

  const syncProvidersNow = async () => {
    const result = await run(() => call<CommandResult<Record<string, never>>>("sync_providers_now"));
    if (result) {
      showNotice("历史会话修复", result.message, result.status);
    }
  };

  const applyRelayInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice("设置保存", settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_relay_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("官方混入 API Key", result.message, result.status);
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
      if (!silent) showNotice("页面增强模式", result.message, result.status);
    }
    return result;
  };

  const applyPureApiInjection = async (silent = false) => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice("设置保存", settingsResult.message, settingsResult.status);
        return false;
      }
    } else {
      return false;
    }
    const result = await run(() => call<RelayResult>("apply_pure_api_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("纯 API 模式", result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && result.configured;
  };

  const clearRelayInjection = async (silent = false) => {
    const result = await run(() => call<RelayResult>("clear_relay_injection"));
    if (result) {
      setRelay(result);
      await refreshRelayFiles(true);
      if (!silent || !isSuccessStatus(result.status)) showNotice("官方登录模式", result.message, result.status);
    }
    return !!result && isSuccessStatus(result.status) && !result.configured;
  };

  const saveRelayFile = async (kind: "config" | "auth", contents: string, silent = false) => {
    const result = await run(() => call<RelayFilesResult>("save_relay_file", { request: { kind, contents } }));
    if (result) {
      setRelayFiles(result);
      if (!silent || !isSuccessStatus(result.status)) {
        showNotice(kind === "config" ? "config.toml" : "auth.json", result.message, result.status);
      }
      await refreshRelay(true);
    }
  };

  const testRelayProfile = async (profile: RelayProfile) => {
    const result = await run(() => call<RelayProfileTestResult>("test_relay_profile", { profile }));
    if (result) showNotice("供应商测试", result.message, result.status);
  };

  const switchOfficialMode = async () => {
    const switched = await clearRelayInjection(true);
    if (!switched) return;
    const result = await saveLaunchMode("relay", true);
    if (result) showNotice("官方登录模式", "已切回官方登录；页面增强已设为兼容增强。", result.status);
  };

  const switchPureApiMode = async () => {
    const switched = await applyPureApiInjection(true);
    if (!switched) return;
    const result = await saveLaunchMode("patch", true);
    if (result) showNotice("纯 API 模式", "已切换到纯 API；页面增强已设为完整增强。", result.status);
  };

  const switchRelayProfile = async (next: BackendSettings) => {
    const nextWithSnapshot = await snapshotActiveRelayFilesBeforeSwitch(next);
    if (!nextWithSnapshot) return;

    const selectedBeforeSave = activeRelayProfile(nextWithSnapshot);
    const validationError = relayProfileSwitchValidation(selectedBeforeSave);
    if (validationError) {
      showNotice("供应商配置可能不正确", validationError, "failed");
      return;
    }

    let selectedSettings = nextWithSnapshot;
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: nextWithSnapshot }));
    if (settingsResult) {
      selectedSettings = normalizeSettings(settingsResult.settings);
      setSettings(settingsResult);
      setSettingsForm(selectedSettings);
      if (!isSuccessStatus(settingsResult.status)) {
        showNotice("供应商切换", settingsResult.message, settingsResult.status);
        return;
      }
    } else {
      return;
    }

    const selectedAfterSave = activeRelayProfile(selectedSettings);
    const command = relayProfileSwitchCommand(selectedAfterSave);
    const result = await run(() => call<RelayResult>(command));
    if (!result) return;

    setRelay(result);
    await refreshRelayFiles(true);
    if (!isSuccessStatus(result.status)) {
      showNotice("供应商切换", relayProfileReadinessText(selectedAfterSave, result), result.status);
      return;
    }

    const currentSelected = activeRelayProfile(selectedSettings);
    const launchMode = currentSelected.relayMode === "pureApi" ? "patch" : "relay";
    const modeResult = await saveLaunchMode(launchMode, true, selectedSettings);
    if (modeResult) showNotice("供应商切换", relayProfileModeSwitchedText(currentSelected), modeResult.status);
  };

  const snapshotActiveRelayFilesBeforeSwitch = async (next: BackendSettings): Promise<BackendSettings | null> => {
    const current = activeRelayProfile(settingsForm);
    const selected = activeRelayProfile(next);
    if (current.id === selected.id) return next;

    const files = await refreshRelayFiles(true);
    if (!files || !isSuccessStatus(files.status)) {
      showNotice("供应商切换", files?.message ?? "读取当前配置文件失败，已停止切换以避免覆盖用户改动。", files?.status ?? "failed");
      return null;
    }

    return syncLegacyRelayFields({
      ...next,
      relayProfiles: next.relayProfiles.map((profile) =>
        profile.id === current.id
          ? {
              ...profile,
              configContents: files.configContents,
              authContents: files.authContents,
            }
          : profile,
      ),
    });
  };


  const copyText = async (text: string, message: string) => {
    try {
      await navigator.clipboard.writeText(text);
    } catch (error) {
      showNotice("复制失败", stringifyError(error), "failed");
    }
  };

  const openExternalUrl = async (url: string) => {
    const result = await run(() => call<CommandResult<Record<string, unknown>>>("open_external_url", { url }));
    if (result) {
      showResultNotice("打开链接", result, { silentSuccess: true });
    }
  };

  const showNotice = (title: string, message: string, status?: Status) => {
    setNotice({ title, message, status });
  };

  const showResultNotice = (
    title: string,
    result: Pick<CommandResult<unknown>, "message" | "status">,
    options: { silentSuccess?: boolean } = {},
  ) => {
    if (options.silentSuccess && isSuccessStatus(result.status)) return;
    showNotice(title, result.message, result.status);
  };

  useEffect(() => {
    void (async () => {
      const startup = await run(() => call<StartupResult>("startup_options"));
      if (startup?.showUpdate) {
        setRoute("about");
        void checkUpdate(false);
      } else {
        void checkUpdate(true);
      }
      await refreshOverview(true);
      await refreshSettings(true);
      await refreshRelay(true);
    })();
  }, []);

  useEffect(() => {
    document.documentElement.classList.toggle("dark", theme === "dark");
    document.documentElement.classList.toggle("light", theme === "light");
    window.localStorage.setItem("codex-plus-theme", theme);
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

  const actions = useMemo(
    () => ({
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
      resetSettings,
      chooseCodexAppPath: async (mode: "folder" | "file") => {
        const selected = await open(
          mode === "folder"
            ? { directory: true, multiple: false, title: "选择 Codex 应用目录" }
            : {
                directory: false,
                multiple: false,
                title: "选择 Codex.exe 或 Codex.app",
                filters: [{ name: "Codex 应用", extensions: ["exe", "app"] }],
              },
        );
        if (typeof selected === "string" && selected.trim()) {
          const result = await saveCodexAppPath(selected.trim());
          if (result) {
            showNotice("Codex 应用路径", "应用路径已保存，之后启动会自动复用。", result.status);
          }
        }
      },
      clearCodexAppPath: async () => {
        const next = { ...settingsForm, codexAppPath: "" };
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
          setSettings(result);
          setSettingsForm(normalizeSettings(result.settings));
          setLaunchForm((current) => ({ ...current, appPath: "" }));
          showNotice("Codex 应用路径", "已清除保存路径，后续启动会回到自动探测。", result.status);
          await refreshOverview(true);
        }
      },
      saveManualCodexAppPath: async () => {
        const appPath = launchForm.appPath.trim();
        if (!appPath) {
          showNotice("Codex 应用路径", "请先填写或选择应用路径。", "failed");
          return;
        }
        const result = await saveCodexAppPath(appPath);
        if (result) {
          showNotice("Codex 应用路径", "应用路径已保存，之后启动会自动复用。", result.status);
        }
      },
      syncProvidersNow,
      setLaunchMode: async (launchMode: LaunchMode) => {
        await saveLaunchMode(launchMode);
      },
      refreshRelay,
      refreshRelayFiles,
      refreshCcsProviders,
      importCcsProviders,
      refreshAds,
      refreshScriptMarket,
      installMarketScript,
      openExternalUrl,
      applyRelayInjection,
      applyPureApiInjection,
      clearRelayInjection,
      saveRelayFile,
      testRelayProfile,
      switchRelayProfile,
      switchOfficialMode,
      switchPureApiMode,
      refreshLogs,
      refreshDiagnostics,
      copyLogs: () => copyText(logs?.text ?? "", "日志已复制。"),
      copyDiagnostics: () => copyText(diagnostics?.report ?? "", "诊断报告已复制。"),
      goLogs: () => navigate("logs"),
      checkHealth: async () => {
        await refreshOverview(true);
        await refreshRelay(true);
        await refreshWatcher(true);
        showNotice("检查完成", "已刷新 Codex 应用、入口、ChatGPT 登录和 Watcher 状态。", "ok");
      },
      installWatcher: () => watcherAction("install_watcher"),
      uninstallWatcher: () => watcherAction("uninstall_watcher"),
      enableWatcher: () => watcherAction("enable_watcher"),
      disableWatcher: () => watcherAction("disable_watcher"),
      toggleTheme: () => setTheme((current) => (current === "dark" ? "light" : "dark")),
    }),
    [route, launchForm, settingsForm, removeOwnedData, update, logs, diagnostics, theme, relayFiles],
  );
  const hasUpdate = update?.updateAvailable === true;

  return (
    <div className={`shell ${theme}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">C++</div>
          <div className="brand-copy">
            <div className="brand-title-row">
              <div className="brand-title">Codex++</div>
              {hasUpdate ? (
                <button
                  className="update-dot"
                  onClick={() => {
                    setRoute("about");
                    void checkUpdate(false);
                  }}
                  title={`发现新版本 ${update?.latestVersion ?? ""}`}
                  type="button"
                >
                  <CircleArrowUp className="h-4 w-4" aria-hidden="true" />
                </button>
              ) : null}
            </div>
            <div className="brand-subtitle">管理控制台</div>
          </div>
        </div>
        <nav className="nav">
          {routes.map((item) => {
            const Icon = item.icon;
            return (
            <button
              className={`nav-item ${route === item.id ? "active" : ""}`}
              key={item.id}
              onClick={() => void navigate(item.id)}
              title={item.label}
              type="button"
            >
              <span className="nav-icon">
                <Icon className="h-4 w-4" aria-hidden="true" />
              </span>
              <span className="nav-label">{item.label}</span>
            </button>
          );
          })}
        </nav>
      </aside>
      <main className="workspace">
        <header className="topbar">
          <div>
            <h1>{routeTitle(route)}</h1>
            <p>{routeSubtitle(route)}</p>
          </div>
          <div className="topbar-actions">
            <Button
              onClick={actions.toggleTheme}
              size="icon"
              title={theme === "dark" ? "切换到浅色" : "切换到深色"}
              variant="outline"
            >
              {theme === "dark" ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </Button>
            <Button onClick={() => void actions.restart()} title="重启 Codex" variant="outline">
              <Rocket className="h-4 w-4" />
              重启 Codex
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} size="icon" title="刷新当前页面" variant="outline">
              <RefreshCw className="h-4 w-4" />
            </Button>
          </div>
        </header>
        <section className="screen">
          {route === "overview" ? (
            <OverviewScreen
              overview={overview}
              settings={settings}
              relay={relay}
              actions={actions}
            />
          ) : null}
          {route === "relay" ? (
            <RelayScreen
              settings={settings}
              relay={relay}
              relayFiles={relayFiles}
              ccsProviders={ccsProviders}
              form={settingsForm}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "enhance" ? (
            <EnhanceScreen form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "userScripts" ? <UserScriptsScreen settings={settings} market={scriptMarket} actions={actions} /> : null}
          {route === "providerSync" ? (
            <ProviderSyncScreen settings={settings} form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "recommendations" ? <RecommendationsScreen ads={ads} actions={actions} /> : null}
          {route === "maintenance" ? (
            <MaintenanceScreen
              overview={overview}
              watcher={watcher}
              settings={settings}
              launchForm={launchForm}
              onLaunchFormChange={setLaunchForm}
              removeOwnedData={removeOwnedData}
              onRemoveOwnedDataChange={setRemoveOwnedData}
              actions={actions}
            />
          ) : null}
          {route === "about" ? <AboutScreen overview={overview} update={update} actions={actions} /> : null}
          {route === "settings" ? (
            <SettingsScreen settings={settings} theme={theme} form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "logs" ? <LogsScreen logs={logs} actions={actions} /> : null}
          {route === "diagnostics" ? (
            <DiagnosticsScreen diagnostics={diagnostics} actions={actions} />
          ) : null}
        </section>
      </main>
      {notice ? (
        <NoticeDialog
          key={`${notice.title}-${notice.message}-${notice.status ?? ""}`}
          notice={notice}
          onClose={() => setNotice(null)}
        />
      ) : null}
    </div>
  );
}

type Actions = {
  refreshCurrent: () => Promise<void>;
  launch: () => Promise<void>;
  restart: () => Promise<void>;
  repairBackend: () => Promise<void>;
  installEntrypoints: () => Promise<void>;
  uninstallEntrypoints: () => Promise<void>;
  repairShortcuts: () => Promise<void>;
  checkUpdate: () => Promise<void>;
  performUpdate: () => Promise<void>;
  saveSettings: () => Promise<void>;
  saveSettingsValue: (settings: BackendSettings, silent?: boolean) => Promise<void>;
  resetSettings: () => Promise<void>;
  chooseCodexAppPath: (mode: "folder" | "file") => Promise<void>;
  clearCodexAppPath: () => Promise<void>;
  saveManualCodexAppPath: () => Promise<void>;
  syncProvidersNow: () => Promise<void>;
  setLaunchMode: (launchMode: LaunchMode) => Promise<void>;
  refreshRelay: () => Promise<void>;
  refreshRelayFiles: () => Promise<RelayFilesResult | null>;
  refreshCcsProviders: () => Promise<CcsProvidersResult | null>;
  importCcsProviders: () => Promise<void>;
  refreshAds: () => Promise<void>;
  refreshScriptMarket: () => Promise<void>;
  installMarketScript: (id: string) => Promise<void>;
  openExternalUrl: (url: string) => Promise<void>;
  applyRelayInjection: () => Promise<boolean>;
  applyPureApiInjection: () => Promise<boolean>;
  clearRelayInjection: () => Promise<boolean>;
  saveRelayFile: (kind: "config" | "auth", contents: string, silent?: boolean) => Promise<void>;
  testRelayProfile: (profile: RelayProfile) => Promise<void>;
  switchRelayProfile: (settings: BackendSettings) => Promise<void>;
  switchOfficialMode: () => Promise<void>;
  switchPureApiMode: () => Promise<void>;
  refreshLogs: () => Promise<void>;
  refreshDiagnostics: () => Promise<void>;
  copyLogs: () => Promise<void>;
  copyDiagnostics: () => Promise<void>;
  goLogs: () => Promise<void>;
  installWatcher: () => Promise<void>;
  uninstallWatcher: () => Promise<void>;
  enableWatcher: () => Promise<void>;
  disableWatcher: () => Promise<void>;
  toggleTheme: () => void;
  checkHealth: () => Promise<void>;
};

function OverviewScreen({
  overview,
  settings,
  relay,
  actions,
}: {
  overview: OverviewResult | null;
  settings: SettingsResult | null;
  relay: RelayResult | null;
  actions: Actions;
}) {
  const launchMode = settings?.settings.launchMode ?? "patch";
  const apiMode = apiModeLabel(relay);
  const historyProvider = relay?.configured ? "CodexPlusPlus" : "openai";
  const health = healthItems(overview, relay);
  return (
    <>
      <Panel className="hero-panel">
        <CardContent className="hero-content">
          <div className="hero-layout">
            <div>
              <div className="eyebrow">Codex++ 状态</div>
              <h2>{health.every((item) => item.ok) ? `当前为${apiMode}` : "有项目需要处理"}</h2>
              <p>
                历史会话会按 {historyProvider} 显示；页面增强为
                {launchMode === "relay" ? "兼容模式，插件入口相关功能自动关闭。" : "完整模式，会加载全部页面功能。"}
              </p>
            </div>
            <Toolbar>
              <Button onClick={() => void actions.checkHealth()}>
                <RefreshCw className="h-4 w-4" />
                检查
              </Button>
              <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>
                <Wrench className="h-4 w-4" />
                修复入口
              </Button>
              <Button variant="secondary" onClick={() => void actions.repairBackend()}>
                修复后端
              </Button>
            </Toolbar>
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="健康检查" detail="概览只展示关键问题，具体配置在对应页面处理" />
        <CardContent>
          <div className="health-grid">
            <div className={`health-item ${overview?.codex_version ? "ok" : "needs-fix"}`}>
              {overview?.codex_version ? <CheckCircle2 className="h-4 w-4" /> : <Bell className="h-4 w-4" />}
              <div>
                <strong>Codex 版本</strong>
                <span>{overview?.codex_version ?? "未检测到 Codex 应用版本。"}</span>
              </div>
              <Badge status={overview?.codex_version ? "ok" : "not_checked"} />
            </div>
            {health.map((item) => (
              <div className={`health-item ${item.ok ? "ok" : "needs-fix"}`} key={item.title}>
                {item.ok ? <CheckCircle2 className="h-4 w-4" /> : <Bell className="h-4 w-4" />}
                <div>
                  <strong>{item.title}</strong>
                  <span>{item.detail}</span>
                </div>
                <Badge status={item.status} />
              </div>
            ))}
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="最近启动" detail={overview?.logs_path ?? "暂无状态文件"} />
        <CardContent>
          <LatestLaunch status={overview?.latest_launch ?? null} />
          <Toolbar>
            <Button onClick={() => void actions.launch()}>
              <Rocket className="h-4 w-4" />
              启动 Codex++
            </Button>
            <Button variant="secondary" onClick={() => void actions.goLogs()}>
              打开日志
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function RelayScreen({
  settings,
  relay,
  relayFiles,
  ccsProviders,
  form,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  relay: RelayResult | null;
  relayFiles: RelayFilesResult | null;
  ccsProviders: CcsProvidersResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const normalized = normalizeSettings(form);
  const active = activeRelayProfile(normalized);
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
  const detailProfile = newProfileDraft || (detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null);
  const isNewProfile = !!newProfileDraft;
  const saveRelaySettings = (next: BackendSettings) => {
    onFormChange(next);
    void actions.saveSettingsValue(next, true);
  };
  useEffect(() => {
    if (!newProfileDraft && detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
      setDetailProfileId(null);
    }
  }, [detailProfileId, newProfileDraft, normalized.relayProfiles]);
  useEffect(() => {
    if (!newProfileDraft && detailProfileId === normalized.activeRelayId) {
      void actions.refreshRelayFiles();
    }
  }, [detailProfileId, newProfileDraft, normalized.activeRelayId]);

  if (detailProfile) {
    return (
      <Panel fill>
        <CardHead title="供应商详情" detail="上面修改参数，下面实时预览这个供应商自己的 config.toml / auth.json" />
        <CardContent>
          <RelayProfileDetail
            profile={detailProfile}
            relayFiles={!isNewProfile && detailProfile.id === normalized.activeRelayId ? relayFiles : null}
            form={normalized}
            isNew={isNewProfile}
            onBack={() => {
              setNewProfileDraft(null);
              setDetailProfileId(null);
            }}
            onFormChange={saveRelaySettings}
            onSaved={() => {
              setNewProfileDraft(null);
              setDetailProfileId(null);
            }}
            actions={actions}
          />
        </CardContent>
      </Panel>
    );
  }

  return (
    <>
      <Panel>
        <CardHead title="当前供应商状态" detail={relay?.configPath ?? "运行状态跟随供应商列表里的当前配置"} />
        <CardContent>
          <div className="relay-grid">
            <Metric label="当前模式" value={apiModeLabel(relay)} />
            <Metric label="ChatGPT 登录" value={relay?.authenticated ? "已检测" : "未检测"} />
            <Metric label="登录账号" value={relay?.accountLabel ?? "-"} />
            <Metric label="当前供应商" value={active.name || "-"} />
            <Metric label="接入模式" value={relayModeLabel(active.relayMode)} />
            <Metric label="上游协议" value={relayProtocolLabel(active.protocol)} />
            <Metric label="历史会话" value={relay?.configured ? "CodexPlusPlus" : "openai"} />
            <Metric label="页面增强" value={normalized.launchMode === "relay" ? "兼容模式" : "完整模式"} />
            <Metric label="配置状态" value={relay?.configured ? "已写入" : "官方默认"} />
          </div>
          <div className="hint-line">
            <ShieldCheck className="h-4 w-4" />
            <span>{relayProfileReadinessText(active, relay)}</span>
          </div>
          {relay?.backupPath ? <div className="path-line compact-path">备份：{relay.backupPath}</div> : null}
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="供应商列表" detail={`${normalized.relayProfiles.length} 个供应商配置；可拖动排序，点编辑进入详情`} />
        <CardContent>
          <div className="relay-import-row">
            <div>
              <strong>CCSwitch 配置</strong>
              <span>{ccsProviderSummary(ccsProviders)}</span>
            </div>
            <Toolbar>
              <Button onClick={() => void actions.refreshCcsProviders()} size="sm" variant="ghost">
                <RefreshCw className="h-4 w-4" />
                刷新
              </Button>
              <Button
                disabled={!ccsProviders?.providers.length}
                onClick={() => void actions.importCcsProviders()}
                size="sm"
                variant="secondary"
              >
                <Download className="h-4 w-4" />
                导入 CCSwitch 配置
              </Button>
            </Toolbar>
          </div>
          <div className="relay-add-row">
            <Button
              variant="secondary"
              onClick={() => {
                setNewProfileDraft(createRelayProfile(normalized));
                setDetailProfileId(null);
              }}
            >
              <Plus className="h-4 w-4" />
              添加供应商
            </Button>
          </div>
          <RelayProfileList
            form={normalized}
            onEdit={(profileId) => {
              setNewProfileDraft(null);
              setDetailProfileId(profileId);
            }}
            onFormChange={saveRelaySettings}
            actions={actions}
          />
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="配置文件" detail="进入某个供应商详情后可查看和保存 config.toml / auth.json" />
        <CardContent>
          <div className="path-line loose">Codex++ 设置：{settings?.settings_path ?? "未加载设置文件。"}</div>
          <div className="path-line loose">Codex config.toml：{relayFiles?.configPath ?? "-"}</div>
          <div className="path-line loose">Codex auth.json：{relayFiles?.authPath ?? "-"}</div>
        </CardContent>
      </Panel>
    </>
  );
}

function EnhanceScreen({
  form,
  onFormChange,
  actions,
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title="页面功能增强" detail="会话删除、导出、项目移动、Timeline 和用户脚本等界面能力" />
        <CardContent>
          <label className="switch-row">
            <input
              checked={form.enhancementsEnabled}
              onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启用 Codex++ 页面增强</strong>
              <small>关闭后会停用删除、导出、项目移动、Timeline、插件相关和菜单位置增强。</small>
            </span>
          </label>
          <ModeSelector launchMode={form.launchMode} actions={actions} />
          {form.launchMode === "relay" ? (
            <div className="hint-line">
              <ShieldCheck className="h-4 w-4" />
              <span>当前为兼容增强模式，插件入口解锁和特殊插件强制安装不会启用；其他页面功能仍可用。</span>
            </div>
          ) : null}
          <div className="feature-list">
            <FeatureItem title="会话删除" detail="在会话列表悬停显示删除按钮，并支持撤销。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="Markdown 导出" detail="按本地 rollout 导出带时间戳的 Markdown。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="项目移动" detail="把会话移动到普通对话或其他本地项目。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="Timeline" detail="在对话右侧显示用户提问时间线。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="插件入口解锁" detail="仅完整增强模式启用。" enabled={form.enhancementsEnabled && form.launchMode === "patch"} />
            <FeatureItem title="特殊插件强制安装" detail="仅完整增强模式启用。" enabled={form.enhancementsEnabled && form.launchMode === "patch"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存增强设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function UserScriptsScreen({ settings, market, actions }: { settings: SettingsResult | null; market: ScriptMarketResult | null; actions: Actions }) {
  const inventory = settings?.user_scripts;
  const scripts = inventory?.scripts ?? [];
  const marketScripts = market?.market.scripts ?? [];
  const installedCount = marketScripts.filter((script) => script.installed).length;
  return (
    <>
      <Panel>
        <CardHead title="脚本市场" detail={`${marketScripts.length} 个市场脚本，已安装 ${installedCount} 个，本地整体 ${inventory?.enabled === false ? "关闭" : "开启"}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="市场状态" value={market?.market.message ?? "尚未刷新"} />
            <Metric label="远程脚本" value={`${marketScripts.length} 个`} />
            <Metric label="已安装" value={`${installedCount} 个`} />
            <Metric label="本地整体" value={inventory?.enabled === false ? "关闭" : "开启"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshScriptMarket()}>
              <RefreshCw className="h-4 w-4" />
              刷新市场
            </Button>
            <Button onClick={() => void actions.openExternalUrl(SCRIPT_MARKET_REPOSITORY_URL)} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              投稿
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} variant="secondary">
              <RefreshCw className="h-4 w-4" />
              刷新本地
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="市场脚本" detail={market?.market.updatedAt ? `清单更新时间：${market.market.updatedAt}` : "从 GitHub 静态清单加载"} />
        <CardContent>
          {marketScripts.length ? (
            <div className="script-market-grid">
              {marketScripts.map((script) => (
                <MarketScriptCard key={script.id} script={script} actions={actions} />
              ))}
            </div>
          ) : (
            <div className="empty">{market?.status === "failed" ? market.message : "点击刷新市场加载远程脚本。"}</div>
          )}
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="本地脚本" detail="内置、手动和市场安装脚本；单脚本启停仍在 Codex++ 注入菜单中管理" />
        <CardContent>
          <div className="table">
            {scripts.length ? scripts.map((script) => <ScriptRow key={script.key} script={script} />) : <div className="empty">未发现用户脚本。</div>}
          </div>
        </CardContent>
      </Panel>
    </>
  );
}

function ProviderSyncScreen({
  settings,
  form,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title="历史会话修复" detail="切换官方或 API 后，让旧对话重新出现在当前模式下" />
        <CardContent>
          <label className="switch-row">
            <input
              checked={form.providerSyncEnabled}
              onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启动前自动修复历史会话</strong>
              <small>开启后，通过 Codex++ 启动 Codex 前自动整理一次旧对话的归属标记。</small>
            </span>
          </label>
          <div className="relay-grid compact">
            <Metric label="自动修复" value={form.providerSyncEnabled ? "启动前执行" : "关闭"} />
            <Metric label="设置文件" value={settings?.settings_path ?? "未加载"} />
            <Metric label="页面增强" value={form.launchMode === "relay" ? "兼容模式" : "完整模式"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存自动修复设置</Button>
            <Button onClick={() => void actions.syncProvidersNow()} variant="outline">
              <RefreshCw className="h-4 w-4" />
              立刻修复历史会话
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="说明" detail="这是独立于页面增强的会话数据维护功能" />
        <CardContent>
          <GuideList
            items={[
              "自动修复只在 Codex++ 启动 Codex 前运行，不会常驻监控或反复改写。",
              "需要马上整理旧对话时，可以点击“立刻修复历史会话”。",
              "它不控制页面功能，也不影响 API URL 或 Key。",
              "切回官方时历史会话会整理为 openai；切到 API 时会整理为 CodexPlusPlus。",
            ]}
          />
        </CardContent>
      </Panel>
    </>
  );
}

function RecommendationsScreen({ ads, actions }: { ads: AdsResult | null; actions: Actions }) {
  const items = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad));
  const sponsors = items.filter((ad) => ad.type === "sponsor");
  const normal = items.filter((ad) => ad.type === "normal");
  return (
    <>
      <Panel>
        <CardHead title="推荐内容" detail="与 Codex 内插件菜单使用同一个远端广告源" />
        <CardContent>
          <div className="recommend-hero">
            <div>
              <strong>{ads ? `已加载 ${items.length} 条推荐` : "尚未加载推荐内容"}</strong>
              <span>内容来自 BigPizzaV3/Ad-List，分为赞助商推荐和普通推荐。</span>
            </div>
            <Button onClick={() => void actions.refreshAds()}>
              <RefreshCw className="h-4 w-4" />
              刷新推荐
            </Button>
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="赞助商推荐" detail={`${sponsors.length} 条`} />
        <CardContent>
          <AdGrid actions={actions} ads={sponsors} empty="暂无赞助商推荐。" />
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="普通推荐" detail={`${normal.length} 条`} />
        <CardContent>
          <AdGrid actions={actions} ads={normal} empty="暂无普通推荐。" />
        </CardContent>
      </Panel>
    </>
  );
}

function MaintenanceScreen({
  overview,
  watcher,
  settings,
  launchForm,
  onLaunchFormChange,
  removeOwnedData,
  onRemoveOwnedDataChange,
  actions,
}: {
  overview: OverviewResult | null;
  watcher: WatcherResult | null;
  settings: SettingsResult | null;
  launchForm: { appPath: string; debugPort: string; helperPort: string };
  onLaunchFormChange: (next: { appPath: string; debugPort: string; helperPort: string }) => void;
  removeOwnedData: boolean;
  onRemoveOwnedDataChange: (value: boolean) => void;
  actions: Actions;
}) {
  const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
  return (
    <>
      <Panel>
        <CardHead title="检查与修复" detail="检查入口、Codex 应用和 Watcher 状态" />
        <CardContent>
          <div className="status-table">
            <StatusRow title="Codex 应用" status={overview?.codex_app.status} path={overview?.codex_app.path} />
            <StatusRow title="静默启动入口" status={overview?.silent_shortcut.status} path={overview?.silent_shortcut.path} />
            <StatusRow title="管理控制台入口" status={overview?.management_shortcut.status} path={overview?.management_shortcut.path} />
            <StatusRow title="Watcher 自动接管" status={watcher?.enabled ? "ok" : "disabled"} path={watcher?.disabled_flag} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkHealth()}>检查</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>修复快捷方式</Button>
            <Button variant="secondary" onClick={() => void actions.repairBackend()}>修复后端</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="入口管理" detail="快捷方式写入系统实际桌面位置，不使用写死桌面路径" />
        <CardContent>
          <label className="check-row">
            <input checked={removeOwnedData} onChange={(event) => onRemoveOwnedDataChange(event.currentTarget.checked)} type="checkbox" />
            <span>卸载时移除 Codex++ 托管数据</span>
          </label>
          <Toolbar>
            <Button onClick={() => void actions.installEntrypoints()}>安装入口</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallEntrypoints()}>卸载入口</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>修复入口</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="自动接管" detail="Watcher 用于保持 Codex++ 接管状态" />
        <CardContent>
          <Toolbar>
            <Button variant="secondary" onClick={() => void actions.installWatcher()}>安装 watcher</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallWatcher()}>移除 watcher</Button>
            <Button variant="secondary" onClick={() => void actions.enableWatcher()}>启用</Button>
            <Button variant="secondary" onClick={() => void actions.disableWatcher()}>禁用</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="Codex 应用路径" detail="免安装版或解包版只需要选择一次，之后静默启动会自动复用" />
        <CardContent>
          <div className="status-table">
            <StatusRow title="保存路径" status={savedCodexAppPath ? "ok" : "not_checked"} path={savedCodexAppPath || null} />
            <StatusRow title="当前识别" status={overview?.codex_app.status} path={overview?.codex_app.path} />
          </div>
          <Field label="保存的应用路径">
            <Input
              value={settings?.settings.codexAppPath ?? ""}
              placeholder="选择 Codex.exe、Codex.app、app 目录或解包目录"
              readOnly
            />
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.chooseCodexAppPath("folder")}>选择应用目录</Button>
            <Button variant="secondary" onClick={() => void actions.chooseCodexAppPath("file")}>选择 Codex.exe</Button>
            <Button variant="secondary" onClick={() => void actions.clearCodexAppPath()}>清除保存路径</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="手动启动" detail="应用路径留空时使用已保存路径；没有保存路径时使用自动探测" />
        <CardContent>
          <Field label="应用路径覆盖">
            <Input
              value={launchForm.appPath}
              onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })}
              placeholder={savedCodexAppPath || "例如 C:\\Program Files\\WindowsApps\\OpenAI.Codex...\\app"}
            />
          </Field>
          <div className="form-row">
            <Field label="Debug 端口">
              <Input
                value={launchForm.debugPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}
              />
            </Field>
            <Field label="Helper 端口">
              <Input
                value={launchForm.helperPort}
                onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}
              />
            </Field>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.launch()}>启动 Codex++</Button>
            <Button variant="secondary" onClick={() => void actions.saveManualCodexAppPath()}>
              保存为默认路径
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function AboutScreen({
  overview,
  update,
  actions,
}: {
  overview: OverviewResult | null;
  update: UpdateResult | null;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title="关于 Codex++" detail="本地 Codex 增强、管理工具和安装包维护" />
        <CardContent>
          <div className="metric-list">
            <Metric label="Codex++ 版本" value={overview?.current_version ?? update?.currentVersion ?? "-"} />
            <Metric label="Codex 版本" value={overview?.codex_version ?? "未检测到"} />
            <Metric label="项目地址" value="github.com/BigPizzaV3/CodexPlusPlus" />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus")} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              打开项目主页
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://github.com/BigPizzaV3/CodexPlusPlus/issues")} variant="secondary">
              <ExternalLink className="h-4 w-4" />
              反馈问题
            </Button>
            <Button onClick={() => void actions.openExternalUrl("https://discord.gg/y96kX7A76v")} variant="secondary">
              <MessageCircle className="h-4 w-4" />
              Discord
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="GitHub Release 更新" detail={`当前版本 ${overview?.current_version ?? update?.currentVersion ?? "-"}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="状态" value={update?.status ?? "not_checked"} />
            <Metric label="最新版本" value={update?.latestVersion ?? "未检查"} />
            <Metric label="资源" value={update?.assetName ?? "-"} />
            <Metric label="进度" value={`${update?.progress ?? 0}%`} />
          </div>
          <Textarea className="log-view" readOnly value={update?.releaseSummary || update?.message || "尚未检查 GitHub Release；更新会下载并启动安装包。"} />
          <Toolbar>
            <Button onClick={() => void actions.checkUpdate()}>检查更新</Button>
            <Button variant="secondary" onClick={() => void actions.performUpdate()}>下载并运行安装包</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function SettingsScreen({
  settings,
  theme,
  form,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  theme: Theme;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  return (
    <>
      <Panel>
        <CardHead title="基础设置" detail={settings?.settings_path ?? ""} />
        <CardContent>
          <div className="theme-row">
            <div>
              <strong>界面主题</strong>
              <span>当前为{theme === "dark" ? "深色" : "浅色"}模式。</span>
            </div>
            <Button variant="secondary" onClick={actions.toggleTheme}>切换主题</Button>
          </div>
          <Field label="供应商测试模型">
            <Input
              value={form.relayTestModel}
              onChange={(event) => onFormChange({ ...form, relayTestModel: event.currentTarget.value })}
              placeholder="例如 gpt-5-mini"
            />
          </Field>
          <label className="check-row">
            <input
              checked={form.cliWrapperEnabled}
              onChange={(event) => onFormChange({ ...form, cliWrapperEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>启用 Codex 命令包装器</span>
          </label>
          <div className="form-row">
            <Field label="包装器 Base URL">
              <Input
                value={form.cliWrapperBaseUrl}
                onChange={(event) => onFormChange({ ...form, cliWrapperBaseUrl: event.currentTarget.value })}
              />
            </Field>
            <Field label="API Key 环境变量">
              <Input
                value={form.cliWrapperApiKeyEnv}
                onChange={(event) => onFormChange({ ...form, cliWrapperApiKeyEnv: event.currentTarget.value })}
              />
            </Field>
          </div>
          <Field label="API Key">
            <Input
              type="password"
              value={form.cliWrapperApiKey}
              onChange={(event) => onFormChange({ ...form, cliWrapperApiKey: event.currentTarget.value })}
            />
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存设置</Button>
            <Button variant="secondary" onClick={() => void actions.resetSettings()}>
              重置设置
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="Codex 启动参数" detail="启动 Codex App 时追加到默认 CDP 参数后。留空则保持默认启动行为。" />
        <CardContent>
          <Field label="额外参数">
            <Textarea
              className="launch-args-input"
              placeholder="--force_high_performance_gpu"
              spellCheck={false}
              value={codexExtraArgsToInput(form.codexExtraArgs)}
              onChange={(event) =>
                onFormChange({
                  ...form,
                  codexExtraArgs: inputToCodexExtraArgs(event.currentTarget.value),
                })
              }
            />
          </Field>
          <p className="field-hint">每行一个参数，例如 --force_high_performance_gpu。不需要填写 open 或 --args。</p>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function LogsScreen({ logs, actions }: { logs: LogsResult | null; actions: Actions }) {
  const lines = splitLogLines(logs?.text ?? "");
  return (
    <Panel fill>
      <CardHead title="最近日志" detail={logs?.path ?? ""} />
      <CardContent>
        <div className="log-lines">
          {lines.length ? (
            lines.map((line, index) => (
              <div className="log-line" key={`${index}-${line.slice(0, 12)}`}>
                <span>{index + 1}</span>
                <code>{line || " "}</code>
              </div>
            ))
          ) : (
            <div className="empty">暂无日志。</div>
          )}
        </div>
        <Toolbar>
          <Button onClick={() => void actions.refreshLogs()}>刷新</Button>
          <Button variant="secondary" onClick={() => void actions.copyLogs()}>
            复制
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function DiagnosticsScreen({ diagnostics, actions }: { diagnostics: DiagnosticsResult | null; actions: Actions }) {
  return (
    <Panel fill>
      <CardHead title="诊断报告" detail="包含版本、路径、设置和平台信息" />
      <CardContent>
        <Textarea className="log-view tall" readOnly value={diagnostics?.report ?? "尚未生成诊断报告。"} />
        <Toolbar>
          <Button onClick={() => void actions.refreshDiagnostics()}>重新生成</Button>
          <Button variant="secondary" onClick={() => void actions.copyDiagnostics()}>
            复制报告
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>
  );
}

function RelayProfileList({
  form,
  onFormChange,
  onEdit,
  actions,
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  actions: Actions;
}) {
  const sensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  const handleDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const next = reorderRelayProfiles(form, String(active.id), String(over.id));
    if (next !== form) onFormChange(next);
  };
  return (
    <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={form.relayProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {form.relayProfiles.map((profile, index) => (
            <SortableRelayProfileCard
              actions={actions}
              form={form}
              index={index}
              key={profile.id}
              onEdit={onEdit}
              onFormChange={onFormChange}
              profile={profile}
            />
          ))}
        </div>
      </SortableContext>
    </DndContext>
  );
}

function SortableRelayProfileCard({
  form,
  profile,
  index,
  onFormChange,
  onEdit,
  actions,
}: {
  form: BackendSettings;
  profile: RelayProfile;
  index: number;
  onFormChange: (value: BackendSettings) => void;
  onEdit: (id: string) => void;
  actions: Actions;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: profile.id });
  const active = profile.id === form.activeRelayId;
  const style: CSSProperties = {
    transform: CSS.Transform.toString(transform),
    transition,
  };

  return (
    <div
      className={`relay-profile-card ${active ? "active" : ""} ${isDragging ? "dragging" : ""}`}
      data-relay-profile-id={profile.id}
      key={profile.id}
      onKeyDown={(event) => {
        if (event.key === "Enter") onEdit(profile.id);
      }}
      ref={setNodeRef}
      style={style}
      tabIndex={0}
    >
      <button
        aria-label="拖动排序"
        className="relay-drag"
        title="拖动排序"
        type="button"
        {...attributes}
        {...listeners}
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="relay-index" title={profile.name || "未命名供应商"}>
        {providerInitial(profile.name)}
      </span>
      <span className="relay-summary">
        <strong>{profile.name || "未命名供应商"}</strong>
        <small>{relayModeLabel(profile.relayMode)} · {relayProtocolLabel(profile.protocol)} · {relayProfileConfigBrief(profile)}</small>
      </span>
      <span className="relay-card-actions">
        <Button
          className={`relay-use-button ${active ? "active" : ""}`}
          onClick={(event) => {
            event.stopPropagation();
            const next = syncLegacyRelayFields({ ...form, activeRelayId: profile.id });
            void actions.switchRelayProfile(next);
          }}
          size="sm"
          title={active ? "当前正在使用" : "设为当前"}
          variant={active ? "secondary" : "outline"}
        >
          <CheckCircle2 className="h-4 w-4" />
          {active ? "使用中" : "使用"}
        </Button>
        <span className="relay-card-extra">
          <Button
            onClick={(event) => {
              event.stopPropagation();
              void actions.testRelayProfile(profile);
            }}
            size="icon"
            title="发送 hi 测试"
            variant="ghost"
          >
            <TestTube className="h-4 w-4" />
          </Button>
          <Button
            onClick={(event) => {
              event.stopPropagation();
              onEdit(profile.id);
            }}
            size="icon"
            title="编辑"
            variant="ghost"
          >
            <Edit3 className="h-4 w-4" />
          </Button>
          <Button
            onClick={(event) => {
              event.stopPropagation();
              onFormChange(duplicateRelayProfile(form, profile.id));
            }}
            size="icon"
            title="复制"
            variant="ghost"
          >
            <Copy className="h-4 w-4" />
          </Button>
          <Button
            disabled={form.relayProfiles.length <= 1}
            onClick={(event) => {
              event.stopPropagation();
              onFormChange(removeRelayProfile(form, profile.id));
            }}
            size="icon"
            title="删除供应商"
            variant="ghost"
          >
            <Trash2 className="h-4 w-4" />
          </Button>
        </span>
      </span>
    </div>
  );
}

function MarketScriptCard({ script, actions }: { script: ScriptMarketItem; actions: Actions }) {
  const status = script.updateAvailable ? "可更新" : script.installed ? `已安装 ${script.installedVersion}` : "未安装";
  return (
    <div className="script-market-card">
      <div className="script-market-title">
        <div>
          <strong>{script.name}</strong>
          <span>{script.author || "未知作者"}</span>
        </div>
        <UiBadge variant={script.updateAvailable ? "default" : script.installed ? "secondary" : "outline"}>{status}</UiBadge>
      </div>
      <p className="script-market-description">{script.description || "暂无描述。"}</p>
      <div className="script-market-tags">
        <span className="script-market-tag">v{script.version}</span>
        {script.tags.map((tag) => (
          <span className="script-market-tag" key={tag}>{tag}</span>
        ))}
      </div>
      <div className="script-market-actions">
        <Button onClick={() => void actions.installMarketScript(script.id)} size="sm">
          <Download className="h-4 w-4" />
          {script.updateAvailable ? "更新" : script.installed ? "重新安装" : "安装"}
        </Button>
        {script.homepage ? (
          <Button onClick={() => void actions.openExternalUrl(script.homepage)} size="sm" variant="secondary">
            <ExternalLink className="h-4 w-4" />
            主页
          </Button>
        ) : null}
      </div>
    </div>
  );
}

function RelayProfileDetail({
  profile,
  relayFiles,
  form,
  isNew = false,
  onBack,
  onFormChange,
  onSaved,
  actions,
}: {
  profile: RelayProfile;
  relayFiles: RelayFilesResult | null;
  form: BackendSettings;
  isNew?: boolean;
  onBack: () => void;
  onFormChange: (value: BackendSettings) => void;
  onSaved?: () => void;
  actions: Actions;
}) {
  const [draft, setDraft] = useState<RelayProfile>(profile);
  const isActive = !isNew && profile.id === form.activeRelayId;
  useEffect(() => {
    setDraft(
      isActive && relayFiles
        ? { ...profile, configContents: relayFiles.configContents, authContents: relayFiles.authContents }
        : profile,
    );
  }, [profile.id, isActive, isNew, relayFiles?.configContents, relayFiles?.authContents]);
  const saveDraft = async () => {
    const next = isNew ? addRelayProfile(form, draft) : updateRelayProfile(form, profile.id, draft);
    onFormChange(next);
    if (isActive) {
      await actions.saveRelayFile("config", draft.configContents, true);
      await actions.saveRelayFile("auth", draft.authContents, true);
    }
    onSaved?.();
  };
  const switchDraft = () => {
    if (isNew) return;
    const next = syncLegacyRelayFields({
      ...form,
      relayProfiles: form.relayProfiles.map((item) => (item.id === profile.id ? draft : item)),
      activeRelayId: profile.id,
    });
    void actions.switchRelayProfile(next);
  };
  return (
    <div className="relay-detail-page">
      <Toolbar>
        <Button onClick={onBack} variant="secondary">
          <ArrowLeft className="h-4 w-4" />
          返回列表
        </Button>
        <Button onClick={() => void saveDraft()}>
          <Save className="h-4 w-4" />
          保存
        </Button>
      </Toolbar>
      <RelayProfileEditor profile={draft} form={form} isNew={isNew} onProfileChange={setDraft} onSwitch={switchDraft} />
      <RelayFileEditors profile={draft} isActive={isActive} onProfileChange={setDraft} />
    </div>
  );
}

function RelayProfileEditor({
  profile,
  form,
  isNew = false,
  onProfileChange,
  onSwitch,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  isNew?: boolean;
  onProfileChange: (value: RelayProfile) => void;
  onSwitch: () => void;
}) {
  const showApiFields = profile.relayMode !== "official" || profile.officialMixApiKey;
  const updateDraft = (patch: Partial<RelayProfile>) => {
    const shouldRegenerateFiles = ["baseUrl", "apiKey", "protocol", "relayMode", "officialMixApiKey"].some((key) => key in patch);
    const updated = { ...profile, ...patch };
    onProfileChange(shouldRegenerateFiles ? withGeneratedRelayFiles(updated) : updated);
  };
  return (
    <div className="relay-profile-editor">
      <div className="relay-editor-head">
        <div>
          <strong>{profile.name || "未命名供应商"}</strong>
          <span>{isNew ? "新建供应商需要先保存到列表" : profile.id === form.activeRelayId ? "当前正在使用" : "编辑后保存列表，再切换模式时会使用新配置"}</span>
        </div>
        {isNew ? null : (
          <Button
            onClick={onSwitch}
            variant={profile.id === form.activeRelayId ? "secondary" : "default"}
          >
            {profile.id === form.activeRelayId ? "使用中" : "设为当前"}
          </Button>
        )}
      </div>
      <div className="relay-fields">
        <Field className="relay-field-name" label="名称">
          <Input
            value={profile.name}
            onChange={(event) => updateDraft({ name: event.currentTarget.value })}
          />
        </Field>
        <Field className="relay-field-mode" label="接入模式">
          <select
            className="field-select"
            value={profile.relayMode}
            onChange={(event) => {
              const relayMode = event.currentTarget.value as RelayMode;
              updateDraft(relayMode === "official" ? { relayMode, officialMixApiKey: false } : { relayMode });
            }}
          >
            <option value="official">官方登录</option>
            <option value="pureApi">纯 API</option>
          </select>
        </Field>
        <Field className="relay-field-test-model" label="测试模型">
          <Input
            value={profile.testModel}
            onChange={(event) => updateDraft({ testModel: event.currentTarget.value })}
            placeholder={`留空使用默认：${form.relayTestModel || defaultSettings.relayTestModel}`}
          />
        </Field>
        {profile.relayMode === "official" ? (
          <Field className="relay-field-official-key" label="API Key">
            <label className="inline-check">
              <input
                checked={profile.officialMixApiKey}
                onChange={(event) => updateDraft({ officialMixApiKey: event.currentTarget.checked })}
                type="checkbox"
              />
              <span>混入 API KEY</span>
            </label>
          </Field>
        ) : null}
        {showApiFields ? (
          <>
            <Field className="relay-field-base-url" label="Base URL">
              <Input
                value={profile.baseUrl}
                onChange={(event) => updateDraft({ baseUrl: event.currentTarget.value })}
                placeholder="填写中转服务 Base URL"
              />
            </Field>
            <Field className="relay-field-key" label="Key">
              <Input
                type="password"
                value={profile.apiKey}
                onChange={(event) => updateDraft({ apiKey: event.currentTarget.value })}
                placeholder="输入中转服务的 API Key"
              />
            </Field>
            <Field className="relay-field-protocol" label="上游协议">
              <div className="protocol-options">
                <button
                  className={`protocol-option ${profile.protocol === "responses" ? "active" : ""}`}
                  onClick={() => updateDraft({ protocol: "responses" })}
                  type="button"
                >
                  Responses API
                </button>
                <button
                  className={`protocol-option ${profile.protocol === "chatCompletions" ? "active" : ""}`}
                  onClick={() => updateDraft({ protocol: "chatCompletions" })}
                  type="button"
                >
                  Chat Completions
                </button>
              </div>
            </Field>
          </>
        ) : null}
      </div>
      {showApiFields && profile.protocol === "chatCompletions" ? (
        <div className="hint-line relay-protocol-hint">
          <MessageCircle className="h-4 w-4" />
          <span>此上游会通过本地 127.0.0.1:57321 转成 Responses API，需要从 Codex++ 启动 Codex。</span>
        </div>
      ) : null}
      <div className="hint-line relay-protocol-hint">
        <ShieldCheck className="h-4 w-4" />
        <span>{relayProfileModeHelp(profile)}</span>
      </div>
    </div>
  );
}

function RelayFileEditors({
  profile,
  isActive,
  onProfileChange,
}: {
  profile: RelayProfile;
  isActive: boolean;
  onProfileChange: (value: RelayProfile) => void;
}) {
  return (
    <div className="relay-file-grid">
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>config.toml</strong>
            <span>{isActive ? "当前使用中：打开时从 ~/.codex/config.toml 回填，保存时写回真实文件" : "切换到此供应商时完整写入 ~/.codex/config.toml"}</span>
          </div>
        </div>
        <Textarea
          className="relay-file-textarea"
          value={profile.configContents}
          onChange={(event) => onProfileChange({ ...profile, configContents: event.currentTarget.value })}
          spellCheck={false}
        />
      </div>
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>auth.json</strong>
            <span>{isActive ? "当前使用中：打开时从 ~/.codex/auth.json 回填，保存时写回真实文件" : "切换到此供应商时完整写入 ~/.codex/auth.json"}</span>
          </div>
        </div>
        <Textarea
          className="relay-file-textarea"
          value={profile.authContents}
          onChange={(event) => onProfileChange({ ...profile, authContents: event.currentTarget.value })}
          spellCheck={false}
        />
      </div>
    </div>
  );
}

function ModeSelector({ launchMode, actions }: { launchMode: LaunchMode; actions: Actions }) {
  return (
    <div className="mode-grid">
      <button
        className={`mode-option ${launchMode === "relay" ? "active" : ""}`}
        onClick={() => void actions.setLaunchMode("relay")}
        type="button"
      >
        <strong>兼容增强</strong>
        <span>适合官方登录或官方混入 API Key；保留会话删除、导出、项目移动、Timeline 和用户脚本，关闭插件入口相关增强。</span>
      </button>
      <button
        className={`mode-option ${launchMode === "patch" ? "active" : ""}`}
        onClick={() => void actions.setLaunchMode("patch")}
        type="button"
      >
        <strong>完整增强</strong>
        <span>适合纯 API；启用插件入口、强制安装、会话删除导出、项目移动等全部页面能力。</span>
      </button>
    </div>
  );
}

function FeatureItem({ title, detail, enabled }: { title: string; detail: string; enabled: boolean }) {
  return (
    <div className="feature-item">
      <div>
        <strong>{title}</strong>
        <span>{detail}</span>
      </div>
      <Badge status={enabled ? "ok" : "disabled"} />
    </div>
  );
}

function GuideList({ items }: { items: string[] }) {
  return (
    <div className="guide-list">
      {items.map((item, index) => (
        <div className="guide-step" key={item}>
          <span>{index + 1}</span>
          <p>{item}</p>
        </div>
      ))}
    </div>
  );
}

function NoticeDialog({
  notice,
  onClose,
}: {
  notice: { title: string; message: string; status?: Status };
  onClose: () => void;
}) {
  useEffect(() => {
    const timer = window.setTimeout(onClose, 4200);
    return () => window.clearTimeout(timer);
  }, []);

  return (
    <div className="toast-wrap" role="status" aria-live="polite">
      <div className={`toast-card ${notice.status === "failed" ? "failed" : ""}`}>
        <div className="toast-progress" />
        <div className="toast-icon">
          {notice.status === "failed" ? <Bell className="h-5 w-5" /> : <CheckCircle2 className="h-5 w-5" />}
        </div>
        <div className="toast-body">
          <h2>{notice.title}</h2>
          <p>{notice.message}</p>
        </div>
        <button className="toast-close" onClick={onClose} type="button">×</button>
      </div>
    </div>
  );
}

function Panel({ children, fill = false, className = "" }: { children: React.ReactNode; fill?: boolean; className?: string }) {
  return (
    <Card className={`panel ${fill ? "fill" : ""} ${className}`}>
      {children}
    </Card>
  );
}

function CardHead({ title, detail }: { title: string; detail: string }) {
  return (
    <CardHeader className="panel-head">
      <CardTitle>{title}</CardTitle>
      <CardDescription>{detail}</CardDescription>
    </CardHeader>
  );
}

function Toolbar({ children }: { children: React.ReactNode }) {
  return <div className="toolbar">{children}</div>;
}

function Field({ label, children, className = "" }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <Label className={`field ${className}`}>
      <span>{label}</span>
      {children}
    </Label>
  );
}

function StatusRow({ title, status = "unknown", path }: { title: string; status?: string; path?: string | null }) {
  return (
    <div className="status-row">
      <span>{title}</span>
      <Badge status={status} />
      <code>{path || "未记录路径"}</code>
    </div>
  );
}

function Badge({ status }: { status: string }) {
  return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}

function LatestLaunch({ status }: { status: LaunchStatus | null }) {
  if (!status) return <div className="empty">暂无启动状态。</div>;
  return (
    <div className="metric-list">
      <Metric label="状态" value={status.status} />
      <Metric label="消息" value={status.message} />
      <Metric label="Debug" value={String(status.debug_port ?? "-")} />
      <Metric label="Helper" value={String(status.helper_port ?? "-")} />
      <Metric label="时间" value={formatTime(status.started_at_ms)} />
    </div>
  );
}

function Metric({ label, value }: { label: string; value: string }) {
  return (
    <div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function ScriptRow({ script }: { script: NonNullable<UserScriptInventory["scripts"]>[number] }) {
  const source = script.market_id ? `市场 · ${script.version || "未知版本"}` : script.source === "builtin" ? "内置" : "用户";
  return (
    <div className="table-row">
      <span>{script.name}</span>
      <span>{source}</span>
      <span>{script.enabled ? "启用" : "关闭"}</span>
      <span>{script.status}</span>
    </div>
  );
}

function AdGrid({ ads, empty, actions }: { ads: AdItem[]; empty: string; actions: Actions }) {
  if (!ads.length) return <div className="empty">{empty}</div>;
  return (
    <div className="ad-grid">
      {ads.map((ad) => (
        <button className="ad-card" key={ad.id || `${ad.type}-${ad.title}`} onClick={() => void actions.openExternalUrl(ad.url)} type="button">
          <div>
            <strong>{ad.title}</strong>
            <p>{ad.description}</p>
          </div>
          {ad.highlights?.length ? (
            <div className="ad-tags">
              {ad.highlights.map((item) => (
                <span key={item}>{item}</span>
              ))}
            </div>
          ) : null}
          <span className="ad-link">
            打开
            <ExternalLink className="h-4 w-4" />
          </span>
        </button>
      ))}
    </div>
  );
}

function isExpiredAd(ad: AdItem) {
  if (!ad.expires_at) return false;
  const expiresAt = Date.parse(ad.expires_at);
  return Number.isFinite(expiresAt) && expiresAt < Date.now();
}

function routeTitle(route: Route) {
  return routes.find((item) => item.id === route)?.label ?? "概览";
}

function routeSubtitle(route: Route) {
  const subtitles: Record<Route, string> = {
    overview: "检查问题、启动与快速修复",
    relay: "管理 API 供应商、协议、Key 与配置文件",
    enhance: "会话删除、导出、项目移动和脚本能力",
    userScripts: "内置和用户自定义脚本清单",
    providerSync: "切换模式后让旧对话重新可见",
    recommendations: "赞助商推荐与普通推荐",
    maintenance: "入口安装、修复、Watcher 与手动启动",
    about: "版本信息、项目链接与 GitHub Release 更新",
    settings: "主题、命令包装器和启动参数",
    logs: "最近状态文件内容",
    diagnostics: "可复制的运行诊断报告",
  };
  return subtitles[route];
}

function ccsProviderSummary(result: CcsProvidersResult | null) {
  if (!result) return "尚未读取 CCS 数据库。";
  if (!isSuccessStatus(result.status)) return result.message;
  if (!result.providers.length) return `未发现 CCS Codex 供应商：${result.dbPath}`;
  return `发现 ${result.providers.length} 个 CCS Codex 供应商：${result.dbPath}`;
}

function providerInitial(name: string) {
  const trimmed = (name || "供应商").trim();
  return Array.from(trimmed)[0]?.toUpperCase() || "供";
}

function statusLabel(status: string) {
  const labels: Record<string, string> = {
    found: "已找到",
    missing: "缺失",
    installed: "已安装",
    ok: "正常",
    running: "运行中",
    failed: "失败",
    accepted: "已受理",
    not_checked: "未检查",
    not_implemented: "未实现",
    disabled: "已禁用",
    unknown: "未知",
  };
  return labels[status] ?? status;
}

function statusClass(status: string) {
  if (["found", "installed", "ok", "running"].includes(status)) return "good";
  if (["failed", "missing"].includes(status)) return "bad";
  return "warn";
}

function isSuccessStatus(status?: Status) {
  return status === "ok" || status === "accepted";
}

function apiModeLabel(relay: RelayResult | null) {
  if (!relay?.configured) return "官方登录";
  return relay.authenticated ? "官方混入 API Key" : "纯 API";
}

function healthItems(overview: OverviewResult | null, relay: RelayResult | null) {
  return [
    {
      title: "Codex 应用",
      status: overview?.codex_app.status ?? "not_checked",
      ok: overview?.codex_app.status === "found",
      detail: overview?.codex_app.path || "尚未检查 Codex 应用路径。",
    },
    {
      title: "静默启动入口",
      status: overview?.silent_shortcut.status ?? "not_checked",
      ok: overview?.silent_shortcut.status === "installed",
      detail: overview?.silent_shortcut.path || "缺少 Codex++ 静默启动快捷方式时可在安装维护页修复。",
    },
    {
      title: "管理工具入口",
      status: overview?.management_shortcut.status ?? "not_checked",
      ok: overview?.management_shortcut.status === "installed",
      detail: overview?.management_shortcut.path || "缺少管理工具快捷方式时可在安装维护页修复。",
    },
    {
      title: "ChatGPT 登录",
      status: relay?.authenticated ? "ok" : "missing",
      ok: !!relay?.authenticated,
      detail: relay?.accountLabel || relay?.authSource || "官方混入 API Key 需要官方登录；纯 API 可不用官方登录。",
    },
  ];
}

function normalizeSettings(settings: BackendSettings): BackendSettings {
  const profiles =
    settings.relayProfiles?.length
      ? settings.relayProfiles.map(normalizeRelayProfile)
      : [
          {
            id: settings.activeRelayId || "default",
            name: "默认中转",
            baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            apiKey: settings.relayApiKey || "",
            protocol: "responses" as RelayProtocol,
            relayMode: "official" as RelayMode,
            officialMixApiKey: false,
            testModel: "",
            configContents: "",
            authContents: "",
          },
        ];
  const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
    ? settings.activeRelayId
    : profiles[0]?.id || "default";
  return syncLegacyRelayFields({ ...defaultSettings, ...settings, relayProfiles: profiles, activeRelayId });
}

function codexExtraArgsToInput(args: string[] | undefined) {
  return (args ?? []).join("\n");
}

function inputToCodexExtraArgs(value: string) {
  return value === "" ? [] : value.split(/\r?\n/);
}

function normalizeRelayProfile(profile: RelayProfile): RelayProfile {
  const legacyMixedApi = profile.relayMode === "mixedApi";
  const normalized: RelayProfile = {
    ...profile,
    protocol: profile.protocol === "chatCompletions" ? "chatCompletions" : "responses",
    relayMode: normalizeRelayMode(profile.relayMode),
    officialMixApiKey: profile.officialMixApiKey === true || legacyMixedApi,
    testModel: profile.testModel || "",
    configContents: profile.configContents || "",
    authContents: profile.authContents || "",
  };
  if (!normalized.configContents.trim() || !normalized.authContents.trim()) {
    return withGeneratedRelayFiles(normalized);
  }
  return normalized;
}

function activeRelayProfile(settings: BackendSettings): RelayProfile {
  return (
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    defaultSettings.relayProfiles[0]
  );
}

function relayProtocolLabel(protocol: RelayProtocol): string {
  return protocol === "chatCompletions" ? "Chat Completions 转 Responses" : "Responses API";
}

function normalizeRelayMode(mode: RelayMode | undefined): RelayMode {
  if (mode === "pureApi") return mode;
  return "official";
}

function relayModeLabel(mode: RelayMode): string {
  if (mode === "pureApi") return "纯 API";
  return "官方登录";
}

function relayProfileConfigBrief(profile: RelayProfile): string {
  if (profile.relayMode === "official") return profile.officialMixApiKey ? "混入 API Key" : "不写 API 文件";
  return profile.baseUrl || "未填写 URL";
}

function relayProfileModeHelp(profile: RelayProfile): string {
  if (profile.relayMode === "official") {
    if (profile.officialMixApiKey) {
      return "此供应商会保留官方登录模式，并把请求混入当前 API Key；页面增强仍使用兼容模式。";
    }
    return "此供应商会切回官方登录模式，使用 ChatGPT 官方账号，不写入 API Key。";
  }
  if (profile.relayMode === "pureApi") {
    return "此供应商会完整写入 config.toml / auth.json，并启用完整页面增强。";
  }
  return "此供应商会保留官方登录模式，并把请求混入当前 API Key；页面增强仍使用兼容模式。";
}

function relayProfileReadinessText(profile: RelayProfile, relay: RelayResult | null): string {
  if (profile.relayMode === "official") {
    if (profile.officialMixApiKey) {
      const hasApiFields = profile.baseUrl.trim() && profile.apiKey.trim();
      if (!relay?.authenticated && !hasApiFields) return "当前未登录官方账号，也未配置混入 API 的 Base URL / Key。";
      if (!relay?.authenticated) return "当前未登录官方账号；官方登录混入 API Key 需要先登录官方账号。";
      if (!hasApiFields) return "当前还没有填写混入 API 的 Base URL / Key。";
      return `官方登录已就绪：${relay.accountLabel || "已登录"}，会混入当前 API Key。`;
    }
    return relay?.authenticated
      ? `官方账号已登录：${relay.accountLabel || relay.authSource || "已检测"}。`
      : "当前未登录官方账号；切到官方登录模式后仍需要先在 Codex/ChatGPT 登录。";
  }
  const hasFiles = profile.configContents.trim() && profile.authContents.trim();
  if (!hasFiles) return "当前供应商还没有完整 config.toml / auth.json。";
  return "纯 API 就绪：会直接写入此供应商的完整 config.toml / auth.json。";
}

function relayProfileSwitchCommand(profile: RelayProfile): "clear_relay_injection" | "apply_relay_injection" | "apply_pure_api_injection" {
  if (profile.relayMode === "pureApi") return "apply_pure_api_injection";
  if (profile.configContents.trim() && profile.authContents.trim()) return "apply_relay_injection";
  return profile.officialMixApiKey ? "apply_relay_injection" : "clear_relay_injection";
}

function relayProfileModeSwitchedText(profile: RelayProfile): string {
  if (profile.relayMode === "pureApi") return "已按此供应商切换到纯 API；页面增强已设为完整增强。";
  if (profile.officialMixApiKey) return "已按此供应商使用官方登录，并混入 API Key；页面增强已设为兼容增强。";
  return "已按此供应商切回官方登录；页面增强已设为兼容增强。";
}

function withGeneratedRelayFiles(profile: RelayProfile): RelayProfile {
  if (profile.relayMode === "official") {
    return {
      ...profile,
      configContents: profile.officialMixApiKey ? buildRelayConfigToml(profile) : "",
      authContents: "",
    };
  }
  return {
    ...profile,
    configContents: buildRelayConfigToml(profile),
    authContents: buildRelayAuthJson(profile),
  };
}

function buildRelayConfigToml(profile: Pick<RelayProfile, "baseUrl" | "apiKey" | "protocol">): string {
  const baseUrl = profile.protocol === "chatCompletions" ? PROTOCOL_PROXY_BASE_URL : profile.baseUrl.trim();
  const apiKey = profile.apiKey.trim();
  return [
    'model_provider = "CodexPlusPlus"',
    "",
    "[model_providers.CodexPlusPlus]",
    'name = "CodexPlusPlus"',
    'wire_api = "responses"',
    "requires_openai_auth = true",
    `base_url = "${tomlString(baseUrl)}"`,
    `experimental_bearer_token = "${tomlString(apiKey)}"`,
    "",
  ].join("\n");
}

function buildRelayAuthJson(profile: Pick<RelayProfile, "apiKey">): string {
  return `${JSON.stringify({ OPENAI_API_KEY: profile.apiKey.trim() }, null, 2)}\n`;
}

function relayProfileSwitchValidation(profile: RelayProfile): string | null {
  if (profile.relayMode === "official" && !profile.officialMixApiKey) return null;
  if (!profile.configContents.trim()) {
    return `供应商「${profile.name || profile.id}」缺少独立 config.toml，已停止切换，避免继续显示上一套配置文件。请先在该供应商详情里保存 config.toml。`;
  }
  if (profile.relayMode !== "official" || !authJsonHasOpenAiApiKey(profile.authContents)) return null;
  const mode = profile.officialMixApiKey ? "官方混合 API" : "官方登录";
  return `${mode} 的 auth.json 检测到 OPENAI_API_KEY，这通常是纯 API 登录态。请检查此供应商的 auth.json，确认它是 ChatGPT 官方登录态后再切换。`;
}

function authJsonHasOpenAiApiKey(contents: string): boolean {
  const trimmed = contents.trim();
  if (!trimmed) return false;
  try {
    const value = JSON.parse(trimmed);
    return !!value && typeof value === "object" && typeof value.OPENAI_API_KEY === "string" && value.OPENAI_API_KEY.trim().length > 0;
  } catch {
    return /"OPENAI_API_KEY"\s*:/.test(trimmed);
  }
}

function tomlString(value: string): string {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function syncLegacyRelayFields(settings: BackendSettings): BackendSettings {
  const active = activeRelayProfile(settings);
  return {
    ...settings,
    activeRelayId: active.id,
    relayBaseUrl: active.baseUrl,
    relayApiKey: active.apiKey,
  };
}

function updateRelayProfile(settings: BackendSettings, id: string, patch: Partial<RelayProfile>): BackendSettings {
  const shouldRegenerateFiles = ["baseUrl", "apiKey", "protocol", "relayMode", "officialMixApiKey"].some((key) => key in patch);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: settings.relayProfiles.map((profile) => {
      if (profile.id !== id) return profile;
      const updated = { ...profile, ...patch };
      return shouldRegenerateFiles ? withGeneratedRelayFiles(updated) : updated;
    }),
  });
}

function createRelayProfile(settings: BackendSettings): RelayProfile {
  const id = `relay-${Date.now().toString(36)}`;
  const next = {
    id,
    name: `供应商 ${settings.relayProfiles.length + 1}`,
    baseUrl: defaultSettings.relayBaseUrl,
    apiKey: "",
    protocol: "responses" as RelayProtocol,
    relayMode: "official" as RelayMode,
    officialMixApiKey: false,
    testModel: "",
    configContents: "",
    authContents: "",
  };
  return withGeneratedRelayFiles(next);
}

function addRelayProfile(settings: BackendSettings, profile: RelayProfile): BackendSettings {
  const nextWithFiles = profile.configContents.trim() || profile.authContents.trim() ? profile : withGeneratedRelayFiles(profile);
  const activeId = settings.relayProfiles.some((item) => item.id === settings.activeRelayId)
    ? settings.activeRelayId
    : activeRelayProfile(settings).id;
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: [...settings.relayProfiles, nextWithFiles],
    activeRelayId: activeId,
  });
}

function duplicateRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === id);
  const source = settings.relayProfiles[sourceIndex] || activeRelayProfile(settings);
  const nextId = `relay-${Date.now().toString(36)}`;
  const next = {
    ...source,
    id: nextId,
    name: `${source.name || "未命名供应商"} 副本`,
  };
  const relayProfiles = [...settings.relayProfiles];
  relayProfiles.splice(sourceIndex >= 0 ? sourceIndex + 1 : relayProfiles.length, 0, next);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

function reorderRelayProfiles(settings: BackendSettings, sourceId: string, targetId: string): BackendSettings {
  if (sourceId === targetId) return settings;
  const sourceIndex = settings.relayProfiles.findIndex((profile) => profile.id === sourceId);
  const targetIndex = settings.relayProfiles.findIndex((profile) => profile.id === targetId);
  if (sourceIndex < 0 || targetIndex < 0) return settings;
  const relayProfiles = [...settings.relayProfiles];
  const [moved] = relayProfiles.splice(sourceIndex, 1);
  relayProfiles.splice(targetIndex, 0, moved);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles,
  });
}

function removeRelayProfile(settings: BackendSettings, id: string): BackendSettings {
  const profiles = settings.relayProfiles.filter((profile) => profile.id !== id);
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: profiles.length ? profiles : defaultSettings.relayProfiles,
    activeRelayId: settings.activeRelayId === id ? profiles[0]?.id || "default" : settings.activeRelayId,
  });
}

function numberOrDefault(value: string, fallback: number) {
  const parsed = Number.parseInt(value, 10);
  return Number.isFinite(parsed) ? parsed : fallback;
}

function splitLogLines(text: string) {
  return text.trimEnd().split(/\r?\n/).filter((line, index, lines) => line.length > 0 || index < lines.length - 1);
}

function formatTime(value: number) {
  if (!value) return "-";
  return new Date(value).toLocaleString("zh-CN");
}

function stringifyError(error: unknown) {
  if (error instanceof Error) return error.message;
  return String(error);
}

function loadInitialTheme(): Theme {
  if (typeof window === "undefined") return "dark";
  return window.localStorage.getItem("codex-plus-theme") === "light" ? "light" : "dark";
}

function loadInitialRoute(): Route {
  if (typeof window === "undefined") return "overview";
  const params = new URLSearchParams(window.location.search);
  if (params.get("showUpdate") === "1" || window.location.hash === "#about") {
    return "about";
  }
  return "overview";
}
