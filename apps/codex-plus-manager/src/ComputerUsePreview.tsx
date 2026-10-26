import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Loader2, MonitorPlay, RefreshCw, ShieldAlert, Square } from "lucide-react";
import { t } from "./i18n";

/// 与 Rust 侧 `window_capture::CaptureWindow` 对齐
type PreviewWindow = {
  window_id: number;
  pid: number;
  app_name: string;
  bundle_id: string;
  title: string;
  width: number;
  height: number;
  on_screen: boolean;
};

/// `CommandResult` 用 #[serde(flatten)] 把载荷摊在顶层，所以字段直接挂在 result 上
type CuResult = {
  status: string;
  message?: string;
  windows?: PreviewWindow[];
  permitted?: boolean;
};

type CaptureMode = "screenshot" | "stream";

type FrameEvent = { seq: number; data: string };
type EndEvent = { reason: string };

/// 预览用的缩放与帧率：0.5 倍时单帧约 45 KB、12 fps 已经很顺，且不抢 CU 自己的算力。
const PREVIEW_SCALE = 0.5;
const PREVIEW_FPS = 12;

function errorText(reason: unknown): string {
  if (typeof reason === "string") return reason;
  if (reason instanceof Error) return reason.message;
  return String(reason);
}

function describe(win: PreviewWindow): string {
  const name = win.app_name || win.bundle_id || `pid ${win.pid}`;
  const title = win.title ? ` — ${win.title}` : "";
  return `${name}${title} (${Math.round(win.width)}×${Math.round(win.height)})`;
}

/// 目标窗口实时预览：自己用 ScreenCaptureKit 抓，不经过官方 PiP 通道。
export function ComputerUsePreview() {
  const [windows, setWindows] = useState<PreviewWindow[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [captureMode, setCaptureMode] = useState<CaptureMode>("screenshot");
  const [frame, setFrame] = useState<string>("");
  const [frames, setFrames] = useState(0);
  const [running, setRunning] = useState(false);
  const [busy, setBusy] = useState("");
  const [hint, setHint] = useState("");
  const [permitted, setPermitted] = useState(true);
  const [notice, setNotice] = useState("");
  const [error, setError] = useState("");
  const mounted = useRef(false);

  const loadWindows = useCallback(async () => {
    setBusy("list");
    setError("");
    try {
      const result = await invoke<CuResult>("computer_use_preview_windows");
      if (!mounted.current) return;
      const list = Array.isArray(result.windows) ? result.windows : [];
      setPermitted(result.permitted !== false);
      setWindows(list);
      setHint(result.message || "");
      // 保持已有选择；没有就优先选屏幕上的窗口
      setSelected((current) => {
        if (current != null && list.some((win) => win.window_id === current)) return current;
        const first = list.find((win) => win.on_screen && win.title) ?? list[0];
        return first ? first.window_id : null;
      });
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    } finally {
      if (mounted.current) setBusy("");
    }
  }, []);

  // 申请屏幕录制权限：系统会在第一次弹窗，之后需要用户去设置里手动勾选
  const requestPermission = async () => {
    setBusy("permission");
    setError("");
    try {
      const result = await invoke<CuResult>("computer_use_preview_request_permission");
      if (!mounted.current) return;
      if (result.status === "ok") {
        setNotice(result.message || t("已获得屏幕录制权限。"));
        await loadWindows();
      } else {
        setError(result.message || t("授权失败。"));
      }
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    } finally {
      if (mounted.current) setBusy("");
    }
  };

  const stop = useCallback(async () => {
    setRunning(false);
    try {
      await invoke("computer_use_preview_stop");
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    }
  }, []);

  const start = async () => {
    if (selected == null) return;
    setBusy("start");
    setError("");
    setFrames(0);
    setFrame("");
    try {
      const result = await invoke<CuResult>("computer_use_preview_start", {
        windowId: selected,
        scale: PREVIEW_SCALE,
        maxFps: PREVIEW_FPS,
        mode: captureMode,
      });
      if (!mounted.current) return;
      if (result.status === "ok") {
        setRunning(true);
        setHint(result.message || t("预览已开始。"));
      } else {
        setError(result.message || t("开始预览失败。"));
      }
    } catch (reason) {
      if (mounted.current) setError(errorText(reason));
    } finally {
      if (mounted.current) setBusy("");
    }
  };

  useEffect(() => {
    mounted.current = true;
    void loadWindows();
    return () => {
      mounted.current = false;
      // 离开页面必须停掉取流，否则后台一直在抓
      void invoke("computer_use_preview_stop").catch(() => undefined);
    };
  }, [loadWindows]);

  useEffect(() => {
    let disposed = false;
    let stopFrame: (() => void) | undefined;
    let stopEnd: (() => void) | undefined;
    void listen<FrameEvent>("cu-preview-frame", (event) => {
      if (disposed) return;
      setFrame(event.payload.data);
      setFrames(event.payload.seq + 1);
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopFrame = unlisten;
    });
    void listen<EndEvent>("cu-preview-end", (event) => {
      if (disposed) return;
      setRunning(false);
      setError(event.payload.reason || t("预览已结束。"));
    }).then((unlisten) => {
      if (disposed) unlisten();
      else stopEnd = unlisten;
    });
    return () => {
      disposed = true;
      stopFrame?.();
      stopEnd?.();
    };
  }, []);

  const locked = busy !== "";

  return (
    <section className="cu-card" aria-labelledby="cu-preview-title">
      <div className="cu-card-head">
        <div>
          <h2 id="cu-preview-title">{t("窗口预览")}</h2>
          <p>{t("实时看 Codex 正在操作的目标窗口。画面由 Codex++ 自己抓取，和官方窗口镜像同源，只是不经官方通道。")}</p>
        </div>
        {permitted ? (
          <button className="mg-button" type="button" disabled={locked} onClick={() => void loadWindows()}>
            {busy === "list" ? <Loader2 size={15} className="cu-spin" /> : <RefreshCw size={15} />}
            {t("刷新窗口")}
          </button>
        ) : (
          <button className="mg-button mg-primary" type="button" disabled={locked} onClick={() => void requestPermission()}>
            {busy === "permission" ? <Loader2 size={15} className="cu-spin" /> : <ShieldAlert size={15} />}
            {t("申请权限")}
          </button>
        )}
      </div>

      <div className="cu-preview-picker">
        <label className="cu-preview-select">
          <span>{t("目标窗口")}</span>
          <select
            value={selected ?? ""}
            disabled={locked || running || windows.length === 0}
            onChange={(event) => setSelected(event.target.value ? Number(event.target.value) : null)}
          >
            {windows.length === 0 && <option value="">{t("没有可预览的窗口")}</option>}
            {windows.map((win) => (
              <option key={win.window_id} value={win.window_id}>
                {describe(win)}
              </option>
            ))}
          </select>
        </label>
        <label className="cu-preview-select">
          <span>{t("抓取方式")}</span>
          <select
            value={captureMode}
            disabled={locked || running}
            onChange={(e) => setCaptureMode(e.target.value as CaptureMode)}
          >
            <option value="screenshot">{t("截图模式（无共享标志）")}</option>
            <option value="stream">{t("流模式（低延迟，显示共享标志）")}</option>
          </select>
        </label>
        {running ? (
          <button className="mg-button" type="button" onClick={() => void stop()}>
            <Square size={14} />{t("停止预览")}
          </button>
        ) : (
          <button className="mg-button mg-primary" type="button" disabled={locked || selected == null} onClick={() => void start()}>
            {busy === "start" ? <Loader2 size={14} className="cu-spin" /> : <MonitorPlay size={14} />}
            {busy === "start" ? t("启动中…") : t("开始预览")}
          </button>
        )}
      </div>

      <div className="cu-preview-stage" data-live={running && frame !== ""}>
        {frame ? (
          <img src={frame} alt={t("目标窗口实时画面")} />
        ) : (
          <p className="cu-preview-empty">
            {running ? t("正在等待第一帧…") : t("选一个窗口后点「开始预览」。")}
          </p>
        )}
      </div>

      {running && (
        <p className="cu-hint">
          {t("已接收")}<code>{frames}</code>{t("帧")} · {PREVIEW_SCALE}× · {PREVIEW_FPS} fps
          {" · "}{captureMode === "stream" ? t("流模式") : t("截图模式")}
        </p>
      )}
      {error && <p className="cu-failure" role="alert">{error}</p>}
      {!error && notice && <p className="cu-hint">{notice}</p>}
      {!error && hint && <p className="cu-hint">{hint}</p>}
      <p className="cu-note">
        {t("第一次使用会在 系统设置 → 隐私与安全性 → 屏幕录制 里请求授权，授权后需重启 Codex++ 才生效。")}
      </p>
    </section>
  );
}
