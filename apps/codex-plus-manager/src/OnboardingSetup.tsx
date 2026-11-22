import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, Check, ChevronDown, ChevronUp, CircleHelp, Loader2, Play, Plus, RefreshCw, Settings2, X, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AppSelect } from "./AppSelect";
import { AgentGlyph } from "./HubOverview";
import type { Notify } from "./gateway-ui";
import type { AgentFocus } from "./agent-focus-model";
import { setupDirectState, type SetupDirectMode, type SetupRelaySettings, type SetupRelayStatus } from "./onboarding-setup-model";
import { t, tf } from "./i18n";
import "./OnboardingSetup.css";

type Destination = "relay" | "agentSettings" | "enhance" | "userScripts" | "claude" | "overview";
type ClaudeSetupStatus = {
  status: string; message: string; supported: boolean; sourceApp: string;
  prepared: boolean; needsRebuild: boolean; nodeAvailable: boolean;
  localization: boolean; voiceInput: boolean; extensions: boolean; computerUse: boolean; computerUseSupported: boolean;
};
type RelayTestResult = { status: string; message: string };
/** 一次供应商测试的结果，只对发起时的那份配置有效。 */
type VerifiedTest = { fingerprint: string; ok: boolean; message: string };

export function OnboardingSetup({
  focus, relaySettings, codexInstalled, codexRunning, launchPending, codexEnhancementsEnabled,
  notify, onChangeMode, onDismiss, onTour, onNavigate, onAddProvider, onCheckCodex, onLaunch, defaultExpanded = true,
}: {
  focus: AgentFocus;
  /** 已保存的设置：当前直连供应商和配置切换开关。 */
  relaySettings: SetupRelaySettings | null;
  codexInstalled: boolean | null;
  codexRunning: boolean;
  launchPending: boolean;
  codexEnhancementsEnabled: boolean;
  notify: Notify;
  onChangeMode: () => void;
  onDismiss: (completed: boolean) => void;
  onTour: () => void;
  onNavigate: (target: Destination, agent?: string) => void;
  onAddProvider: () => void;
  onCheckCodex: () => Promise<boolean>;
  onLaunch: () => Promise<boolean>;
  /** 放在侧栏底部时默认收成一张进度卡，点开再展开清单。 */
  defaultExpanded?: boolean;
}) {
  const enhance = focus === "enhance";
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [selectedAgent, setSelectedAgent] = useState("codex");
  const [status, setStatus] = useState<SetupRelayStatus | null>(null);
  const [loadError, setLoadError] = useState("");
  const [testing, setTesting] = useState(false);
  const [verified, setVerified] = useState<VerifiedTest | null>(null);
  const [claude, setClaude] = useState<ClaudeSetupStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkedEnhancement, setCheckedEnhancement] = useState("");
  const mounted = useRef(true);
  const loadSequence = useRef(0);
  const loadingRequest = useRef(false);
  const loadTimeout = useRef<number | undefined>(undefined);
  const checkSequence = useRef(0);
  const testSequence = useRef(0);
  const pending = useRef(false);

  const refresh = useCallback(async () => {
    if (loadingRequest.current) return;
    loadingRequest.current = true;
    const sequence = ++loadSequence.current;
    const timeout = window.setTimeout(() => {
      if (!mounted.current || sequence !== loadSequence.current) return;
      loadSequence.current += 1;
      loadingRequest.current = false;
      setLoadError(t("配置读取超时，请重试。"));
    }, 15_000);
    loadTimeout.current = timeout;
    try {
      const result = await invoke<SetupRelayStatus & { status: string }>("relay_status");
      if (!mounted.current || sequence !== loadSequence.current) return;
      if (result.status !== "ok" || typeof result.authenticated !== "boolean" || typeof result.configured !== "boolean") {
        setLoadError(t("配置读取失败，请重试。"));
        return;
      }
      setLoadError("");
      setStatus((current) => current?.authenticated === result.authenticated && current.configured === result.configured
        ? current
        : { authenticated: result.authenticated, configured: result.configured });
    } catch {
      if (mounted.current && sequence === loadSequence.current) setLoadError(t("配置读取失败，请重试。"));
    } finally {
      window.clearTimeout(timeout);
      if (sequence === loadSequence.current) {
        loadingRequest.current = false;
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    // 在 Codex 里登录或换供应商会改 auth.json / config.toml，展开期间每 5 秒重读一次。增强模式用不到。
    const timer = enhance ? undefined : window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, 5000);
    if (!enhance) void refresh();
    return () => {
      mounted.current = false;
      checkSequence.current += 1;
      loadSequence.current += 1;
      testSequence.current += 1;
      loadingRequest.current = false;
      window.clearInterval(timer);
      window.clearTimeout(loadTimeout.current);
    };
  }, [refresh, enhance]);

  // 增强模式可以选 Codex 或 Claude；其余模式的清单只配置 Codex 的直连供应商。
  const agentId = enhance ? selectedAgent : "codex";
  const direct = setupDirectState(relaySettings, status);
  // 配置一变，旧测试结果和进行中的测试都作废；改回原配置也不复活。
  const previousConfiguration = useRef(direct.fingerprint);
  useEffect(() => {
    if (previousConfiguration.current !== direct.fingerprint) {
      previousConfiguration.current = direct.fingerprint;
      testSequence.current += 1;
      setVerified(null);
      setTesting(false);
    }
  }, [direct.fingerprint]);
  const tested = !loadError && direct.ready && verified?.ok === true && verified.fingerprint === direct.fingerprint;
  const claudeReady = Boolean(claude?.status === "ok" && claude.supported && claude.sourceApp
    && claude.prepared && !claude.needsRebuild && claude.nodeAvailable
    && (claude.localization || claude.voiceInput || claude.extensions || (claude.computerUse && claude.computerUseSupported)));
  const enhancementReady = agentId === "codex" ? codexInstalled === true && codexEnhancementsEnabled : claudeReady;
  const enhancementKey = JSON.stringify([agentId, codexInstalled, codexEnhancementsEnabled, claudeReady]);
  const enhancementChecked = enhancementReady && checkedEnhancement === enhancementKey;
  const configured = enhance ? enhancementReady : direct.ready && !loadError;
  const ready = enhance ? enhancementChecked : configured && (direct.testable ? tested : true);
  const changeAgent = (next: string) => {
    checkSequence.current += 1;
    setSelectedAgent(next);
    setCheckedEnhancement("");
  };
  const testProfile = async () => {
    if (!direct.profile || !configured || testing) return;
    const sequence = ++testSequence.current;
    const fingerprint = direct.fingerprint;
    setTesting(true);
    setVerified(null);
    try {
      // 真实发一条短请求（hi），结果绑定发起时的配置。
      const result = await invoke<RelayTestResult>("test_relay_profile", { profile: direct.profile });
      if (!mounted.current || sequence !== testSequence.current) return;
      setVerified({ fingerprint, ok: result.status !== "failed", message: result.message });
    } catch (cause) {
      if (mounted.current && sequence === testSequence.current) setVerified({ fingerprint, ok: false, message: tf("测试失败：{0}", [String(cause)]) });
    } finally {
      if (mounted.current && sequence === testSequence.current) setTesting(false);
    }
  };
  const checkEnhancement = async () => {
    if (pending.current) return;
    pending.current = true;
    const sequence = ++checkSequence.current;
    setChecking(true);
    setCheckedEnhancement("");
    try {
      if (agentId === "claude") {
        const result = await invoke<ClaudeSetupStatus>("claude_localization_status");
        if (!mounted.current || sequence !== checkSequence.current) return;
        setClaude(result);
        const usable = result.status === "ok" && result.supported && Boolean(result.sourceApp)
          && result.prepared && !result.needsRebuild && result.nodeAvailable
          && (result.localization || result.voiceInput || result.extensions || (result.computerUse && result.computerUseSupported));
        if (usable) setCheckedEnhancement(JSON.stringify([agentId, codexInstalled, codexEnhancementsEnabled, true]));
        else await notify(t("检查增强配置"), t("请先在增强设置中完成安装并启用所需功能。"), "warning");
      } else {
        const usable = await onCheckCodex();
        if (!mounted.current || sequence !== checkSequence.current) return;
        if (usable) setCheckedEnhancement("pending:codex");
        else await notify(t("检查增强配置"), t("请先在增强设置中完成安装并启用所需功能。"), "warning");
      }
    } catch {
      if (mounted.current && sequence === checkSequence.current) await notify(t("检查增强配置"), t("检查失败，请重试。"), "failed");
    } finally {
      pending.current = false;
      if (mounted.current) setChecking(false);
    }
  };
  useEffect(() => {
    if (checkedEnhancement === "pending:codex") {
      setCheckedEnhancement(enhancementReady ? enhancementKey : "");
      if (!enhancementReady) void notify(t("检查增强配置"), t("请先在增强设置中完成安装并启用所需功能。"), "warning");
    }
  }, [checkedEnhancement, enhancementReady, enhancementKey, notify]);

  const finish = async () => {
    if (!ready || pending.current || launchPending) return;
    if (agentId !== "codex") {
      onDismiss(true);
      return;
    }
    pending.current = true;
    try {
      if (await onLaunch() && mounted.current) onDismiss(true);
    } finally {
      pending.current = false;
    }
  };
  const directIssue = {
    loading: t("正在读取配置…"),
    switching_off: t("供应商配置切换已关闭，Codex++ 不会改动 Codex 的连接。在「连接与供应商」里打开。"),
    no_profile: t("还没有供应商。添加一个，或在 Codex 里登录 ChatGPT。"),
    not_logged_in: t("Codex 还没登录 ChatGPT。在 Codex 里登录，或添加一个 API 供应商并设为当前。"),
    api_missing: t("当前供应商还没填混入 API 的 Base URL / Key。"),
    not_configured: t("当前供应商的配置没有完整写入 Codex，请检查 Base URL 和 API Key。"),
  };
  const modeLabel: Record<SetupDirectMode, string> = {
    official: t("官方登录"),
    mixedApi: t("官方登录 + API"),
    pureApi: t("纯 API"),
    aggregate: t("聚合供应商"),
    gateway: t("经聚合网关"),
  };

  // 三步里完成了几步：侧栏进度卡上的「1/3」和进度环。
  const doneSteps = [Boolean(focus), configured, ready].filter(Boolean).length;
  return (
    <section aria-label={t("入门清单")} className="onboarding-setup" data-expanded={expanded}>
      <header className="setup-header">
        <div>
          <strong title={t("开始使用 Codex++")}>{t("开始使用")}</strong>
          <span className="setup-progress" title={tf("已完成 {0} / 3 步", [doneSteps])}>
            {`${doneSteps}/3`}
            <svg aria-hidden="true" className="setup-ring" viewBox="0 0 16 16">
              <circle cx="8" cy="8" fill="none" r="6" strokeWidth="2" />
              <circle cx="8" cy="8" fill="none" r="6" strokeDasharray={`${(doneSteps / 3) * 37.7} 37.7`} strokeLinecap="round" strokeWidth="2" transform="rotate(-90 8 8)" />
            </svg>
          </span>
          <span>{ready ? t("已验证，可以开始") : configured ? t("下一步：验证并开始") : t("下一步：完成配置")}</span>
        </div>
        <div className="setup-tools">
          <button aria-label={t("查看界面导览")} title={t("查看界面导览")} onClick={onTour} type="button"><CircleHelp size={15} /></button>
          <button aria-label={expanded ? t("收起入门清单") : t("展开入门清单")} aria-expanded={expanded} title={expanded ? t("收起入门清单") : t("展开入门清单")} onClick={() => setExpanded(!expanded)} type="button">{expanded ? <ChevronUp size={15} /> : <ChevronDown size={15} />}</button>
          <button aria-label={t("稍后设置")} title={t("稍后设置")} disabled={launchPending || pending.current} onClick={() => onDismiss(false)} type="button"><X size={15} /></button>
        </div>
      </header>
      {/* 收起时：一个按钮直达下一步（参照 Cursor 侧栏的 Getting Started）。都验证过就直接完成。 */}
      {!expanded ? (
        <button className="setup-next" disabled={launchPending} onClick={() => (ready ? void finish() : setExpanded(true))} type="button">
          {ready ? (agentId === "codex" ? t("启动 Codex 并完成") : t("完成配置")) : configured ? t("验证并开始") : enhance ? t("完成增强配置") : t("配置 Codex 连接")}
        </button>
      ) : null}
      {expanded ? (
        <div className="setup-steps">
          <div className="setup-step">
            <span className="setup-number" data-done={Boolean(focus)}>{focus ? <Check size={14} /> : "1"}</span>
            <div className="setup-step-content">
              <strong>{t("选择使用目标")}</strong>
              <div className="setup-choice">
                <span>{t(focus === "enhance" ? "只用增强功能" : focus === "codex" ? "只管理 Codex" : "全部功能")}</span>
                <button className="setup-text-action" disabled={launchPending || checking} onClick={onChangeMode} type="button">{t("修改")}</button>
              </div>
              {enhance ? <AppSelect ariaLabel={t("本次配置的 Agent")} disabled={checking || launchPending} onChange={changeAgent} options={[{ value: "codex", label: "Codex" }, { value: "claude", label: "Claude" }]} size="sm" value={agentId} /> : <span className="setup-agent"><AgentGlyph agent="codex" />Codex</span>}
            </div>
          </div>
          <div className="setup-step">
            <span className="setup-number" data-done={configured}>{configured ? <Check size={14} /> : "2"}</span>
            <div className="setup-step-content">
              <strong>{t("完成必要配置")}</strong>
              {enhance ? (
                <>
                  <p>{enhancementReady ? t("应用与增强配置已就绪。") : t("确认目标应用已安装，并启用所需增强或拓展。")}</p>
                  <div className="setup-actions">
                    <Button onClick={() => onNavigate(agentId === "claude" ? "claude" : "enhance", agentId)} size="sm" variant="secondary"><Settings2 size={13} />{t("增强设置")}</Button>
                    <Button onClick={() => onNavigate("userScripts", agentId)} size="sm" variant="ghost">{t("万物皆拓展")}<ArrowRight size={13} /></Button>
                  </div>
                </>
              ) : (
                <>
                  <p role={loadError ? "alert" : undefined}>{loadError || (direct.issue ? directIssue[direct.issue] : tf("当前供应商：{0} · {1}", [direct.profile?.name || t("未命名供应商"), modeLabel[direct.mode]]))}</p>
                  <div className="setup-actions">
                    {loadError ? <Button onClick={() => void refresh()} size="sm" variant="secondary"><RefreshCw size={13} />{t("重试")}</Button> : (
                      <>
                        <Button disabled={launchPending} onClick={onAddProvider} size="sm" variant="secondary"><Plus size={13} />{t("添加供应商")}</Button>
                        <Button disabled={launchPending} onClick={() => onNavigate("relay", "codex")} size="sm" variant="ghost">{t("连接与供应商")}<ArrowRight size={13} /></Button>
                      </>
                    )}
                  </div>
                </>
              )}
            </div>
          </div>
          <div className="setup-step">
            <span className="setup-number" data-done={ready}>{ready ? <Check size={14} /> : "3"}</span>
            <div className="setup-step-content">
              <strong>{t("验证并开始")}</strong>
              {enhance ? (
                <Button disabled={checking || launchPending} onClick={() => void checkEnhancement()} size="sm" variant="secondary">{checking ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}{t("检查安装与配置")}</Button>
              ) : direct.testable ? (
                <>
                  <Button disabled={!configured || testing} onClick={() => void testProfile()} size="sm" variant="secondary">{testing ? <Loader2 size={13} className="animate-spin" /> : <Zap size={13} />}{tested ? t("重新测试模型请求") : t("测试模型请求")}</Button>
                  {verified && !verified.ok && verified.fingerprint === direct.fingerprint
                    ? <p role="alert">{verified.message}</p>
                    : <p>{tested ? t("当前供应商的模型请求测试通过。") : t("手动发送一条短请求，可能产生少量用量。")}</p>}
                </>
              ) : (
                <p>{t("聚合供应商会在多个成员间轮转，请在「连接与供应商」里逐个测试成员。")}</p>
              )}
              <Button disabled={!ready || launchPending || (agentId === "codex" && codexInstalled !== true)} onClick={() => void finish()} size="sm"><Play size={13} />{launchPending ? t("正在启动…") : agentId === "codex" ? codexRunning ? t("重启 Codex 并完成") : t("启动 Codex 并完成") : t("完成配置")}</Button>
              {agentId !== "codex" ? <p>{t("完成后可在增强设置中启动应用。")}</p> : null}
              {agentId === "codex" && codexInstalled === false ? <button className="setup-text-action" onClick={() => onNavigate("agentSettings", "codex")} type="button">{t("未找到 Codex，打开安装设置")}</button> : null}
            </div>
          </div>
        </div>
      ) : null}
    </section>
  );
}
