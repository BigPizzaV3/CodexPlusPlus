import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowRight, Check, ChevronDown, ChevronUp, CircleHelp, Loader2, Play, Plus, RefreshCw, Settings2, X, Zap } from "lucide-react";
import { Button } from "@/components/ui/button";
import { AppSelect } from "./AppSelect";
import { AgentGlyph } from "./HubOverview";
import { AgentConnectDialog } from "./GatewayAgents";
import { ProviderAdder } from "./GatewayProviders";
import { ProviderTestDialog } from "./ProviderTestDialog";
import { agentName, groupDisplayName, maskAccountName, type GatewayPayload, type GatewayResult } from "./gateway-model";
import { gatewayGeneration, publishGatewayPayload, subscribeGatewayPayload } from "./gateway-store";
import { makeRun, type Notify } from "./gateway-ui";
import { agentVisible, type AgentFocus } from "./agent-focus-model";
import { probeMatches, setupGatewayState } from "./onboarding-setup-model";
import { t, tf } from "./i18n";
import "./OnboardingSetup.css";

type Destination = "gateway" | "gatewayProviders" | "agentSettings" | "enhance" | "userScripts" | "claude" | "overview";
type ClaudeSetupStatus = {
  status: string; message: string; supported: boolean; sourceApp: string;
  prepared: boolean; needsRebuild: boolean; nodeAvailable: boolean;
  localization: boolean; voiceInput: boolean; extensions: boolean; computerUse: boolean; computerUseSupported: boolean;
};
type VerifiedProbe = { fingerprint: string; candidateId: string; ok: boolean };

export function OnboardingSetup({
  focus, codexInstalled, codexRunning, launchPending, codexEnhancementsEnabled,
  notify, onChangeMode, onDismiss, onTour, onNavigate, onCheckCodex, onLaunch, defaultExpanded = true,
}: {
  focus: AgentFocus;
  codexInstalled: boolean | null;
  codexRunning: boolean;
  launchPending: boolean;
  codexEnhancementsEnabled: boolean;
  notify: Notify;
  onChangeMode: () => void;
  onDismiss: (completed: boolean) => void;
  onTour: () => void;
  onNavigate: (target: Destination, agent?: string) => void;
  onCheckCodex: () => Promise<boolean>;
  onLaunch: () => Promise<boolean>;
  /** 放在侧栏底部时默认收成一张进度卡，点开再展开清单。 */
  defaultExpanded?: boolean;
}) {
  const enhance = focus === "enhance";
  const [expanded, setExpanded] = useState(defaultExpanded);
  const [selectedAgent, setSelectedAgent] = useState("codex");
  const [payload, setPayload] = useState<GatewayPayload | null>(null);
  const [loadError, setLoadError] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState<string | null>(null);
  const [adding, setAdding] = useState<"provider" | "agent" | null>(null);
  const [candidateId, setCandidateId] = useState("");
  const [testing, setTesting] = useState<{ candidateId: string; fingerprint: string } | null>(null);
  const [verified, setVerified] = useState<VerifiedProbe | null>(null);
  const [claude, setClaude] = useState<ClaudeSetupStatus | null>(null);
  const [checking, setChecking] = useState(false);
  const [checkedEnhancement, setCheckedEnhancement] = useState("");
  const mounted = useRef(true);
  const loadSequence = useRef(0);
  const loadingRequest = useRef(false);
  const loadTimeout = useRef<number | undefined>(undefined);
  const checkSequence = useRef(0);
  const pending = useRef(false);

  const refresh = useCallback(async () => {
    if (loadingRequest.current) return;
    loadingRequest.current = true;
    const sequence = ++loadSequence.current;
    const generation = gatewayGeneration();
    const timeout = window.setTimeout(() => {
      if (!mounted.current || sequence !== loadSequence.current) return;
      loadSequence.current += 1;
      loadingRequest.current = false;
      setLoadError(t("配置读取超时，请重试。"));
      setLoading(false);
    }, 15_000);
    loadTimeout.current = timeout;
    try {
      const result = await invoke<GatewayResult>("load_gateway");
      if (!mounted.current || sequence !== loadSequence.current || generation !== gatewayGeneration()) return;
      if (result.status !== "ok" || !result.gateway || !Array.isArray(result.pool) || !Array.isArray(result.agents)) {
        setLoadError(t("配置读取失败，请重试。"));
        return;
      }
      setLoadError("");
      publishGatewayPayload(result);
    } catch {
      if (mounted.current && sequence === loadSequence.current) setLoadError(t("配置读取失败，请重试。"));
    } finally {
      window.clearTimeout(timeout);
      if (sequence === loadSequence.current) {
        loadingRequest.current = false;
        if (mounted.current) setLoading(false);
      }
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    const unsubscribe = subscribeGatewayPayload((next) => {
      setPayload(next);
      setLoadError("");
      setLoading(false);
    });
    void refresh();
    const timer = window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, 5000);
    return () => {
      mounted.current = false;
      checkSequence.current += 1;
      loadSequence.current += 1;
      loadingRequest.current = false;
      unsubscribe();
      window.clearInterval(timer);
      window.clearTimeout(loadTimeout.current);
    };
  }, [refresh]);

  const agents = enhance
    ? [{ value: "codex", label: "Codex" }, { value: "claude", label: "Claude" }]
    : (payload?.agents ?? []).filter((agent) => agentVisible(focus, agent.agent)).map((agent) => ({ value: agent.agent, label: agent.name }));
  const agentId = focus === "codex" ? "codex" : selectedAgent;
  const gateway = setupGatewayState(focus, payload, agentId);
  const previousConfiguration = useRef(gateway.fingerprint);
  useEffect(() => {
    if (previousConfiguration.current !== gateway.fingerprint) {
      previousConfiguration.current = gateway.fingerprint;
      setVerified(null);
      setTesting(null);
    }
  }, [gateway.fingerprint]);
  const candidate = gateway.candidates.find((item) => item.id === candidateId) ?? gateway.candidates[0];
  const tested = !loadError && probeMatches(gateway, verified);
  const activeTest = testing?.fingerprint === gateway.fingerprint
    ? gateway.candidates.find((item) => item.id === testing.candidateId)
    : undefined;
  const testProvider = activeTest ? payload?.pool.find((item) => item.id === activeTest.providerId) : undefined;
  useEffect(() => {
    if (testing && (!activeTest || loadError)) {
      setTesting(null);
      setVerified(null);
    }
  }, [testing, activeTest, loadError]);
  const claudeReady = Boolean(claude?.status === "ok" && claude.supported && claude.sourceApp
    && claude.prepared && !claude.needsRebuild && claude.nodeAvailable
    && (claude.localization || claude.voiceInput || claude.extensions || (claude.computerUse && claude.computerUseSupported)));
  const enhancementReady = agentId === "codex" ? codexInstalled === true && codexEnhancementsEnabled : claudeReady;
  const enhancementKey = JSON.stringify([agentId, codexInstalled, codexEnhancementsEnabled, claudeReady]);
  const enhancementChecked = enhancementReady && checkedEnhancement === enhancementKey;
  const configured = enhance ? enhancementReady : gateway.bindingReady && !loadError;
  const ready = enhance ? enhancementChecked : configured && tested;
  const run = makeRun(setBusy, publishGatewayPayload, notify);
  const changeAgent = (next: string) => {
    checkSequence.current += 1;
    setSelectedAgent(next);
    setVerified(null);
    setTesting(null);
    setCheckedEnhancement("");
    setCandidateId("");
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
  const bindingIssue = {
    loading: t("正在读取配置…"),
    agent_missing: t("请选择要接入的 Agent。"),
    no_providers: t("添加供应商或登录订阅，并配置一个聊天模型。"),
    not_connected: t("接入 Agent 并选择默认模型。"),
    drift: t("Agent 配置已被其他工具更改，请重新确认接入。"),
    helper_stopped: t("网关未运行，请检查接入设置。"),
    no_model: t("请为 Agent 选择默认模型。"),
    unroutable: t("默认模型没有可用供应商，请检查模型和供应商范围。"),
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
          {ready ? (agentId === "codex" ? t("启动 Codex 并完成") : t("完成配置")) : configured ? t("验证并开始") : enhance ? t("完成增强配置") : t("添加供应商并接入")}
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
              {focus !== "codex" && agents.length ? <AppSelect ariaLabel={t("本次配置的 Agent")} disabled={checking || launchPending} onChange={changeAgent} options={agents} size="sm" value={agentId} /> : <span className="setup-agent"><AgentGlyph agent="codex" />Codex</span>}
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
                  <p role={loadError ? "alert" : undefined}>{loadError || (gateway.issue ? bindingIssue[gateway.issue] : tf("{0} 已接入 · {1}", [agentName(agentId), groupDisplayName(payload, gateway.model) ?? gateway.model]))}</p>
                  <div className="setup-actions">
                    {loadError ? <Button onClick={() => { setLoading(true); void refresh(); }} size="sm" variant="secondary"><RefreshCw size={13} />{t("重试")}</Button> : (
                      <>
                        <Button disabled={loading || busy !== null} onClick={() => setAdding("provider")} size="sm" variant="secondary"><Plus size={13} />{t("添加供应商")}</Button>
                        <Button disabled={loading || !gateway.providersConfigured || busy !== null} onClick={() => setAdding("agent")} size="sm" variant="ghost">{t("接入与模型")}<ArrowRight size={13} /></Button>
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
              ) : (
                <>
                  {gateway.candidates.length > 1 ? <AppSelect ariaLabel={t("测试供应商与模型")} disabled={Boolean(testing)} onChange={(next) => { setCandidateId(next); setVerified(null); }} options={gateway.candidates.map((item) => ({ value: item.id, label: `${maskAccountName(item.providerName)} · ${item.model}` }))} size="sm" value={candidate?.id ?? ""} /> : null}
                  <Button disabled={!configured || !candidate} onClick={() => { if (candidate) { setVerified(null); setTesting({ candidateId: candidate.id, fingerprint: gateway.fingerprint }); } }} size="sm" variant="secondary"><Zap size={13} />{tested ? t("重新测试模型请求") : t("测试模型请求")}</Button>
                  <p>{tested ? t("所选供应商的模型请求测试通过。") : t("手动发送一条短请求，可能产生少量用量。")}</p>
                  {gateway.multipleTargets ? <p>{t("这里只验证所选供应商与模型，不代表整个模型组。")}</p> : null}
                </>
              )}
              <Button disabled={!ready || launchPending || (agentId === "codex" && codexInstalled !== true)} onClick={() => void finish()} size="sm"><Play size={13} />{launchPending ? t("正在启动…") : agentId === "codex" ? codexRunning ? t("重启 Codex 并完成") : t("启动 Codex 并完成") : t("完成配置")}</Button>
              {agentId !== "codex" ? <p>{enhance ? t("完成后可在增强设置中启动应用。") : t("完成后，在 Agent 中发起一次对话即可使用。")}</p> : null}
              {agentId === "codex" && codexInstalled === false ? <button className="setup-text-action" onClick={() => onNavigate("agentSettings", "codex")} type="button">{t("未找到 Codex，打开安装设置")}</button> : null}
            </div>
          </div>
        </div>
      ) : null}
      {adding === "provider" ? <ProviderAdder busy={busy} notify={notify} onClose={() => { setAdding(null); void refresh(); }} payload={payload} run={run} /> : null}
      {adding === "agent" ? <AgentConnectDialog busy={busy} notify={notify} onClose={() => { setAdding(null); void refresh(); }} onManage={() => setAdding("provider")} payload={payload} run={run} /> : null}
      {testing && activeTest && testProvider ? (
        <ProviderTestDialog
          entry={testProvider}
          initialModel={activeTest.model}
          lockModel
          keyId={null}
          onClose={() => setTesting(null)}
          onStart={() => setVerified(null)}
          onResult={(result) => setVerified({ ...testing, ok: result.ok === true })}
        />
      ) : null}
    </section>
  );
}
