/** 托盘快捷面板：各 Agent 当前用的模型（可直接切换）、最近的请求、打开主窗口 / 退出。 */
import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronDown, ExternalLink, RefreshCw, Waypoints } from "lucide-react";
import { t, tf } from "./i18n";
import { ModelList, Floating } from "./GatewayAgents";
import {
  agentModels,
  agentName,
  bindingFromDraft,
  draftFromBinding,
  formatTokens,
  modelDisplay,
  relativeTime,
  type GatewayAgentView,
  type GatewayPayload,
  type GatewayResult,
  maskAccountName,
} from "./gateway-model";
import { AGENT_ICONS, APP_LOGO_URL, Avatar, AgentIcon } from "./gateway-ui";
import "./GatewayPage.css";

/** 面板与主窗口共用主题设置（主窗口写在 localStorage 里）。 */
function applyTheme() {
  let theme = "dark";
  try {
    theme = window.localStorage.getItem("codex-plus-theme") === "light" ? "light" : "dark";
  } catch {
    // 读不到就用深色。
  }
  document.documentElement.classList.toggle("dark", theme === "dark");
  document.documentElement.classList.toggle("light", theme === "light");
}

type LogEntry = {
  id: string;
  timestampMs: number;
  model: string;
  upstreamModel?: string;
  relayName?: string;
  statusCode?: number;
  success?: boolean;
  state?: string | null;
  durationMs?: number;
  agent?: string | null;
  usage?: { inputTokens: number; outputTokens: number } | null;
};

/** 面板每隔一会儿刷新一次（只在面板可见时）。 */
const REFRESH_MS = 3000;

export function TrayPanel() {
  const [payload, setPayload] = useState<GatewayPayload | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [message, setMessage] = useState("");

  const load = async () => {
    try {
      const result = await invoke<GatewayResult>("load_gateway");
      if (result.gateway && Array.isArray(result.pool)) setPayload(result as GatewayPayload);
    } catch {
      // 网关读不到时面板只显示请求。
    }
    try {
      const result = await invoke<{ entries?: LogEntry[] }>("load_request_logs", { limit: 40 });
      setLogs(result.entries ?? []);
    } catch {
      setLogs([]);
    }
  };

  useEffect(() => {
    applyTheme();
    void load();
    const onStorage = () => applyTheme();
    window.addEventListener("storage", onStorage);
    const onFocus = () => {
      applyTheme();
      void load();
    };
    window.addEventListener("focus", onFocus);
    const timer = window.setInterval(() => {
      if (document.visibilityState === "visible") void load();
    }, REFRESH_MS);
    return () => {
      window.removeEventListener("storage", onStorage);
      window.removeEventListener("focus", onFocus);
      window.clearInterval(timer);
    };
  }, []);

  const action = (name: "open" | "quit" | "hide") => void invoke("tray_panel_action", { action: name });
  const run = async (command: string, args: Record<string, unknown>) => {
    try {
      const result = await invoke<GatewayResult>(command, args);
      if (result.gateway && Array.isArray(result.pool)) setPayload(result as GatewayPayload);
      setMessage(result.status === "ok" ? "" : result.message);
    } catch (error) {
      setMessage(String(error));
    }
  };

  const now = Date.now();
  const recent = logs.slice(0, 12);

  return (
    <div className="gateway-page tray-panel">
      <header className="tray-panel-head" data-tauri-drag-region>
        <img alt="" src={APP_LOGO_URL} />
        <strong data-tauri-drag-region>Codex++</strong>
        <span className="tray-panel-status" data-tauri-drag-region>
          <i className="gw-dot" data-tone={payload?.helperRunning ? "on" : "off"} />
          {payload?.helperRunning ? `127.0.0.1:${payload.port}` : t("网关没有运行")}
        </span>
        <button aria-label={t("刷新")} className="gw-icon-button" onClick={() => void load()} title={t("刷新")} type="button"><RefreshCw className="h-4 w-4" /></button>
        <button aria-label={t("打开 Codex++")} className="gw-icon-button" onClick={() => action("open")} title={t("打开 Codex++")} type="button"><ExternalLink className="h-4 w-4" /></button>
      </header>

      <div className="tray-panel-body">
        <div className="gw-card tray-panel-agents">
          {(payload?.agents ?? []).map((agent) => (
            <PanelAgentRow agent={agent} key={agent.agent} onRun={run} payload={payload} />
          ))}
          {!payload ? <div className="gw-empty">{t("正在读取…")}</div> : null}
        </div>
        {message ? <p className="tray-panel-message">{message}</p> : null}

        <div className="tray-panel-label">{t("最近请求")}</div>
        <div className="gw-card tray-panel-logs">
          {recent.map((entry) => {
            const running = entry.state === "running";
            const ok = !running && entry.success !== false && (entry.statusCode ?? 200) < 400;
            const when = relativeTime(entry.timestampMs, now);
            const agent = entry.agent || "codex";
            const tokens = entry.usage ? `${formatTokens(entry.usage.inputTokens)} → ${formatTokens(entry.usage.outputTokens)}` : "";
            return (
              <div className="tray-log-row" key={entry.id}>
                <img alt="" src={AGENT_ICONS[agent === "codex" ? "codexApp" : agent] ?? AGENT_ICONS.codexApp} title={agentName(agent)} />
                <div className="tray-log-main">
                  <strong>{entry.upstreamModel || entry.model || "—"}</strong>
                  <small>{[maskAccountName(entry.relayName ?? ""), tokens, entry.durationMs ? `${(entry.durationMs / 1000).toFixed(1)}s` : ""].filter(Boolean).join(" · ")}</small>
                </div>
                <span className="tray-log-side">
                  <i className="gw-dot" data-tone={running ? "agent" : ok ? "on" : "error"} />
                  {running ? t("进行中") : when.args.length ? tf(when.text, when.args) : t(when.text)}
                </span>
              </div>
            );
          })}
          {recent.length === 0 ? <div className="gw-empty">{t("还没有请求")}</div> : null}
        </div>
      </div>

      <footer className="tray-panel-foot">
        <button className="gw-text-button" onClick={() => action("open")} type="button">{t("打开 Codex++")}</button>
        <button className="gw-text-button" onClick={() => action("quit")} type="button">{t("退出")}</button>
      </footer>
    </div>
  );
}

/** 一个 Agent：名字 + 当前模型下拉；选模型即接入，选「默认」即断开。 */
function PanelAgentRow({
  agent,
  payload,
  onRun,
}: {
  agent: GatewayAgentView;
  payload: GatewayPayload | null;
  onRun: (command: string, args: Record<string, unknown>) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const buttonRef = useRef<HTMLButtonElement>(null);
  const binding = payload?.gateway.bindings[agent.agent];
  const draft = draftFromBinding(binding);
  const display = modelDisplay(payload, draft.model, draft.poolExclude);
  const head = (
    <div
      aria-selected={!agent.enabled}
      className="gw-popover-item"
      data-selected={!agent.enabled}
      onClick={() => {
        setOpen(false);
        if (agent.enabled) void onRun("disable_gateway_binding", { agent: agent.agent });
      }}
      role="option"
      tabIndex={0}
    >
      <AgentIcon agent={agent.agent} className="gw-popover-agent" />
      <span className="gw-popover-item-label">{t("默认")}</span>
      <span className="gw-via gw-via-off">{t("不经 Codex++")}</span>
      <small>{tf("{0} 自带的默认值", [agent.name])}</small>
    </div>
  );
  return (
    <div className="tray-agent-row">
      <AgentIcon agent={agent.agent} className="gw-agent-icon" />
      <strong>{agent.name}</strong>
      <button aria-expanded={open} className="gw-picker-button tray-agent-picker" onClick={() => setOpen(!open)} ref={buttonRef} type="button">
        {!agent.enabled ? (
          <AgentIcon agent={agent.agent} className="gw-picker-agent" />
        ) : display.group ? (
          <Waypoints className="gw-picker-glyph h-4 w-4" />
        ) : (
          <Avatar name={display.sub || display.title} size="sm" />
        )}
        <span>{agent.enabled ? display.title : t("默认")}</span>
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      <Floating anchorRef={buttonRef} className="gw-model-popover" onClose={() => setOpen(false)} open={open} width={400}>
        <ModelList
          exclude={draft.poolExclude}
          head={head}
          onPick={(item) => {
            setOpen(false);
            void onRun("save_gateway_binding", { agent: agent.agent, binding: bindingFromDraft({ ...draft, model: item.model }, binding) });
          }}
          payload={payload}
          selectedModel={agent.enabled ? draft.model || agentModels(payload, draft.poolExclude)[0] || "" : ""}
        />
      </Floating>
    </div>
  );
}
