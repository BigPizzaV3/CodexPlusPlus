/**
 * 启动引导：一个两步向导（连接模型 → 启动 Codex），加上侧栏底部的一张进度卡。
 * 组件挂在应用顶层（切到设置模式也不卸载），进度卡投到侧栏留的位置里。
 *
 * 第一次启动时向导直接打开；中途要去别的页面（添加供应商、打开增强设置）时向导先收起，
 * 进度卡留在侧栏，点它回到没做完的那一步。连接状态、测试结果都记在这个组件里，收起再打开不会丢。
 * 只读取现有配置：Codex 直连当前用的供应商是否就绪，以及对它发一次真实请求的测试结果。
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, Check, CircleAlert, Compass, Loader2, Play, Plus, Puzzle, RefreshCw, X, Zap } from "lucide-react";
import { AgentGlyph } from "./hub-shared";
import { APP_LOGO_URL } from "./app-shared";
import { setupDirectState, type SetupDirectMode, type SetupRelaySettings, type SetupRelayStatus } from "./onboarding-setup-model";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";
import { t, tf } from "./i18n";
// 遮罩和关闭按钮沿用导览的样式（.tour-root / .tour-dim / .tour-close）。
import "./OnboardingTour.css";
import "./OnboardingSetup.css";

type Destination = "relay" | "agentSettings" | "enhance" | "userScripts" | "claude" | "overview";
type RelayTestResult = { status: string; message: string };
/** 一次供应商测试的结果，只对发起时的那份配置有效。 */
type VerifiedTest = { fingerprint: string; ok: boolean; message: string };

/** 向导的两步。 */
export type SetupStep = "connect" | "start";
const STEPS: SetupStep[] = ["connect", "start"];

export function OnboardingSetup({
  open, onOpenChange, cardSlot, relaySettings, codexInstalled, codexRunning, launchPending, codexEnhancementsEnabled,
  onDismiss, onTour, onNavigate, onAddProvider, onLaunch,
}: {
  /** 向导是否打开；关着时只有侧栏的进度卡。 */
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 侧栏底部给进度卡留的位置；设置模式的侧栏没有这个位置，那时只有向导。 */
  cardSlot: Element | null;
  /** 已保存的设置：当前直连供应商和配置切换开关。 */
  relaySettings: SetupRelaySettings | null;
  codexInstalled: boolean | null;
  codexRunning: boolean;
  launchPending: boolean;
  codexEnhancementsEnabled: boolean;
  /** 走完（true）或关掉不再显示（false）：进度卡收走。 */
  onDismiss: (completed: boolean) => void;
  onTour: () => void;
  onNavigate: (target: Destination, agent?: string) => void;
  onAddProvider: () => void;
  onLaunch: () => Promise<boolean>;
}) {
  // 挂上时就开着（第一次启动、从设置里重开）：从第一步讲起；从进度卡点开时跳到没做完的那一步。
  const [step, setStep] = useState<SetupStep>("connect");
  const [status, setStatus] = useState<SetupRelayStatus | null>(null);
  const [loadError, setLoadError] = useState("");
  const [testing, setTesting] = useState(false);
  const [verified, setVerified] = useState<VerifiedTest | null>(null);
  const dialog = useRef<HTMLDivElement>(null);
  const mounted = useRef(true);
  const loadSequence = useRef(0);
  const loadingRequest = useRef(false);
  const loadTimeout = useRef<number | undefined>(undefined);
  const testSequence = useRef(0);
  const pending = useRef(false);
  useModalFocus(dialog, open);

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
    // 侧栏进度卡也要知道配好没有：挂上时读一次。
    void refresh();
    return () => {
      mounted.current = false;
      loadSequence.current += 1;
      testSequence.current += 1;
      loadingRequest.current = false;
      window.clearTimeout(loadTimeout.current);
    };
  }, [refresh]);

  useEffect(() => {
    // 在 Codex 里登录或换供应商会改 auth.json / config.toml：只在向导开着时每 5 秒重读一次（窗口可见时）。
    if (!open) return;
    void refresh();
    const timer = window.setInterval(() => { if (document.visibilityState === "visible") void refresh(); }, 5000);
    return () => window.clearInterval(timer);
  }, [refresh, open]);

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
  const configured = direct.ready && !loadError;
  const tested = configured && verified?.ok === true && verified.fingerprint === direct.fingerprint;
  const testFailed = verified?.ok === false && verified.fingerprint === direct.fingerprint;
  const codexReady = codexInstalled === true && codexEnhancementsEnabled;
  // 两步里完成了几步：侧栏进度卡上的「1/2」和进度环。第二步做完卡片就收走了，所以最多到 1。
  const doneSteps = configured ? 1 : 0;
  const nextStep: SetupStep = configured ? "start" : "connect";

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

  /** 稍后再说：向导收起，进度卡留在侧栏。启动 Codex 的过程中不让关。 */
  const later = () => {
    if (pending.current) return;
    onOpenChange(false);
  };
  useEscapeLayer(open, later);

  /** 去别的页面接着配：向导先收起，进度卡留在侧栏。 */
  const leaveFor = (go: () => void) => {
    onOpenChange(false);
    go();
  };
  const finish = async (launchCodex: boolean) => {
    if (pending.current || launchPending) return;
    if (!launchCodex) {
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
    gateway: t("经本地服务"),
  };
  const connection = loadError
    || (direct.issue ? directIssue[direct.issue] : tf("当前供应商：{0} · {1}", [direct.profile?.name || t("未命名供应商"), modeLabel[direct.mode]]));
  const connectionTone = loadError ? "error" : direct.issue === "loading" ? "loading" : direct.issue ? "todo" : "ok";
  const codexLine = codexInstalled === null ? t("正在检查…")
    : codexInstalled === false ? t("没有找到 Codex 桌面版。")
    : codexEnhancementsEnabled ? t("已安装，增强已开启。") : t("增强总开关是关着的，启动后不会注入增强和拓展。");
  const stepLabel: Record<SetupStep, string> = { connect: t("连接模型"), start: t("开始使用") };
  const stepDone: Record<SetupStep, boolean> = { connect: configured, start: false };
  const launchLabel = launchPending ? t("正在启动…") : codexRunning ? t("重启 Codex") : t("启动 Codex");

  const card = (
    <section aria-label={t("启动引导")} className="onboarding-card">
      <button
        className="onboarding-card-main"
        onClick={() => { setStep(nextStep); onOpenChange(true); }}
        title={t("继续设置")}
        type="button"
      >
        <svg aria-hidden="true" className="onboarding-card-ring" viewBox="0 0 16 16">
          <circle cx="8" cy="8" fill="none" r="6" strokeWidth="2" />
          <circle cx="8" cy="8" fill="none" r="6" strokeDasharray={`${(doneSteps / STEPS.length) * 37.7} 37.7`} strokeLinecap="round" strokeWidth="2" transform="rotate(-90 8 8)" />
        </svg>
        <span className="onboarding-card-text">
          <strong>{t("开始使用")}</strong>
          <small>{nextStep === "connect" ? t("下一步：连接模型") : t("下一步：启动 Codex")}</small>
        </span>
        <span className="onboarding-card-count" title={tf("已完成 {0} / {1} 步", [doneSteps, STEPS.length])}>{`${doneSteps}/${STEPS.length}`}</span>
      </button>
      <button aria-label={t("不再显示启动引导")} className="onboarding-card-close" disabled={launchPending} onClick={() => onDismiss(false)} title={t("不再显示启动引导")} type="button"><X size={13} /></button>
    </section>
  );

  return (
    <>
      {cardSlot ? createPortal(card, cardSlot) : null}
      {!open ? null : createPortal(
        <div className="tour-root" ref={dialog} role="dialog" aria-modal="true" aria-labelledby="onboarding-title" tabIndex={-1}>
          <div className="tour-dim" />
          <div className="ob-card" data-step={step}>
            <header className="ob-head">
              <img alt="" className="ob-logo" draggable={false} src={APP_LOGO_URL} />
              <ol className="ob-steps">
                {STEPS.map((item, index) => (
                  <li data-state={item === step ? "current" : stepDone[item] ? "done" : "todo"} key={item}>
                    <button aria-current={item === step ? "step" : undefined} disabled={launchPending || item === step} onClick={() => setStep(item)} type="button">
                      <i aria-hidden="true">{stepDone[item] && item !== step ? <Check size={11} strokeWidth={3} /> : index + 1}</i>
                      {stepLabel[item]}
                    </button>
                  </li>
                ))}
              </ol>
              <button aria-label={t("稍后设置")} className="tour-close" disabled={launchPending} onClick={later} title={t("稍后设置")} type="button"><X aria-hidden="true" size={14} /></button>
            </header>

            {step === "connect" ? (
              <div className="ob-body">
                <span className="ob-kicker">{t("欢迎使用 Codex++")}</span>
                <h2 id="onboarding-title">{t("先让 Codex 连上模型")}</h2>
                <p className="ob-lead">{t("Codex++ 给 Codex 和 Claude 加上增强与拓展，也替你管理供应商。在 Codex 里登录 ChatGPT 账号，或者添加一个 API 供应商，两种都可以，之后随时能切换。")}</p>
                <div className="ob-status" data-tone={connectionTone}>
                  <span className="ob-status-icon">
                    {connectionTone === "ok" ? <Check aria-hidden="true" size={14} strokeWidth={2.6} /> : connectionTone === "loading" ? <Loader2 aria-hidden="true" className="animate-spin" size={14} /> : <CircleAlert aria-hidden="true" size={14} />}
                  </span>
                  <div className="ob-status-text">
                    <strong>{connectionTone === "ok" ? t("已经连上") : connectionTone === "loading" ? t("正在读取") : loadError ? t("读取失败") : t("还没有连上")}</strong>
                    <p role={loadError ? "alert" : undefined}>{connection}</p>
                  </div>
                  <div className="ob-status-actions">
                    {loadError ? (
                      <button className="ob-btn" onClick={() => void refresh()} type="button"><RefreshCw aria-hidden="true" size={13} />{t("重试")}</button>
                    ) : (
                      <>
                        <button className="ob-btn" disabled={launchPending} onClick={() => leaveFor(onAddProvider)} type="button"><Plus aria-hidden="true" size={13} />{t("添加供应商")}</button>
                        <button className="ob-btn ob-btn-quiet" disabled={launchPending} onClick={() => leaveFor(() => onNavigate("relay", "codex"))} type="button">{t("连接与供应商")}<ArrowRight aria-hidden="true" size={13} /></button>
                      </>
                    )}
                  </div>
                </div>
                {!configured ? null : direct.testable ? (
                  <div className="ob-status" data-tone={tested ? "ok" : testFailed ? "error" : "idle"}>
                    <span className="ob-status-icon">
                      {tested ? <Check aria-hidden="true" size={14} strokeWidth={2.6} /> : testFailed ? <CircleAlert aria-hidden="true" size={14} /> : <Zap aria-hidden="true" size={14} />}
                    </span>
                    <div className="ob-status-text">
                      <strong>{tested ? t("测试通过") : testFailed ? t("测试没有通过") : t("发一条测试请求（可选）")}</strong>
                      {testFailed
                        ? <p role="alert">{verified?.message}</p>
                        : <p>{tested ? t("当前供应商的模型请求测试通过。") : t("手动发送一条短请求，可能产生少量用量。")}</p>}
                    </div>
                    <div className="ob-status-actions">
                      <button className="ob-btn" disabled={testing} onClick={() => void testProfile()} type="button">
                        {testing ? <Loader2 aria-hidden="true" className="animate-spin" size={13} /> : <Zap aria-hidden="true" size={13} />}
                        {tested || testFailed ? t("重新测试") : t("测试模型请求")}
                      </button>
                    </div>
                  </div>
                ) : (
                  <p className="ob-hint">{t("聚合供应商会在多个成员间轮转，请在「连接与供应商」里逐个测试成员。")}</p>
                )}
              </div>
            ) : (
              <div className="ob-body">
                <h2 id="onboarding-title">{configured && codexReady ? t("都准备好了") : t("可以开始了")}</h2>
                <p className="ob-lead">{t("从 Codex++ 启动 Codex，增强和拓展才会生效。")}</p>
                <div className="ob-status" data-tone={connectionTone}>
                  <span className="ob-status-icon">
                    {connectionTone === "ok" ? <Check aria-hidden="true" size={14} strokeWidth={2.6} /> : connectionTone === "loading" ? <Loader2 aria-hidden="true" className="animate-spin" size={14} /> : <CircleAlert aria-hidden="true" size={14} />}
                  </span>
                  <div className="ob-status-text">
                    <strong>{t("连接模型")}</strong>
                    <p>{configured ? (tested ? tf("{0}，测试通过。", [connection]) : connection) : t("还没有连上模型，可以稍后在「连接与供应商」里完成。")}</p>
                  </div>
                  <div className="ob-status-actions">
                    <button className="ob-btn ob-btn-quiet" disabled={launchPending} onClick={() => setStep("connect")} type="button">{configured ? t("查看") : t("去完成")}</button>
                  </div>
                </div>
                <div className="ob-status" data-tone={codexInstalled === null ? "loading" : codexReady ? "ok" : "todo"}>
                  <span className="ob-status-icon ob-status-glyph"><AgentGlyph agent="codex" /></span>
                  <div className="ob-status-text"><strong>Codex</strong><p>{codexLine}</p></div>
                  {codexInstalled === null || codexReady ? null : (
                    <div className="ob-status-actions">
                      <button className="ob-btn" onClick={() => leaveFor(() => onNavigate(codexInstalled === false ? "agentSettings" : "enhance", "codex"))} type="button">{codexInstalled === false ? t("安装设置") : t("增强设置")}<ArrowRight aria-hidden="true" size={13} /></button>
                    </div>
                  )}
                </div>
                <div className="ob-next">
                  <span>{t("接下来可以看看")}</span>
                  <button className="ob-chip" onClick={onTour} type="button"><Compass aria-hidden="true" size={13} />{t("界面导览")}</button>
                  <button className="ob-chip" onClick={() => leaveFor(() => onNavigate("userScripts"))} type="button"><Puzzle aria-hidden="true" size={13} />{t("万物皆拓展")}</button>
                  <button className="ob-chip" onClick={() => leaveFor(() => onNavigate("claude", "claude"))} type="button"><AgentGlyph agent="claude" />{t("Claude 增强")}</button>
                </div>
              </div>
            )}

            <footer className="ob-foot">
              {step === "connect" ? (
                <button className="ob-btn ob-btn-quiet" onClick={later} type="button">{t("稍后设置")}</button>
              ) : (
                <button className="ob-btn ob-btn-quiet" disabled={launchPending} onClick={() => setStep("connect")} type="button"><ArrowLeft aria-hidden="true" size={13} />{t("上一步")}</button>
              )}
              <span className="ob-foot-spacer" />
              {step === "connect" ? (
                <button className={configured ? "ob-btn ob-btn-primary" : "ob-btn"} data-autofocus onClick={() => setStep("start")} type="button">
                  {configured ? t("下一步") : t("先跳过")}<ArrowRight aria-hidden="true" size={13} />
                </button>
              ) : codexInstalled === true ? (
                <>
                  <button className="ob-btn ob-btn-quiet" disabled={launchPending} onClick={() => void finish(false)} type="button">{t("完成，暂不启动")}</button>
                  <button className="ob-btn ob-btn-primary" data-autofocus disabled={launchPending} onClick={() => void finish(true)} type="button"><Play aria-hidden="true" size={13} />{launchLabel}</button>
                </>
              ) : (
                <button className="ob-btn ob-btn-primary" data-autofocus onClick={() => void finish(false)} type="button">{t("完成")}</button>
              )}
            </footer>
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}
