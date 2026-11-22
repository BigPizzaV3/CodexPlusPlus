// 供应商详情与编辑器（Codex 连接页、多开页共用）：普通 / 聚合供应商编辑、模型窗口、VLM 测试、配置文件编辑、供应商体检。
import {
  closestCenter,
  DndContext,
  KeyboardSensor,
  PointerSensor,
  useSensor,
  useSensors,
  type DragEndEvent,
} from "@dnd-kit/core";
import {
  SortableContext,
  sortableKeyboardCoordinates,
  useSortable,
  verticalListSortingStrategy,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import {
  ArrowLeft,
  Cable,
  Check,
  CheckCircle2,
  ChevronDown,
  Download,
  GripVertical,
  Info,
  KeyRound,
  MessageCircle,
  FileCode2,
  Plus,
  RefreshCw,
  Save,
  ShieldCheck,
  ShieldAlert,
  Stethoscope,
  Trash2,
} from "lucide-react";
import { createPortal } from "react-dom";
import { useEffect, useId, useMemo, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { selectedContextConfigToml, splitContextConfigText, stripContextEntriesFromConfig, type CodexContextEntries } from "./context-config-text";
import { AppSelect } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { proxyAddressError } from "./gateway-model";
import { DEFAULT_AUTO_COMPACT_PERCENT, autoCompactOptions, normalizeAutoCompactPercent } from "./auto-compact";
import {
  builtinEntryToImportDocument,
  builtinRowBackfillValue,
  clearModelMetadataForSlug,
  importDocumentSyncPatch,
  importPanelControls,
  importSaveDecision,
  metadataMatchesBuiltin,
  metadataSourceTags,
  modelSlugFromRowName,
  suffixWindowString,
  normalizeTokenCountInput,
  modelMetadataKey,
  modelFastSupportMode,
  modelMultiAgentMode,
  synchronizeModelMetadataLiteSupport,
  synchronizeModelMetadataFastSupport,
  synchronizeModelMetadataMultiAgentVersion,
  type ModelFastSupportMode,
  type ModelMultiAgentMode,
  parseModelMetadataDocument,
  parseModelMetadataMap,
  remapModelMetadataSlugs,
  replaceModelMetadataForSlug,
  resolveModelMetadataRowKey,
  retainModelMetadataForSlugs,
  serializeModelMetadataDocument,
  synchronizeModelMetadataDocumentLimitsPreview,
  type ActiveImportDraft,
  createActiveImportDraft,
  updateActiveImportDraft,
  cancelActiveImportDraft,
  rematchActiveImportDraft,
  builtinMetadataQueryState,
  type BuiltinMetadataQueryState,
  type BuiltinModelMetadataMatch,
  type ImportedModelMetadata,
} from "./model-metadata";
import { setIndependentProxyConfigured, modelRouteSaveRequiresRestart, normalizeRelayModelRoutes, type RelayModelRoute } from "./model-routes";
import {
  mergeModelWindowRows,
  modelWindowRowsFromProfile,
  modelWindowRowsValidationError,
  serializeModelWindowRows,
  reorderModelWindowRows,
  type ModelWindowRowsValidationIssue,
  type ModelWindowRow,
} from "./model-windows";
import {
  reasoningParameterMode,
  setModelReasoningParameterMode,
  type ReasoningParameterMode,
  formatReasoningLevelsLabel,
  normalizeReasoningLevels,
  REASONING_LEVELS,
  resolveModelReasoningLevels,
  setModelReasoningLevelsForSlug,
} from "./model-reasoning";
import { clampAggregateRoutePriority } from "./aggregate-routes";
import { aggregateMemberCandidates, isOfficialLoginMemberProfile } from "./aggregate-members";
import { relayAuthForLiveDraft } from "./relay-live-files";
import { SavedAuthAccountField } from "./NativeAuthControls";
import { RelayHeaderRows } from "./RelayHeaderRows";
import { apiKeyPreview, createRelayApiKey, normalizeRelayApiKeys, type RelayApiKey } from "./provider-api-keys";
import { relayHeadersValidationMessage, serializeRelayHeaders } from "./relay-headers";
import { CompatToggles } from "./CompatToggles";
import { errorText } from "@/lib/utils";
import { useModalShell } from "./use-modal-shell";
import { usePopoverLayer } from "./use-popover-layer";
import { t, tf } from "@/i18n";
import {
  deriveRelayProfileFromFiles,
  applyRelayProfilePatchToFiles,
  parseModelSuffix,
  codexBaseUrlFromConfig,
  relayModelRoutesSettingsValidation,
  relaySettingsValidationForSave,
  relaySessionProviderValidation,
  relaySettingsWithDraft,
  relayProfileUsesLiveFiles,
  syncLegacyRelayFields,
  updateRelayProfile,
  addRelayProfile,
  aggregateStrategyOptions,
  isAggregateRelayProfile,
  normalizeAggregateRelayProfile,
  aggregateProfileForMode,
  normalizeAggregateConfig,
  clampAggregateWeight,
  aggregateStrategyLabel,
  aggregateStrategyHelp,
  aggregateRelayProfileValidation,
  ensureTrailingNewline,
  normalizeRelaySessionProvider,
  relaySessionProvider,
  normalizeAnthropicMaxTokens,
} from "./relay-profile-editing";
import { vlmTestTranslation } from "./vlm-test-translation";
import type {
  Actions,
  AggregateMode,
  BackendSettings,
  ProviderDoctorResult,
  RelayAggregateConfig,
  RelayAggregateRoute,
  RelayContextSelection,
  RelayEditorMode,
  RelayFilesResult,
  RelayProfile,
  SettingsResult,
} from "./app-types";
import {
  CODEX_GATEWAY_RELAY_ID,
  contextEntriesFromSettings,
  isSuccessStatus,
  joinTomlSectionsRootFirst,
  Metric,
  normalizeSettings,
  relayEditorMode,
  relayProfileConfigBrief,
  relayProfileModeLabel,
  relayProfileTagTone,
  relayProtocolShortLabel,
  ToggleVisual,
  tomlRootKeyFromLine,
  Toolbar,
} from "./app-shared";

function modelWindowRowsValidationMessage(issue: ModelWindowRowsValidationIssue | null): string | null {
  if (!issue) return null;
  if (issue.code === "duplicateModel") return tf("模型名称重复：{0}", [issue.model]);
  if (issue.code === "invalidWindow") {
    return tf("模型 {0} 的上下文窗口无效；请输入正整数，或使用 K/M 整数后缀。", [issue.model]);
  }
  return tf("模型 {0} 的自动压缩百分比无效；请输入 0 到 100 之间、最多 6 位小数的十进制数。", [issue.model]);
}

// 模型行「思考等级」列：多选档位 + 默认档。选择直接写回该模型的
// supported_reasoning_levels，不下发本地草稿，面板与「模型配置」JSON 共用一份数据。
function ModelReasoningLevelsCell({
  levels,
  inherited,
  defaultLevel,
  parameterMode,
  onParameterModeChange,
  onChange,
}: {
  levels: string[];
  parameterMode: ReasoningParameterMode;
  onParameterModeChange: (mode: ReasoningParameterMode) => void;
  /// true = 当前值来自内置声明而非该模型的自定义配置。
  inherited: boolean;
  defaultLevel: string | null;
  onChange: (levels: string[], defaultLevel: string | null) => void;
}) {
  const [open, setOpen] = useState(false);
  const label = parameterMode === "omit" ? t("不发送思考参数") : formatReasoningLevelsLabel(levels);
  const toggleLevel = (level: string) => {
    const picked = levels.includes(level)
      ? levels.filter((item) => item !== level)
      : [...levels, level];
    const next = normalizeReasoningLevels(picked);
    // 默认档必须落在选中集合里，取消勾选时一起清掉。
    onChange(next, defaultLevel && next.includes(defaultLevel) ? defaultLevel : null);
  };
  const triggerTip = levels.length
    ? `${levels.join(", ")}${defaultLevel ? ` · ${t("默认等级")}: ${defaultLevel}` : ""}`
    : t("未设置时沿用内置默认档位");
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({
    open,
    onOpenChange: setOpen,
    align: "end",
    menuWidth: 240,
    minWidth: 240,
    maxHeight: 320,
  });
  return (
    <div
      className={`app-select relay-reasoning ${open ? "open" : ""} ${inherited ? "inherited" : ""}`.trim()}
      ref={rootRef}
    >
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        className="app-select-trigger"
        onClick={() => setOpen((current) => !current)}
        title={triggerTip}
        type="button"
      >
        <span className={label ? "" : "relay-reasoning-empty"}>{label || t("未设置")}</span>
        <ChevronDown className="h-4 w-4" />
      </button>
      {open ? createPortal(
        <div
          aria-multiselectable="true"
          className="app-select-menu app-select-menu-portal relay-reasoning-menu"
          ref={menuRef}
          role="listbox"
          style={menuStyle}
        >
          <div className="relay-reasoning-default">
            <span>{t("请求参数")}</span>
            <div className="relay-reasoning-chips">
              {([ ["auto", t("自动兼容")], ["passthrough", t("原样传等级（Chat）")], ["omit", t("不发送")] ] as const).map(([mode, text]) =>
                <button key={mode} type="button" className={parameterMode === mode ? "selected" : ""} onClick={() => onParameterModeChange(mode)}>{text}</button>)}
            </div>
          </div>
          {REASONING_LEVELS.map((level) => {
            const selected = levels.includes(level);
            return (
              <button
                aria-selected={selected}
                className={`app-select-option ${selected ? "selected" : ""}`.trim()}
                key={level}
                disabled={parameterMode === "omit"}
                onClick={() => toggleLevel(level)}
                onMouseDown={(event) => event.preventDefault()}
                role="option"
                type="button"
              >
                <span className="app-select-option-label">{level}</span>
                {selected ? <Check aria-hidden className="app-select-check" /> : null}
              </button>
            );
          })}
          <div className="relay-reasoning-default">
            <span>{t("默认等级")}</span>
            <div className="relay-reasoning-chips">
              <button
                className={defaultLevel ? "" : "selected"}
                disabled={parameterMode === "omit"}
                onClick={() => onChange(levels, null)}
                type="button"
              >
                {t("自动")}
              </button>
              {levels.map((level) => (
                <button
                  className={defaultLevel === level ? "selected" : ""}
                  key={level}
                  disabled={parameterMode === "omit"}
                  onClick={() => onChange(levels, level)}
                  type="button"
                >
                  {level}
                </button>
              ))}
            </div>
          </div>
          <p className="relay-reasoning-hint">
            {parameterMode === "omit" ? t("请求不发送思考参数；档位草稿保留，切回自动兼容后可恢复。")
              : parameterMode === "passthrough" ? t("Chat 上游按原值接收 reasoning_effort，不按模型名改写档位。")
              : inherited
              ? t("当前为内置默认，修改后保存为该模型的自定义思考等级。")
              : t("未设置时沿用内置默认；清空全部档位即恢复内置。")}
          </p>
        </div>,
        document.body,
      ) : null}
    </div>
  );
}

// 模型行的拖动 ID 要稳定：原来用行号当 ID，排完序动画会对到别的行上。
type SortableModelWindowRow = ModelWindowRow & { sortId: string };
let nextModelRowSortId = 0;

function withModelRowSortId(row: ModelWindowRow, sortId?: string): SortableModelWindowRow {
  return { ...row, sortId: sortId ?? `model-row-${++nextModelRowSortId}` };
}

/** 把编辑中的模型行写回草稿（保存、诊断、未保存判断共用同一份序列化）。 */
function relayDraftWithModelRows(draft: RelayProfile, rows: ModelWindowRow[]): RelayProfile {
  const serializedRows = serializeModelWindowRows(rows);
  const validSlugs = serializedRows.modelList.split("\n").map((slug) => slug.trim()).filter(Boolean);
  return {
    ...draft,
    modelList: serializedRows.modelList,
    modelWindows: serializedRows.modelWindows,
    modelAutoCompact: serializedRows.modelAutoCompact,
    modelMetadata: retainModelMetadataForSlugs(draft.modelMetadata, validSlugs),
    customHeaders: serializeRelayHeaders(draft.customHeaders || []),
    modelVlm: serializedRows.modelVlm,
  };
}

export function RelayProfileDetail({
  profile,
  relayFiles,
  form,
  isNew = false,
  initialAggregateMode = null,
  initialSection = null,
  onBack,
  onFormChange,
  onSaved,
  actions,
}: {
  profile: RelayProfile;
  relayFiles: RelayFilesResult | null;
  form: BackendSettings;
  isNew?: boolean;
  initialAggregateMode?: AggregateMode | null;
  /** 打开时直接跳到的设置（「Codex 连接 → 高级」里点某个供应商的独立代理进来）。 */
  initialSection?: "proxy" | null;
  onBack: () => void;
  onFormChange: (value: BackendSettings) => Promise<BackendSettings | null>;
  onSaved?: () => void;
  actions: Actions;
}) {
  const detailBodyRef = useRef<HTMLDivElement>(null);
  const [draft, setDraft] = useState<RelayProfile>(profile);
  const [modelWindowRows, setModelWindowRows] = useState<SortableModelWindowRow[]>(() =>
    modelWindowRowsFromProfile(profile.modelList, profile.modelWindows || "", profile.modelVlm, profile.modelAutoCompact)
      .map((row) => withModelRowSortId(row)),
  );
  // 供应商独立代理、前缀稳定层按供应商 id 存在设置之外，也算进草稿：改了随「保存供应商」一起写，不再各自立即保存。
  const [proxyUrl, setProxyUrl] = useState("");
  const [savedProxy, setSavedProxy] = useState<{ url: string; configured: boolean }>({ url: "", configured: false });
  const [promptPrefix, setPromptPrefix] = useState(false);
  const [savedPromptPrefix, setSavedPromptPrefix] = useState(false);
  const extrasTouchedRef = useRef({ proxy: false, prefix: false });
  useEffect(() => {
    let cancelled = false;
    extrasTouchedRef.current = { proxy: false, prefix: false };
    setProxyUrl("");
    setSavedProxy({ url: "", configured: false });
    setPromptPrefix(false);
    setSavedPromptPrefix(false);
    setIndependentProxyConfigured(profile.id, false);
    void invoke<{ status: string; proxyUrl?: string; configured?: boolean }>("load_relay_proxy", { profileId: profile.id })
      .then((result) => {
        if (cancelled || result.status !== "ok") return;
        const url = result.proxyUrl || "";
        const configured = result.configured ?? Boolean(url.trim());
        setIndependentProxyConfigured(profile.id, configured);
        setSavedProxy({ url, configured });
        // 读回来之前已经开始改了：不覆盖用户的输入。
        if (!extrasTouchedRef.current.proxy) setProxyUrl(url);
      })
      .catch(() => {});
    void invoke<{ stabilizePromptPrefix?: boolean }>("load_relay_feature", { profileId: profile.id })
      .then((result) => {
        if (cancelled) return;
        const enabled = result.stabilizePromptPrefix === true;
        setSavedPromptPrefix(enabled);
        if (!extrasTouchedRef.current.prefix) setPromptPrefix(enabled);
      })
      .catch(() => {});
    return () => { cancelled = true; };
  }, [profile.id]);
  const changeProxyUrl = (value: string) => {
    extrasTouchedRef.current.proxy = true;
    setProxyUrl(value);
  };
  const changePromptPrefix = (value: boolean) => {
    extrasTouchedRef.current.prefix = true;
    setPromptPrefix(value);
  };
  const proxyChanged = !isAggregateRelayProfile(draft) && proxyUrl.trim() !== savedProxy.url.trim();
  const promptPrefixChanged = !isAggregateRelayProfile(draft) && promptPrefix !== savedPromptPrefix;
  const proxyProblem = isAggregateRelayProfile(draft) ? null : proxyAddressError(proxyUrl);
  const proxyError = proxyProblem ? t(proxyProblem) : null;
  // 从「高级」里的供应商代理列表进来：展开「请求」分区并聚焦代理输入框。
  useEffect(() => {
    if (initialSection !== "proxy") return;
    const root = detailBodyRef.current;
    jumpToRelaySection(root, "requests");
    requestAnimationFrame(() => root?.querySelector<HTMLInputElement>(".relay-field-proxy input")?.focus());
  }, [initialSection, profile.id]);
  const [doctorResult, setDoctorResult] = useState<ProviderDoctorResult | null>(null);
  const [doctorOpen, setDoctorOpen] = useState(false);
  const [doctorRunning, setDoctorRunning] = useState(false);
  const [savingDraft, setSavingDraft] = useState(false);
  const [aggregateMode, setAggregateMode] = useState<AggregateMode | null>(() => {
    if (!isAggregateRelayProfile(profile)) return null;
    if (!isNew) return profile.aggregate?.modelAggregationEnabled === true ? "model" : "provider";
    // 从「故障转移 / 模型聚合」分页进入时类型已确定，不再弹选择器。
    return initialAggregateMode ?? null;
  });
  const isActive = !isNew && profile.id === form.activeRelayId;
  const profileUsesLiveFiles = relayProfileUsesLiveFiles(profile);
  // 只有当前供应商才从 live 文件回填；非当前供应商时不能把 relayFiles 当依赖——
  // live 文件一变就会重建整份模型行，编辑中的增删被无声撤销（保存后看起来「没生效」）。
  const liveConfigContents = isActive && profileUsesLiveFiles ? relayFiles?.configContents ?? "" : "";
  const liveAuthContents = isActive && profileUsesLiveFiles ? relayFiles?.authContents ?? "" : "";
  // 行编辑守卫：编辑器里还有未保存的模型行改动时，外部变化（元数据、live 文件、别处保存设置
  // 触发的重渲染）不许把用户的增删覆盖回已保存列表。
  const unsavedRowEditsRef = useRef(false);
  const lastDetailProfileIdRef = useRef(profile.id);
  useEffect(() => {
    const useLiveFiles = isActive && profileUsesLiveFiles && relayFiles;
    const liveDraft = isAggregateRelayProfile(profile)
      ? normalizeAggregateRelayProfile(profile, form)
      : deriveRelayProfileFromFiles(
          useLiveFiles
            ? {
              ...profile,
              configContents: relayFiles.configContents,
              authContents: relayAuthForLiveDraft(profile, relayFiles.authContents),
            }
            : profile,
        );
    const storedApiKey = useLiveFiles ? profile.apiKey.trim() : "";
    const nextDraft = useLiveFiles && !isAggregateRelayProfile(liveDraft)
      ? applyRelayProfilePatchToFiles(liveDraft, { apiKey: storedApiKey })
      : liveDraft;
    setDraft(nextDraft);
    const switchedProfile = lastDetailProfileIdRef.current !== profile.id;
    lastDetailProfileIdRef.current = profile.id;
    const nextRows = modelWindowRowsFromProfile(nextDraft.modelList, nextDraft.modelWindows || "", nextDraft.modelVlm, nextDraft.modelAutoCompact)
      .map((row) => withModelRowSortId(row));
    // 未保存判断的基线：用载入后的草稿按同样方式序列化，刚打开时不会误报「有未保存修改」。
    baselineDraftRef.current = JSON.stringify(relayDraftWithModelRows(nextDraft, nextRows));
    if (switchedProfile || !unsavedRowEditsRef.current) {
      setModelWindowRows(nextRows);
    }
    // 依赖里刻意不放 profile.modelMetadata / 原始 relayFiles：元数据改动是本编辑器自己触发的
    // （删行会清该模型元数据），把它当依赖会让「删行 → 清元数据 → 重建行」把删掉的行加回来。
  }, [profile.id, profile.modelList, profile.modelWindows, profile.modelAutoCompact, profile.modelVlm, profileUsesLiveFiles, isActive, isNew, liveConfigContents, liveAuthContents]);
  // 每次按键都会重算：relaySettingsWithDraft 要解析 TOML，模型行要序列化，按依赖缓存。
  const validationSettings = useMemo(() => relaySettingsWithDraft(form, profile.id, draft, isNew), [form, profile.id, draft, isNew]);
  const validationError = useMemo(() => relaySessionProviderValidation(draft)
    ?? (isAggregateRelayProfile(draft)
      ? aggregateRelayProfileValidation(draft)
      : relayModelRoutesSettingsValidation(validationSettings)), [draft, validationSettings]);
  const modelRowsError = useMemo(() => modelWindowRowsValidationMessage(modelWindowRowsValidationError(modelWindowRows)), [modelWindowRows]);
  // 请求头写错（名字非法、值里有换行）也挡住保存，和模型行错误一样就地写出原因。
  const headersError = useMemo(() => relayHeadersValidationMessage(draft.customHeaders || []), [draft.customHeaders]);
  const blockingIssue = validationError || modelRowsError || headersError || proxyError;
  const currentDraft = useMemo(() => relayDraftWithModelRows(draft, modelWindowRows), [draft, modelWindowRows]);
  const draftWithModelRows = () => currentDraft;
  // 「有未保存修改」：整份草稿（名称、地址、Key、协议、请求头、文件……）和打开时的基线比，
  // 不只比模型行；基线在载入 / 外部更新草稿时重设。
  const baselineDraftRef = useRef<string | null>(null);
  const currentDraftJson = useMemo(() => JSON.stringify(currentDraft), [currentDraft]);
  if (baselineDraftRef.current === null) baselineDraftRef.current = currentDraftJson;
  const hasUnsavedChanges = currentDraftJson !== baselineDraftRef.current || proxyChanged || promptPrefixChanged;
  const hasUnsavedModelChanges = useMemo(() => JSON.stringify({
    modelList: currentDraft.modelList,
    modelWindows: currentDraft.modelWindows,
    modelAutoCompact: currentDraft.modelAutoCompact,
    modelMetadata: currentDraft.modelMetadata,
    modelVlm: currentDraft.modelVlm,
  }) !== JSON.stringify({
    modelList: profile.modelList || "",
    modelWindows: profile.modelWindows || "",
    modelAutoCompact: profile.modelAutoCompact || "",
    modelMetadata: profile.modelMetadata || "",
    modelVlm: profile.modelVlm || "",
  }), [currentDraft, profile.modelList, profile.modelWindows, profile.modelAutoCompact, profile.modelMetadata, profile.modelVlm]);
  // 交给上面的行编辑守卫：有未保存行改动时，外部变化不覆盖模型行。
  unsavedRowEditsRef.current = hasUnsavedModelChanges;
  const saveDraft = async () => {
    if (savingDraft) return;
    // 配置有误不再静默 return：把原因弹出来（按钮也不再因此禁用，见头部那一行提示）。
    if (isNew && isAggregateRelayProfile(draft) && aggregateMode === null) {
      void actions.showMessage(t("保存供应商"), t("先选择聚合类型（模型聚合 / 故障转移）再保存。"), "failed");
      return;
    }
    if (blockingIssue) {
      void actions.showMessage(t("保存供应商"), blockingIssue, "failed");
      return;
    }
    setSavingDraft(true);
    // 代理决定这个供应商是否经本地代理转发：先按改后的代理重算配置文件，写入失败或取消时还原。
    const proxyConfiguredNow = Boolean(proxyUrl.trim()) || (savedProxy.configured && !savedProxy.url.trim());
    if (proxyChanged) setIndependentProxyConfigured(profile.id, proxyConfiguredNow);
    let proxySaved = !proxyChanged;
    try {
      const draftWithWindows = proxyChanged && !isAggregateRelayProfile(draftWithModelRows())
        ? applyRelayProfilePatchToFiles(draftWithModelRows(), { baseUrl: draftWithModelRows().baseUrl }, { allowGenerateFiles: isNew })
        : draftWithModelRows();
      const normalizedDraft = isAggregateRelayProfile(draftWithWindows) ? normalizeAggregateRelayProfile(draftWithWindows, form) : deriveRelayProfileFromFiles(draftWithWindows);
      const next = normalizeSettings(isNew
        ? addRelayProfile(form, normalizedDraft)
        : updateRelayProfile(form, profile.id, normalizedDraft));
      // 校验的是保存后的整份设置：别的供应商有问题也存不了，弹出原因并点名是哪一个，不再静默不动。
      const settingsValidationError = relaySettingsValidationForSave(next, normalizedDraft.id);
      if (settingsValidationError) {
        void actions.showMessage(t("保存供应商"), settingsValidationError, "failed");
        return;
      }
      const activeLiveBaseUrl = codexBaseUrlFromConfig(
        relayFiles?.configContents ?? profile.configContents,
      );
      const requiresRestart = isActive && modelRouteSaveRequiresRestart(
        normalizeSettings(form),
        next,
        activeLiveBaseUrl,
      );
      if (requiresRestart && !(await appConfirm(t("此请求设置需要启动本地协议代理。保存后将立即重启 Codex，使设置安全生效。是否继续？"), { title: t("需要重启 Codex"), confirmText: t("保存并重启") }))) {
        return;
      }
      // 代理、前缀稳定层单独存：先写它们，失败就停下，不留一半。
      if (proxyChanged) {
        const result = await invoke<{ status: string; message: string; proxyUrl?: string; configured?: boolean }>("set_relay_proxy", { profileId: normalizedDraft.id, proxyUrl });
        if (result.status !== "ok") {
          void actions.showMessage(t("供应商独立代理"), t(result.message), "failed");
          return;
        }
        proxySaved = true;
        const url = result.proxyUrl ?? proxyUrl.trim();
        const configured = result.configured ?? Boolean(url);
        setIndependentProxyConfigured(normalizedDraft.id, configured);
        setSavedProxy({ url, configured });
      }
      if (promptPrefixChanged) {
        const result = await invoke<{ status: string; message: string }>("set_relay_feature", { profileId: normalizedDraft.id, stabilizePromptPrefix: promptPrefix });
        if (result.status !== "ok") {
          void actions.showMessage(t("前缀稳定层"), t(result.message), "failed");
          return;
        }
        setSavedPromptPrefix(promptPrefix);
      }
      const savedSettings = await onFormChange(next);
      if (!savedSettings) return;
      if (requiresRestart) {
        const restarted = await actions.restart(true);
        if (!restarted) return;
        onSaved?.();
        return;
      }
      const savedProfile = savedSettings.relayProfiles.find((candidate) => candidate.id === normalizedDraft.id)
        ?? normalizedDraft;
      if (isActive && savedSettings.relayProfilesEnabled && relayProfileUsesLiveFiles(savedProfile)) {
        await actions.switchRelayProfile(savedSettings, savedSettings.activeRelayId);
      }
      onSaved?.();
    } catch (error) {
      void actions.showMessage(t("保存供应商"), errorText(error), "failed");
    } finally {
      // 没写成代理（校验没过、取消重启、写入失败）：把按新代理算的状态还原。
      if (!proxySaved) setIndependentProxyConfigured(profile.id, savedProxy.configured);
      setSavingDraft(false);
    }
  };
  const switchDraft = () => {
    if (isNew || blockingIssue) return;
    const draftWithWindows = draftWithModelRows();
    const normalizedDraft = isAggregateRelayProfile(draftWithWindows) ? normalizeAggregateRelayProfile(draftWithWindows, form) : deriveRelayProfileFromFiles(draftWithWindows);
    const previousActiveRelayId = form.activeRelayId;
    const next = syncLegacyRelayFields({
      ...form,
      relayProfiles: form.relayProfiles.map((item) => (item.id === profile.id ? normalizedDraft : item)),
      activeRelayId: profile.id,
    });
    void actions.switchRelayProfile(next, previousActiveRelayId);
  };
  const runProviderDoctor = async () => {
    setDoctorOpen(true);
    setDoctorRunning(true);
    setDoctorResult(null);
    const draftWithWindows = draftWithModelRows();
    const result = await actions.diagnoseRelayProfile(deriveRelayProfileFromFiles(draftWithWindows));
    setDoctorResult(result);
    setDoctorRunning(false);
  };
  const aggregateProfile = isAggregateRelayProfile(draft);
  const aggregateModeRequired = isNew && aggregateProfile && aggregateMode === null;
  const selectAggregateMode = (mode: AggregateMode) => {
    if (!aggregateProfile) return;
    setAggregateMode(mode);
    setDraft(aggregateProfileForMode(draft, form, mode));
  };
  const showDoctor = !aggregateProfile && (draft.relayMode !== "official" || draft.officialMixApiKey);
  const detailStatus = aggregateProfile
    ? aggregateModeRequired
      ? t("先选择类型")
      : draft.aggregate?.modelAggregationEnabled === true
        ? t("模型聚合：模型固定绑定供应商")
        : t("故障转移：按策略分配请求")
    : relayProfileEditorStatus(draft, form, isNew);
  return (
    <div className="relay-detail-page" key={profile.id}>
      <div className="relay-detail-header">
        <div className="relay-editor-heading">
          <Button aria-label={t("返回列表")} onClick={() => void (async () => {
            if (hasUnsavedChanges && !(await appConfirm(t("有未保存的修改，确定放弃吗？"), { title: t("放弃修改"), confirmText: t("放弃"), danger: true }))) return;
            onBack();
          })()} size="icon" title={t("返回列表")} type="button" variant="ghost">
            <ArrowLeft className="h-4 w-4" />
          </Button>
          <div className="relay-editor-heading-copy">
            <strong>{draft.name || (aggregateProfile ? t("未命名聚合供应商") : t("未命名供应商"))}</strong>
            <span>{hasUnsavedChanges ? `${detailStatus} · ${t("有未保存修改")}` : detailStatus}</span>
            {blockingIssue ? (
              // 就地写出「为什么存不了」：按钮不再禁用，这里也给人一眼能看到的原因。
              <span className="relay-editor-validation" role="alert">
                {blockingIssue}
              </span>
            ) : null}
          </div>
        </div>
        <div className="relay-editor-actions">
          {showDoctor ? (
            <Button disabled={doctorRunning} onClick={() => void runProviderDoctor()} type="button" variant="secondary">
              <Stethoscope className="h-4 w-4" />
              {doctorRunning ? t("诊断中") : t("诊断供应商")}
            </Button>
          ) : null}
          {aggregateProfile ? (
            <UiBadge variant="secondary">{aggregateModeRequired ? t("聚合")
              : draft.aggregate?.modelAggregationEnabled === true ? t("模型聚合") : t("故障转移")}</UiBadge>
          ) : isNew || form.activeRelayId === CODEX_GATEWAY_RELAY_ID ? null : (
            // 网关接管期间（从多开页进来编辑）不给「设为当前」：切走会绕过「断开并还原」（#2801）。
            <Button
              disabled={actions.relaySwitching}
              onClick={switchDraft}
              title={!form.relayProfilesEnabled ? t("供应商配置总开关已关闭，点击后按提示开启") : actions.relaySwitching ? t("供应商切换中") : undefined}
              variant={draft.id === form.activeRelayId ? "secondary" : "default"}
            >
              {actions.relaySwitching ? t("切换中") : draft.id === form.activeRelayId ? t("使用中") : t("设为当前")}
            </Button>
          )}
          <Button
            // 配置有误时也让人点：点一下把原因弹出来，比一个点不动的按钮清楚。
            disabled={savingDraft}
            onClick={() => void saveDraft()}
            title={aggregateModeRequired ? t("先选择类型") : blockingIssue || t("保存")}
            type="button"
          >
            <Save className="h-4 w-4" />
            {savingDraft ? t("保存中") : t("保存供应商")}
          </Button>
        </div>
      </div>
      {!aggregateModeRequired && !isAggregateRelayProfile(draft) ? (
        // 编辑页很长：标题栏下给一排分区跳转，点哪个就展开该分区并滚过去。
        <nav className="relay-section-nav" aria-label={t("跳转到分区")}>
          {relaySectionLinks().map((link) => (
            <button key={link.id} onClick={() => jumpToRelaySection(detailBodyRef.current, link.id)} type="button">
              {link.label}
            </button>
          ))}
        </nav>
      ) : null}
      <div className="relay-detail-body" ref={detailBodyRef}>
        {aggregateModeRequired ? (
          <AggregateModeChooser onSelect={selectAggregateMode} />
        ) : (
          <RelayProfileEditor
            profile={draft}
            form={form}
            isNew={isNew}
            proxy={{ value: proxyUrl, error: proxyError, onChange: changeProxyUrl }}
            promptPrefix={{ value: promptPrefix, onChange: changePromptPrefix }}
            onProfileChange={setDraft}
            actions={actions}
            modelWindowRows={modelWindowRows}
            setModelWindowRows={setModelWindowRows}
          />
        )}
        {isAggregateRelayProfile(draft) ? null : (
        <RelayFileEditors
          contextProfile={profile}
          profile={draft}
          form={form}
          isActive={isActive}
          profileId={profile.id}
          onFormChange={onFormChange}
          onProfileChange={setDraft}
          actions={actions}
        />
        )}
      </div>
      {doctorOpen ? (
        <ProviderDoctorModal
          result={doctorResult}
          running={doctorRunning}
          onClose={() => {
            if (!doctorRunning) setDoctorOpen(false);
          }}
        />
      ) : null}
    </div>
  );
}

type SortableModelWindowEntryProps = {
  id: string;
  children: (sortable: ReturnType<typeof useSortable>) => ReactNode;
};

function SortableModelWindowEntry({ id, children }: SortableModelWindowEntryProps) {
  const sortable = useSortable({ id });
  const style: CSSProperties = {
    transform: CSS.Transform.toString(sortable.transform),
    transition: sortable.isDragging ? "none" : sortable.transition ?? "transform 200ms ease",
  };

  return (
    <div
      className={`relay-model-entry ${sortable.isDragging ? "dragging" : ""}`}
      ref={sortable.setNodeRef}
      style={style}
    >
      {children(sortable)}
    </div>
  );
}

/// 编辑页分区导航的条目。id 与 RelayFold 的 sectionId 一致；`basic` 是不可折叠的基础设置。
function relaySectionLinks() {
  return [
    { id: "basic", label: t("基础") },
    { id: "models", label: t("模型") },
    { id: "requests", label: t("请求") },
    { id: "advanced", label: t("更多选项") },
    { id: "config-preview", label: t("配置预览") },
  ];
}

/// 跳到编辑页某个分区：折叠着就先展开（会触发 RelayFold 记住展开状态），再滚到分区顶部。
function jumpToRelaySection(root: HTMLElement | null, id: string) {
  if (!root) return;
  const target = id === "basic"
    ? root.querySelector<HTMLElement>(".relay-basic-section")
    : root.querySelector<HTMLElement>(`details[data-section="${id}"]`);
  if (!target) return;
  if (target instanceof HTMLDetailsElement && !target.open) target.open = true;
  requestAnimationFrame(() => target.scrollIntoView({ behavior: "smooth", block: "start" }));
}

function RelayFold({ title, children, className = "", bodyClassName = "", issue, sectionId, defaultOpen = false }: {
  title: string;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  issue?: string | null;
  sectionId?: string;
  defaultOpen?: boolean;
}) {
  const preferenceKey = sectionId ? `codex-plus-relay-fold:${sectionId}` : null;
  const [open, setOpen] = useState(() => {
    if (issue) return true;
    try {
      const stored = preferenceKey ? window.localStorage.getItem(preferenceKey) : null;
      if (stored === "true" || stored === "false") return stored === "true";
    } catch {
      // 无法使用本地存储时仍可展开、编辑并保存供应商草稿。
    }
    return defaultOpen;
  });
  const previousIssue = useRef(issue);
  useEffect(() => {
    // 同一项错误更新文字时不抢回用户刚收起的面板，新出现的错误才展开。
    if (issue && !previousIssue.current) setOpen(true);
    previousIssue.current = issue;
  }, [issue]);
  return (
    <details className={`relay-fold ${className}`.trim()} data-section={sectionId} open={open} onToggle={(event) => {
      const next = event.currentTarget.open;
      if (next === open) return;
      setOpen(next);
      if (preferenceKey) {
        try { window.localStorage.setItem(preferenceKey, String(next)); } catch { /* 偏好不可写不影响编辑。 */ }
      }
    }}>
      <summary className="relay-fold-summary">
        <ChevronDown aria-hidden="true" className="relay-fold-chevron h-4 w-4" />
        <strong>{title}</strong>
        {issue ? <span className="relay-fold-issue" title={issue}>{t("需要检查")}</span> : null}
      </summary>
      <div className={`relay-fold-body ${bodyClassName}`.trim()}>{children}</div>
    </details>
  );
}

// 模型行级内置元数据索引：窗口/压缩用于回填，「思考等级」列用作内置初值。
type BuiltinMetadataIndexEntry = {
  source: string;
  context_window: unknown;
  auto_compact_token_limit: unknown;
  supported_reasoning_levels?: unknown;
};

function RelayProfileEditor({
  profile,
  form,
  isNew = false,
  proxy,
  promptPrefix,
  onProfileChange,
  actions,
  modelWindowRows,
  setModelWindowRows,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  isNew?: boolean;
  /** 供应商独立代理：草稿的一部分，随「保存供应商」写入。 */
  proxy: { value: string; error: string | null; onChange: (value: string) => void };
  /** 前缀稳定层开关：同上。 */
  promptPrefix: { value: boolean; onChange: (value: boolean) => void };
  onProfileChange: (value: RelayProfile) => void;
  actions: Actions;
  modelWindowRows: SortableModelWindowRow[];
  setModelWindowRows: (value: SortableModelWindowRow[]) => void;
}) {
  const [vlmTestOpen, setVlmTestOpen] = useState(false);
  // 「高级兼容选项」分组：null = 用户没手动开合过，按是否有开启项决定。
  const [compatOpen, setCompatOpen] = useState<boolean | null>(null);
  const compatEnabledCount = [
    profile.standardOpenaiProtocol,
    promptPrefix.value,
    profile.webSearchHistoryCompat,
    profile.localCompactionPromptAtEnd === true,
    profile.trustUpstreamCatalogLite === true,
    profile.strictResponsesReasoning === true,
    profile.stripImageGenerationTool === true,
    profile.stripResponsesImageDetail === true,
    profile.stripResponsesOutputEncryptedContent === true,
    profile.disableToolSearch === true,
    profile.pureApiImageGeneration === true,
  ].filter(Boolean).length;
  const useCommonConfig = profile.useCommonConfig !== false;
  const [activeImportDraft, setActiveImportDraft] = useState<ActiveImportDraft | null>(null);
  const [metadataImportError, setMetadataImportError] = useState("");
  const [builtinMatch, setBuiltinMatch] = useState<BuiltinModelMetadataMatch | null>(null);
  const [builtinMatchSlug, setBuiltinMatchSlug] = useState("");
  const [builtinQueryState, setBuiltinQueryState] = useState<BuiltinMetadataQueryState | null>(null);
  const [importPrefillSource, setImportPrefillSource] = useState<"builtin" | "existing" | null>(null);
  const [builtinIndex, setBuiltinIndex] = useState<Map<string, BuiltinMetadataIndexEntry>>(new Map());
  const modelSensors = useSensors(
    useSensor(PointerSensor, {
      activationConstraint: { distance: 8 },
    }),
    useSensor(KeyboardSensor, {
      coordinateGetter: sortableKeyboardCoordinates,
    }),
  );
  // 面板内置查询的请求代数：begin/rematch 是命令式调用（没有 effect cleanup
  // 的 cancelled 通道），响应返回时代数不匹配即丢弃全部 setState——防止迟到
  // 响应把已取消的面板重新打开，或覆盖用户改名后的新查询结果。
  const builtinQuerySeqRef = useRef(0);
  const queryBuiltinCommand = async <T,>(command: string, args?: Record<string, unknown>): Promise<T | null> => {
    try { return await invoke<T>(command, args); } catch (error) {
      // 索引只影响行级标记与便利回填（失败是假阴性、不产错误数据），可诊断即可。
      console.warn(`[Codex++] ${command} 查询失败`, error);
      return null;
    }
  };
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const result = await queryBuiltinCommand<{ entries: Array<BuiltinMetadataIndexEntry & { slug: string }> }>("builtin_model_metadata_index");
      if (cancelled || !result?.entries) return;
      const map = new Map<string, BuiltinMetadataIndexEntry>();
      for (const entry of result.entries) {
        map.set(modelMetadataKey(entry.slug), {
          source: entry.source,
          context_window: entry.context_window,
          auto_compact_token_limit: entry.auto_compact_token_limit,
          supported_reasoning_levels: entry.supported_reasoning_levels,
        });
      }
      setBuiltinIndex(map);
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  // 导入区打开期间模型名被修改：标签与内置预填实时跟随新名字（覆盖
  // 「创建模型后改名」「编辑模型名」场景，而不是沿用旧名字的匹配结果）。
  // 面板身份只用 draft.index；行已删除时回退打开面板时的行名兜底。
  const activeImportSlug = activeImportDraft
    ? modelWindowRows[activeImportDraft.index]?.model.trim() ?? activeImportDraft.originalSlug
    : "";
  const metadataImportTarget = activeImportDraft;
  const metadataImportDocument = activeImportDraft?.document ?? "";
  const metadataImportPreview = activeImportDraft?.preview ?? null;
  const setMetadataImportDocument = (document: string) =>
    setActiveImportDraft((draft) => draft ? updateActiveImportDraft(draft, { document }) : draft);
  const setMetadataImportPreview = (preview: ImportedModelMetadata | null) =>
    setActiveImportDraft((draft) => draft ? updateActiveImportDraft(draft, { preview }) : draft);
  useEffect(() => {
    if (!metadataImportTarget || !activeImportSlug || activeImportSlug === builtinMatchSlug) return;
    let cancelled = false;
    void (async () => {
      let match: BuiltinModelMetadataMatch | null = null;
      try {
        const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug: activeImportSlug });
        // 迟到响应守卫：改名后旧查询一律丢弃（含错误态），否则旧 error 会把
        // 当前正确匹配的徽标误藏成回退/错误态（cancelled 只由 cleanup 置位，
        // cleanup 跑过必有新 run 接手，不存在「最后一次响应被误丢」）。
        if (cancelled) return;
        const state = builtinMetadataQueryState(result);
        setBuiltinQueryState(state);
        match = state.status === "error" ? null : state.value;
      } catch (error) {
        if (cancelled) return;
        const state = builtinMetadataQueryState(null, error);
        setBuiltinQueryState(state);
        setMetadataImportError(state.status === "error" ? state.error : "");
      }
      if (cancelled) return;
      setBuiltinMatch(match);
      setBuiltinMatchSlug(activeImportSlug);
      // 内置预填态（用户尚未编辑）跟随新名字重新预填；已编辑/自有内容不动。
      if (importPrefillSource === "builtin" && match?.matched && match.entry) {
        const document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(activeImportSlug));
        // 文档写的是后端返回的规范 slug，匹配时也要用规范 slug（剥掉 [1M] 后缀），
        // 否则带后缀的行名永远匹配不到，面板一打开就报「找不到 slug」。
        const preview = parseModelMetadataDocument(document, modelSlugFromRowName(activeImportSlug));
        setMetadataImportDocument(document);
        setMetadataImportError("");
        setMetadataImportPreview(preview.ok ? preview.value : null);
        if (preview.ok && preview.value.contextWindow && !suffixWindowString(activeImportSlug)) {
          updateModelWindowRow(metadataImportTarget.index, { window: preview.value.contextWindow });
        }
      }
    })();
    return () => { cancelled = true; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeImportSlug, builtinMatchSlug, metadataImportTarget, importPrefillSource]);

  const modelSlugOriginsRef = useRef(modelWindowRows.map((row) => row.model.trim()));
  useEffect(() => {
    modelSlugOriginsRef.current = modelWindowRows.map((row) => row.model.trim());
  }, [profile.id, profile.modelList]);
  const importedModelMetadata = useMemo(
    () => parseModelMetadataMap(profile.modelMetadata),
    [profile.modelMetadata],
  );
  // 当前内置匹配条目的「全字段」基线：用来判断面板里的内容是否只是内置的复刻。
  // 与预填走同一管道（同行名、同 slug 文本），并保留窗口/压缩字段——窗口/
  // 压缩偏离内置值同样是用户编辑，保存应落自定义；写 map 仍走白名单过滤。
  const builtinMetadata = useMemo(() => {
    if (!builtinMatch?.matched || !builtinMatch.entry) return null;
    const rowName = activeImportSlug || builtinMatchSlug || builtinMatch.entry.slug;
    const canonical = modelSlugFromRowName(rowName);
    const parsed = parseModelMetadataDocument(builtinEntryToImportDocument(builtinMatch.entry, canonical), canonical);
    return parsed.ok ? parsed.value.documentEntry : null;
  }, [builtinMatch, activeImportSlug, builtinMatchSlug]);
  // VLM/Strip 对 Chat Completions 与 Responses 协议均可用(注入块类型已按协议适配)。
  const vlmUnsupportedProtocol = false;
  if (isAggregateRelayProfile(profile)) {
    return (
      <AggregateRelayProfileEditor
        profile={profile}
        form={form}
        onProfileChange={onProfileChange}
      />
    );
  }

  const editorMode = relayEditorMode(profile);
  const showApiFields = editorMode !== "official";
  const sessionProvider = relaySessionProvider(profile);
  const canUseOpenAiSessionProvider = profile.relayMode !== "official" || profile.officialMixApiKey;
  const updateDraft = (patch: Partial<RelayProfile>) => {
    onProfileChange(applyRelayProfilePatchToFiles(profile, patch, { allowGenerateFiles: isNew }));
  };
  const normalizedApiKeys = normalizeRelayApiKeys(profile);
  const updateApiKeys = (apiKeys: RelayApiKey[], activeApiKeyId = normalizedApiKeys.activeApiKeyId) => {
    const selected = apiKeys.find((entry) => entry.id === activeApiKeyId) || apiKeys[0];
    updateDraft({ apiKeys, activeApiKeyId: selected?.id || "", apiKey: selected?.apiKey || "" });
  };
  const selectEditorMode = (mode: RelayEditorMode) => {
    if (mode === "official") {
      updateDraft({ relayMode: "official", officialMixApiKey: false });
      return;
    }
    if (mode === "mixedApi") {
      updateDraft({ relayMode: "official", officialMixApiKey: true });
      return;
    }
    updateDraft({ relayMode: "pureApi", officialMixApiKey: false });
  };
  const modelRoutes = normalizeRelayModelRoutes(profile.modelRoutes);
  const modelRouteTargets = form.relayProfiles.filter(
    (candidate) => candidate.id !== profile.id && !isAggregateRelayProfile(candidate) && candidate.protocol === "responses",
  );
  const updateModelRoute = (index: number, patch: Partial<RelayModelRoute>) => {
    updateDraft({
      modelRoutes: modelRoutes.map((route, routeIndex) => (routeIndex === index ? { ...route, ...patch } : route)),
    });
  };
  const commitModelMetadata = (modelMetadata: string) => {
    updateDraft({ modelMetadata });
  };
  const updateModelWindowRow = (index: number, patch: Partial<ModelWindowRow>) => {
    setModelWindowRows(
      modelWindowRows.map((row, rowIndex) => (rowIndex === index ? { ...row, ...patch } : row)),
    );
  };
  // 表头「全部设为」：十几行模型的压缩比例 / 图片处理通常一样，不必逐行改。
  const updateAllModelWindowRows = (patch: Partial<ModelWindowRow>) => {
    setModelWindowRows(modelWindowRows.map((row) => ({ ...row, ...patch })));
  };
  // 内置命中 → 回填空窗口/压缩列的统一入口：上游获取、手动提交行名、打开导入
  // 面板都走这一条规则，不再只有「重新匹配」才回填（裁决见 builtinRowBackfillValue）。
  const backfillRowFromBuiltin = (index: number, slug: string) => {
    const row = modelWindowRows[index];
    if (!row) return;
    const patch = builtinRowBackfillValue(slug, row.window, row.autoCompact, builtinIndex.get(modelMetadataKey(slug)));
    if (patch.window || patch.autoCompact) {
      updateModelWindowRow(index, {
        ...(patch.window ? { window: patch.window } : {}),
        ...(patch.autoCompact ? { autoCompact: patch.autoCompact } : {}),
      });
    }
  };
  const resolvePendingModelSlugRenames = (
    rows: ModelWindowRow[],
    origins: string[],
    modelMetadata: string,
  ) => {
    const slugs = rows.map((row) => row.model.trim()).filter(Boolean);
    if (new Set(slugs).size !== slugs.length) return { modelMetadata, origins };
    const nextOrigins = rows.map((row, index) => row.model.trim() || origins[index] || "");
    return {
      modelMetadata: remapModelMetadataSlugs(
        modelMetadata,
        rows.map((row, index) => ({
          previousSlug: origins[index] || "",
          nextSlug: row.model,
        })),
      ),
      origins: nextOrigins,
    };
  };
  const commitModelSlug = (index: number) => {
    const nextSlug = modelWindowRows[index]?.model.trim() ?? "";
    if (!nextSlug) return;
    // 面板身份只用 index（activeImportSlug 实时跟随输入框），改名只需在这里
    // 迁移 metadata map 的 key；draft 不再持有 slug 副本，避免双轨漂移。
    const resolved = resolvePendingModelSlugRenames(
      modelWindowRows,
      modelSlugOriginsRef.current,
      profile.modelMetadata,
    );
    modelSlugOriginsRef.current = resolved.origins;
    if (resolved.modelMetadata !== profile.modelMetadata) commitModelMetadata(resolved.modelMetadata);
    // 手动新增/改名提交即同步：内置命中且窗口/压缩列为空时立即回填
    backfillRowFromBuiltin(index, nextSlug);
  };
  const closeModelMetadataImport = () => {
    // 作废所有在飞的命令式内置查询（begin/rematch）：面板已关，迟到响应
    // 不得再把面板重新打开或写入陈旧匹配状态。
    builtinQuerySeqRef.current += 1;
    setActiveImportDraft(null);
    setMetadataImportError("");
  };
  const handleModelRowsDragEnd = (event: DragEndEvent) => {
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const activeIndex = modelWindowRows.findIndex((row) => row.sortId === active.id);
    const overIndex = modelWindowRows.findIndex((row) => row.sortId === over.id);
    if (activeIndex < 0 || overIndex < 0) return;
    const nextRows = reorderModelWindowRows(modelWindowRows, activeIndex, overIndex);
    if (nextRows === modelWindowRows) return;
    // 导入面板的身份按行索引保存；排序时关闭它，避免面板跟着错误的行移动。
    if (activeImportDraft) closeModelMetadataImport();
    const nextOrigins = [...modelSlugOriginsRef.current];
    const [movedOrigin] = nextOrigins.splice(activeIndex, 1);
    nextOrigins.splice(overIndex, 0, movedOrigin);
    modelSlugOriginsRef.current = nextOrigins;
    setModelWindowRows(nextRows);
  };
  const cancelModelMetadataImport = () => {
    if (activeImportDraft) {
      const cancelled = cancelActiveImportDraft(activeImportDraft);
      updateModelWindowRow(activeImportDraft.index, {
        window: cancelled.rowPatch.window,
        autoCompact: cancelled.rowPatch.autoCompact,
      });
    }
    closeModelMetadataImport();
  };
  const beginModelMetadataImport = async (index: number, slug: string) => {
    // 配置可能还挂在「改名尚未提交」的旧 key 下（resolvePendingModelSlugRenames
    // 的 previousSlug 同源），按行解析而不是按实时名硬查。
    const existingKey = resolveModelMetadataRowKey(importedModelMetadata, {
      current: slug,
      origin: modelSlugOriginsRef.current[index],
    });
    const existingMetadata = existingKey ? importedModelMetadata[existingKey] : undefined;
    const existingDocument = existingMetadata
      ? serializeModelMetadataDocument(
          // 文档 slug 必须与解析目标（剥后缀的行名）一致，否则带 [1M] 的行
          // 现有配置文档永远匹配不上，保存键被解析失败锁死。
          modelSlugFromRowName(slug),
          existingMetadata,
          modelWindowRows[index]?.window ?? "",
          modelWindowRows[index]?.autoCompact ?? "",
        )
      : "";
    // 无论有无已导入配置都查询内置匹配（标签需要准确的匹配状态）；
    // 无已导入配置且命中内置时，把内置条目预填为可编辑底稿（预填 ≠ 导入）。
    let match: BuiltinModelMetadataMatch | null = null;
    let metadataQueryError = "";
    if (slug.trim()) {
      const querySeq = ++builtinQuerySeqRef.current;
      try {
        const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug });
        // 代数守卫：await 期间面板被取消或被另一次 begin/rematch 抢占
        // （seq 已变）时，本次响应整体丢弃——迟到响应不得重开已关闭的面板。
        if (querySeq !== builtinQuerySeqRef.current) return;
        const state = builtinMetadataQueryState(result);
        setBuiltinQueryState(state);
        match = state.status === "error" ? null : state.value;
      } catch (error) {
        if (querySeq !== builtinQuerySeqRef.current) return;
        const state = builtinMetadataQueryState(null, error);
        setBuiltinQueryState(state);
        metadataQueryError = state.status === "error" ? state.error : "";
      }
    }
    let document = existingDocument;
    if (!document && match?.matched && match.entry) {
      document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(slug));
      setImportPrefillSource("builtin");
      // 打开面板即同步：内置条目的窗口/压缩回填空列（后缀与已填值不覆盖，
      // 与上游获取/手动提交同一规则）
      const backfill = builtinRowBackfillValue(
        slug,
        modelWindowRows[index]?.window ?? "",
        modelWindowRows[index]?.autoCompact ?? "",
        match.entry,
      );
      if (backfill.window || backfill.autoCompact) {
        updateModelWindowRow(index, {
          ...(backfill.window ? { window: backfill.window } : {}),
          ...(backfill.autoCompact ? { autoCompact: backfill.autoCompact } : {}),
        });
      }
    } else {
      setImportPrefillSource(existingMetadata ? "existing" : null);
    }
    setBuiltinMatch(match);
    setBuiltinMatchSlug(slug);
    const existingPreview = document
      ? parseModelMetadataDocument(document, modelSlugFromRowName(slug))
      : null;
    setActiveImportDraft(createActiveImportDraft({
      index,
      rowName: slug,
      window: modelWindowRows[index]?.window ?? "",
      autoCompact: modelWindowRows[index]?.autoCompact ?? "",
      document,
      preview: existingPreview?.ok ? existingPreview.value : null,
    }));
    setMetadataImportError(metadataQueryError);
  };
  const applyModelMetadataImport = () => {
    if (!metadataImportTarget) return;
    // 空文档是合法保存输入（清除 → 保存 = 放弃自定义回退内置）；解析失败才无内容可存。
    const documentBlank = !metadataImportDocument.trim();
    if (!metadataImportPreview && !documentBlank) return;
    // 保存跟当前行名（改名后保存写回新 slug，不存旧名）
    const slug = modelWindowRows[metadataImportTarget.index]?.model.trim()
      || metadataImportPreview?.slug || "";
    const key = modelMetadataKey(slug);
    const decision = importSaveDecision({
      parseOk: true,
      documentBlank,
      imported: Boolean(importedModelMetadata[key]),
      // 内容与内置全字段一致时目标态就是「用内置」，不写自定义覆盖——
      // 否则「重新匹配后保存」会把内置数据复制成一份自定义配置。
      matchesBuiltin: metadataImportPreview && !(importPrefillSource === "existing"
        && modelFastSupportMode(metadataImportPreview.metadata) !== "inherit")
        ? metadataMatchesBuiltin(metadataImportPreview.documentEntry, builtinMetadata)
        : false,
    });
    if (!decision.needsSave) {
      closeModelMetadataImport();
      return;
    }
    if (decision.effect === "builtin") {
      commitModelMetadata(clearModelMetadataForSlug(profile.modelMetadata, slug));
      closeModelMetadataImport();
      return;
    }
    if (!metadataImportPreview) {
      closeModelMetadataImport();
      return;
    }
    commitModelMetadata(replaceModelMetadataForSlug(
      profile.modelMetadata,
      slug,
      metadataImportPreview.metadata,
    ));
    // 保存的窗口/压缩写回与实时同步规则一致：解析有值才写，null 不写，
    // 不再 fallback 回导入前的旧值（保存把用户清空的窗口又写回旧值）。
    const row = modelWindowRows[metadataImportTarget.index];
    if (row) {
      const patch = importDocumentSyncPatch(row, metadataImportPreview);
      if (suffixWindowString(row.model)) delete patch.window;
      if (patch.window !== undefined || patch.autoCompact !== undefined) {
        updateModelWindowRow(metadataImportTarget.index, patch);
      }
    }
    closeModelMetadataImport();
  };
  // 「重新匹配」：按当前模型名重查内置元数据并重填下方内容（含实时写回行窗口）。
  const rematchBuiltinImport = async (slug: string) => {
    setImportPrefillSource("builtin");
    if (!slug.trim()) return;
    const querySeq = ++builtinQuerySeqRef.current;
    let match: BuiltinModelMetadataMatch | null = null;
    try {
      const result = await invoke<BuiltinModelMetadataMatch>("query_builtin_model_metadata", { slug });
      // 代数守卫：连点重新匹配或面板已关闭时，旧响应整体丢弃。
      if (querySeq !== builtinQuerySeqRef.current) return;
      const state = builtinMetadataQueryState(result);
      setBuiltinQueryState(state);
      match = state.status === "error" ? null : state.value;
    } catch (error) {
      if (querySeq !== builtinQuerySeqRef.current) return;
      const state = builtinMetadataQueryState(null, error);
      setBuiltinQueryState(state);
      setMetadataImportError(state.status === "error" ? state.error : "");
    }
    setBuiltinMatch(match);
    // 未命中也要同步 slug：否则跟随 effect 的「slug 未变」早退会挡住后续查询，
    // 用户把名字改对后按钮状态不更新。
    setBuiltinMatchSlug(slug);
    if (!match?.matched || !match.entry) return;
    // 重新匹配只替换当前 draft；窗口后缀是用户显式意图，不能被内置值覆盖。
    const document = builtinEntryToImportDocument(match.entry, modelSlugFromRowName(slug));
    const preview = parseModelMetadataDocument(document, modelSlugFromRowName(slug));
    if (activeImportDraft) {
      setActiveImportDraft(rematchActiveImportDraft(
        activeImportDraft, document, preview.ok ? preview.value : null,
      ));
    }
    setMetadataImportError("");
    if (metadataImportTarget && preview.ok && preview.value.contextWindow
      && !suffixWindowString(slug)) {
      updateModelWindowRow(metadataImportTarget.index, { window: preview.value.contextWindow });
    }
  };
  // 「清除」：只清空面板文档（draft 内容），面板保持打开——不摘已保存配置、
  // 不回滚窗口/压缩列、不关闭面板。回到内置走 清除 → 重新匹配 → 保存（内容=
  // 内置 → 目标态内置）；粘贴最新供应商元数据也以清空后的面板为起点。
  const clearImportDocument = () => {
    setActiveImportDraft((draft) => draft
      ? updateActiveImportDraft(draft, { document: "", preview: null })
      : draft);
    // 内容已被用户接管：后续改名不再自动回填内置预填
    setImportPrefillSource(null);
    setMetadataImportError("");
  };
  const removeModelWindowRow = (index: number) => {
    const removedSlug = modelWindowRows[index]?.model.trim() || modelSlugOriginsRef.current[index] || "";
    // 「配置模型」不能被删掉：保存时后端会把 profile.model 合并回 model_list
    // （relay_config::merge_model_into_model_list），删了也会被加回来、看起来像"保存没生效"。
    const configuredSlug = parseModelSuffix(profile.model || "").slug.trim();
    if (removedSlug && configuredSlug && removedSlug === configuredSlug) {
      void actions.showMessage(
        t("配置模型不能从列表里删除"),
        tf("「{0}」是当前「配置模型」：Codex 会始终把它发布在模型列表里，删掉保存后也会被加回来。要减少模型，请先把上方「配置模型」改成要保留的那个，再删这一行。", [removedSlug]),
      );
      return;
    }
    const nextRows = modelWindowRows.filter((_, rowIndex) => rowIndex !== index);
    const nextOrigins = modelSlugOriginsRef.current.filter((_, rowIndex) => rowIndex !== index);
    const resolved = resolvePendingModelSlugRenames(nextRows, nextOrigins, profile.modelMetadata);
    modelSlugOriginsRef.current = resolved.origins;
    // 删到没有模型就保持空列表，不再补一行空行。
    setModelWindowRows(nextRows);
    const slugStillPresent = nextRows.some((row) => row.model.trim() === removedSlug)
      || resolved.origins.includes(removedSlug);
    const nextMetadata = removedSlug && !slugStillPresent
      ? clearModelMetadataForSlug(resolved.modelMetadata, removedSlug)
      : resolved.modelMetadata;
    if (nextMetadata !== profile.modelMetadata) commitModelMetadata(nextMetadata);
    if (metadataImportTarget?.index === index) {
      closeModelMetadataImport();
    } else if (metadataImportTarget && metadataImportTarget.index > index) {
      setActiveImportDraft({ ...activeImportDraft, index: activeImportDraft.index - 1 });
    }
  };
  const addModelWindowRows = (rows: ModelWindowRow[]) => {
    const merged = mergeModelWindowRows(modelWindowRows, rows);
    modelSlugOriginsRef.current = merged.map((row) => {
      const currentIndex = modelWindowRows.findIndex((current) => current.model.trim() === row.model.trim());
      return currentIndex >= 0 ? modelSlugOriginsRef.current[currentIndex] || row.model.trim() : row.model.trim();
    });
    // 上游获取的行窗口/压缩列一律为空：内置命中时立即回填（同一回填裁决）
    setModelWindowRows(merged.map((row) => {
      const current = modelWindowRows.find((existing) => existing.model.trim() === row.model.trim());
      const sortableRow = withModelRowSortId(row, current?.sortId);
      if (row.window.trim() && row.autoCompact.trim()) return sortableRow;
      const entry = builtinIndex.get(modelMetadataKey(row.model));
      const patch = builtinRowBackfillValue(row.model, row.window, row.autoCompact, entry);
      return patch.window || patch.autoCompact
        ? {
            ...sortableRow,
            ...(patch.window ? { window: patch.window } : {}),
            ...(patch.autoCompact ? { autoCompact: patch.autoCompact } : {}),
          }
        : sortableRow;
    }));
  };
  const appendEmptyModelRow = () => {
    modelSlugOriginsRef.current = [...modelSlugOriginsRef.current, ""];
    setModelWindowRows([...modelWindowRows, withModelRowSortId({ model: "", window: "", autoCompact: "", imageHandling: "" })]);
  };
  const modelRowsError = modelWindowRowsValidationMessage(modelWindowRowsValidationError(modelWindowRows));
  const customHeadersError = relayHeadersValidationMessage(profile.customHeaders || []);
  const localizeMetadataSourceTag = (tag: ReturnType<typeof metadataSourceTags>[number]) => {
    if (tag.kind === "match") {
      return {
        text: tf("匹配：{0}", [tag.source]),
        title: tf("内置元数据：{0}", [tag.source]),
      };
    }
    if (tag.kind === "fallback") {
      return {
        text: tf("回退：{0}", [tag.source]),
        title: tf("无内置元数据，生成时回退 {0} 官方模板", [tag.source]),
      };
    }
    return {
      text: t("自定义"),
      title: tag.source
        ? tf("已导入自定义元数据，生成时覆盖内置（{0}）", [tag.source])
        : t("已导入自定义元数据，生成时以该配置为准"),
    };
  };
  return (
    <div className="relay-profile-editor">
      <div className="relay-fields">
        <section className="relay-config-section relay-basic-section">
          <div className="relay-config-section-head">
            <div>
              <strong>{t("基础设置")}</strong>
              <span>{t("先选择接入方式，相关字段会立即显示在这里。")}</span>
            </div>
          </div>
          <div className="relay-basic-fields">
            <Field className="relay-field-name" label={t("名称")}>
              <Input
                value={profile.name}
                onChange={(event) => updateDraft({ name: event.currentTarget.value })}
              />
            </Field>
          </div>
          <div className="relay-mode-block">
            <div className="relay-mode-block-head">
              <div>
                <strong>{t("接入方式")}</strong>
                <span>{t("官方登录 + API 会保留官方账号，同时使用下方 API 连接；不需要再到其他面板勾选。")}</span>
              </div>
            </div>
            <div className="relay-mode-options" role="radiogroup" aria-label={t("接入方式")}>
              {([
                {
                  mode: "official" as const,
                  icon: ShieldCheck,
                  title: t("官方登录"),
                  description: t("使用 ChatGPT 官方账号，不写入 API 凭据。"),
                },
                {
                  mode: "mixedApi" as const,
                  icon: Cable,
                  title: t("官方登录 + API"),
                  description: t("保留官方登录，同时使用 API Base URL 与 Key。"),
                },
                {
                  mode: "pureApi" as const,
                  icon: KeyRound,
                  title: t("纯 API"),
                  description: t("使用 API 凭据，原生登录账号独立选择。"),
                },
              ]).map(({ mode, icon: Icon, title, description }) => (
                <button
                  aria-checked={editorMode === mode}
                  className={`relay-mode-option ${editorMode === mode ? "active" : ""}`}
                  key={mode}
                  onClick={() => selectEditorMode(mode)}
                  role="radio"
                  type="button"
                >
                  <span className="relay-mode-option-icon"><Icon className="h-4 w-4" /></span>
                  <span className="relay-mode-option-copy">
                    <strong>{title}</strong>
                    <small>{description}</small>
                  </span>
                  <span className="relay-mode-option-indicator" aria-hidden="true" />
                </button>
              ))}
            </div>
            {editorMode !== "pureApi" ? (
              <SavedAuthAccountField onChange={(officialAccountRef) => updateDraft({ officialAccountRef })} value={profile.officialAccountRef ?? ""} />
            ) : null}
            {showApiFields ? (
              <div className="relay-inline-connection">
                <div className="relay-inline-connection-head">
                  <div>
                    <strong>{t("API 连接")}</strong>
                    <span>
                      {editorMode === "mixedApi"
                        ? t("官方账号负责登录与会话，API 连接负责实际请求。")
                        : t("保存后会把这组凭据用于当前供应商的请求。")}
                    </span>
                  </div>
                  <UiBadge variant={profile.baseUrl.trim() && profile.apiKey.trim() ? "secondary" : "outline"}>
                    {profile.baseUrl.trim() && profile.apiKey.trim() ? t("已填写") : t("待填写")}
                  </UiBadge>
                </div>
                <div className="relay-api-fields">
                  <div className="relay-api-connection-fields">
                    <Field className="relay-field-base-url" label="Base URL">
                      <Input
                        value={profile.baseUrl}
                        onChange={(event) => updateDraft({ baseUrl: event.currentTarget.value })}
                        placeholder={t("填写供应商的 Base URL")}
                      />
                    </Field>
                    <Field className="relay-field-protocol" label={t("上游协议")}>
                      <AppSelect
                        ariaLabel={t("上游协议")}
                        value={profile.protocol}
                        onChange={(protocol) => updateDraft({ protocol })}
                        options={[
                          { value: "responses", label: "Responses API" },
                          { value: "chatCompletions", label: "Chat Completions" },
                          { value: "anthropicMessages", label: "Anthropic Messages" },
                        ]}
                      />
                    </Field>
                  </div>
                  <Field className="relay-field-key" label="API Keys">
                    <div className="relay-api-key-list">
                      {normalizedApiKeys.apiKeys.map((entry) => {
                        const active = entry.id === normalizedApiKeys.activeApiKeyId;
                        return (
                          <div className={`relay-api-key-row ${active ? "active" : ""}`} key={entry.id}>
                            <button
                              aria-label={t("设为当前 Key")}
                              aria-pressed={active}
                              className="relay-api-key-select"
                              onClick={() => updateApiKeys(normalizedApiKeys.apiKeys, entry.id)}
                              title={t("设为当前 Key")}
                              type="button"
                            >
                              <CheckCircle2 className="h-4 w-4" />
                            </button>
                            <Input
                              aria-label={t("Key 名称")}
                              value={entry.name}
                              onChange={(event) => updateApiKeys(normalizedApiKeys.apiKeys.map((key) =>
                                key.id === entry.id ? { ...key, name: event.currentTarget.value } : key))}
                            />
                            <Input
                              aria-label={tf("{0} 的 API Key", [entry.name])}
                              type="password"
                              value={entry.apiKey}
                              onChange={(event) => updateApiKeys(normalizedApiKeys.apiKeys.map((key) =>
                                key.id === entry.id ? { ...key, apiKey: event.currentTarget.value } : key))}
                              placeholder={t("输入供应商的 API Key")}
                            />
                            {apiKeyPreview(entry.apiKey) ? (
                              <span className="relay-api-key-preview" title={t("Key 首尾预览")}>
                                {apiKeyPreview(entry.apiKey)}
                              </span>
                            ) : null}
                            <Button
                              aria-label={t("删除 Key")}
                              disabled={normalizedApiKeys.apiKeys.length === 1}
                              onClick={() => updateApiKeys(normalizedApiKeys.apiKeys.filter((key) => key.id !== entry.id))}
                              size="icon"
                              title={t("删除 Key")}
                              type="button"
                              variant="ghost"
                            >
                              <Trash2 className="h-4 w-4" />
                            </Button>
                          </div>
                        );
                      })}
                      <Button onClick={() => {
                        const key = createRelayApiKey(normalizedApiKeys.apiKeys);
                        updateApiKeys([...normalizedApiKeys.apiKeys, key]);
                      }} size="sm" type="button" variant="secondary">
                        <Plus className="h-4 w-4" />
                        {t("添加 Key")}
                      </Button>
                    </div>
                  </Field>
                  {profile.protocol === "anthropicMessages" ? (
                    <>
                      <Field className="relay-field-anthropic-auth" label={t("认证字段")}>
                        <AppSelect
                          ariaLabel={t("认证字段")}
                          value={profile.anthropicAuthHeader || "x-api-key"}
                          onChange={(value) => updateDraft({ anthropicAuthHeader: value })}
                          options={[
                            { value: "x-api-key", label: t("x-api-key（Anthropic 原生）") },
                            { value: "authorization", label: t("Authorization: Bearer") },
                          ]}
                        />
                      </Field>
                      <Field className="relay-field-anthropic-max-tokens" label={t("最大输出 tokens")}>
                        <Input
                          inputMode="numeric"
                          value={profile.anthropicMaxTokens ? String(profile.anthropicMaxTokens) : ""}
                          placeholder="8192"
                          onChange={(event) =>
                            updateDraft({ anthropicMaxTokens: normalizeAnthropicMaxTokens(Number(event.currentTarget.value)) })
                          }
                        />
                      </Field>
                      <Field className="relay-field-anthropic-thinking" label={t("思考模式")}>
                        <AppSelect
                          ariaLabel={t("思考模式")}
                          value={profile.anthropicThinkingMode || "auto"}
                          onChange={(value) =>
                            updateDraft({ anthropicThinkingMode: value === "auto" ? "" : value })
                          }
                          options={[
                            { value: "auto", label: t("自动（新代模型透传等级）") },
                            { value: "adaptive", label: t("等级透传（adaptive）") },
                            { value: "budget", label: t("经典预算（budget_tokens）") },
                            { value: "off", label: t("关闭思考") },
                          ]}
                        />
                      </Field>
                      <label className="switch-row compact relay-switch-row relay-field-anthropic-cache">
                        <input
                          checked={profile.anthropicPromptCache !== false}
                          onChange={(event) =>
                            updateDraft({ anthropicPromptCache: event.currentTarget.checked })
                          }
                          type="checkbox"
                        />
                        <span>
                          <strong>{t("提示缓存")}</strong>
                          <small>
                            {t("给系统提示、工具定义和最新一轮对话打 5 分钟缓存断点（最多 4 个），长对话不会每轮全价重发。")}
                          </small>
                        </span>
                        <ToggleVisual />
                      </label>
                      <label className="switch-row compact relay-switch-row relay-field-anthropic-impersonate">
                        <input
                          checked={profile.anthropicImpersonateClaudeCode === true}
                          onChange={(event) =>
                            updateDraft({ anthropicImpersonateClaudeCode: event.currentTarget.checked })
                          }
                          type="checkbox"
                        />
                        <span>
                          <strong>{t("模拟 Claude Code 客户端")}</strong>
                          <small>
                            {t("伪装 User-Agent、anthropic-beta、x-app，并在系统提示首行注入 Claude Code 身份。仅在网关限制「只能通过 Claude Code 使用」时开启。")}
                          </small>
                        </span>
                        <ToggleVisual />
                      </label>
                      <div className="hint-line relay-protocol-hint">
                        <MessageCircle className="h-4 w-4" />
                        <span>
                          {t("思考档位跟随 Codex 的 reasoning effort：自动模式下新代模型按等级透传（adaptive + effort），旧代模型折算成 thinking 预算；模型声明 1M 窗口时会自动附上 1m 上下文 beta 头。")}
                        </span>
                      </div>
                    </>
                  ) : null}
                </div>
              </div>
            ) : (
              <div className="relay-mode-note">
                <ShieldCheck className="h-4 w-4" />
                <span>{t("当前使用官方账号登录；切换到“官方登录 + API”或“纯 API”后，API 连接字段会在这里展开。")}</span>
              </div>
            )}
          </div>
        </section>
        <RelayFold
          className="relay-config-section relay-model-settings"
          sectionId="models"
          defaultOpen
          issue={modelRowsError || relayModelRoutesSettingsValidation(relaySettingsWithDraft(form, profile.id, profile, isNew))}
          title={t("模型配置")}
        >
          <div className="relay-options-grid">
            <Field className="relay-field-config-model" label={t("配置模型")}>
              <Input
                value={profile.model}
                onChange={(event) => updateDraft({ model: event.currentTarget.value })}
                placeholder={t("例如 deepseek-v4-pro")}
              />
              <p className="field-hint">
                {t("默认启动 Codex 时使用的模型名，请勿带后缀；上下文窗口请在下方「模型列表」中按模型单独配置。")}
              </p>
            </Field>
            {/* 官方登录 profile 同样维护模型列表：按模型聚合与官方模型目录拉取都依赖它。 */}
            <section className="relay-config-section relay-field-model-list">
                <div className="relay-config-section-head">
                  <div>
                    <strong>{t("模型列表")}</strong>
                    <span>
                      {t("每行一个模型；上下文窗口可填")} <code>1M</code>{t("、")}<code>200K</code> {t("或")} <code>1000000</code>{t("，留空表示使用 Codex 默认长度。")}
                    </span>
                  </div>
                  <div className="relay-model-list-tools">
                    <Button
                      onClick={appendEmptyModelRow}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <Plus className="h-4 w-4" />
                      {t("添加模型")}
                    </Button>
                    <Button
                      onClick={async () => {
                        const serializedRows = serializeModelWindowRows(modelWindowRows);
                        const models = await actions.fetchRelayProfileModels({
                          ...profile,
                          modelList: serializedRows.modelList,
                          modelWindows: serializedRows.modelWindows,
                          modelAutoCompact: serializedRows.modelAutoCompact,
                        });
                        if (models?.length) {
                          addModelWindowRows(models.map((model) => ({ model, window: "", autoCompact: "", imageHandling: "" })));
                        }
                      }}
                      size="sm"
                      type="button"
                      variant="secondary"
                    >
                      <Download className="h-4 w-4" />
                      {t("从上游获取")}
                    </Button>
                    <Button
                      disabled={!modelWindowRows.some((row) => row.model.trim())}
                      onClick={() => setModelWindowRows([])}
                      size="sm"
                      title={t("清空模型")}
                      type="button"
                      variant="outline"
                    >
                      <Trash2 className="h-4 w-4" />
                      {t("清空模型")}
                    </Button>
                  </div>
                </div>
                <div className="relay-model-row-editor">
                  {modelWindowRows.length ? <div className="relay-model-row relay-model-row-head">
                    <span aria-hidden="true" />
                    <span>{t("模型名称")}</span>
                    <span>{t("上下文窗口")}</span>
                    <span className="relay-model-head-bulk">
                      {t("自动压缩")}
                      <AppSelect
                        ariaLabel={t("全部模型的自动压缩")}
                        className="relay-model-bulk-select"
                        menuWidth={140}
                        onChange={(next) => {
                          if (next) updateAllModelWindowRows({ autoCompact: next });
                        }}
                        options={autoCompactOptions("").map((value) => ({ value, label: value }))}
                        placeholder={t("全部设为…")}
                        size="sm"
                        title={t("把所有模型设为同一个自动压缩比例")}
                        value=""
                      />
                    </span>
                    <span className="relay-model-head-bulk">
                      {t("图片处理方式")}
                      <AppSelect
                        ariaLabel={t("全部模型的图片处理方式")}
                        className="relay-model-bulk-select"
                        disabled={vlmUnsupportedProtocol}
                        menuWidth={160}
                        onChange={(next) => {
                          if (next) updateAllModelWindowRows({ imageHandling: next as ModelWindowRow["imageHandling"] });
                        }}
                        options={[
                          { value: "send-as-is", label: t("原样发送图片") },
                          { value: "strip", label: t("移除图片") },
                          { value: "vlm", label: t("视觉辅助分析") },
                        ]}
                        placeholder={t("全部设为…")}
                        size="sm"
                        title={t("把所有模型设为同一种图片处理方式")}
                        value=""
                      />
                    </span>
                    <span>{t("思考等级")}</span>
                    <span>{t("模型配置")}</span>
                    <span aria-hidden="true" />
                  </div> : null}
                  <DndContext
                    sensors={modelSensors}
                    collisionDetection={closestCenter}
                    onDragEnd={handleModelRowsDragEnd}
                  >
                    <SortableContext
                      items={modelWindowRows.map((row) => row.sortId)}
                      strategy={verticalListSortingStrategy}
                    >
                      {modelWindowRows.map((row, index) => {
                        const slug = row.model.trim();
                        // 面板身份只用 index：slug 双轨（draft 副本 vs 实时输入）曾导致
                        // 改名时面板整体卸载、blur 后重挂抢焦点。
                        const importing = metadataImportTarget?.index === index;
                        // 配置可能还挂在「改名尚未提交」的旧 key 下，按行解析而不是按实时名硬查。
                        const rowMetadataKey = resolveModelMetadataRowKey(importedModelMetadata, {
                          current: slug,
                          origin: modelSlugOriginsRef.current[index],
                        });
                        const imported = rowMetadataKey !== null;
                        // 思考等级列：自定义声明优先，其次内置初值；写回沿用到
                        // 改名前尚未提交的 key，blur 迁移时不会丢这次编辑。
                        const reasoning = resolveModelReasoningLevels(
                          rowMetadataKey ? importedModelMetadata[rowMetadataKey] : undefined,
                          builtinIndex.get(modelMetadataKey(slug))?.supported_reasoning_levels,
                        );
                        // 按钮可用性与状态行都从这一个纯函数出（见 model-metadata.ts）。
                        // 面板关闭时不创建导入控件，避免用一个面板级状态为所有行派生按钮状态。
                        const importControls = importing
                          ? importPanelControls({
                              slug,
                              document: metadataImportDocument,
                              imported,
                              // 空文档（清除后）不算解析失败：保存键保持可用，
                              // 走「放弃自定义回退内置」的保存路径。
                              parseOk: !metadataImportDocument.trim() || Boolean(metadataImportPreview),
                              matched: builtinQueryState?.status === "error" ? false : Boolean(builtinMatch?.matched),
                              // 面板内容与内置条目全字段等价：保存的目标态就是「用内置」。
                              matchesBuiltin: metadataImportPreview
                                ? metadataMatchesBuiltin(metadataImportPreview.documentEntry, builtinMetadata)
                                : false,
                            })
                          : null;

                        return (
                      <SortableModelWindowEntry id={row.sortId} key={row.sortId}>
                        {(sortable) => (
                          <>
                        <div className="relay-model-row">
                          <button
                            aria-label={t("拖动排序")}
                            className="relay-model-drag"
                            title={t("拖动排序")}
                            type="button"
                            {...sortable.attributes}
                            {...sortable.listeners}
                          >
                            <GripVertical className="h-4 w-4" />
                          </button>
                          <div className="relay-model-cell relay-model-cell-name">
                            <span className="relay-model-cell-label">{t("模型名称")}</span>
                            <Input
                              aria-label={t("模型名称")}
                              value={row.model}
                              onChange={(event) => updateModelWindowRow(index, { model: event.currentTarget.value })}
                              onBlur={() => commitModelSlug(index)}
                              placeholder="deepseek/deepseek-v4-flash"
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-window">
                            <span className="relay-model-cell-label">{t("上下文窗口")}</span>
                            <Input
                              aria-label={t("上下文窗口")}
                              value={row.window}
                              onChange={(event) => {
                                const window = event.currentTarget.value;
                                updateModelWindowRow(index, { window });
                                // 导入面板尚未粘贴 JSON 时，只编辑模型行；不要把空文档同步失败显示成错误。
                                if (!importing || !metadataImportDocument.trim() || !metadataImportPreview) return;
                                const synchronized = synchronizeModelMetadataDocumentLimitsPreview(
                                  metadataImportDocument,
                                  slug,
                                  window,
                                  metadataImportPreview?.autoCompactCalculationPercent
                                    ?? metadataImportPreview?.autoCompactPercent
                                    ?? row.autoCompact,
                                );
                                if (!synchronized) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t("上下文窗口与自动压缩值无效，无法同步模型配置。"));
                                  return;
                                }
                                setMetadataImportDocument(synchronized.document);
                                setMetadataImportPreview(synchronized.preview);
                                setMetadataImportError("");
                              }}
                              placeholder={t("留空使用默认值")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-compact">
                            <span className="relay-model-cell-label">{t("自动压缩")}</span>
                            <AppSelect
                              ariaLabel={t("自动压缩")}
                              className="text-xs"
                              value={normalizeAutoCompactPercent(row.autoCompact) || DEFAULT_AUTO_COMPACT_PERCENT}
                              onChange={(value) => {
                                updateModelWindowRow(index, { autoCompact: value });
                                // 导入面板尚未粘贴 JSON 时，只编辑模型行；不要把空文档同步失败显示成错误。
                                if (!importing || !metadataImportDocument.trim() || !metadataImportPreview) return;
                                const synchronized = synchronizeModelMetadataDocumentLimitsPreview(
                                  metadataImportDocument,
                                  slug,
                                  row.window,
                                  value,
                                );
                                if (!synchronized) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t("上下文窗口与自动压缩值无效，无法同步模型配置。"));
                                  return;
                                }
                                setMetadataImportDocument(synchronized.document);
                                setMetadataImportPreview(synchronized.preview);
                                setMetadataImportError("");
                              }}
                              options={autoCompactOptions(row.autoCompact).map((value) => ({ value, label: value }))}
                              title={t("自动压缩留空时沿用 Codex 默认行为；填写百分比后会按该模型的上下文窗口重新计算阈值。")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-image">
                            <span className="relay-model-cell-label">{t("图片处理方式")}</span>
                            <AppSelect
                              ariaLabel={t("图片处理方式")}
                              className="text-xs"
                              value={row.imageHandling}
                              disabled={vlmUnsupportedProtocol}
                              onChange={(value) => updateModelWindowRow(index, { imageHandling: value })}
                              options={[
                                { value: "", label: t("纯文本模型请配置此项"), disabled: true },
                                { value: "send-as-is", label: t("原样发送图片"), title: t("多模态模型直接接收图片,不经过任何处理") },
                                { value: "strip", label: t("移除图片"), title: t("删掉图片只发文字,避免纯文本模型报错(模型看不到图)") },
                                { value: "vlm", label: t("视觉辅助分析"), title: t("图片先由视觉辅助模型(Qwen)转成文字描述,纯文本模型也能\"看图\"") },
                              ]}
                              title={vlmUnsupportedProtocol ? t("VLM 仅支持 Chat Completions 协议和聚合模式") : t("多模态模型（支持图片输入的模型）请保持 send-as-is。")}
                            />
                          </div>
                          <div className="relay-model-cell relay-model-cell-reasoning">
                            <span className="relay-model-cell-label">{t("思考等级")}</span>
                            <ModelReasoningLevelsCell
                              levels={reasoning.levels}
                              parameterMode={reasoningParameterMode(rowMetadataKey ? importedModelMetadata[rowMetadataKey] : undefined)}
                              onParameterModeChange={(mode) => commitModelMetadata(setModelReasoningParameterMode(profile.modelMetadata, rowMetadataKey ?? slug, mode))}
                              inherited={!reasoning.overridden}
                              defaultLevel={reasoning.defaultLevel}
                              onChange={(levels, defaultLevel) => commitModelMetadata(
                                setModelReasoningLevelsForSlug(
                                  profile.modelMetadata,
                                  rowMetadataKey ?? slug,
                                  levels,
                                  defaultLevel,
                                ),
                              )}
                            />
                          </div>
                          <Button
                            className={`relay-model-import-button${imported ? " relay-model-import-custom" : builtinIndex.has(modelMetadataKey(slug)) ? " relay-model-import-builtin" : ""}`}
                            aria-expanded={importing}
                            disabled={!slug}
                            onClick={() => (importing ? cancelModelMetadataImport() : beginModelMetadataImport(index, slug))}
                            size="icon"
                            title={t("模型配置：元数据与 Fast 支持")}
                            type="button"
                            variant="ghost"
                          >
                            <FileCode2 className="h-4 w-4" />
                          </Button>
                          <Button
                            className="relay-model-delete"
                            aria-label={t("删除模型")}
                            onClick={() => removeModelWindowRow(index)}
                            size="icon"
                            title={t("删除模型")}
                            type="button"
                            variant="ghost"
                          >
                            <Trash2 className="h-4 w-4" />
                          </Button>
                        </div>
                        {importing ? (
                          <section className="relay-model-import-workbench">
                            <Field label={t("Fast（priority）支持")}>
                              <AppSelect
                                ariaLabel={t("Fast（priority）支持")}
                                value={importPrefillSource === "builtin" ? "inherit" : modelFastSupportMode(metadataImportPreview?.metadata)}
                                onChange={(value) => {
                                  const changed = synchronizeModelMetadataFastSupport(metadataImportDocument, slug, value as ModelFastSupportMode);
                                  if (!changed) {
                                    setMetadataImportError(t("请先修正当前模型的 JSON 配置，再设置 Fast 支持。"));
                                    return;
                                  }
                                  setMetadataImportDocument(changed.document);
                                  setMetadataImportPreview(changed.preview);
                                  setImportPrefillSource("existing");
                                  setMetadataImportError("");
                                }}
                                options={[
                                  { value: "inherit", label: t("继承模型默认能力") },
                                  { value: "supported", label: t("明确支持 priority") },
                                  { value: "unsupported", label: t("不支持 priority") },
                                ]}
                              />
                              <p className="field-hint">{t("按当前供应商逐模型声明。仅在上游支持 service_tier=priority 时选择支持；继承会清除自定义声明，取消不会保存修改。")}</p>
                            </Field>
                            <div className="form-row">
                              <Field label={t("Responses Lite 支持")}>
                                <AppSelect value={metadataImportPreview?.metadata.use_responses_lite === true ? "supported" : metadataImportPreview?.metadata.use_responses_lite === false ? "unsupported" : "inherit"}
                                  options={[{ value: "inherit", label: t("继承模型默认能力") }, { value: "supported", label: t("明确支持 Lite") }, { value: "unsupported", label: t("标准 Responses") }]}
                                  onChange={(value) => {
                                    const changed = synchronizeModelMetadataLiteSupport(metadataImportDocument, slug, value as "inherit" | "supported" | "unsupported");
                                    if (!changed) { setMetadataImportError(t("请先修正当前模型的 JSON 配置。")); return; }
                                    setMetadataImportDocument(changed.document); setMetadataImportPreview(changed.preview); setImportPrefillSource("existing"); setMetadataImportError("");
                                  }} />
                                <p className="field-hint">{t("只为已核实支持 Lite 的上游选择支持。")}</p>
                              </Field>
                              <Field label={t("Multi Agent 协议")}>
                                <AppSelect value={modelMultiAgentMode(metadataImportPreview?.metadata)}
                                  options={[
                                    { value: "inherit", label: t("跟随供应商设置") },
                                    { value: "v1", label: t("v1（经典）") },
                                    { value: "v2", label: t("v2（新版）") },
                                    { value: "disabled", label: t("关闭子代理") },
                                  ]}
                                  onChange={(value) => {
                                    const changed = synchronizeModelMetadataMultiAgentVersion(metadataImportDocument, slug, value as ModelMultiAgentMode);
                                    if (!changed) { setMetadataImportError(t("请先修正当前模型的 JSON 配置。")); return; }
                                    setMetadataImportDocument(changed.document); setMetadataImportPreview(changed.preview); setImportPrefillSource("existing"); setMetadataImportError("");
                                  }} />
                                <p className="field-hint">{t("v2 需上游支持加密子代理载荷，转 Chat 协议的中转请选 v1；仅新会话生效。")}</p>
                              </Field>
                            </div>
                            <Textarea
                              autoFocus
                              value={metadataImportDocument}
                              onChange={(event) => {
                                const document = event.currentTarget.value;
                                setImportPrefillSource("existing");
                                setMetadataImportDocument(document);
                                setMetadataImportError("");
                                const parsed = parseModelMetadataDocument(document, slug);
                                if (!parsed.ok) {
                                  setMetadataImportPreview(null);
                                  setMetadataImportError(t(parsed.error));
                                  return;
                                }
                                setMetadataImportPreview(parsed.value);
                                // 实时替换：JSON 里的窗口/压缩比即时写回模型行输入框（所见即所得）。
                                if (metadataImportTarget) {
                                  const row = modelWindowRows[metadataImportTarget.index];
                                  if (row) {
                                    const patch = importDocumentSyncPatch(row, parsed.value);
                                    if (suffixWindowString(row.model)) delete patch.window;
                                    if (patch.window !== undefined || patch.autoCompact !== undefined) {
                                      updateModelWindowRow(metadataImportTarget.index, patch);
                                    }
                                  }
                                }
                              }}
                              placeholder={t("需要补充供应商模型信息时填写；不填则使用 Codex++ 默认配置（自动压缩 90%、图片原样发送）。从供应商的 models.json 或 model.json 复制，支持多个模型。")}
                              rows={7}
                            />
                            {metadataImportError ? <div className="relay-model-metadata-import-error" role="alert">{metadataImportError}</div> : null}
                            {metadataImportPreview?.contextWindow && row.window.trim() && metadataImportPreview.contextWindow !== row.window.trim() ? (
                              <div className="relay-model-metadata-import-warning" role="status">
                                {tf("文档里的上下文窗口是 {0}，当前列填的是 {1}，已保留当前值；要用文档的值请先清空窗口。", [metadataImportPreview.contextWindow, row.window.trim()])}
                              </div>
                            ) : null}
                            {metadataImportPreview?.ignoredFields.length ? (
                              <div className="relay-model-metadata-import-warning" role="status">
                                {tf("以下字段由 Codex++ 计算或维护，导入不会覆盖：{0}", [metadataImportPreview.ignoredFields.join(", ")])}
                              </div>
                            ) : null}
                            <div className="relay-model-metadata-import-actions">
                              <div className="relay-model-import-copy">
                                <strong>{slug}</strong>
                                {metadataSourceTags({
                                  slug,
                                  imported,
                                  builtinMatch: builtinQueryState?.status === "error" ? null : builtinMatch,
                                  builtinIndexSlug: builtinIndex.get(modelMetadataKey(slug)),
                                  // 回退模板名从后端 fallback 字段实时取，不写死
                                  fallbackSlug: builtinMatch?.fallback?.slug,
                                }).map((tag) => (
                                  (() => {
                                    const localized = localizeMetadataSourceTag(tag);
                                    return (
                                      <span
                                        key={tag.kind}
                                        className={`relay-model-source-badge relay-model-source-${tag.tone}`}
                                        title={localized.title}
                                      >
                                        {localized.text}
                                      </span>
                                    );
                                  })()
                                ))}
                              </div>
                              <div className="relay-model-metadata-import-flow">
                                {/* 四个按钮恒定渲染：只用置灰表达可用性，不再随状态出现/消失 */}
                                <Button
                                  disabled={importControls?.rematch.disabled ?? true}
                                  onClick={() => void rematchBuiltinImport(slug)}
                                  size="sm"
                                  title={t(importControls?.rematch.title ?? "重新匹配")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("重新匹配")}
                                </Button>
                                <Button
                                  disabled={importControls?.clear.disabled ?? true}
                                  onClick={() => clearImportDocument()}
                                  size="sm"
                                  title={t(importControls?.clear.title ?? "清除")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("清除")}
                                </Button>
                                <Button
                                  disabled={importControls?.cancel.disabled ?? true}
                                  onClick={cancelModelMetadataImport}
                                  size="sm"
                                  title={t(importControls?.cancel.title ?? "取消")}
                                  type="button"
                                  variant="ghost"
                                >
                                  {t("取消")}
                                </Button>
                                <Button
                                  disabled={importControls?.save.disabled ?? true}
                                  onClick={applyModelMetadataImport}
                                  size="sm"
                                  type="button"
                                  title={t(importControls?.save.title ?? "保存此模型")}
                                >
                                  {t(importControls?.save.label ?? "保存此模型")}
                                </Button>
                              </div>
                            </div>
                          </section>
                        ) : null}
                          </>
                        )}
                      </SortableModelWindowEntry>
                        );
                      })}
                    </SortableContext>
                  </DndContext>
                </div>
                {modelRowsError ? <div className="relay-model-metadata-import-error" role="alert">{modelRowsError}</div> : null}
                <p className="field-hint">
                  {t("自动压缩留空时沿用 Codex 默认行为；填写百分比后会按该模型的上下文窗口重新计算阈值。")}
                  {" "}{t("需要自定义 Fast 白名单时，打开对应模型的配置按钮，设置 Fast（priority）支持并保存此模型与供应商。")}
                </p>
            </section>
            {showApiFields ? (
              <section className="relay-config-section relay-field-model-routes">
                <div className="relay-config-section-head">
                  <div>
                    <strong>{t("单模型协议转换")}</strong>
                    <span>{t("仅在当前供应商启用时生效；精确匹配模型名并使用目标供应商的 URL 与 Key。目标必须是 Responses API，且需要从 Codex++ 启动。")}</span>
                  </div>
                  <div className="relay-model-list-tools">
                    <Button
                      disabled={modelRouteTargets.length === 0}
                      onClick={() => updateDraft({ modelRoutes: [...modelRoutes, { model: "", targetRelayId: "", targetModel: "" }] })}
                      size="sm"
                      title={modelRouteTargets.length === 0 ? t("请先创建一个 Responses API 目标供应商") : t("添加协议转换")}
                      type="button"
                      variant="secondary"
                    >
                      <Plus className="h-4 w-4" />
                      {t("添加协议转换")}
                    </Button>
                  </div>
                </div>
                <div className="relay-model-route-editor">
                  {modelRoutes.length ? (
                    <div className="relay-model-route-row relay-model-route-head">
                      <span>{t("匹配模型")}</span>
                      <span>{t("目标供应商")}</span>
                      <span>{t("目标模型（可选）")}</span>
                    </div>
                  ) : null}
                  {modelRoutes.map((route, index) => (
                    <div className="relay-model-route-row" key={`model-route-${index}`}>
                      <Input
                        value={route.model}
                        onChange={(event) => updateModelRoute(index, { model: event.currentTarget.value })}
                        placeholder={t("例：gpt-5.6-luna")}
                      />
                      <AppSelect
                        value={route.targetRelayId}
                        onChange={(targetRelayId) => updateModelRoute(index, { targetRelayId })}
                        options={[
                          { value: "", label: t("选择 Responses 供应商"), disabled: true },
                          ...modelRouteTargets.map((candidate) => ({ value: candidate.id, label: candidate.name || candidate.id })),
                        ]}
                      />
                      <Input
                        value={route.targetModel}
                        onChange={(event) => updateModelRoute(index, { targetModel: event.currentTarget.value })}
                        placeholder={t("留空保持原模型名")}
                      />
                      <Button
                        aria-label={t("删除协议转换")}
                        onClick={() => updateDraft({ modelRoutes: modelRoutes.filter((_, routeIndex) => routeIndex !== index) })}
                        size="icon"
                        title={t("删除协议转换")}
                        type="button"
                        variant="ghost"
                      >
                        <Trash2 className="h-4 w-4" />
                      </Button>
                    </div>
                  ))}
                </div>
              </section>
            ) : null}
            {showApiFields && modelWindowRows.some((row) => row.imageHandling === "vlm") ? (
              <div className="relay-vlm-section">
                <div className="relay-vlm-section-header">{t("Vision Analysis Provider")}</div>
                <Field className="relay-field-vlm-api-key" label={t("VLM API Key")}>
                  <Input
                    type="password"
                    value={profile.vlmApiKey}
                    onChange={(event) => updateDraft({ vlmApiKey: event.currentTarget.value })}
                    placeholder="sk-..."
                  />
                </Field>
                <Field className="relay-field-vlm-model" label={t("VLM Model")}>
                  <Input
                    value={profile.vlmModel}
                    onChange={(event) => updateDraft({ vlmModel: event.currentTarget.value })}
                    placeholder="qwen-vl-plus"
                  />
                </Field>
                <Field className="relay-field-vlm-base-url" label={t("VLM Base URL")}>
                  <Input
                    value={profile.vlmBaseUrl}
                    onChange={(event) => updateDraft({ vlmBaseUrl: event.currentTarget.value })}
                    placeholder="https://dashscope.aliyuncs.com/compatible-mode/v1"
                  />
                </Field>
                <p className="field-hint">
                  {t("若开启 VLM analysis，请确认 VLM 配置项完整且服务可用。")}
                  <br />
                  {t("仅在 Chat Completion 和聚合模式生效。")}
                </p>
                {modelWindowRows.some((row) => row.imageHandling === "vlm") && (!profile.vlmApiKey || !profile.vlmModel || !profile.vlmBaseUrl) ? (
                  <p className="field-hint warn">{t("VLM 配置不完整：API Key、Model 和 Base URL 为必填项，否则 VLM 不会生效。")}</p>
                ) : null}
                <div className="vlm-test-entry">
                  <Button
                    onClick={() => setVlmTestOpen((v) => !v)}
                    size="sm"
                    type="button"
                    variant="secondary"
                  >
                    {vlmTestOpen ? t("收起测试面板") : t("测试 VLM")}
                  </Button>
                </div>
                {vlmTestOpen ? <VlmTestPanel profile={profile} onClose={() => setVlmTestOpen(false)} /> : null}
              </div>
            ) : null}
          </div>
        </RelayFold>
        {showApiFields ? (
          <RelayFold
            className="relay-config-section relay-request-settings"
            defaultOpen
            sectionId="requests"
            issue={customHeadersError || relaySessionProviderValidation(profile)}
            title={t("请求设置")}
          >
          <div className="relay-options-grid">
            {showApiFields ? (
              <Field className="relay-field-session-provider" label={t("Codex 会话身份")}>
                <AppSelect
                  value={sessionProvider}
                  onChange={(value) => updateDraft({ sessionProvider: value })}
                  options={[
                    { value: "custom", label: t("Custom（默认）") },
                    {
                      value: "openai",
                      label: t("OpenAI（兼容 ChatGPT Remote）"),
                      disabled: !canUseOpenAiSessionProvider || profile.protocol !== "responses",
                    },
                  ]}
                />
                <p className="field-hint">
                  {profile.protocol !== "responses"
                    ? t("OpenAI 会话身份需要 Responses API；Chat Completions 不支持远程压缩。")
                    : canUseOpenAiSessionProvider
                      ? t("选择 OpenAI 后，Codex Remote 会把当前会话识别为 ChatGPT 会话；中转仍使用 custom 表。")
                      : t("官方登录未混入 API 时不写入会话 provider")}
                </p>
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-user-agent" label="User-Agent">
                <Input
                  value={profile.userAgent}
                  onChange={(event) => updateDraft({ userAgent: event.currentTarget.value })}
                  placeholder={t("留空使用默认值")}
                />
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-auto-review-model" label={t("自动审批模型")}>
                <Input
                  value={profile.autoReviewModel ?? ""}
                  onChange={(event) => updateDraft({ autoReviewModel: event.currentTarget.value })}
                  placeholder={t("留空则原样转发 codex-auto-review")}
                />
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-multi-agent" label={t("Multi Agent 协议")}>
                <AppSelect
                  value={profile.multiAgent === "v1" || profile.multiAgent === "v2" ? profile.multiAgent : "auto"}
                  options={[
                    { value: "auto", label: t("自动（推荐）") },
                    { value: "v1", label: t("v1（经典，转 Chat 中转适用）") },
                    { value: "v2", label: t("v2（新版，需上游支持加密载荷）") },
                  ]}
                  onChange={(value) => updateDraft({ multiAgent: value === "v1" || value === "v2" ? value : "" })}
                />
                <p className="field-hint">
                  {t("子代理协议的供应商级默认；模型元数据里的 multi_agent_version 声明优先于它。选 v1 会在使用该供应商期间压下 [features] multi_agent_v2，切走时自动还原。仅对直连供应商生效，Codex 经本地服务时由本地服务的接入设置决定。仅新会话生效。")}
                </p>
              </Field>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-proxy" label={t("供应商独立代理")}>
                <Input
                  aria-invalid={proxy.error ? true : undefined}
                  value={proxy.value}
                  onChange={(event) => proxy.onChange(event.currentTarget.value)}
                  placeholder="http://127.0.0.1:7890"
                />
                <span className="hint-line">{t("留空表示使用系统代理；仅影响当前供应商的模型拉取、测试和请求。随「保存供应商」一起保存。")}</span>
                {proxy.error ? <span className="hint-line relay-custom-headers-error">{proxy.error}</span> : null}
              </Field>
            ) : null}
            {showApiFields ? (
              // 五个兼容性开关默认都关，且只在上游明确需要时才开：收成一组，摘要上写开了几项。
              <details
                className="relay-compat-group"
                open={compatOpen ?? compatEnabledCount > 0}
                onToggle={(event) => setCompatOpen(event.currentTarget.open)}
              >
                <summary>
                  <ChevronDown aria-hidden="true" className="relay-fold-chevron h-4 w-4" />
                  <strong>{t("高级兼容选项")}</strong>
                  <small>{compatEnabledCount ? tf("已开启 {0} 项", [compatEnabledCount]) : t("全部关闭；只在上游明确需要时开启")}</small>
                </summary>
                <div className="relay-compat-body">
                {showApiFields ? (
                  <label
                    className={`switch-row compact relay-switch-row relay-field-standard${profile.protocol === "chatCompletions" ? "" : " is-disabled"}`}
                    title={profile.protocol === "chatCompletions" ? undefined : t("仅在上游协议为 Chat Completions 时可用。Responses API 会原样转发。")}
                  >
                    <input
                      checked={profile.standardOpenaiProtocol}
                      disabled={profile.protocol !== "chatCompletions"}
                      onChange={(event) =>
                        updateDraft({ standardOpenaiProtocol: event.currentTarget.checked })
                      }
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("纯标准协议")}</strong>
                      <small>
                        {t("强制走标准 OpenAI 协议，不注入厂商私有 reasoning 参数。面向只认标准 OpenAI 字段、拒绝厂商私有参数的第三方网关。")}
                        {" "}
                        {t("若网关提示 thinking type: adaptive 无效，可启用此项后重试；这会停用厂商私有推理参数。")}
                      </small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields ? (
                  <label className="switch-row compact relay-switch-row">
                    <input
                      checked={promptPrefix.value}
                      onChange={(event) => promptPrefix.onChange(event.currentTarget.checked)}
                      type="checkbox"
                    />
                    <span><strong>{t("前缀稳定层")}</strong><small>{t("按供应商稳定可选历史块的顺序并移除成对 image_resize_notice；默认关闭，适合已确认上游支持的缓存场景。")}</small></span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields ? (
                  <label
                    className={`switch-row compact relay-switch-row relay-field-search-history${profile.protocol === "responses" ? "" : " is-disabled"}`}
                    title={profile.protocol === "responses" ? undefined : t("仅限 Responses API。")}
                  >
                    <input
                      checked={profile.webSearchHistoryCompat}
                      disabled={profile.protocol !== "responses"}
                      onChange={(event) =>
                        updateDraft({ webSearchHistoryCompat: event.currentTarget.checked })
                      }
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("搜索历史压缩兼容（实验）")}</strong>
                      <small>response protection is unavailable</small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields && profile.relayMode !== "aggregate" && profile.protocol !== "anthropicMessages" ? (
                  <label className="switch-row compact relay-switch-row relay-field-local-compaction">
                    <input
                      checked={profile.localCompactionPromptAtEnd === true}
                      onChange={(event) => updateDraft({ localCompactionPromptAtEnd: event.currentTarget.checked })}
                      type="checkbox"
                    />
                    <span>
                      <strong>{t("本地压缩缓存复用（实验）")}</strong>
                      <small>{t("仅支持直连供应商，暂不支持经本地服务。实测可改善压缩请求的缓存命中概率，但不保证每次命中，也不保证后置压缩提示不影响摘要质量。缓存按 Key 隔离，切换或轮换 Key 可能降低命中率。未命中供应商缓存时，补回的工具声明会增加输入 token 开销；上游若忽略禁用工具调用要求，可能返回空摘要。默认关闭。")}</small>
                    </span>
                    <ToggleVisual />
                  </label>
                ) : null}
                {showApiFields && profile.protocol === "responses" ? <>
                  <label className="switch-row compact relay-switch-row">
                    <input type="checkbox" checked={profile.trustUpstreamCatalogLite === true} onChange={(event) => updateDraft({ trustUpstreamCatalogLite: event.currentTarget.checked })} />
                    <span><strong>{t("保留外部目录的 Lite 声明")}</strong><small>{t("仅在已确认上游支持时启用；保留每个模型的显式声明，不会为全部模型开启 Lite。")}</small></span><ToggleVisual />
                  </label>
                  <label className="switch-row compact relay-switch-row">
                    <input type="checkbox" checked={profile.strictResponsesReasoning === true} onChange={(event) => updateDraft({ strictResponsesReasoning: event.currentTarget.checked })} />
                    <span><strong>{t("严格 Responses 推理历史兼容")}</strong><small>{t("仅当目标上游接受当前账号的加密历史时启用。混合推理只发送可识别的加密信封；占位或无效密文会明确报错。本地记录不变。")}</small></span><ToggleVisual />
                  </label>
                  <CompatToggles onChange={(patch) => updateDraft(patch)} values={profile} variant="direct" />
                </> : null}
                {showApiFields && profile.relayMode === "pureApi" ? (
                  <label className="switch-row compact relay-switch-row relay-field-pure-api-image-generation">
                    <input type="checkbox" checked={profile.pureApiImageGeneration === true} onChange={(event) => updateDraft({ pureApiImageGeneration: event.currentTarget.checked })} />
                    <span><strong>{t("纯 API 启用 Codex 原生生图（实验）")}</strong><small>{t("Codex 默认只在登录 ChatGPT 时注册 image_gen 工具。开启后 Codex++ 在 config.toml 里写入占位请求头让 Codex 注册该工具，生图请求经本地代理发给本供应商的 /images/generations、/images/edits，占位头不会发往上游。上游需支持 OpenAI 图片接口；依赖 Codex 内部判据，Codex 更新后可能失效。默认关闭。")}</small></span><ToggleVisual />
                  </label>
                ) : null}
                </div>
              </details>
            ) : null}
            {showApiFields ? (
              <Field className="relay-field-custom-headers" label={t("自定义请求头")}>
                <div className="relay-custom-headers">
                  <RelayHeaderRows onChange={(customHeaders) => updateDraft({ customHeaders })} rows={profile.customHeaders || []} />
                  <span className="hint-line">
                    {t("自定义请求头会同时用于测试连接、模型列表与实际代理请求。")}
                  </span>
                  <span className="hint-line">
                    {t("Host、Content-Length 等传输头由协议层掌控，不能覆盖；配置 Authorization 时以它为准，不再注入 API Key。")}
                  </span>
                  {customHeadersError ? (
                    <span className="hint-line relay-custom-headers-error">{customHeadersError}</span>
                  ) : null}
                </div>
              </Field>
            ) : null}
            {showApiFields && profile.protocol !== "responses" ? (
              <div className="hint-line relay-protocol-hint">
                <MessageCircle className="h-4 w-4" />
                <span>
                  {profile.protocol === "anthropicMessages"
                    ? t("此上游说 Anthropic Messages（/v1/messages）：本地 127.0.0.1:57321 会把请求转成 Anthropic 协议发出，再转回 Responses，需要从 Codex++ 启动 Codex。")
                    : t("此上游会通过本地 127.0.0.1:57321 转成 Responses API，需要从 Codex++ 启动 Codex。")}
                </span>
              </div>
            ) : null}
            <div className="hint-line relay-protocol-hint">
              <ShieldCheck className="h-4 w-4" />
              <span>{relayProfileModeHelp(profile)}</span>
            </div>
          </div>
          </RelayFold>
        ) : null}
      </div>
      <div className="relay-bottom-options">
        <RelayFold className="relay-advanced-block" defaultOpen sectionId="advanced" title={t("更多选项")}>
          <p className="relay-fold-description">
            {t("包含上下文大小与压缩阈值；留空即沿用全局默认值。")}
          </p>
          <div className="relay-advanced-fields">
            <Field className="relay-field-context-window" label={t("上下文大小")}>
              <Input
                inputMode="numeric"
                value={profile.contextWindow}
                onChange={(event) => updateDraft({ contextWindow: normalizeTokenCountInput(event.currentTarget.value) })}
                placeholder={t("留空不改写，例如 200000")}
              />
            </Field>
            <Field className="relay-field-auto-compact" label={t("压缩上下文大小")}>
              <Input
                inputMode="numeric"
                value={profile.autoCompactLimit}
                onChange={(event) => updateDraft({ autoCompactLimit: normalizeTokenCountInput(event.currentTarget.value) })}
                placeholder={t("留空不改写，例如 160000")}
              />
            </Field>
          </div>
        </RelayFold>
      </div>
    </div>
  );
}

type VlmTestState =
  | { kind: "idle" }
  | { kind: "running" }
  | { kind: "done"; result: TestVlmResult };

type TestVlmResult = {
  status: string;
  message: string;
  vlmStatus: string;
  httpCode: number | null;
  durationMs: number;
  error: string | null;
  description: string | null;
  model: string;
  rawRequest: string | null;
  rawResponse: string | null;
};

const VLM_TEST_MAX_IMAGE_BYTES = 10 * 1024 * 1024;

/// 方向 C：选图即测 + 排障增强（spec §4）。
/// 选完文件自动开跑；失败时给通俗诊断 + 复制错误 + 原始请求/响应折叠。
function VlmTestPanel({
  profile,
  onClose,
}: {
  profile: Pick<RelayProfile, "vlmApiKey" | "vlmModel" | "vlmBaseUrl">;
  onClose: () => void;
}) {
  const [dataUrl, setDataUrl] = useState<string | null>(null);
  const [state, setState] = useState<VlmTestState>({ kind: "idle" });
  const [showRaw, setShowRaw] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const tr = (zh: string, params?: string[]) => (params ? tf(zh, params) : t(zh));

  const runTest = async (url: string) => {
    setState({ kind: "running" });
    setLocalError(null);
    try {
      const res = await invoke<TestVlmResult>("test_vlm", {
        request: {
          apiKey: profile.vlmApiKey,
          model: profile.vlmModel,
          baseUrl: profile.vlmBaseUrl,
          imageDataUrl: url,
        },
      });
      setState({ kind: "done", result: res });
    } catch (error) {
      const msg = error instanceof Error ? error.message : String(error);
      setState({
        kind: "done",
        result: {
          status: "failed",
          message: msg,
          vlmStatus: "client_error",
          httpCode: null,
          durationMs: 0,
          error: msg,
          description: null,
          model: profile.vlmModel,
          rawRequest: null,
          rawResponse: null,
        },
      });
    }
  };

  // 选图即测：选完文件自动发起，无需二次点击（spec §4）。
  const onFile = (file: File | undefined) => {
    if (!file) return;
    if (!file.type.startsWith("image/")) {
      setLocalError(t("请选择图片文件。"));
      return;
    }
    if (file.size > VLM_TEST_MAX_IMAGE_BYTES) {
      setLocalError(t("图片超过 10MB，请换一张较小的图片。"));
      return;
    }
    const reader = new FileReader();
    reader.onerror = () => setLocalError(t("读取图片失败，请重新选择"));
    reader.onload = () => {
      const url = typeof reader.result === "string" ? reader.result : null;
      if (url) {
        setDataUrl(url);
        void runTest(url);
      }
    };
    reader.readAsDataURL(file);
  };

  // 复制完整排障信息（诊断 + 原始报文，无 API Key），可直接贴 issue（spec §3）。
  const copyError = async () => {
    if (state.kind !== "done") return;
    const r = state.result;
    const text = [
      vlmTestTranslation(r.vlmStatus, r.httpCode ?? undefined, r.durationMs, tr),
      `model: ${r.model}`,
      r.httpCode != null ? `HTTP ${r.httpCode}` : null,
      r.error ? `error: ${r.error}` : null,
      r.rawRequest ? `--- request ---\n${r.rawRequest}` : null,
      r.rawResponse ? `--- response ---\n${r.rawResponse}` : null,
    ]
      .filter((x) => x !== null)
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      setLocalError(t("复制失败，请从报文中手动复制。"));
    }
  };

  const done = state.kind === "done" ? state.result : null;
  const running = state.kind === "running";
  const formReady = !!profile.vlmApiKey && !!profile.vlmModel && !!profile.vlmBaseUrl;
  const canRun = !!dataUrl && formReady;

  return (
    <div className="vlm-test-panel">
      <div className="modal-head">
        <div>
          <h2>{t("测试 VLM")}</h2>
          <p className="modal-message">
            {t("选一张图片立即验证当前 VLM 配置（使用表单当前值，无需保存）。")}
          </p>
        </div>
        <button className="toast-close" aria-label={t("关闭窗口")} onClick={onClose} type="button">×</button>
      </div>

      <div className="vlm-test-upload">
        <input
          ref={fileInputRef}
          accept="image/*"
          onChange={(e) => {
            onFile(e.currentTarget.files?.[0]);
            e.currentTarget.value = "";
          }}
          type="file"
          style={{ display: "none" }}
        />
        <Button disabled={running || !formReady} onClick={() => fileInputRef.current?.click()} size="sm" type="button" variant="secondary">
          {dataUrl ? t("换图并测试") : t("选择图片并测试")}
        </Button>
        {dataUrl ? <img alt={t("图片预览")} className="vlm-test-preview" src={dataUrl} /> : null}
        {running ? (
          <p className="vlm-test-running">
            <span className="vlm-test-spinner" aria-hidden="true" />
            {t("正在调用 VLM…")}
          </p>
        ) : null}
      </div>

      {localError ? <p className="field-hint warn">{localError}</p> : null}

      {done ? (
        <div className="vlm-test-result">
          <p className="vlm-test-summary" role="status">
            {vlmTestTranslation(done.vlmStatus, done.httpCode ?? undefined, done.durationMs, tr)}
          </p>
          {done.description ? (
            <pre className="vlm-test-description">{done.description}</pre>
          ) : null}
          {done.vlmStatus !== "ok" ? (
            <button className="vlm-test-detail-toggle" onClick={() => void copyError()} type="button">
              {t("复制错误")}
            </button>
          ) : null}
          <button
            className="vlm-test-detail-toggle"
            aria-expanded={showRaw}
            onClick={() => setShowRaw((v) => !v)}
            type="button"
          >
            {showRaw ? t("隐藏原始报文") : t("显示原始报文")}
          </button>
          {showRaw ? (
            <div className="vlm-test-raw">
              <div className="label">{t("原始请求")}</div>
              <pre className="vlm-test-description">{done.rawRequest ?? "-"}</pre>
              <div className="label">{t("原始响应")}</div>
              <pre className="vlm-test-description">{done.rawResponse ?? "-"}</pre>
            </div>
          ) : null}
        </div>
      ) : null}

      <Toolbar>
        {dataUrl ? (
          <Button disabled={!canRun || running} onClick={() => dataUrl && void runTest(dataUrl)} type="button">
            {t("重测")}
          </Button>
        ) : null}
        <Button onClick={onClose} type="button" variant="secondary">
          {t("收起")}
        </Button>
      </Toolbar>
    </div>
  );
}

function AggregateModeChooser({ onSelect }: { onSelect: (mode: AggregateMode) => void }) {
  return (
    <div className="aggregate-mode-chooser">
      <div className="aggregate-mode-chooser-copy">
        <h2>{t("先选择类型")}</h2>
        <p>{t("故障转移与模型聚合彼此独立；选择后只显示对应的成员和路由表单。")}</p>
      </div>
      <div className="aggregate-mode-choice-grid">
        <button className="mode-option aggregate-mode-choice" onClick={() => onSelect("provider")} type="button">
          <strong>{t("故障转移")}</strong>
          <span>{t("多个供应商组成请求池，支持失败切换、会话轮转、请求轮转和权重。")}</span>
        </button>
        <button className="mode-option aggregate-mode-choice" onClick={() => onSelect("model")} type="button">
          <strong>{t("模型聚合")}</strong>
          <span>{t("把成员模型合并到 Codex 列表，选择模型后固定发送到对应供应商。")}</span>
        </button>
      </div>
    </div>
  );
}

function AggregateRelayProfileEditor({
  profile,
  form,
  onProfileChange,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  onProfileChange: (value: RelayProfile) => void;
}) {
  const candidates = aggregateMemberCandidates(form.relayProfiles, profile.id);
  const aggregate = normalizeAggregateConfig(profile.aggregate, candidates);
  const modelMode = aggregate.modelAggregationEnabled === true;
  const memberIds = new Set(aggregate.members.map((member) => member.profileId));
  const sessionProvider = normalizeRelaySessionProvider(profile.sessionProvider);
  const updateAggregate = (nextAggregate: RelayAggregateConfig) => {
    onProfileChange(normalizeAggregateRelayProfile({ ...profile, aggregate: nextAggregate }, form));
  };
  const toggleMember = (profileId: string, checked: boolean) => {
    const members = checked
      ? [...aggregate.members, { profileId, weight: 1 }]
      : aggregate.members.filter((member) => member.profileId !== profileId);
    updateAggregate({ ...aggregate, members });
  };
  const updateWeight = (profileId: string, weight: number) => {
    updateAggregate({
      ...aggregate,
      members: aggregate.members.map((member) =>
        member.profileId === profileId ? { ...member, weight: clampAggregateWeight(weight) } : member,
      ),
    });
  };
  const totalWeight = aggregate.members.reduce((total, member) => total + clampAggregateWeight(member.weight), 0);
  // 只有"纯官方登录"成员才有 ChatGPT 凭据可同步；带 URL/Key 的官方混入走 API 通道。
  const defaultMemberCandidate = candidates.find((candidate) => candidate.id === aggregate.defaultRelayId);
  const defaultMemberIsOfficialLogin = !!defaultMemberCandidate
    && defaultMemberCandidate.relayMode === "official"
    && !defaultMemberCandidate.officialMixApiKey;
  const routes = aggregate.routes ?? [];
  const routeTargetOptions = aggregate.members
    .map((member) => {
      const candidate = candidates.find((item) => item.id === member.profileId);
      return { value: member.profileId, label: candidate?.name || t("未命名供应商") };
    })
    .filter((option) => option.value.trim() !== "");
  const updateRoute = (index: number, patch: Partial<RelayAggregateRoute>) => {
    updateAggregate({
      ...aggregate,
      routes: routes.map((route, routeIndex) => (routeIndex === index ? { ...route, ...patch } : route)),
    });
  };
  const removeRoute = (index: number) => {
    updateAggregate({ ...aggregate, routes: routes.filter((_, routeIndex) => routeIndex !== index) });
  };
  const addRoute = () => {
    updateAggregate({
      ...aggregate,
      routes: [...routes, { pattern: "", profileId: aggregate.members[0]?.profileId ?? "", priority: 0 }],
    });
  };
  // 模型聚合的最终效果：默认成员的模型以裸名发布，其余成员带 cpx- 前缀。勾选时就告诉用户 Codex 会看到什么。
  const memberModelCount = (candidate: RelayProfile) => new Set([
    ...candidate.modelList.split(/[\r\n,]+/).map((line) => line.trim()).filter(Boolean),
    ...(candidate.model.trim() ? [candidate.model.trim()] : []),
  ]).size;
  const selectedCandidates = candidates.filter((candidate) => memberIds.has(candidate.id));
  const defaultCandidate = selectedCandidates.find((candidate) => candidate.id === aggregate.defaultRelayId);
  const bareModelCount = defaultCandidate ? memberModelCount(defaultCandidate) : 0;
  // 官方登录成员一个模型都没填时，后端按 Codex 缓存的官方模型目录发布（#2763），这里给不出确切数量。
  const defaultUsesOfficialDirectory = !!defaultCandidate
    && isOfficialLoginMemberProfile(defaultCandidate)
    && bareModelCount === 0;
  const prefixedModelCount = selectedCandidates
    .filter((candidate) => candidate.id !== defaultCandidate?.id)
    .reduce((sum, candidate) => sum + memberModelCount(candidate), 0);

  return (
    <div className="relay-profile-editor aggregate-editor" data-aggregate-mode={modelMode ? "model" : "provider"}>
      <div className="relay-fields aggregate-fields">
        <Field className="relay-field-name" label={t("名称")}>
          <Input
            value={profile.name}
            onChange={(event) => onProfileChange({ ...profile, name: event.currentTarget.value })}
            placeholder={t("例如 主力聚合池")}
          />
        </Field>
        {!modelMode ? (
          <Field className="aggregate-strategy-field" label={t("聚合策略")}>
            <AppSelect
              value={aggregate.strategy}
              onChange={(value) => updateAggregate({ ...aggregate, strategy: value })}
              options={aggregateStrategyOptions.map((option) => ({ value: option.value, label: option.label }))}
            />
          </Field>
        ) : null}
        <Field className="relay-field-session-provider" label={t("Codex 会话身份")}>
          <AppSelect
            value={sessionProvider}
            onChange={(value) => onProfileChange(normalizeAggregateRelayProfile({ ...profile, sessionProvider: value }, form))}
            options={[
              { value: "custom", label: t("Custom（默认）") },
              { value: "openai", label: t("OpenAI（兼容 ChatGPT Remote）") },
            ]}
          />
          <p className="field-hint">
            {modelMode
              ? t("模型聚合请求仍由本地 Responses 代理转发；OpenAI 身份用于让 ChatGPT Remote 识别会话。")
              : t("聚合请求仍由本地 Responses 代理轮转成员；OpenAI 身份用于让 ChatGPT Remote 识别会话。")}
          </p>
        </Field>
      </div>
      {!modelMode ? (
        <div className="aggregate-strategy-grid">
          {aggregateStrategyOptions.map((option) => (
            <button
              className={`mode-option aggregate-strategy-option ${aggregate.strategy === option.value ? "active" : ""}`}
              key={option.value}
              onClick={() => updateAggregate({ ...aggregate, strategy: option.value })}
              type="button"
            >
              <strong>{option.label}</strong>
              <span>{option.description}</span>
            </button>
          ))}
        </div>
      ) : (
        null
      )}
      <div className="aggregate-members">
        <div className="aggregate-members-head">
          <div>
            <strong>{t("成员供应商")}</strong>
            <span>
              {modelMode
                ? t("默认成员的模型以裸名发布，其他成员带 cpx- 前缀；悬停已勾选的成员可「设为默认」。本模式不使用轮转策略或通配符路由。")
                : t("可勾选 API 供应商或已完成 ChatGPT 登录的官方账号；聚合供应商不会作为成员。")}
            </span>
            {modelMode && selectedCandidates.length ? (
              <span className="aggregate-model-summary">
                {defaultCandidate && defaultUsesOfficialDirectory
                  ? tf("Codex 将看到 {0} 的官方模型目录（裸名）+ {1} 个 cpx- 前缀模型", [defaultCandidate.name || t("未命名供应商"), prefixedModelCount])
                  : defaultCandidate
                  ? tf("Codex 将看到 {0} 个裸名模型（来自 {1}）+ {2} 个 cpx- 前缀模型", [bareModelCount, defaultCandidate.name || t("未命名供应商"), prefixedModelCount])
                  : tf("Codex 将看到 {0} 个 cpx- 前缀模型；还没指定默认成员，不带前缀的请求会报错", [prefixedModelCount])}
              </span>
            ) : null}
          </div>
          <UiBadge variant="outline">{aggregate.members.length} / {candidates.length}</UiBadge>
        </div>
        {candidates.length ? (
          <div className="aggregate-member-list">
            {candidates.map((candidate) => {
              const member = aggregate.members.find((item) => item.profileId === candidate.id);
              const checked = memberIds.has(candidate.id);
              const isDefaultMember = !!aggregate.defaultRelayId && aggregate.defaultRelayId === candidate.id;
              // 只显示模型数量 + 行内「设为默认」：整串模型名会把每张卡片撑成不同高度。
              // 完整名单放 title；「默认」和勾选在同一行，不再需要下方单独的下拉。
              const memberModelNames = [...new Set([
                ...candidate.modelList.split(/[\r\n,]+/).map((line) => line.trim()).filter(Boolean),
                ...(candidate.model.trim() ? [candidate.model.trim()] : []),
              ])];
              return (
                <label className={`aggregate-member-row ${checked ? "selected" : ""}`} key={candidate.id}>
                  <input
                    checked={checked}
                    onChange={(event) => toggleMember(candidate.id, event.currentTarget.checked)}
                    type="checkbox"
                  />
                  <span className="aggregate-member-summary">
                    {/* 与供应商列表一致：接入方式做成标签，第二行只放地址。 */}
                    <span className="aggregate-member-title">
                      <strong>{candidate.name || t("未命名供应商")}</strong>
                      <span className="relay-tag" data-tone={relayProfileTagTone(candidate)}>{relayProfileModeLabel(candidate)}</span>
                      {candidate.protocol !== "responses" ? (
                        <span className="relay-tag" data-tone="protocol">{relayProtocolShortLabel(candidate.protocol)}</span>
                      ) : null}
                    </span>
                    <small>
                      {isOfficialLoginMemberProfile(candidate)
                        ? t("使用已登录的 ChatGPT 账号凭据")
                        : relayProfileConfigBrief(candidate)}
                    </small>
                  </span>
                  {modelMode ? (
                    <span className="aggregate-member-models">
                      <span
                        className="aggregate-member-models-count"
                        title={memberModelNames.length ? memberModelNames.join("\n") : undefined}
                      >
                        {memberModelNames.length
                          ? tf("{0} 个模型", [memberModelNames.length])
                          : (isOfficialLoginMemberProfile(candidate)
                            ? t("未填写：用 Codex 的官方模型目录")
                            : t("未填写模型"))}
                      </span>
                      {checked ? (isDefaultMember ? (
                        <UiBadge className="aggregate-member-default-badge" variant="secondary">
                          {t("默认")}
                        </UiBadge>
                      ) : (
                        <Button
                          className="aggregate-member-default-action"
                          onClick={(event) => {
                            // 行本身是 <label>：不拦默认行为会顺带切换勾选状态。
                            event.preventDefault();
                            event.stopPropagation();
                            updateAggregate({ ...aggregate, defaultRelayId: candidate.id });
                          }}
                          size="sm"
                          title={t("不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名）会原样交给它")}
                          type="button"
                          variant="ghost"
                        >
                          {t("设为默认")}
                        </Button>
                      )) : null}
                    </span>
                  ) : (
                    <span className="aggregate-weight-box">
                      <span>{t("权重")}</span>
                      <Input
                        disabled={!checked}
                        min={1}
                        onChange={(event) => updateWeight(candidate.id, Number.parseInt(event.currentTarget.value, 10))}
                        type="number"
                        value={String(member?.weight ?? 1)}
                      />
                    </span>
                  )}
                </label>
              );
            })}
          </div>
        ) : (
          <div className="empty">{t("先添加至少 1 个可用的 API 供应商或官方登录账号，再创建聚合供应商。")}</div>
        )}
      </div>
      {modelMode ? (
        <div className="aggregate-default-provider aggregate-default-hint">
          <span>
            {aggregate.defaultRelayId
              ? tf("默认供应商：{0}。不带 cpx- 前缀的请求（旧会话、自动压缩带的模型名等）会原样交给它；带 cpx- 前缀但解析不出来的 ID 一律报错，不回落。", [
                candidates.find((candidate) => candidate.id === aggregate.defaultRelayId)?.name || aggregate.defaultRelayId,
              ])
              : t("还没指定默认供应商：在上方成员行尾点「设为默认」。不带 cpx- 前缀的请求会原样交给它，没设就报错。")}
          </span>
          {defaultMemberIsOfficialLogin ? (
            <label className="aggregate-default-login-sync">
              <input
                checked={aggregate.syncDefaultMemberLogin === true}
                onChange={(event) => updateAggregate({
                  ...aggregate,
                  syncDefaultMemberLogin: event.currentTarget.checked,
                })}
                type="checkbox"
              />
              <span>
                <strong>{t("把默认成员的官方登录同步到 Codex")}</strong>
                <small>{t("开启后切换到本聚合时，会把默认成员的 ChatGPT 凭据一并写进 Codex（代理 Key 保留），账号在 Codex 里就是登录态。默认成员不是官方登录时无效。")}</small>
              </span>
            </label>
          ) : (
            <span className="aggregate-default-login-note">
              {t("当前默认成员不是官方登录账号，无法把登录同步到 Codex；只有「官方登录」类型的成员才有 ChatGPT 凭据。")}
            </span>
          )}
        </div>
      ) : null}
      {routeTargetOptions.length ? (
        <div className="aggregate-default-provider aggregate-auto-review">
          <Field label={t("审批成员")}>
            <AppSelect
              label={t("审批成员")}
              onChange={(value) => updateAggregate({ ...aggregate, autoReviewRelayId: value })}
              options={[
                { value: "", label: t("不单独指定"), hint: t("审批请求跟普通请求一样交给默认成员或轮转选中的成员") },
                ...routeTargetOptions.map((option) => {
                  const candidate = candidates.find((item) => item.id === option.value);
                  const reviewModel = candidate?.autoReviewModel?.trim() || candidate?.model.trim() || "";
                  return {
                    value: option.value,
                    label: option.label,
                    hint: reviewModel ? tf("审批请求发给它，模型 {0}", [reviewModel]) : t("审批请求发给它"),
                  };
                }),
              ]}
              value={aggregate.autoReviewRelayId ?? ""}
            />
            <p className="field-hint">
              {t("Codex「替我审批」的请求（codex-auto-review）只发给这个成员，模型取它的「自动审批模型」，没填用它的默认模型。默认成员是官方登录时，指定一个 API 成员可避免审批消耗官方额度。")}
            </p>
          </Field>
        </div>
      ) : null}
      {!modelMode ? <div className="aggregate-routes">
        <div className="aggregate-routes-head">
          <div>
            <strong>{t("路由规则")}</strong>
            <span>{t("按模型名自动路由到指定成员；仅支持 * 通配符，chat/completions 协议不走路由。")}</span>
          </div>
          <UiBadge variant="outline">{routes.length}</UiBadge>
        </div>
        {routes.length ? (
          <div className="aggregate-route-list">
            {routes.map((route, index) => (
              <div className="aggregate-route-row" key={index}>
                <Input
                  onChange={(event) => updateRoute(index, { pattern: event.currentTarget.value })}
                  placeholder={t("例如 deepseek-*")}
                  value={route.pattern}
                />
                <AppSelect
                  onChange={(value) => updateRoute(index, { profileId: value })}
                  options={routeTargetOptions}
                  value={route.profileId}
                />
                {!routeTargetOptions.some((option) => option.value === route.profileId) ? (
                  <span className="aggregate-route-target-error">{t("路由目标必须是已勾选的聚合成员，请先在成员供应商中勾选。")}</span>
                ) : null}
                <div className="aggregate-route-priority">
                  <span>{t("优先级")}</span>
                  <Input
                    min={0}
                    onChange={(event) =>
                      updateRoute(index, { priority: clampAggregateRoutePriority(Number.parseInt(event.currentTarget.value, 10)) })
                    }
                    type="number"
                    value={String(route.priority)}
                  />
                </div>
                <button
                  className="aggregate-route-remove"
                  onClick={() => removeRoute(index)}
                  title={t("删除规则")}
                  type="button"
                >
                  <Trash2 className="h-4 w-4" />
                </button>
              </div>
            ))}
          </div>
        ) : (
          <div className="empty">{t("暂无路由规则，未匹配的模型会按聚合策略选择成员。")}</div>
        )}
        <div>
          <Button disabled={!aggregate.members.length} onClick={addRoute} size="sm" variant="secondary">
            <Plus className="h-4 w-4" />
            {t("添加规则")}
          </Button>
        </div>
      </div> : null}
      {!modelMode ? (
        <>
          <div className="relay-grid compact aggregate-preview">
            <Metric label={t("策略")} value={aggregateStrategyLabel(aggregate.strategy)} />
            <Metric label={t("成员数量")} value={tf("{0} 个", [aggregate.members.length])} />
            <Metric label={t("总权重")} value={`${totalWeight}`} />
            <Metric label={t("序列化字段")} value="aggregate.strategy / aggregate.members / aggregate.routes" />
          </div>
          <div className="hint-line relay-protocol-hint">
            <ShieldCheck className="h-4 w-4" />
            <span>{aggregateStrategyHelp(aggregate.strategy)}</span>
          </div>
        </>
      ) : null}
    </div>
  );
}

function SyncedTextarea({
  value,
  onValueChange,
  className,
}: {
  value: string;
  onValueChange: (value: string) => void;
  className?: string;
}) {
  const [localValue, setLocalValue] = useState(value);
  const isFocusedRef = useRef(false);
  const latestExternalValueRef = useRef(value);

  useEffect(() => {
    latestExternalValueRef.current = value;
    if (!isFocusedRef.current) {
      setLocalValue(value);
    }
  }, [value]);

  return (
    <Textarea
      className={className}
      value={localValue}
      onBlur={() => {
        isFocusedRef.current = false;
        setLocalValue(latestExternalValueRef.current);
      }}
      onChange={(event) => {
        const next = event.currentTarget.value;
        setLocalValue(next);
        onValueChange(next);
      }}
      onFocus={() => {
        isFocusedRef.current = true;
      }}
      spellCheck={false}
    />
  );
}

export function RelayCommonConfigScreen({ onSave, onBack }: {
  onSave: (original: string, contents: string) => Promise<SettingsResult | null>;
  onBack: () => void;
}) {
  const [contents, setContents] = useState<string | null>(null);
  const [draft, setDraft] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  useEffect(() => {
    let cancelled = false;
    void invoke<BackendSettings>("read_relay_common_config").then(settings => {
      if (!cancelled) setContents(settings.relayCommonConfigContents ?? "");
    }).catch(error => { if (!cancelled) setLoadError(errorText(error)); });
    return () => { cancelled = true; };
  }, []);
  const save = async () => {
    if (contents === null || draft === null || saving) return;
    setSaving(true);
    try {
      const result = await onSave(contents, draft);
      if (result) {
        setContents(result.settings.relayCommonConfigContents ?? "");
        setDraft(null);
      }
    } finally {
      setSaving(false);
    }
  };
  return (
    <section className="screen management-screen">
      <header className="cx-page-head" data-tauri-drag-region="deep">
        <div className="cx-page-title"><h1>{t("通用配置文件")}</h1></div>
        <div className="topbar-actions">
          {contents !== null ? <Button type="button" disabled={draft === null || saving} onClick={() => void save()}>
            <Save className="h-4 w-4" />
            {t("保存")}
          </Button> : null}
          <Button variant="ghost" onClick={onBack}>{t("返回列表")}</Button>
        </div>
      </header>
      {contents !== null
        ? <fieldset className="min-w-0" disabled={saving}>
            <SyncedTextarea
              className="relay-file-textarea"
              value={draft ?? contents}
              onValueChange={text => setDraft(text === contents ? null : text)}
            />
          </fieldset>
        : <p role={loadError ? "alert" : "status"}>{loadError ?? t("正在读取…")}</p>}
    </section>
  );
}

function RelayFileEditors({
  contextProfile,
  profile,
  form,
  isActive,
  profileId,
  onFormChange,
  onProfileChange,
  actions,
}: {
  contextProfile: RelayProfile;
  profile: RelayProfile;
  form: BackendSettings;
  isActive: boolean;
  profileId: string;
  onFormChange: (value: BackendSettings) => void;
  onProfileChange: (value: RelayProfile) => void;
  actions: Actions;
}) {
  const configPreview = effectiveRelayConfigPreview(profile, form, contextProfile);
  const entries = contextEntriesForProfile(form, contextProfile);
  return (
    <div className="relay-file-grid">
      <RelayFold className="relay-file-panel relay-config-preview" sectionId="config-preview" title={t("config.toml 预览")}>
        <p className="relay-fold-description">
          {isActive ? t("当前供应商切换后会写入的预览；上下文开关变化会立即反映") : t("切换到此供应商时会写入的预览；上下文开关变化会立即反映")}
        </p>
        <SyncedTextarea
          className="relay-file-textarea"
          value={configPreview}
          onValueChange={(value) => {
            const withoutCommon = stripCommonConfigTextFallback(
              value,
              relayCombinedCommonConfig(form),
            );
            const configContents = stripContextEntriesFromConfig(withoutCommon, entries);
            onProfileChange(deriveRelayProfileFromFiles({
              ...profile,
              configContents,
            }, "config"));
          }}
        />
      </RelayFold>
      <RelayFold
        bodyClassName="relay-common-config-body"
        className="relay-file-panel relay-common-config-panel"
        defaultOpen
        sectionId="common-config"
        title={t("通用配置文件")}
      >
        <p className="relay-fold-description">
          {t("只保留非 MCP、插件的跨供应商配置；MCP&插件在独立页面管理。点此展开编辑。")}
        </p>
            <Button
              onClick={async () => {
                const extracted = await actions.extractRelayCommonConfig(profile.configContents || "");
                if (!extracted) return;
                const split = splitContextConfigText(extracted.commonConfigContents || "");
                if (!split.common.trim() && !split.context.trim()) {
                  await actions.showMessage(t("通用配置文件"), t("当前供应商 config.toml 里没有可提取的通用配置。"), "failed");
                  return;
                }
                const promotedProfile = {
                  ...profile,
                  configContents: extracted.profileConfigContents,
                };
                const next = syncLegacyRelayFields({
                  ...form,
                  relayCommonConfigContents: split.common,
                  relayContextConfigContents: joinTomlSectionsRootFirst([form.relayContextConfigContents || "", split.context]),
                  relayProfiles: form.relayProfiles.map((item) => (item.id === profileId ? promotedProfile : item)),
                });
                onFormChange(next);
                onProfileChange(promotedProfile);
                await actions.saveSettingsValue(next, false);
              }}
              size="sm"
              type="button"
              variant="secondary"
            >
              <Download className="h-4 w-4" />
              {t("提取当前供应商配置")}
            </Button>
          <SyncedTextarea
            className="relay-file-textarea"
            value={form.relayCommonConfigContents}
            onValueChange={(value) => onFormChange({ ...form, relayCommonConfigContents: value })}
          />
      </RelayFold>
      <RelayFold className="relay-file-panel relay-auth-preview" defaultOpen sectionId="auth-preview" title="auth.json">
        <p className="relay-fold-description">
          {isActive
            ? profile.relayMode === "pureApi"
              ? t("当前使用中：保留此供应商的 auth 存档，避免 Codex 登录密钥覆盖供应商密钥")
              : t("当前使用中：打开时从 Codex 主目录的 auth.json 回填，保存后会作为此供应商 auth 存档")
            : t("切换到此供应商时会写入 Codex 主目录的 auth.json")}
        </p>
        <SyncedTextarea
          className="relay-file-textarea"
          value={profile.authContents}
          onValueChange={(value) => onProfileChange(deriveRelayProfileFromFiles({ ...profile, authContents: value }, "auth"))}
        />
      </RelayFold>
    </div>
  );
}

function ProviderDoctorModal({
  result,
  running,
  onClose,
}: {
  result: ProviderDoctorResult | null;
  running: boolean;
  onClose: () => void;
}) {
  const steps = providerDoctorSteps(result, running);
  const doneCount = steps.filter((step) => step.state === "ok" || step.state === "warning" || step.state === "failed").length;
  const progress = Math.round((doneCount / steps.length) * 100);
  const titleId = useId();
  // Esc、点遮罩关闭（诊断中 onClose 自己不关）；焦点圈在弹窗里。
  const { backdropProps, dialogProps } = useModalShell<HTMLDivElement>({ onClose, labelledBy: titleId });
  return (
    <div className="modal-backdrop" {...backdropProps}>
      <div {...dialogProps} className="modal-card provider-doctor-modal">
        <div className="modal-head">
          <div>
            <h2 id={titleId}>{t("诊断供应商")}</h2>
            <p>{running ? t("正在诊断供应商，请稍候。") : result?.summary ?? t("诊断已完成。")}</p>
          </div>
          <UiBadge variant={result && !isSuccessStatus(result.status) ? "outline" : "secondary"}>
            {running ? t("诊断中") : result && !isSuccessStatus(result.status) ? t("异常") : t("完成")}
          </UiBadge>
        </div>
        <div className="provider-doctor-progress" aria-valuemin={0} aria-valuemax={100} aria-valuenow={progress} role="progressbar">
          <div style={{ width: `${progress}%` }} />
        </div>
        <div className="provider-doctor-step-list">
          {steps.map((step) => (
            <div className={`provider-doctor-step ${step.state}`} key={step.id}>
              <span className="provider-doctor-step-icon">
                {step.state === "running" ? (
                  <RefreshCw className="h-4 w-4" />
                ) : step.state === "ok" ? (
                  <CheckCircle2 className="h-4 w-4" />
                ) : step.state === "warning" ? (
                  <ShieldAlert className="h-4 w-4" />
                ) : step.state === "failed" ? (
                  <Info className="h-4 w-4" />
                ) : (
                  <span />
                )}
              </span>
              <div>
                <strong>{step.title}</strong>
                <small>{step.detail}</small>
              </div>
            </div>
          ))}
        </div>
        {result?.recommendation ? <p className="provider-doctor-recommendation">{result.recommendation}</p> : null}
        <div className="modal-actions">
          <Button disabled={running} onClick={onClose} variant="secondary">
            {running ? t("诊断中") : t("关闭")}
          </Button>
        </div>
      </div>
    </div>
  );
}

type ProviderDoctorStepState = "pending" | "running" | "ok" | "warning" | "failed";

function providerDoctorSteps(
  result: ProviderDoctorResult | null,
  running: boolean,
): Array<{ id: string; title: string; detail: string; state: ProviderDoctorStepState }> {
  const base = [
    { id: "config", title: t("配置完整性"), pending: t("等待检查 Base URL / API Key。") },
    { id: "models", title: t("模型列表"), pending: t("等待检查 /v1/models。") },
    { id: "request", title: t("真实请求"), pending: t("等待发送一次测试请求。") },
    { id: "recommendation", title: t("处理建议"), pending: t("等待生成建议。") },
  ];
  if (!result) {
    return base.map((step, index) => ({
      id: step.id,
      title: step.title,
      detail: index === 0 && running ? t("正在检查配置完整性…") : step.pending,
      state: index === 0 && running ? "running" : "pending",
    }));
  }
  const checks = new Map(result.checks.map((check) => [check.id, check]));
  return base.map((step) => {
    if (step.id === "recommendation") {
      return {
        id: step.id,
        title: step.title,
        detail: result.recommendation || step.pending,
        state: result.status === "failed" ? "warning" : "ok",
      };
    }
    const check = checks.get(step.id);
    if (!check) {
      return {
        id: step.id,
        title: step.title,
        detail: step.id === "models" || step.id === "request" ? t("该步骤未执行。") : step.pending,
        state: "pending",
      };
    }
    return {
      id: step.id,
      title: check.title || step.title,
      detail: check.detail,
      state: check.status === "ok" ? "ok" : check.status === "warning" ? "warning" : "failed",
    };
  });
}

function Field({ label, children, className = "" }: { label: string; children: React.ReactNode; className?: string }) {
  return (
    <Label className={`field ${className}`}>
      <span>{label}</span>
      {children}
    </Label>
  );
}

function contextEntriesForProfile(settings: BackendSettings, profile: RelayProfile): CodexContextEntries {
  return filterContextEntriesBySelection(contextEntriesFromSettings(settings), profile.contextSelection);
}

function filterContextEntriesBySelection(entries: CodexContextEntries, selection: RelayContextSelection): CodexContextEntries {
  const selected = {
    mcp: new Set(selection.mcpServers.map((id) => id.trim()).filter(Boolean)),
    skill: new Set(selection.skills.map((id) => id.trim()).filter(Boolean)),
    plugin: new Set(selection.plugins.map((id) => id.trim()).filter(Boolean)),
  };
  return {
    mcpServers: entries.mcpServers.filter((entry) => selected.mcp.has(entry.id)),
    skills: entries.skills.filter((entry) => selected.skill.has(entry.id)),
    plugins: entries.plugins.filter((entry) => selected.plugin.has(entry.id)),
  };
}

function effectiveRelayConfigPreview(profile: RelayProfile, settings: BackendSettings, contextProfile = profile): string {
  const entries = contextEntriesForProfile(settings, contextProfile);
  const isolatedConfig = stripContextEntriesFromConfig(profile.configContents, entries);
  const configWithLimits = applyContextLimitPreview(isolatedConfig, profile);
  const profileAndCommon = mergeFeaturesTableForPreview(configWithLimits, settings.relayCommonConfigContents || "");
  return joinTomlSectionsRootFirst([profileAndCommon, selectedContextConfigToml(entries)]);
}

function mergeFeaturesTableForPreview(profileConfig: string, commonConfig: string): string {
  const profile = splitFeaturesTable(profileConfig);
  const common = splitFeaturesTable(commonConfig);
  if (!profile.body && !common.body) return joinTomlSectionsRootFirst([profileConfig, commonConfig]);

  const profileKeys = new Set(tomlAssignmentKeys(profile.body));
  const commonBody = common.body
    .split(/\r?\n/)
    .filter((line) => {
      const key = tomlAssignmentKey(line);
      return !key || !profileKeys.has(key);
    })
    .join("\n");
  const mergedFeatures = ["[features]", commonBody, profile.body]
    .filter((part) => part.trim())
    .join("\n");
  return joinTomlSectionsRootFirst([
    profile.without,
    common.without,
    mergedFeatures,
  ]);
}

function splitFeaturesTable(contents: string): { without: string; body: string } {
  const lines = contents.trim().split(/\r?\n/);
  const start = lines.findIndex((line) => line.trim() === "[features]");
  if (start < 0) return { without: contents, body: "" };
  let end = lines.length;
  for (let index = start + 1; index < lines.length; index += 1) {
    if (/^\s*\[[^\]]+\]\s*$/.test(lines[index])) {
      end = index;
      break;
    }
  }
  return {
    without: [...lines.slice(0, start), ...lines.slice(end)].join("\n"),
    body: lines.slice(start + 1, end).join("\n"),
  };
}

function tomlAssignmentKey(line: string): string | undefined {
  return /^\s*([A-Za-z0-9_-]+)\s*=/.exec(line)?.[1];
}

function tomlAssignmentKeys(contents: string): string[] {
  return contents.split(/\r?\n/).map(tomlAssignmentKey).filter((key): key is string => Boolean(key));
}

function relayCombinedCommonConfig(settings: BackendSettings): string {
  return joinTomlSectionsRootFirst([settings.relayCommonConfigContents || "", settings.relayContextConfigContents || ""]);
}

function stripCommonConfigTextFallback(configContents: string, commonConfig: string): string {
  const anchors = commonConfigAnchors(commonConfig);
  if (!anchors.rootKeys.size && !anchors.tableHeaders.size) return ensureTrailingNewline(configContents.trimEnd());

  const kept: string[] = [];
  let skippingTable = false;

  for (const line of configContents.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      skippingTable = anchors.tableHeaders.has(trimmed);
      if (skippingTable) continue;
    }
    if (skippingTable) continue;
    const key = tomlRootKeyFromLine(trimmed);
    if (key && anchors.rootKeys.has(key)) continue;
    kept.push(line);
  }

  return ensureTrailingNewline(kept.join("\n").trimEnd());
}

function commonConfigAnchors(commonConfig: string): { rootKeys: Set<string>; tableHeaders: Set<string> } {
  const rootKeys = new Set<string>();
  const tableHeaders = new Set<string>();
  let inRoot = true;

  for (const line of commonConfig.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (/^\[[^\]]+\]$/.test(trimmed)) {
      inRoot = false;
      tableHeaders.add(trimmed);
      continue;
    }
    if (inRoot) {
      const key = tomlRootKeyFromLine(trimmed);
      if (key) rootKeys.add(key);
    }
  }

  return { rootKeys, tableHeaders };
}

function applyContextLimitPreview(configContents: string, profile: RelayProfile): string {
  const replacements: Array<[string, string]> = [
    ["model_context_window", profile.contextWindow],
    ["model_auto_compact_token_limit", profile.autoCompactLimit],
  ];
  let lines = configContents.split(/\r?\n/);

  for (const [key, value] of replacements) {
    const trimmed = value.trim();
    if (!trimmed) continue;
    let replaced = false;
    lines = lines.map((line) => {
      if (!replaced && new RegExp(`^\\s*${key}\\s*=`).test(line)) {
        replaced = true;
        return `${key} = ${trimmed}`;
      }
      return line;
    });
    if (!replaced) {
      const firstTable = lines.findIndex((line) => /^\s*\[[^\]]+\]\s*$/.test(line));
      const insertAt = firstTable >= 0 ? firstTable : lines.length;
      lines.splice(insertAt, 0, `${key} = ${trimmed}`);
    }
  }

  return ensureTrailingNewline(lines.join("\n").trimEnd());
}

function relayProfileEditorStatus(profile: RelayProfile, form: BackendSettings, isNew: boolean) {
  if (isNew) return t("新建供应商需要先保存到列表");
  if (!form.relayProfilesEnabled) return t("供应商配置总开关已关闭；当前只保存配置，不写入 Codex live 文件");
  return profile.id === form.activeRelayId ? t("当前正在使用") : t("编辑后保存列表，再切换模式时会使用新配置");
}

function relayProfileModeHelp(profile: RelayProfile): string {
  if (isAggregateRelayProfile(profile)) {
    const aggregate = normalizeAggregateConfig(profile.aggregate, []);
    if (aggregate.modelAggregationEnabled) {
      return t("此聚合会把成员模型合并到 Codex 列表；带供应商前缀的模型固定发送到对应成员，普通模型仍按策略轮转。");
    }
    return t("聚合供应商只保存成员和策略配置，成员来自已有 API 供应商；切为当前后会通过本地协议代理轮转请求。");
  }
  if (profile.relayMode === "official") {
    if (profile.officialMixApiKey) {
      return t("此供应商会保留官方登录模式，并把请求混入当前 API Key；Codex增强仍使用兼容模式。");
    }
    return t("此供应商会切回官方登录模式，使用 ChatGPT 官方账号，不写入 API Key。");
  }
  if (profile.relayMode === "pureApi") {
    return t("此供应商会同时写入 config.toml 和 auth.json；API Key 也会注入到 provider bearer token。");
  }
  return t("此供应商会保留官方登录模式，并把请求混入当前 API Key；Codex增强仍使用兼容模式。");
}
