import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import {
  Activity,
  Bell,
  CheckCircle2,
  Info,
  ExternalLink,
  Hammer,
  KeyRound,
  LayoutDashboard,
  Link2,
  FileCode2,
  Moon,
  Plus,
  RefreshCw,
  Rocket,
  ScrollText,
  Settings,
  ShieldCheck,
  Sun,
  Trash2,
  Wrench,
  type LucideIcon,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

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
  providerSyncEnabled: boolean;
  enhancementsEnabled: boolean;
  launchMode: LaunchMode;
  relayBaseUrl: string;
  relayApiKey: string;
  relayProfiles: RelayProfile[];
  activeRelayId: string;
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
};

type UserScriptInventory = {
  enabled?: boolean;
  scripts?: Array<{
    key: string;
    name: string;
    source: string;
    enabled: boolean;
    status: string;
    error: string;
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

type StartupResult = CommandResult<{
  showUpdate: boolean;
}>;

type Route = "overview" | "relay" | "enhance" | "userScripts" | "providerSync" | "recommendations" | "maintenance" | "about" | "settings" | "logs" | "diagnostics";
type Theme = "dark" | "light";

const routes: Array<{ id: Route; label: string; icon: LucideIcon }> = [
  { id: "overview", label: "概览", icon: LayoutDashboard },
  { id: "relay", label: "中转注入", icon: KeyRound },
  { id: "enhance", label: "增强功能", icon: Hammer },
  { id: "userScripts", label: "用户脚本", icon: FileCode2 },
  { id: "providerSync", label: "供应商同步", icon: Link2 },
  { id: "recommendations", label: "推荐内容", icon: ExternalLink },
  { id: "maintenance", label: "安装维护", icon: Wrench },
  { id: "about", label: "关于", icon: Info },
  { id: "settings", label: "设置", icon: Settings },
  { id: "logs", label: "日志", icon: ScrollText },
  { id: "diagnostics", label: "诊断", icon: Activity },
];

const defaultSettings: BackendSettings = {
  codexAppPath: "",
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
    },
  ],
  activeRelayId: "default",
  cliWrapperEnabled: false,
  cliWrapperBaseUrl: "",
  cliWrapperApiKey: "",
  cliWrapperApiKeyEnv: "CUSTOM_OPENAI_API_KEY",
};

export function App() {
  const [theme, setTheme] = useState<Theme>(() => loadInitialTheme());
  const [route, setRoute] = useState<Route>(() => loadInitialRoute());
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState<{ title: string; message: string; status?: Status } | null>(null);
  const [overview, setOverview] = useState<OverviewResult | null>(null);
  const [settings, setSettings] = useState<SettingsResult | null>(null);
  const [relay, setRelay] = useState<RelayResult | null>(null);
  const [logs, setLogs] = useState<LogsResult | null>(null);
  const [diagnostics, setDiagnostics] = useState<DiagnosticsResult | null>(null);
  const [watcher, setWatcher] = useState<WatcherResult | null>(null);
  const [update, setUpdate] = useState<UpdateResult | null>(null);
  const [ads, setAds] = useState<AdsResult | null>(null);
  const [launchForm, setLaunchForm] = useState({
    appPath: "",
    debugPort: "9229",
    helperPort: "57321",
  });
  const [settingsForm, setSettingsForm] = useState<BackendSettings>({ ...defaultSettings });
  const [removeOwnedData, setRemoveOwnedData] = useState(false);

  const call = <T,>(command: string, args?: Record<string, unknown>) => invoke<T>(command, args);

  const run = async <T,>(task: () => Promise<T>): Promise<T | null> => {
    setBusy(true);
    try {
      return await task();
    } catch (error) {
      showNotice("调用失败", stringifyError(error), "failed");
      return null;
    } finally {
      setBusy(false);
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

  const refreshRelay = async (silent = false) => {
    const result = await run(() => call<RelayResult>("relay_status"));
    if (result) {
      setRelay(result);
      if (!silent) showResultNotice("登录状态", result, { silentSuccess: true });
    }
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
    }
    if (next === "settings") await refreshSettings(true);
    if (next === "userScripts") await refreshSettings(true);
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
      showNotice("供应商同步", result.message, result.status);
    }
  };

  const applyRelayInjection = async () => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
    } else {
      return;
    }
    const result = await run(() => call<RelayResult>("apply_relay_injection"));
    if (result) {
      setRelay(result);
      showNotice("中转配置", result.message, result.status);
    }
  };

  const applyPureApiInjection = async () => {
    const settingsResult = await run(() => call<SettingsResult>("save_settings", { settings: settingsForm }));
    if (settingsResult) {
      setSettings(settingsResult);
      setSettingsForm(normalizeSettings(settingsResult.settings));
    } else {
      return;
    }
    const result = await run(() => call<RelayResult>("apply_pure_api_injection"));
    if (result) {
      setRelay(result);
      showNotice("纯 API 模式", result.message, result.status);
    }
  };

  const clearRelayInjection = async () => {
    const result = await run(() => call<RelayResult>("clear_relay_injection"));
    if (result) {
      setRelay(result);
      showNotice("官方登录模式", result.message, result.status);
    }
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
        const next = { ...settingsForm, launchMode };
        setSettingsForm(next);
        const result = await run(() => call<SettingsResult>("save_settings", { settings: next }));
        if (result) {
          setSettings(result);
          setSettingsForm(normalizeSettings(result.settings));
          showNotice("启动模式", result.message, result.status);
        }
      },
      refreshRelay,
      refreshAds,
      openExternalUrl,
      applyRelayInjection,
      applyPureApiInjection,
      clearRelayInjection,
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
    [route, launchForm, settingsForm, removeOwnedData, update, logs, diagnostics, theme],
  );

  return (
    <div className={`shell ${theme}`}>
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark">C++</div>
          <div>
            <div className="brand-title">Codex++</div>
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
        {busy ? <div className="busy">正在处理...</div> : null}
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
              form={settingsForm}
              onFormChange={setSettingsForm}
              actions={actions}
            />
          ) : null}
          {route === "enhance" ? (
            <EnhanceScreen form={settingsForm} onFormChange={setSettingsForm} actions={actions} />
          ) : null}
          {route === "userScripts" ? <UserScriptsScreen settings={settings} actions={actions} /> : null}
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
      {notice ? <NoticeDialog notice={notice} onClose={() => setNotice(null)} /> : null}
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
  resetSettings: () => Promise<void>;
  chooseCodexAppPath: (mode: "folder" | "file") => Promise<void>;
  clearCodexAppPath: () => Promise<void>;
  saveManualCodexAppPath: () => Promise<void>;
  syncProvidersNow: () => Promise<void>;
  setLaunchMode: (launchMode: LaunchMode) => Promise<void>;
  refreshRelay: () => Promise<void>;
  refreshAds: () => Promise<void>;
  openExternalUrl: (url: string) => Promise<void>;
  applyRelayInjection: () => Promise<void>;
  applyPureApiInjection: () => Promise<void>;
  clearRelayInjection: () => Promise<void>;
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
  const health = healthItems(overview, relay);
  return (
    <>
      <Panel className="hero-panel">
        <CardContent>
          <div className="hero-layout">
            <div>
              <div className="eyebrow">Codex++ 状态</div>
              <h2>{health.every((item) => item.ok) ? "运行环境看起来正常" : "有项目需要处理"}</h2>
              <p>
                当前使用{launchMode === "relay" ? "中转注入" : "传统 patch"}模式。
                {launchMode === "relay" ? "脚本增强仍会加载，只禁用插件入口解锁和强制安装。" : "全部前端增强都会启用。"}
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
  form,
  onFormChange,
  actions,
}: {
  settings: SettingsResult | null;
  relay: RelayResult | null;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const normalized = normalizeSettings(form);
  const active = activeRelayProfile(normalized);
  return (
    <>
      <Panel>
        <CardHead title="中转注入" detail={relay?.configPath ?? "检测登录后写入 Codex 配置"} />
        <CardContent>
          <GuideList
            items={[
              "先在 Codex/ChatGPT 中使用正常 ChatGPT 账号登录，软件只读取 auth.json 判断登录态。",
              "在下方添加一个中转，填写 Base URL 和 Key，然后点选它作为当前中转。",
              "点击“写入混合API（官方+API）”，会把 Codex 配置切到 CodexPlusPlus provider，并启用 ChatGPT 登录态混合中转。",
              "点击“写入纯API模式”，会把 auth.json 改为 OPENAI_API_KEY，并使用当前中转 URL 和 Key。",
              "如果需要回到官方登录态，点击“切回官方登录模式”后再打开 Codex++ 登录官方账号。",
              "需要切换中转时，点选列表里的另一项并保存，再重新写入即可。",
            ]}
          />
          <div className="relay-grid">
            <Metric label="ChatGPT 登录" value={relay?.authenticated ? "已检测" : "未检测"} />
            <Metric label="登录账号" value={relay?.accountLabel ?? "-"} />
            <Metric label="中转配置" value={relay?.configured ? "已写入" : "未写入"} />
            <Metric label="当前中转" value={active.name || "-"} />
            <Metric label="当前模式" value={normalized.launchMode === "relay" ? "中转注入" : "传统 patch"} />
            <Metric label="OpenAI 认证" value={relay?.requiresOpenaiAuth ? "已启用" : "未启用"} />
            <Metric label="Bearer Token" value={relay?.hasBearerToken ? "已配置" : "未配置"} />
          </div>
          <div className="hint-line">
            <ShieldCheck className="h-4 w-4" />
            <span>{relay?.authSource || "只读取 auth.json 检测正常 ChatGPT 登录；写入时使用当前选中的中转 URL 和 Key。"}</span>
          </div>
          {relay?.backupPath ? <div className="path-line compact-path">备份：{relay.backupPath}</div> : null}
          <Toolbar>
            <Button variant="secondary" onClick={() => void actions.refreshRelay()}>
              检测登录
            </Button>
            <Button variant="secondary" onClick={() => void actions.clearRelayInjection()}>
              切回官方登录模式
            </Button>
            <Button variant="secondary" onClick={() => void actions.applyPureApiInjection()}>
              写入纯API模式
            </Button>
            <Button onClick={() => void actions.applyRelayInjection()}>写入混合API（官方+API）</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="中转列表" detail={`${normalized.relayProfiles.length} 个中转，可随时切换当前使用项`} />
        <CardContent>
          <RelayProfileList form={normalized} onFormChange={onFormChange} />
          <Toolbar>
            <Button
              variant="secondary"
              onClick={() => onFormChange(addRelayProfile(normalized))}
            >
              <Plus className="h-4 w-4" />
              添加中转
            </Button>
            <Button onClick={() => void actions.saveSettings()}>保存列表</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="配置文件" detail={settings?.settings_path ?? "设置文件路径"} />
        <CardContent>
          <div className="path-line loose">{settings?.settings_path ?? "未加载设置文件。"}</div>
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
        <CardHead title="增强模式" detail="中转注入会保留脚本增强，仅禁用插件入口解锁和强制安装" />
        <CardContent>
          <label className="switch-row">
            <input
              checked={form.enhancementsEnabled}
              onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启用 Codex++ 增强功能</strong>
              <small>关闭后会停用删除、导出、项目移动、Timeline、插件相关和注入菜单位置增强。</small>
            </span>
          </label>
          <ModeSelector launchMode={form.launchMode} actions={actions} />
          {form.launchMode === "relay" ? (
            <div className="hint-line">
              <ShieldCheck className="h-4 w-4" />
              <span>当前为中转注入模式，插件入口解锁和特殊插件强制安装不会启用；其他脚本增强仍会注入。</span>
            </div>
          ) : null}
          <div className="feature-list">
            <FeatureItem title="会话删除" detail="在会话列表悬停显示删除按钮，并支持撤销。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="Markdown 导出" detail="按本地 rollout 导出带时间戳的 Markdown。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="项目移动" detail="把会话移动到普通对话或其他本地项目。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="Timeline" detail="在对话右侧显示用户提问时间线。" enabled={form.enhancementsEnabled} />
            <FeatureItem title="插件入口解锁" detail="仅传统 patch 模式启用。" enabled={form.enhancementsEnabled && form.launchMode === "patch"} />
            <FeatureItem title="特殊插件强制安装" detail="仅传统 patch 模式启用。" enabled={form.enhancementsEnabled && form.launchMode === "patch"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存增强设置</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>
  );
}

function UserScriptsScreen({ settings, actions }: { settings: SettingsResult | null; actions: Actions }) {
  const inventory = settings?.user_scripts;
  const scripts = inventory?.scripts ?? [];
  return (
    <>
      <Panel>
        <CardHead title="用户脚本" detail={`${scripts.length} 个脚本，整体 ${inventory?.enabled === false ? "关闭" : "开启"}`} />
        <CardContent>
          <div className="metric-list">
            <Metric label="整体状态" value={inventory?.enabled === false ? "关闭" : "开启"} />
            <Metric label="设置文件" value={settings?.settings_path ?? "未加载"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshCurrent()}>
              <RefreshCw className="h-4 w-4" />
              刷新脚本列表
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="脚本列表" detail="插件内可启用、禁用和重新加载；管理工具用于集中查看" />
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
        <CardHead title="供应商同步" detail="启动前自动同步，也可以手动立即同步一次" />
        <CardContent>
          <label className="switch-row">
            <input
              checked={form.providerSyncEnabled}
              onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
              type="checkbox"
            />
            <span>
              <strong>启动前自动同步供应商</strong>
              <small>开启后，仅在通过 Codex++ 启动 Codex 前自动同步一次历史会话的供应商字段。</small>
            </span>
          </label>
          <div className="relay-grid compact">
            <Metric label="自动同步" value={form.providerSyncEnabled ? "启动前执行" : "关闭"} />
            <Metric label="设置文件" value={settings?.settings_path ?? "未加载"} />
            <Metric label="当前模式" value={form.launchMode === "relay" ? "中转注入" : "传统 patch"} />
          </div>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>保存自动同步设置</Button>
            <Button onClick={() => void actions.syncProvidersNow()} variant="outline">
              <RefreshCw className="h-4 w-4" />
              立刻同步一次
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title="说明" detail="这是独立于增强功能的会话数据维护功能" />
        <CardContent>
          <GuideList
            items={[
              "自动同步只在 Codex++ 启动 Codex 前运行，不会常驻监控或反复改写。",
              "需要马上整理历史会话时，可以点击“立刻同步一次”。",
              "它不控制页面注入，也不影响中转 URL 或 Key。",
              "如果你经常在官方登录、中转和其他 provider 之间切换，建议开启。",
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
}: {
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
}) {
  return (
    <div className="relay-profile-list">
      {form.relayProfiles.map((profile) => (
        <div className={`relay-profile ${profile.id === form.activeRelayId ? "active" : ""}`} key={profile.id}>
          <div className="relay-profile-head">
            <button
              className="relay-select"
              onClick={() => onFormChange(syncLegacyRelayFields({ ...form, activeRelayId: profile.id }))}
              type="button"
            >
              <strong>{profile.name || "未命名中转"}</strong>
              <span>{profile.baseUrl || "未填写 URL"}</span>
            </button>
            <Button
              disabled={form.relayProfiles.length <= 1}
              onClick={() => onFormChange(removeRelayProfile(form, profile.id))}
              size="icon"
              title="删除中转"
              variant="ghost"
            >
              <Trash2 className="h-4 w-4" />
            </Button>
          </div>
          <div className="relay-fields">
            <Field label="名称">
              <Input
                value={profile.name}
                onChange={(event) => onFormChange(updateRelayProfile(form, profile.id, { name: event.currentTarget.value }))}
              />
            </Field>
            <Field label="Base URL">
              <Input
                value={profile.baseUrl}
                onChange={(event) => onFormChange(updateRelayProfile(form, profile.id, { baseUrl: event.currentTarget.value }))}
                placeholder="填写中转服务 Base URL"
              />
            </Field>
            <Field label="Key">
              <Input
                type="password"
                value={profile.apiKey}
                onChange={(event) => onFormChange(updateRelayProfile(form, profile.id, { apiKey: event.currentTarget.value }))}
                placeholder="输入中转服务的 API Key"
              />
            </Field>
          </div>
        </div>
      ))}
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
        <strong>中转注入</strong>
        <span>使用 ChatGPT 登录态与配置文件混合中转 API，保留脚本增强，仅禁用插件入口解锁和强制安装。</span>
      </button>
      <button
        className={`mode-option ${launchMode === "patch" ? "active" : ""}`}
        onClick={() => void actions.setLaunchMode("patch")}
        type="button"
      >
        <strong>传统 patch</strong>
        <span>启用全部前端注入能力，包括插件入口解锁、强制安装、会话删除导出、项目移动等增强。</span>
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
  return (
    <div className="modal-backdrop" role="presentation" onMouseDown={onClose}>
      <div className="modal-card" role="dialog" aria-modal="true" onMouseDown={(event) => event.stopPropagation()}>
        <div className="modal-icon">
          {notice.status === "failed" ? <Bell className="h-5 w-5" /> : <CheckCircle2 className="h-5 w-5" />}
        </div>
        <div className="modal-body">
          <h2>{notice.title}</h2>
          <p>{notice.message}</p>
        </div>
        <Toolbar>
          <Button onClick={onClose}>知道了</Button>
        </Toolbar>
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

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <Label className="field">
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
  return (
    <div className="table-row">
      <span>{script.name}</span>
      <span>{script.source}</span>
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
    relay: "ChatGPT 登录态、中转列表与配置写入",
    enhance: "脚本增强与传统 patch 开关",
    userScripts: "内置和用户自定义脚本清单",
    providerSync: "切换供应商时保持历史会话可见",
    recommendations: "赞助商推荐与普通推荐",
    maintenance: "入口安装、修复、Watcher 与手动启动",
    about: "版本信息、项目链接与 GitHub Release 更新",
    settings: "主题与命令包装器设置",
    logs: "最近状态文件内容",
    diagnostics: "可复制的运行诊断报告",
  };
  return subtitles[route];
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
      detail: relay?.accountLabel || relay?.authSource || "中转注入需要先存在 auth.json 登录态。",
    },
  ];
}

function normalizeSettings(settings: BackendSettings): BackendSettings {
  const profiles =
    settings.relayProfiles?.length
      ? settings.relayProfiles
      : [
          {
            id: settings.activeRelayId || "default",
            name: "默认中转",
            baseUrl: settings.relayBaseUrl || defaultSettings.relayBaseUrl,
            apiKey: settings.relayApiKey || "",
          },
        ];
  const activeRelayId = profiles.some((profile) => profile.id === settings.activeRelayId)
    ? settings.activeRelayId
    : profiles[0]?.id || "default";
  return syncLegacyRelayFields({ ...defaultSettings, ...settings, relayProfiles: profiles, activeRelayId });
}

function activeRelayProfile(settings: BackendSettings): RelayProfile {
  return (
    settings.relayProfiles.find((profile) => profile.id === settings.activeRelayId) ||
    settings.relayProfiles[0] ||
    defaultSettings.relayProfiles[0]
  );
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
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: settings.relayProfiles.map((profile) => (profile.id === id ? { ...profile, ...patch } : profile)),
  });
}

function addRelayProfile(settings: BackendSettings): BackendSettings {
  const id = `relay-${Date.now().toString(36)}`;
  const next = {
    id,
    name: `中转 ${settings.relayProfiles.length + 1}`,
    baseUrl: defaultSettings.relayBaseUrl,
    apiKey: "",
  };
  return syncLegacyRelayFields({
    ...settings,
    relayProfiles: [...settings.relayProfiles, next],
    activeRelayId: id,
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
