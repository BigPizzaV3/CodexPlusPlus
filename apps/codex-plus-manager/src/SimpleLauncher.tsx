import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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

function isSuccessStatus(status: string) {
  return status === "ok" || status === "accepted";
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  return invoke<T>(command, args);
}

function launchRequest(settings: BackendSettings) {
  return {
    appPath: settings.codexAppPath,
    debugPort: 9229,
    helperPort: 57321,
  };
}

function ResetConfirmDialog({
  open,
  confirming,
  onCancel,
  onConfirm,
}: {
  open: boolean;
  confirming: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const [countdown, setCountdown] = useState(3);

  useEffect(() => {
    if (!open) {
      setCountdown(3);
      return;
    }

    setCountdown(3);
    const timer = window.setInterval(() => {
      setCountdown((prev) => (prev <= 1 ? 0 : prev - 1));
    }, 1000);

    return () => window.clearInterval(timer);
  }, [open]);

  if (!open) {
    return null;
  }

  const confirmDisabled = countdown > 0 || confirming;
  const confirmLabel = countdown > 0 ? `确认重置 (${countdown})` : "确认重置";

  return (
    <div className="reset-confirm-overlay">
      <div className="reset-confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="reset-title">
        <h2 id="reset-title">重置 Codex</h2>
        <p>
          即将删除所有会话记录和 Codex 使用的 skill、MCP、插件、配置等，并将其重置到初始可用状态，是否确认重置？
        </p>
        <div className="reset-confirm-actions">
          <Button variant="secondary" disabled={confirming} onClick={onCancel}>
            取消
          </Button>
          <Button variant="destructive" disabled={confirmDisabled} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      </div>
    </div>
  );
}

export function SimpleLauncher() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [token, setToken] = useState("");
  const [statusText, setStatusText] = useState("");
  const [errorText, setErrorText] = useState("");
  const [codexInstalled, setCodexInstalled] = useState(true);
  const [codexRunning, setCodexRunning] = useState(false);
  const [resetDialogOpen, setResetDialogOpen] = useState(false);
  const [resetConfirming, setResetConfirming] = useState(false);
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
    async (adaToken: string, settings: BackendSettings, silent = false) => {
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

  const confirmReset = useCallback(async () => {
    const settings = settingsRef.current;
    if (!settings) {
      setErrorText("设置尚未加载，请稍后重试。");
      setPhase("error");
      setResetDialogOpen(false);
      return;
    }

    setResetConfirming(true);
    setPhase("launching");
    setErrorText("");
    setStatusText("正在重置 Codex…");
    setResetDialogOpen(false);

    const resetResult = await call<LaunchResult>("reset_ctrip_codex", {
      request: launchRequest(settings),
    });

    setResetConfirming(false);

    if (!isSuccessStatus(resetResult.status)) {
      setErrorText(resetResult.message || "重置 Codex 失败。");
      setPhase("error");
      return;
    }

    await markLaunchSuccess();
  }, [markLaunchSuccess]);

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

  useEffect(() => {
    let unlisten: (() => void) | undefined;

    void listen("tray-reset-requested", () => {
      setResetDialogOpen(true);
    }).then((dispose) => {
      unlisten = dispose;
    });

    return () => {
      unlisten?.();
    };
  }, []);

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
    await launchCodex(token, settings, false);
  };

  const formDisabled = phase === "loading" || phase === "launching" || phase === "missingCodex";
  const showStatusText =
    statusText &&
    phase !== "launching" &&
    phase !== "loading" &&
    !codexRunning;

  return (
    <div className="simple-launcher">
      <ResetConfirmDialog
        open={resetDialogOpen}
        confirming={resetConfirming}
        onCancel={() => setResetDialogOpen(false)}
        onConfirm={() => void confirmReset()}
      />

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
