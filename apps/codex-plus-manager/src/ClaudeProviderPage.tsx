/** Claude Code 供应商配置：管理供应商列表，切换时改写 ~/.claude/settings.json 的关键字段。 */
import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, CheckCircle2, Copy, Download, Edit3, ExternalLink, Eye, EyeOff, KeyRound, Plus, RefreshCw, Trash2, Wand2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { ManagementDrawer, PageHeader } from "./ManagementPages";
import { t, tf } from "./i18n";
import { providerAvatarText } from "./provider-name";
import {
  CLAUDE_PRESETS,
  COMPAT_SWITCHES,
  PRESET_CATEGORY_LABELS,
  applyFormFields,
  blankClaudeProvider,
  compatEnabled,
  duplicateClaudeProvider,
  formatSettingsConfig,
  parseSettingsConfig,
  providerBrief,
  providerFromPreset,
  readFormFields,
  setCompat,
  validateProviderDraft,
  type ClaudeFormFields,
  type ClaudePresetCategory,
  type ClaudeProvider,
  type ClaudeProvidersPayload,
  type ClaudeSettingsConfig,
} from "./claude-provider-model";
import "./ClaudeProviderPage.css";

type Status = "ok" | "failed" | "warning" | "info";
type ClaudeResult = { status: string; message: string } & Partial<ClaudeProvidersPayload>;
type Notify = (title: string, message: string, status?: Status) => Promise<void>;

/** 跨挂载保留上次读到的状态：切页回来先显示旧数据，再静默刷新。 */
const memo: { payload: ClaudeProvidersPayload | null } = { payload: null };

function hasPayload(result: ClaudeResult): result is ClaudeResult & ClaudeProvidersPayload {
  return Array.isArray(result.providers);
}

export function ClaudeProviderPage({
  notify,
  openUrl,
  onProvidersChanged,
}: {
  notify: Notify;
  openUrl: (url: string) => void;
  /** 供应商数量或当前供应商变化后通知外层刷新 Agent 栏提示。 */
  onProvidersChanged: () => void;
}) {
  const [payload, setPayload] = useState<ClaudeProvidersPayload | null>(memo.payload);
  const [busy, setBusy] = useState<string | null>(memo.payload ? null : "load");
  const [editing, setEditing] = useState<{ provider: ClaudeProvider; isNew: boolean } | null>(null);

  const accept = (result: ClaudeResult) => {
    if (hasPayload(result)) {
      memo.payload = result;
      setPayload(result);
    }
  };

  /** 所有写操作的统一入口：失败弹提示，成功按需提示并刷新 Agent 栏。 */
  const run = async (key: string, command: string, args: Record<string, unknown> | undefined, title: string, silent = false) => {
    setBusy(key);
    try {
      const result = await invoke<ClaudeResult>(command, args);
      accept(result);
      if (result.status !== "ok") {
        await notify(tf("{0}失败", [title]), result.message, "failed");
        return null;
      }
      if (!silent) await notify(title, result.message, "ok");
      onProvidersChanged();
      return result;
    } catch (error) {
      await notify(t("调用失败"), String(error), "failed");
      return null;
    } finally {
      setBusy(null);
    }
  };

  const refresh = async (silent = true) => {
    setBusy("load");
    try {
      const result = await invoke<ClaudeResult>("load_claude_providers");
      accept(result);
      if (result.status !== "ok") await notify(t("读取失败"), result.message, "failed");
      else if (!silent) await notify(t("已刷新"), result.message, "ok");
    } catch (error) {
      await notify(t("调用失败"), String(error), "failed");
    } finally {
      setBusy(null);
    }
  };

  useEffect(() => {
    void refresh();
    // 只在进入本页时读一次，之后由本页操作维护。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const providers = payload?.providers ?? [];
  const currentId = payload?.currentId ?? "";
  const live = payload?.live;
  const current = providers.find((provider) => provider.id === currentId);

  const switchTo = async (provider: ClaudeProvider) => {
    if (provider.id === currentId) return;
    await run(`switch:${provider.id}`, "switch_claude_provider", { id: provider.id }, t("切换供应商"));
  };

  const reapplyCurrent = async () => {
    if (!current) return;
    const ok = await confirm(
      tf("用「{0}」的配置覆盖 settings.json 里的地址、Key 和模型？其它设置不受影响，原文件会先备份。", [current.name]),
      { title: t("重新写入当前供应商"), kind: "warning" },
    );
    if (!ok) return;
    await run("reapply", "save_claude_provider", { provider: current }, t("重新写入"));
  };

  const remove = async (provider: ClaudeProvider) => {
    const ok = await confirm(tf("删除供应商「{0}」？此操作不可撤销。", [provider.name]), { title: t("删除供应商"), kind: "warning" });
    if (!ok) return;
    await run(`delete:${provider.id}`, "delete_claude_provider", { id: provider.id }, t("删除供应商"), true);
  };

  const save = async (provider: ClaudeProvider, andSwitch: boolean) => {
    const saved = await run("save", "save_claude_provider", { provider }, t("保存供应商"), andSwitch);
    if (!saved) return;
    setEditing(null);
    if (!andSwitch || !hasPayload(saved)) return;
    // 新建的供应商 id 由后端分配：按创建时间找最新那条。
    const target = provider.id
      ? saved.providers.find((item) => item.id === provider.id)
      : saved.providers.reduce<ClaudeProvider | undefined>((latest, item) => (!latest || item.createdAt > latest.createdAt ? item : latest), undefined);
    if (target && target.id !== saved.currentId) {
      await run(`switch:${target.id}`, "switch_claude_provider", { id: target.id }, t("切换供应商"));
    }
  };

  return (
    <div className="mg-page relay-management-page claude-provider-page">
      <PageHeader
        icon={<KeyRound className="h-5 w-5" />}
        title={t("供应商列表")}
        actions={(
          <>
            <Button disabled={busy !== null} onClick={() => void refresh(false)} size="sm" variant="ghost" title={t("重新读取 settings.json")}>
              <RefreshCw className={`h-4 w-4 ${busy === "load" ? "animate-spin" : ""}`} />
              {t("刷新")}
            </Button>
            <Button
              disabled={busy !== null}
              onClick={() => void run("ccs", "import_claude_providers_from_cc_switch", undefined, t("从 CC Switch 导入"))}
              title={t("只读打开 CC Switch 的数据库，导入其中的 Claude 供应商；已导入过的会跳过。")}
              variant="outline"
            >
              <Download className="h-4 w-4" />
              {t("从 CC Switch 导入")}
            </Button>
            <Button disabled={busy !== null} onClick={() => setEditing({ provider: blankClaudeProvider(), isNew: true })}>
              <Plus className="h-4 w-4" />
              {t("添加供应商")}
            </Button>
          </>
        )}
      />
      <div className="mg-content">
        <LiveStatus
          payload={payload}
          busy={busy}
          onImportLive={() => void run("importLive", "import_claude_live_provider", undefined, t("保存当前配置"))}
          onReapply={() => void reapplyCurrent()}
        />

        {providers.length === 0 ? (
          <div className="claude-provider-empty">
            <strong>{busy === "load" && !payload ? t("正在读取…") : t("还没有 Claude 供应商")}</strong>
            <span>
              {live?.hasKeyFields
                ? t("settings.json 里已有地址或 Key：可以先「保存为供应商」，再添加其他供应商来回切换。")
                : t("点右上角「添加供应商」从预设创建，或从 CC Switch 导入已有的供应商。")}
            </span>
          </div>
        ) : (
          <div className="relay-profile-list">
            {providers.map((provider) => {
              const active = provider.id === currentId;
              const brief = providerBrief(provider);
              const pending = busy === `switch:${provider.id}`;
              return (
                <div
                  className={`relay-profile-card claude-provider-card ${active ? "active" : ""}`}
                  key={provider.id}
                  onKeyDown={(event) => { if (event.key === "Enter" && event.target === event.currentTarget) setEditing({ provider, isNew: false }); }}
                  tabIndex={0}
                >
                  {/* 与 Codex 供应商列表同一套：两字头像、接入方式与来源做成标签，第二行只放地址和模型。 */}
                  <span className="relay-index" title={provider.name}>{providerAvatarText(provider.name, "C")}</span>
                  <span className="relay-summary">
                    <strong>
                      <span className="relay-summary-name">{provider.name.trim() || t("未命名供应商")}</span>
                      <span className="relay-tag" data-tone={brief.endpoint ? "api" : "official"}>
                        {brief.endpoint ? t("API") : t("官方登录")}
                      </span>
                      {provider.ccsSourceId ? <span className="relay-tag">CC Switch</span> : null}
                      {active ? <span className="relay-active-badge">{t("使用中")}</span> : null}
                    </strong>
                    <small>
                      {brief.endpoint
                        ? [brief.endpoint, brief.model].filter(Boolean).join(" · ")
                        : t("使用 Claude Code 当前登录的账号，不设置地址与 Key")}
                    </small>
                  </span>
                  <span className="relay-card-actions">
                    <Button
                      className={`relay-use-button ${active ? "active" : ""}`}
                      disabled={busy !== null}
                      onClick={(event) => { event.stopPropagation(); void switchTo(provider); }}
                      size="sm"
                      title={active ? t("当前正在使用") : t("写入 settings.json，新开的 Claude Code 会话生效")}
                      variant={active ? "secondary" : "outline"}
                    >
                      <CheckCircle2 className="h-4 w-4" />
                      {pending ? t("切换中") : active ? t("使用中") : t("使用")}
                    </Button>
                    <span className="relay-card-extra">
                      {provider.websiteUrl ? (
                        <Button onClick={() => openUrl(provider.websiteUrl)} size="icon" title={t("打开官网")} variant="ghost">
                          <ExternalLink className="h-4 w-4" />
                        </Button>
                      ) : null}
                      <Button onClick={() => setEditing({ provider, isNew: false })} size="icon" title={t("编辑")} variant="ghost">
                        <Edit3 className="h-4 w-4" />
                      </Button>
                      <Button
                        onClick={() => setEditing({ provider: duplicateClaudeProvider(provider, t("副本")), isNew: true })}
                        size="icon"
                        title={t("复制")}
                        variant="ghost"
                      >
                        <Copy className="h-4 w-4" />
                      </Button>
                      <Button
                        disabled={active || busy !== null}
                        onClick={() => void remove(provider)}
                        size="icon"
                        title={active ? t("正在使用的供应商不能删除") : t("删除")}
                        variant="ghost"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </span>
                  </span>
                </div>
              );
            })}
          </div>
        )}
        <p className="claude-provider-footnote">
          {t("切换只改 settings.json 里的地址、Key、模型等关键字段，hooks、权限、插件等其它设置原样保留；每次写入前自动备份。已打开的 Claude Code 会话需要重开才会用上新供应商。")}
        </p>
      </div>

      {editing ? (
        <ClaudeProviderEditor
          initial={editing.provider}
          isNew={editing.isNew}
          isCurrent={!editing.isNew && editing.provider.id === currentId}
          saving={busy === "save"}
          onCancel={() => setEditing(null)}
          onSave={(provider, andSwitch) => void save(provider, andSwitch)}
        />
      ) : null}
    </div>
  );
}

function LiveStatus({
  payload,
  busy,
  onImportLive,
  onReapply,
}: {
  payload: ClaudeProvidersPayload | null;
  busy: string | null;
  onImportLive: () => void;
  onReapply: () => void;
}) {
  if (!payload) return null;
  const { live } = payload;
  const hasCurrent = payload.providers.some((provider) => provider.id === payload.currentId);
  return (
    <>
      <div className="claude-live-strip">
        <span className="claude-live-item" title={live.settingsPath}>
          <em>settings.json</em>
          <span>{live.exists ? live.settingsPath : t("尚未创建，首次切换时生成")}</span>
        </span>
        <span className="claude-live-item">
          <em>{t("当前地址")}</em>
          <span>{live.baseUrl || t("官方（未设置）")}</span>
        </span>
        <span className="claude-live-item">
          <em>{t("Key")}</em>
          <span>{live.authConfigured ? t("已配置") : t("未配置")}</span>
        </span>
        <span className="claude-live-item" title={payload.cliPath ?? ""}>
          <em>Claude Code</em>
          <span>{payload.cliPath ? t("已安装") : t("未检测到")}</span>
        </span>
      </div>
      {live.parseError ? (
        <p className="claude-live-alert" role="alert">
          <AlertTriangle className="h-4 w-4" />
          <span>{tf("settings.json 无法解析，切换已暂停，不会覆盖它：{0}", [live.parseError])}</span>
        </p>
      ) : hasCurrent && !live.matchesCurrent ? (
        <p className="claude-live-alert" role="status">
          <AlertTriangle className="h-4 w-4" />
          <span>{t("settings.json 里的地址、Key 或模型被外部改过，和当前供应商不一致。")}</span>
          <Button disabled={busy !== null} onClick={onReapply} size="sm" variant="outline">{t("重新写入当前供应商")}</Button>
        </p>
      ) : !hasCurrent && live.hasKeyFields ? (
        <p className="claude-live-alert info" role="status">
          <AlertTriangle className="h-4 w-4" />
          <span>{t("settings.json 里已有地址或 Key，但不属于任何已保存的供应商。保存下来以便随时切回。")}</span>
          <Button disabled={busy !== null} onClick={onImportLive} size="sm" variant="outline">{t("保存为供应商")}</Button>
        </p>
      ) : null}
    </>
  );
}

const MODEL_ROLES: Array<{ field: keyof ClaudeFormFields; label: string; hint: string }> = [
  { field: "haikuModel", label: "Haiku", hint: "后台小任务、标题生成" },
  { field: "sonnetModel", label: "Sonnet", hint: "/model 里的 Sonnet 档" },
  { field: "opusModel", label: "Opus", hint: "/model 里的 Opus 档" },
  { field: "fableModel", label: "Fable", hint: "/model 里的 Fable 档，可留空" },
];

function ClaudeProviderEditor({
  initial,
  isNew,
  isCurrent,
  saving,
  onCancel,
  onSave,
}: {
  initial: ClaudeProvider;
  isNew: boolean;
  isCurrent: boolean;
  saving: boolean;
  onCancel: () => void;
  onSave: (provider: ClaudeProvider, andSwitch: boolean) => void;
}) {
  const [draft, setDraft] = useState<ClaudeProvider>(initial);
  const [jsonText, setJsonText] = useState(() => formatSettingsConfig(initial.settingsConfig));
  const [jsonError, setJsonError] = useState<string | null>(null);
  const [showKey, setShowKey] = useState(false);
  const [presetId, setPresetId] = useState<string>(isNew && !initial.name ? "custom" : "");
  const fields = useMemo(() => readFormFields(draft.settingsConfig), [draft.settingsConfig]);

  /** 表单与 JSON 共用一份 settingsConfig：表单改动重写 JSON 文本，JSON 合法时反过来驱动表单。 */
  const updateConfig = (next: ClaudeSettingsConfig) => {
    setDraft((previous) => ({ ...previous, settingsConfig: next }));
    setJsonText(formatSettingsConfig(next));
    setJsonError(null);
  };
  const updateFields = (patch: Partial<ClaudeFormFields>) => updateConfig(applyFormFields(draft.settingsConfig, { ...fields, ...patch }));

  const pickPreset = (id: string) => {
    setPresetId(id);
    const preset = CLAUDE_PRESETS.find((item) => item.id === id);
    const next = preset ? providerFromPreset(preset) : blankClaudeProvider();
    setDraft((previous) => ({ ...next, id: previous.id, createdAt: previous.createdAt }));
    setJsonText(formatSettingsConfig(next.settingsConfig));
    setJsonError(null);
  };

  const onJsonChange = (text: string) => {
    setJsonText(text);
    const parsed = parseSettingsConfig(text);
    if (parsed.ok) {
      setDraft((previous) => ({ ...previous, settingsConfig: parsed.value }));
      setJsonError(null);
    } else {
      setJsonError(parsed.error);
    }
  };

  const validation = jsonError ?? validateProviderDraft(draft);
  const official = !fields.baseUrl && !fields.apiKey;
  const presetGroups = (Object.keys(PRESET_CATEGORY_LABELS) as ClaudePresetCategory[])
    .map((category) => ({ category, presets: CLAUDE_PRESETS.filter((preset) => preset.category === category) }));

  return (
    <ManagementDrawer
      title={isNew ? t("添加 Claude 供应商") : tf("编辑「{0}」", [initial.name])}
      onClose={onCancel}
      dismissOnBackdrop={false}
      footer={(
        <div className="claude-editor-footer">
          <span className="claude-editor-error" role={validation ? "alert" : undefined}>{validation ? t(validation) : ""}</span>
          <Button onClick={onCancel} variant="ghost">{t("取消")}</Button>
          {!isCurrent ? (
            <Button disabled={Boolean(validation) || saving} onClick={() => onSave(draft, true)} variant="outline">
              {t("保存并使用")}
            </Button>
          ) : null}
          <Button disabled={Boolean(validation) || saving} onClick={() => onSave(draft, false)}>
            {saving ? t("保存中") : isCurrent ? t("保存并写入") : t("保存")}
          </Button>
        </div>
      )}
    >
      <div className="claude-editor">
        {isNew ? (
          <section className="claude-editor-section">
            <h3>{t("从预设开始")}</h3>
            <div className="claude-preset-groups">
              <div className="claude-preset-row">
                <button className={`claude-preset-chip ${presetId === "custom" ? "active" : ""}`} onClick={() => pickPreset("custom")} type="button">
                  <Wand2 className="h-3.5 w-3.5" />
                  {t("自定义")}
                </button>
              </div>
              {presetGroups.map(({ category, presets }) => (
                <div className="claude-preset-row" key={category}>
                  <span className="claude-preset-label">{t(PRESET_CATEGORY_LABELS[category])}</span>
                  {presets.map((preset) => (
                    <button
                      className={`claude-preset-chip ${presetId === preset.id ? "active" : ""}`}
                      key={preset.id}
                      onClick={() => pickPreset(preset.id)}
                      title={preset.env.ANTHROPIC_BASE_URL || t("使用 Claude 官方登录")}
                      type="button"
                    >
                      {t(preset.name)}
                    </button>
                  ))}
                </div>
              ))}
            </div>
          </section>
        ) : null}

        <section className="claude-editor-section claude-editor-grid">
          <label className="claude-field">
            <span>{t("名称")}</span>
            <Input value={draft.name} onChange={(event) => setDraft({ ...draft, name: event.currentTarget.value })} placeholder={t("例如 DeepSeek")} />
          </label>
          <label className="claude-field">
            <span>{t("官网")}</span>
            <Input value={draft.websiteUrl} onChange={(event) => setDraft({ ...draft, websiteUrl: event.currentTarget.value })} placeholder="https://" />
          </label>
          <label className="claude-field span-2">
            <span>{t("请求地址（ANTHROPIC_BASE_URL）")}</span>
            <Input value={fields.baseUrl} onChange={(event) => updateFields({ baseUrl: event.currentTarget.value })} placeholder={t("留空则使用 Claude 官方地址")} />
          </label>
          <label className="claude-field span-2">
            <span>API Key</span>
            <div className="claude-key-row">
              <Input
                autoComplete="off"
                onChange={(event) => updateFields({ apiKey: event.currentTarget.value })}
                placeholder={t("留空则使用 Claude 官方登录")}
                spellCheck={false}
                type={showKey ? "text" : "password"}
                value={fields.apiKey}
              />
              <Button onClick={() => setShowKey(!showKey)} size="icon" title={showKey ? t("隐藏") : t("显示")} variant="ghost">
                {showKey ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
              </Button>
              <select
                aria-label={t("Key 写入的变量")}
                className="claude-select"
                onChange={(event) => updateFields({ authField: event.currentTarget.value as ClaudeFormFields["authField"] })}
                title={t("多数网关用 ANTHROPIC_AUTH_TOKEN（Bearer）；少数要求 ANTHROPIC_API_KEY（x-api-key）。")}
                value={fields.authField}
              >
                <option value="ANTHROPIC_AUTH_TOKEN">AUTH_TOKEN</option>
                <option value="ANTHROPIC_API_KEY">API_KEY</option>
              </select>
            </div>
          </label>
          {official ? (
            <p className="claude-field-hint span-2">{t("地址和 Key 都留空时，切换后 Claude Code 使用自己的官方登录（/login）。")}</p>
          ) : null}
        </section>

        <section className="claude-editor-section">
          <div className="claude-section-head">
            <h3>{t("模型")}</h3>
            <Button
              disabled={!fields.model.trim()}
              onClick={() => updateFields({ haikuModel: fields.model, sonnetModel: fields.model, opusModel: fields.model })}
              size="sm"
              title={t("把 Haiku / Sonnet / Opus 都设成主模型")}
              variant="ghost"
            >
              {t("各档都用主模型")}
            </Button>
          </div>
          <div className="claude-editor-grid">
            <label className="claude-field span-2">
              <span>{t("主模型（ANTHROPIC_MODEL）")}</span>
              <Input value={fields.model} onChange={(event) => updateFields({ model: event.currentTarget.value })} placeholder={t("留空则由 Claude Code 选择")} />
            </label>
            {MODEL_ROLES.map((role) => (
              <label className="claude-field" key={role.field}>
                <span>{role.label} <small>{t(role.hint)}</small></span>
                <Input value={fields[role.field]} onChange={(event) => updateFields({ [role.field]: event.currentTarget.value })} />
              </label>
            ))}
          </div>
        </section>

        <section className="claude-editor-section">
          <h3>{t("兼容开关")}</h3>
          <div className="claude-compat-list">
            {COMPAT_SWITCHES.map((item) => (
              <label className="claude-compat" key={item.key}>
                <input
                  checked={compatEnabled(draft.settingsConfig, item.key, item.on)}
                  onChange={(event) => updateConfig(setCompat(draft.settingsConfig, item.key, item.on, event.currentTarget.checked))}
                  type="checkbox"
                />
                <span>
                  <strong>{t(item.label)}</strong>
                  <small>{t(item.detail)} <code>{item.key}={item.on}</code></small>
                </span>
              </label>
            ))}
          </div>
        </section>

        <section className="claude-editor-section">
          <h3>{t("完整配置（JSON）")}</h3>
          <p className="claude-field-hint">
            {t("与 settings.json 同形。切换时只写入其中的关键字段（ANTHROPIC_*、模型、鉴权等）和兼容开关、窗口值；其它键不会写入。")}
          </p>
          <Textarea
            className="claude-json"
            onChange={(event) => onJsonChange(event.currentTarget.value)}
            rows={10}
            spellCheck={false}
            value={jsonText}
          />
        </section>

        <section className="claude-editor-section">
          <label className="claude-field">
            <span>{t("备注")}</span>
            <Input value={draft.notes} onChange={(event) => setDraft({ ...draft, notes: event.currentTarget.value })} />
          </label>
        </section>
      </div>
    </ManagementDrawer>
  );
}
