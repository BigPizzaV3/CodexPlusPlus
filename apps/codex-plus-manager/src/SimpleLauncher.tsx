import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ExternalLink, RefreshCw, Rocket } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { CODEX_DOWNLOAD_URL, type BackendSettings } from "./ctripAda";

type Phase = "loading" | "ready" | "missingCodex" | "launching" | "error";

type CommandResult<T> = T & {
  status: string;
  message: string;
};

type OverviewResult = CommandResult<{
  codex_app: { status: string; path: string | null };
}>;

type SettingsResult = CommandResult<{
  settings: BackendSettings;
}>;

type LaunchResult = CommandResult<Record<string, unknown>>;

type TokenResult = CommandResult<{
  token: string | null;
}>;

type CtripSetupState = {
  configReady: boolean;
  guiAuthCachePresent: boolean;
  chatgptAuthPresent: boolean;
  needsGuiClear: boolean;
  needsConfigWrite: boolean;
  needsClearBeforeApply: boolean;
};

type ClearAuthResult = CommandResult<{
  message: string;
  removedPaths: string[];
}>;

function isSuccessStatus(status: string) {
  return status === "ok" || status === "accepted";
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}

function needsGuiClear(state: CtripSetupState) {
  return state.guiAuthCachePresent || state.chatgptAuthPresent;
}

function isSetupReady(state: CtripSetupState, hasToken: boolean) {
  return state.configReady && !needsGuiClear(state) && hasToken;
}

const CLEAR_CONFIRM_MESSAGE =
  "检测到 Codex 存在 GUI 登录缓存或 ChatGPT 登录态。\n\n" +
  "将按内部文档清除 Codex GUI 登录态与缓存，并重新写入 config.toml。\n\n" +
  "是否继续？";

function launchRequest(settings: BackendSettings) {
  return {
    appPath: settings.codexAppPath,
    debugPort: 9229,
    helperPort: 57321,
  };
}

export function SimpleLauncher() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [token, setToken] = useState("");
  const [statusText, setStatusText] = useState("");
  const [errorText, setErrorText] = useState("");
  const [codexInstalled, setCodexInstalled] = useState(true);
  const [codexRunning, setCodexRunning] = useState(false);
  const settingsRef = useRef<BackendSettings | null>(null);
  const autoLaunchAttempted = useRef(false);

  const markLaunchSuccess = useCallback(async () => {
    setCodexRunning(true);
    setPhase("ready");
    setErrorText("");
    await getCurrentWindow().hide();
  }, []);

  const saveToken = useCallback(async (adaToken: string) => {
    const saveResult = await call<TokenResult>("save_ctrip_token", { token: adaToken });
    if (!isSuccessStatus(saveResult.status)) {
      setErrorText(saveResult.message || "保存 Token 失败。");
      setPhase("error");
      return false;
    }
    return true;
  }, []);

  const launchCodexOnly = useCallback(
    async (settings: BackendSettings) => {
      const launchResult = await call<LaunchResult>("launch_codex_plus", {
        request: launchRequest(settings),
      });
      if (!isSuccessStatus(launchResult.status)) {
        setErrorText(launchResult.message || "启动 Codex 失败。");
        setPhase("error");
        return false;
      }
      await markLaunchSuccess();
      return true;
    },
    [markLaunchSuccess],
  );

  const launchCodex = useCallback(
    async (adaToken: string, settings: BackendSettings, silent = false, skipConfirm = false) => {
      const trimmed = adaToken.trim();
      if (!trimmed) {
        setErrorText("请先填写 ADA Token。");
        setPhase("error");
        return false;
      }

      setPhase("launching");
      setErrorText("");
      setStatusText(silent ? "正在自动启动 Codex…" : "正在启动 Codex…");

      if (!(await saveToken(trimmed))) {
        return false;
      }

      const setupState = await call<CtripSetupState>("detect_ctrip_setup_state");

      if (isSetupReady(setupState, true)) {
        return launchCodexOnly(settings);
      }

      if (needsGuiClear(setupState)) {
        if (!skipConfirm && !window.confirm(CLEAR_CONFIRM_MESSAGE)) {
          setStatusText("已取消启动。请确认清除登录态后再试。");
          setPhase("ready");
          return false;
        }

        const clearResult = await call<ClearAuthResult>("clear_codex_gui_auth");
        if (!isSuccessStatus(clearResult.status)) {
          setErrorText(clearResult.message || "清除 Codex 登录态失败。");
          setPhase("error");
          return false;
        }
      }

      return launchCodexOnly(settings);
    },
    [launchCodexOnly, saveToken],
  );

  const restartCodex = useCallback(async () => {
    const settings = settingsRef.current;
    if (!settings) {
      setErrorText("设置尚未加载，请稍后重试。");
      setPhase("error");
      return;
    }
    if (!token.trim()) {
      setErrorText("请先填写 ADA Token。");
      setPhase("error");
      return;
    }

    setPhase("launching");
    setErrorText("");
    setStatusText("正在重启 Codex…");

    if (!(await saveToken(token))) {
      return;
    }

    const restartResult = await call<LaunchResult>("restart_codex_plus", {
      request: launchRequest(settings),
    });
    if (!isSuccessStatus(restartResult.status)) {
      setErrorText(restartResult.message || "重启 Codex 失败。");
      setPhase("error");
      return;
    }
    await markLaunchSuccess();
  }, [markLaunchSuccess, saveToken, token]);

  const openDownloadPage = async () => {
    await call<CommandResult<Record<string, unknown>>>("open_external_url", {
      url: CODEX_DOWNLOAD_URL,
    });
  };

  useEffect(() => {
    let cancelled = false;

    const bootstrap = async () => {
      try {
        const overview = await call<OverviewResult>("load_overview");
        if (cancelled) return;

        const installed = overview.codex_app.status === "found";
        setCodexInstalled(installed);

        if (!installed) {
          setPhase("missingCodex");
          setStatusText("未检测到 Codex 桌面版，请先下载安装。");
          return;
        }

        const settingsResult = await call<SettingsResult>("load_settings");
        if (cancelled) return;

        settingsRef.current = settingsResult.settings;

        const tokenResult = await call<TokenResult>("load_ctrip_token");
        if (cancelled) return;

        const savedToken = tokenResult.token?.trim();
        if (savedToken) {
          setToken(savedToken);
        }

        if (savedToken && !autoLaunchAttempted.current) {
          autoLaunchAttempted.current = true;
          const setupState = await call<CtripSetupState>("detect_ctrip_setup_state");
          if (cancelled) return;

          if (isSetupReady(setupState, true)) {
            setPhase("launching");
            setStatusText("正在自动启动 Codex…");
            const launched = await launchCodexOnly(settingsResult.settings);
            if (cancelled) return;
            if (!launched) {
              setPhase("ready");
            }
            return;
          }

          setPhase("ready");
          if (needsGuiClear(setupState)) {
            setStatusText("检测到 Codex 登录缓存，请点击启动并完成清除确认。");
          } else if (setupState.needsConfigWrite) {
            setStatusText("检测到 Codex 配置未就绪，点击启动即可写入配置。");
          } else {
            setStatusText("检测到 Codex 配置未就绪，请点击启动。");
          }
          return;
        }

        setPhase("ready");
      } catch (error) {
        if (cancelled) return;
        setErrorText(error instanceof Error ? error.message : String(error));
        setPhase("error");
      }
    };

    void bootstrap();
    return () => {
      cancelled = true;
    };
  }, [launchCodexOnly]);

  const handleLaunch = async () => {
    const settings = settingsRef.current;
    if (!settings) {
      setErrorText("设置尚未加载，请稍后重试。");
      setPhase("error");
      return;
    }
    if (!codexInstalled) {
      setPhase("missingCodex");
      return;
    }
    await launchCodex(token, settings, false, false);
  };

  const formDisabled = phase === "loading" || phase === "launching" || phase === "missingCodex";
  const showStatusText =
    statusText &&
    phase !== "launching" &&
    phase !== "loading" &&
    !codexRunning;

  return (
    <div className="simple-launcher">
      <div className="simple-launcher-card">
        <header className="simple-launcher-header">
          <h1>Codex++</h1>
          <p>填写携程 CodingPlan Token，一键启动 Codex</p>
        </header>

        {phase === "missingCodex" ? (
          <div className="simple-launcher-missing">
            <p>未检测到 Codex 桌面版，请先下载并安装后再启动。</p>
            <Button className="simple-launcher-download" variant="secondary" onClick={() => void openDownloadPage()}>
              <ExternalLink className="h-4 w-4" />
              下载 Codex Desktop
            </Button>
          </div>
        ) : null}

        <div className="simple-launcher-field">
          <Label htmlFor="ada-token">ADA Token</Label>
          <Input
            id="ada-token"
            type="password"
            autoComplete="off"
            placeholder="粘贴 CodingPlan Token"
            value={token}
            disabled={formDisabled}
            onChange={(event) => setToken(event.currentTarget.value)}
          />
        </div>

        <Button
          className="simple-launcher-button"
          disabled={formDisabled || !token.trim()}
          onClick={() => void handleLaunch()}
        >
          <Rocket className="h-4 w-4" />
          启动 Codex
        </Button>

        {codexRunning ? (
          <Button
            className="simple-launcher-restart"
            variant="ghost"
            size="sm"
            disabled={formDisabled}
            onClick={() => void restartCodex()}
          >
            <RefreshCw className="h-3.5 w-3.5" />
            重启 Codex
          </Button>
        ) : null}

        {phase === "loading" ? <p className="simple-launcher-status">正在加载…</p> : null}
        {phase === "launching" && statusText ? (
          <p className="simple-launcher-status">{statusText}</p>
        ) : null}
        {showStatusText ? <p className="simple-launcher-status">{statusText}</p> : null}
        {errorText ? <p className="simple-launcher-error">{errorText}</p> : null}
      </div>
    </div>
  );
}
