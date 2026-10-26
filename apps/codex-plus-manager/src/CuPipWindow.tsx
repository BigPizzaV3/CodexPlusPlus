/** Computer Use 悬浮预览窗口（`#cu-pip`）：实时显示 CU 正在操作的后台应用窗口，同 Codex 的画中画。 */
import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { AppWindow, X } from "lucide-react";
import { t } from "./i18n";
import "./CuPipWindow.css";

type PipTarget = { pid?: number; appName?: string; title?: string; kind?: string };

export function CuPipWindow() {
  const [frame, setFrame] = useState("");
  const [target, setTarget] = useState<PipTarget>({});

  useEffect(() => {
    document.documentElement.classList.add("cu-pip-root");
    // 窗口刚建好时可能错过了第一次的目标事件，加载后主动取一次。
    void invoke<PipTarget>("cu_pip_target").then((current) => {
      if (current && typeof current === "object") setTarget(current);
    }).catch(() => {});
    const frames = listen<{ data: string }>("cu-pip-frame", (event) => setFrame(event.payload.data));
    const targets = listen<PipTarget>("cu-pip-target", (event) => {
      setTarget(event.payload);
      setFrame("");
    });
    return () => {
      void frames.then((stop) => stop());
      void targets.then((stop) => stop());
    };
  }, []);

  const name = target.appName || "";
  // 浏览器档：点一下跳到 agent 正在操作的标签页（同 Codex）。
  const jumpLabel = target.kind === "browser" ? t("切到该标签页") : t("切到该应用");
  return (
    <div className="cu-pip">
      <header className="cu-pip-head" data-tauri-drag-region>
        <span className="cu-pip-dot" aria-hidden="true" />
        <strong className="cu-pip-brand" data-tauri-drag-region>Codex++ Computer Use</strong>
        {name
          ? <span className="cu-pip-app" data-tauri-drag-region title={`${t("正在操作")}：${target.title || name}`}>{name}</span>
          : null}
        <button type="button" title={jumpLabel} aria-label={jumpLabel} onClick={() => void invoke("cu_pip_activate")}>
          <AppWindow size={13} />
        </button>
        <button type="button" title={t("关闭（本次会话不再弹出）")} aria-label={t("关闭")} onClick={() => void invoke("cu_pip_dismiss")}>
          <X size={13} />
        </button>
      </header>
      <div className="cu-pip-body">
        {frame ? <img alt={name || "Computer Use"} src={frame} draggable={false} /> : <span>{t("等待画面…")}</span>}
      </div>
    </div>
  );
}
