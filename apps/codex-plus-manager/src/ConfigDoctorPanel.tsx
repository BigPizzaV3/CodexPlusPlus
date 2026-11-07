import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Loader2, Play, RefreshCw, Save, ShieldCheck, Sparkles, Stethoscope, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { AppSelect, type AppSelectOption } from "./AppSelect";
import {
  aiBlockedHint,
  aiReady,
  aiStatusView,
  currentSourceNote,
  draftFromSettings,
  formatCheckedAt,
  groupFindings,
  localSourceNote,
  matchedPresetId,
  ollamaDocsUrl,
  presetFields,
  savedPayload,
  sourceOptions,
  summaryLine,
  summaryTotal,
  summarize,
  type DoctorPreset,
  type DoctorReport,
  type DoctorSettings,
  type DoctorSource,
  type DoctorState,
  type SettingsDraft,
  sortFindings,
} from "./config-doctor-model";
import { CxBadge, CxCard, CxEmpty, CxRow, CxSection, CxStatusIcon } from "./cx-layout";
import { Switch } from "./gateway-ui";
import { t, tf } from "./i18n";
import "./ConfigDoctorPanel.css";

/** 「AI 配置体检」。本地规则体检离线可用；开启 AI 解释后再把脱敏摘要交给模型解释。
 *  模型来源三选一：复用当前供应商（默认，零配置）/ 本机模型（自动探测）/ 自定义 OpenAI 兼容地址。
 *  卡片 id 必须以 `app-settings-` 开头，设置页的分类过滤才认。 */
export function ConfigDoctorPanel() {
  const [state, setState] = useState<DoctorState | null>(null);
  const [draft, setDraft] = useState<SettingsDraft>({ enabled: false, source: "current", baseUrl: "", model: "", apiKey: "" });
  const [report, setReport] = useState<DoctorReport | null>(null);
  /** 正在进行的动作：status（首次加载）/ save / run（本地）/ run-ai（AI 深度）。 */
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [privacyOpen, setPrivacyOpen] = useState(true);

  const load = useCallback(async () => {
    setBusy("status");
    try {
      const next = await invoke<DoctorState>("config_doctor_request", { action: "status" });
      setState(next);
      setDraft(draftFromSettings(next.settings));
      if (next.status && next.status !== "ok") setNotice({ tone: "error", text: next.message });
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  /** 统一的保存流程：开关与来源变化也走这里，保存完以后端返回的设置重建草稿。 */
  const persist = useCallback(async (target: SettingsDraft) => {
    setBusy("save");
    setNotice(null);
    try {
      const next = await invoke<DoctorState>("config_doctor_request", { action: "save", payload: savedPayload(target) });
      setState(next);
      setDraft(draftFromSettings(next.settings));
      setNotice(next.status && next.status !== "ok"
        ? { tone: "error", text: next.message }
        : { tone: "ok", text: next.message || t("已保存 AI 接入配置。") });
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }, []);

  const save = useCallback(() => void persist(draft), [draft, persist]);

  const chooseEnabled = (enabled: boolean) => {
    const target: SettingsDraft = { ...draft, enabled };
    setDraft(target);
    void persist(target);
  };

  /** 切来源立即持久化：current / local 不需要填任何字段，点完就能用。 */
  const chooseSource = (source: DoctorSource) => {
    if (source === draft.source) return;
    const target: SettingsDraft = { ...draft, source };
    setDraft(target);
    void persist(target);
  };

  const run = useCallback(async (useAi: boolean) => {
    setBusy(useAi ? "run-ai" : "run");
    setNotice(null);
    try {
      // 本地体检不传 useAi；AI 深度体检才带上。
      const result = await invoke<DoctorReport>("config_doctor_request", { action: "run", payload: useAi ? { useAi: true } : {} });
      setReport(result);
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  }, []);

  const settings: DoctorSettings | null = state?.settings ?? null;
  const presets: DoctorPreset[] = state?.presets ?? [];
  /** 草稿地址对得上的预设：自定义来源下下拉回显、一键填表都用它。 */
  const draftPreset = presets.find((preset) => preset.id === matchedPresetId(presets, draft)) ?? null;
  /** AI 体检用的是后端已保存的配置，能不能点按已保存的那份算。 */
  const aiAllowed = settings ? aiReady(settings) : false;
  const blocked = settings ? aiBlockedHint(settings) : null;
  const summary = report ? summarize(report.findings ?? []) : null;
  const line = summary ? summaryLine(summary) : null;
  const aiView = report?.ai ? aiStatusView(report.ai) : null;
  const groups = report ? groupFindings(report.findings ?? []) : [];
  const aiItems = report ? sortFindings(report.ai?.items ?? []) : [];
  const currentNote = currentSourceNote(state?.sources?.current);
  const localNote = localSourceNote(state?.sources?.local);
  const isCustom = draft.source === "custom";

  const choosePreset = (id: string) => {
    const fields = presetFields(presets.find((preset) => preset.id === id) ?? null);
    if (fields) setDraft((current) => ({ ...current, ...fields }));
  };

  /** 输入框只把值交给 updater，不在 updater 里读 event。 */
  const setField = (field: "baseUrl" | "model" | "apiKey", value: string) => setDraft((current) => ({ ...current, [field]: value }));

  const presetOptions: AppSelectOption<string>[] = [
    { value: "", label: t("自定义地址") },
    ...presets.map((preset) => ({ value: preset.id, label: preset.label, hint: preset.note || undefined })),
  ];

  const openDocs = async (url: string) => {
    try {
      await invoke("open_external_url", { url });
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
    }
  };

  return (
    <div className="cx-set-group" id="app-settings-doctor">
      <CxSection
        aside={settings?.hasKey ? <span className="config-doctor-key">{tf("已存 Key：{0}", [settings.apiKeyMasked || t("已保存")])}</span> : null}
        title={t("AI 接入配置")}
      >
        <CxCard>
          <div className="config-doctor-source">
            <div aria-label={t("AI 模型来源")} className="config-doctor-source-pills tp-tabs" role="radiogroup">
              {sourceOptions(state?.sources).map((option) => {
                const active = draft.source === option.value;
                return (
                  <button
                    aria-checked={active}
                    data-active={active}
                    disabled={busy !== null}
                    key={option.value}
                    onClick={() => chooseSource(option.value)}
                    role="radio"
                    title={t(option.hint)}
                    type="button"
                  >
                    {t(option.label)}
                  </button>
                );
              })}
            </div>
            {state && draft.source === "current" ? (
              <div className="config-doctor-source-note">
                <span className="config-doctor-source-line">{currentNote.label}</span>
                {currentNote.hint ? <p className="config-doctor-source-warn">{t(currentNote.hint)}</p> : null}
              </div>
            ) : null}
            {state && draft.source === "local" ? (
              <div className="config-doctor-source-note">
                <span className="config-doctor-source-line">{localNote.label}</span>
                {localNote.found ? null : (
                  <button className="cx-secondary" onClick={() => void openDocs(ollamaDocsUrl(presets))} type="button">
                    {t("Ollama 文档")}
                  </button>
                )}
              </div>
            ) : null}
          </div>
          <CxRow
            detail={t("启用后可用 AI 解释体检结论；本地规则体检始终可用。")}
            title={t("启用 AI 解释")}
          >
            <Switch
              disabled={busy !== null}
              label={t("启用 AI 解释")}
              on={draft.enabled}
              onToggle={() => chooseEnabled(!draft.enabled)}
            />
          </CxRow>
          {isCustom ? (
            <>
              <CxRow
                detail={draftPreset?.note || t("选一个预设，一键填好地址和模型；也可以直接填自己的地址。")}
                title={t("服务预设")}
              >
                {draftPreset?.docsUrl ? (
                  <button className="cx-secondary" onClick={() => void openDocs(draftPreset.docsUrl)} type="button">{t("申请 Key")}</button>
                ) : null}
                <AppSelect
                  ariaLabel={t("AI 服务预设")}
                  className="config-doctor-select"
                  disabled={busy !== null}
                  onChange={choosePreset}
                  options={presetOptions}
                  size="sm"
                  value={draftPreset?.id ?? ""}
                />
              </CxRow>
              <div className="config-doctor-fields">
                <label className="config-doctor-field">
                  <span>{t("Base URL")}</span>
                  <Input
                    aria-label={t("AI 服务的 Base URL")}
                    disabled={busy !== null}
                    onChange={(event) => setField("baseUrl", event.currentTarget.value)}
                    placeholder="https://api.example.com/v1"
                    spellCheck={false}
                    value={draft.baseUrl}
                  />
                </label>
                <label className="config-doctor-field">
                  <span>{t("模型")}</span>
                  <Input
                    aria-label={t("AI 模型名")}
                    disabled={busy !== null}
                    onChange={(event) => setField("model", event.currentTarget.value)}
                    placeholder="free-model"
                    spellCheck={false}
                    value={draft.model}
                  />
                </label>
                <label className="config-doctor-field">
                  <span>{t("API Key")}</span>
                  <Input
                    aria-label={t("AI 服务的 API Key")}
                    disabled={busy !== null}
                    onChange={(event) => setField("apiKey", event.currentTarget.value)}
                    placeholder={settings?.hasKey ? t("已保存，留空表示不改动") : t("粘贴 Key，只存在本机设置里")}
                    type="password"
                    value={draft.apiKey}
                  />
                </label>
              </div>
            </>
          ) : null}
          <CxRow
            detail={isCustom
              ? t("Key 只写入本机设置，界面只回显末几位；留空保存表示沿用已存的 Key。")
              : t("换来源和开关会立即保存，这一路不用填地址、模型或 Key。")}
            title={t("保存接入配置")}
          >
            <button className="cx-secondary" disabled={busy !== null} onClick={save} type="button">
              {busy === "save" ? <Loader2 className="animate-spin" /> : <Save />}
              {t("保存")}
            </button>
          </CxRow>
        </CxCard>
      </CxSection>

      {notice ? <div className={`config-doctor-notice is-${notice.tone}`}>{notice.text}</div> : null}

      <CxSection
        aside={
          <>
            {report ? <span className="config-doctor-time">{tf("体检于 {0}", [formatCheckedAt(report.checkedAt) || "—"])}</span> : null}
            <Button disabled={busy !== null} onClick={() => void run(false)} size="sm" variant="secondary">
              {busy === "run" ? <Loader2 className="animate-spin" /> : report ? <RefreshCw /> : <Play />}
              {report ? t("重新体检") : t("开始体检")}
            </Button>
            <Button disabled={busy !== null || !aiAllowed} onClick={() => void run(true)} size="sm">
              {busy === "run-ai" ? <Loader2 className="animate-spin" /> : <Sparkles />}
              {t("AI 深度体检")}
            </Button>
          </>
        }
        title={t("体检结果")}
      >
        <CxCard>
          {report ? (
            <>
              <CxRow
                badges={
                  <>
                    {summary && summary.error ? <CxBadge tone="danger">{tf("{0} 个错误", [summary.error])}</CxBadge> : null}
                    {summary && summary.warn ? <CxBadge tone="warn">{tf("{0} 个警告", [summary.warn])}</CxBadge> : null}
                    {summary && summary.info ? <CxBadge>{tf("{0} 条提示", [summary.info])}</CxBadge> : null}
                    {summary && !summaryTotal(summary) ? <CxBadge tone="ok">{t("全部正常")}</CxBadge> : null}
                  </>
                }
                detail={line ? tf(line.text, line.args) : t("没有发现问题：接入地址、模型和 Key 都能对上。")}
                icon={<CxStatusIcon tone={summary && summary.error ? "danger" : summary && summary.warn ? "warn" : "ok"}><Stethoscope /></CxStatusIcon>}
                title={t("本地规则体检")}
              />
              {groups.map((group) => (
                <div className="config-doctor-group" key={group.level}>
                  <div className="config-doctor-group-head">
                    <span>{t(group.label)}</span>
                    <span className="config-doctor-group-count">{group.items.length}</span>
                  </div>
                  {group.items.map((finding) => (
                    <div className="config-doctor-finding" data-level={finding.level} key={finding.id}>
                      <div className="config-doctor-finding-head">
                        <strong>{finding.title}</strong>
                        {finding.scope ? <CxBadge>{finding.scope}</CxBadge> : null}
                      </div>
                      <p>{finding.detail}</p>
                      {finding.fix ? <p className="config-doctor-fix">{tf("修复建议：{0}", [finding.fix])}</p> : null}
                    </div>
                  ))}
                </div>
              ))}
            </>
          ) : (
            <CxEmpty
              detail={t("体检只读本机配置，离线可用；AI 深度体检会额外调用你选的模型来源。")}
              title={busy === "status" ? t("正在读取配置…") : t("还没有体检结果，点右上「开始体检」。")}
            />
          )}
        </CxCard>

        {blocked ? <div className="config-doctor-hint">{tf(blocked.text, blocked.args)}</div> : null}

        {report && aiView ? (
          <CxCard className="config-doctor-ai" key={report.ai.status}>
            <CxRow
              badges={<CxBadge tone={aiView.tone === "danger" ? "danger" : aiView.tone === "ok" ? "ok" : "neutral"}>{t("AI 解释")}</CxBadge>}
              detail={aiView.facts.length ? `${aiView.detail}${aiView.detail ? " · " : ""}${aiView.facts.join(" · ")}` : aiView.detail || undefined}
              icon={<CxStatusIcon tone={aiView.tone === "ok" ? "ok" : aiView.tone === "danger" ? "danger" : "off"}><Sparkles /></CxStatusIcon>}
              title={t(aiView.label)}
            >
              {report.ai.status === "failed" ? (
                <Button disabled={busy !== null} onClick={() => void run(true)} size="sm" variant="secondary">
                  {busy === "run-ai" ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                  {t("重试 AI")}
                </Button>
              ) : null}
            </CxRow>
            {aiItems.map((item) => (
              <div className="config-doctor-finding" data-level={item.level} key={`ai-${item.id}`}>
                <div className="config-doctor-finding-head">
                  <strong>{item.title}</strong>
                </div>
                <p>{item.detail}</p>
                {item.fix ? <p className="config-doctor-fix">{tf("修复建议：{0}", [item.fix])}</p> : null}
              </div>
            ))}
          </CxCard>
        ) : null}
      </CxSection>

      {privacyOpen ? (
        <div className="config-doctor-privacy">
          <ShieldCheck aria-hidden />
          <span>{t("AI 深度体检只发送脱敏后的配置摘要（不含 API Key），可随时停用。「复用当前供应商」会用你自己的供应商额度跑一次（约几千 token）；本机模型与自定义不占 Codex++ 的额度。")}</span>
          <button aria-label={t("关闭隐私说明")} className="config-doctor-privacy-close" onClick={() => setPrivacyOpen(false)} type="button">
            <X aria-hidden />
          </button>
        </div>
      ) : null}
    </div>
  );
}
