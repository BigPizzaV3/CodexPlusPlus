/** 「全局管理 · Skills」：已安装 / 发现两个视图，按来源或自定义分组，每行各 Agent 一枚开关。抽屉在 SkillDrawers.tsx。 */
import { memo, startTransition, useDeferredValue, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { ArrowDownAZ, ArrowDownUp, ArrowDownZA, BookOpen, ChevronDown, ChevronsDown, ChevronsUp, CloudDownload, FolderPlus, GitBranch, GripVertical, Link, RefreshCw, X } from "lucide-react";
import { t, tf } from "./i18n";
import { appConfirm, appPrompt } from "./app-dialog";
import { matchesManagementQuery, skillRepoKey } from "./management-model";
import { AgentToggle, Menu, MenuItem, PageButton as Button, PageHeader } from "./ManagementPages";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxRow, CxStatusIcon, CxToolbar } from "./cx-layout";
import { AgentGlyph } from "./hub-shared";
import { MCP_AGENTS, type McpAgentId } from "./agent-mcp-model";
import { agentVisible } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import "./ManagementPages.css";
import { aggregateSkillGroupItems, applySkillBundleGroups, customSkillGroupKey, groupSkills, legacySkillCustomGroupName, migrateSkillGroupAssignments, migrateSkillInstanceAssignments, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillBundleExportGroups, skillBundleSelections, skillGroupAssignment, skillGroupIdentityKey, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, type SkillGroup, type SkillGroupSortDirection } from "./skill-groups";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets, skillCodexStatusError, SkillBatchRegistry, type PendingSkillBatchOperation } from "./skill-agent-batch";
import { SkillSourceMatchDrawer, type SourceSelection } from "./SkillSourceMatchDrawer";
import { SkillDuplicatesDrawer } from "./SkillDuplicatesDrawer";
import { SkillBundleExportDrawer } from "./SkillBundleExportDrawer";
import { SkillInPlaceUpdateDrawer } from "./SkillInPlaceUpdateDrawer";
import { SkillVersionPicker, type LocalSkillVersion, type VersionTask } from "./SkillVersionPicker";
import { ok, sourceLabel, SkillBackupsDrawer, SkillDetailDrawer, SkillRepoDrawer, type Notify, type Repo, type RepoInspection, type Result, type Skill, type SkillBackup, type SkillBundleImportResult, type SkillMatchContext, type SkillPayload, type SourceApplyResult } from "./SkillDrawers";

const SKILL_GROUP_STORAGE_KEY = "codex-plus-plus.skill-groups.v1";
const skillBatchRegistry = new SkillBatchRegistry();
const skillPageCache: {
  value: SkillPayload | null;
  localStartedAt: number;
  catalogStartedAt: number;
  localPending: Promise<SkillPayload> | null;
  catalogPending: Promise<SkillPayload> | null;
  mounted: number;
} = { value: null, localStartedAt: 0, catalogStartedAt: 0, localPending: null, catalogPending: null, mounted: 0 };
skillBatchRegistry.subscribeSettled(() => { skillPageCache.localStartedAt = 0; });
function SortableSkillGroup({ group, disabled, children }: { group: SkillGroup<Skill>; disabled: boolean; children: (handle: ReactNode) => ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: group.key, disabled });
  const handle = disabled ? <span className="mg-skill-drag-placeholder" aria-hidden="true" /> :
    <button type="button" className="mg-icon-button mg-skill-drag" title={t("拖动分组排序")} aria-label={`${t("拖动分组排序")} ${group.label}`} {...attributes} {...listeners}><GripVertical size={15}/></button>;
  return <section className="mg-skill-group" ref={setNodeRef} data-dragging={isDragging}
    style={{ transform: CSS.Transform.toString(transform), transition }}>{children(handle)}</section>;
}
/** 分组标题上的批量开关：和行里的 Agent 胶囊同一列，一组全开是实心、部分开是半实心；点一下全开或全关。 */
function GroupAgentSelect({ group, agent, busy, operations, count, prepareSkill, onChange }: { group: SkillGroup<Skill>; agent: McpAgentId; busy: boolean; operations: PendingSkillBatchOperation[]; count?: number; prepareSkill: (skill: Skill, agent: McpAgentId) => Skill; onChange: (enabled: boolean) => void }) {
  const pendingById = new Map<string, boolean>();
  for (const operation of operations) {
    if (operation.agent !== agent) continue;
    const settled = new Set(operation.settledSkillIds);
    const successful = new Set(operation.successfulSkillIds || []);
    for (const id of operation.skillIds) if (!settled.has(id) || successful.has(id)) pendingById.set(id, operation.enabled);
  }
  const state = skillBatchState(group.items.map((item) => {
    const skill = prepareSkill(item, agent);
    const pending = pendingById.get(skill.id);
    if (pending === undefined) return skill;
    const agents = skillAgents(skill).filter((value) => value !== agent);
    if (pending) agents.push(agent);
    return { ...skill, agents };
  }), agent);
  const name = agent === "codex" ? "Codex" : agent === "claude" ? "Claude Code" : "Grok";
  const label = tf("{0} · {1} · 全选", [group.label, name]);
  return <button type="button" className="cx-agent-toggle cx-skill-batch" aria-label={label}
    title={count === undefined ? label : tf("{0}（已启用 {1} 个 Skill）", [label, count])}
    aria-pressed={state.mixed ? "mixed" : state.checked} disabled={busy || !state.total}
    onClick={() => onChange(!state.checked)}>
    <AgentGlyph agent={agent} /><span>{name}</span>
  </button>;
}
/** 每组先显示的行数；其余点「再显示」展开，长目录（发现页、搜索结果）不必一次画出全部行。 */
const SKILL_GROUP_PAGE = 50;
const SYNC_COPIES_HINT = "把各 Agent 中内容一致的 Skill 同步到 Codex++ 保存的副本中。旧副本会先备份，不会从 GitHub 下载更新。";

/** 行里的操作；页面传进来的对象引用不变，行组件只在自己的数据变化时重画。 */
type SkillRowActions = {
  openDetails: (skill: Skill, locations?: boolean) => void;
  openRepo: (repoKey: string) => void;
  setVersionTask: (task: VersionTask) => void;
  assignGroup: (skill: Skill, group: string) => void;
  install: (skill: Skill) => void;
  update: (skill: Skill, inPlace: boolean) => void;
  syncCopies: (skill: Skill) => Promise<boolean>;
  run: (command: string, args: Record<string, unknown>) => Promise<boolean>;
};

/**
 * 一个 Skill 一行（参照 MCP 页 / Cursor 的 Customize）：右侧是各 Agent 的开关胶囊；
 * 详情、分组、卸载收进行末的三点菜单。`conflict` / `guarded` / `codexToggleSafe` 由页面按全部数据算好传进来。
 */
function SkillRowView({ skill: s, busy, update, conflict, guarded, codexToggleSafe, agentColumns, batchOperations, customGroupNames, assignment, agentSkillsDirs, actions }: {
  skill: Skill; busy: boolean; update?: ReturnType<SkillBatchRegistry["getUpdates"]>[number]; conflict: boolean; guarded: boolean; codexToggleSafe: boolean;
  agentColumns: McpAgentId[]; batchOperations: PendingSkillBatchOperation[]; customGroupNames: string[]; assignment: string;
  agentSkillsDirs?: Record<string, string>; actions: SkillRowActions;
}) {
  const inPlaceUpdate = Boolean(s.originalDir && s.updateOwner !== "manager");
  // 与分组更新（updateTargets）一致：共享位置、未授权渠道的 Skill 后端会拒绝，不显示必然失败的按钮。
  const managerUpdate = (!s.originalDir && !(s.updateOwner === "" && s.installChannel)) || s.updateOwner === "manager";
  const canUpdate = inPlaceUpdate || (managerUpdate && !s.locations?.some((location) => location.rootKind === "shared"));
  const needsSync = s.managedSourceStatus === "needs-sync";
  const syncCopiesHint = t(SYNC_COPIES_HINT);
  const statusError = skillCodexStatusError(s);
  const detail = conflict && s.locations?.length ? s.locations.map((location) => location.manifestPath).join(" · ") : s.description || t("暂无描述");
  // 共享原件由其他 Agent 共用，受控副本需通过重复预览处理。
  const canUninstall = s.installed && !s.bundled && !guarded && !s.locations?.some((location) => location.rootKind === "shared")
    && (s.managed || skillAgents(s).includes("codex") || s.agents === undefined);
  const on = s.installed && agentColumns.some((agent) => (agent !== "codex" || !statusError) && skillAgents(s).includes(agent));
  return <CxRow className="cx-skill-row"
    icon={<CxStatusIcon tone={on ? "ok" : "off"}><BookOpen /></CxStatusIcon>}
    title={s.name || s.id}
    badges={<>
      <small className="cx-skill-source" title={s.repoKey === "cc-switch" ? sourceLabel(s) : s.repoKey || sourceLabel(s)}>{sourceLabel(s)}</small>
      {statusError && <CxBadge tone="warn">{t("Codex 状态未知")}</CxBadge>}
      {conflict && !s.displayInstances ? <CxBadge tone="warn">{t(s.conflictKind === "equal-independent" ? "多个独立安装" : "同名冲突")}</CxBadge> : s.updateAvailable ? <CxBadge tone="warn">{t("可更新")}</CxBadge> : null}
    </>}
    detail={<span title={detail}>{detail}</span>}
    onClick={() => actions.openDetails(s)}>
    {needsSync && <Button title={syncCopiesHint} disabled={busy || !!update} onClick={() => void actions.syncCopies(s)}>{t("同步工作副本")}</Button>}
    {update ? <span title={update.repoKey !== s.repoKey ? t("同 ID 操作中") : undefined}><Button disabled onClick={() => {}}>{t(update.command === "uninstall_skill" ? "卸载排队中" : update.repoKey !== s.repoKey ? "同 ID 操作中" : update.state === "queued" ? "排队中" : update.command === "install_skill" ? "安装中" : "更新中")}</Button></span> : !s.installed && !s.bundled ? <Button disabled={guarded} onClick={() => actions.install(s)}>{t("安装")}</Button> : s.installed && s.updateAvailable && !guarded && canUpdate ? <Button onClick={() => actions.update(s, inPlaceUpdate)}>{t("更新")}</Button> : null}
    <div className="cx-agent-toggles" role="group" aria-label={t("启用到 Agent")}>
      {agentColumns.map((agent) => {
        const safeDisable = guarded && s.conflictKind === "managed-modified"
          && s.safeDisableAgents?.includes(agent) && skillAgents(s).includes(agent);
        const safeCodex = guarded && agent === "codex" && codexToggleSafe;
        const backupModifiedCopy = safeDisable && agent !== "codex";
        const batchSuccess = batchOperations.find((operation) => operation.agent === agent && operation.successfulSkillIds?.includes(s.id));
        return <AgentToggle agent={agent} key={agent} name={s.name || s.id}
          enabled={s.installed && !(agent === "codex" && statusError) && (batchSuccess ? batchSuccess.enabled : skillAgents(s).includes(agent))}
          statusUnknown={agent === "codex" && !!statusError}
          disabled={busy || !s.installed || s.bundled || (agent === "codex" && !!statusError) || (guarded && skillAgents(s).includes(agent) && !safeDisable && !safeCodex) || skillBatchHasPending(batchOperations, [s], agent)}
          title={agentSkillsDirs?.[agent]}
          hint={agent === "codex" && statusError ? tf("状态未知：{0}", [statusError]) : backupModifiedCopy ? t("停用并备份已修改的受控副本") : (safeCodex || (safeDisable && agent === "codex")) ? t("只修改 Codex 的技能启用状态") : conflict ? s.conflict || t("同名 Skill 内容存在冲突") : guarded ? t("请按安装位置管理") : !s.installed ? t("未安装") : s.bundled ? t("Codex 自带，只读") : undefined}
          onToggle={async () => {
            if (needsSync && !skillAgents(s).includes(agent)) {
              if (await actions.syncCopies(s)) await actions.run("set_skill_enabled_for", {agent, id: s.id, enabled: true});
              return;
            }
            if ((guarded || s.managedSourceStatus === "different") && !safeCodex && !skillAgents(s).includes(agent)) { actions.setVersionTask({ id: s.id, mode: "enable", agent }); return; }
            if (backupModifiedCopy && !(await appConfirm(t("此受控副本已修改，停用时将移入备份。"), {
              title: t("停用并备份"), confirmText: t("停用并备份"),
            }))) return;
            await actions.run("set_skill_enabled_for", { agent, id: s.id, enabled: backupModifiedCopy ? false : !skillAgents(s).includes(agent) });
          }} />;
      })}
    </div>
    <Menu>
      <MenuItem onClick={() => actions.openDetails(s)}>{t("查看详情")}</MenuItem>
      <MenuItem disabled={!s.installed || s.bundled} onClick={() => actions.openDetails(s, true)}>{t("安装位置与来源")}</MenuItem>
      <MenuItem onClick={() => actions.openRepo(s.repoKey)}>{t("管理仓库")}</MenuItem>
      {needsSync && <MenuItem title={syncCopiesHint} disabled={busy} onClick={() => void actions.syncCopies(s)}>{t("同步工作副本")}</MenuItem>}
      {s.installed && !s.bundled && (guarded || s.managedSourceStatus === "different") && <MenuItem onClick={() => actions.setVersionTask({ id: s.id, mode: "resolve" })}>{t("解决冲突")}</MenuItem>}
      {!s.bundled && (!s.conflict || s.instanceKey) && customGroupNames.length ? <>
        <div className="mg-menu-label">{t("分组")}</div>
        <MenuItem checked={!assignment} onClick={() => actions.assignGroup(s, "")}>{t("按来源")}</MenuItem>
        {customGroupNames.map((group) => <MenuItem checked={assignment === customSkillGroupKey(group)} key={group} onClick={() => actions.assignGroup(s, group)}>{group}</MenuItem>)}
      </> : null}
      <MenuItem danger disabled={!s.installed || s.bundled || busy || Boolean(update)} onClick={async () => {
        if (!canUninstall) { actions.setVersionTask({ id: s.id, mode: "archive" }); return; }
        if (await appConfirm(`${t("卸载并备份此 Skill？")}\n${s.name || s.id}`, { title: t("卸载"), danger: true })) await actions.run("uninstall_skill", { id: s.id });
      }}>{t("卸载")}</MenuItem>
    </Menu>
  </CxRow>;
}
const SkillRow = memo(SkillRowView);

/**
 * Skills 数据的读取与缓存：先读本地已安装清单并画出来，再在后台刷新仓库目录（发现）。
 * 离开页面再回来先用上次的结果（skillPageCache），30 秒内不重复扫描本地；后台操作结束后重读本地。
 * 返回的 dataRequest / applyCurrentPayload 供页面上的写操作按请求序号丢弃过期结果。
 */
function useSkillsData(notify: Notify) {
  const [dataState, setDataState] = useState<{ version: number; value: SkillPayload | null }>(() => ({ version: 0, value: skillPageCache.value }));
  const data = dataState.value;
  useEffect(() => { skillPageCache.value = data; }, [data]);
  const batchOwner = useRef(Symbol("skills-page"));
  const mounted = useRef(false);
  const loadingRequests = useRef(0);
  const catalogRequests = useRef(0);
  const dataRequest = useRef(0);
  const localReload = useRef<{ pending: Promise<void> | null; dirty: boolean }>({ pending: null, dirty: false });
  const [loading, setLoading] = useState(false);
  const [initialLocalLoaded, setInitialLocalLoaded] = useState(false);
  const [refreshingCatalog, setRefreshingCatalog] = useState(() => !!skillPageCache.catalogPending);
  // 读取失败的原因：手上还没有任何数据时页面显示它和「重试」，不混进「没有匹配的 Skill」。
  const [loadError, setLoadError] = useState("");
  const requestPayload = (command: string) => {
    const local = command === "list_installed_skills";
    const pendingKey = local ? "localPending" : "catalogPending";
    const startedKey = local ? "localStartedAt" : "catalogStartedAt";
    let pending = skillPageCache[pendingKey];
    if (!pending) {
      const startedAt = Date.now();
      skillPageCache[startedKey] = startedAt;
      pending = invoke<SkillPayload>(command);
      skillPageCache[pendingKey] = pending;
      const completed = () => {
        if (skillPageCache[pendingKey] !== pending) return;
        skillPageCache[pendingKey] = null;
        if (skillPageCache[startedKey] === startedAt) {
          skillPageCache[startedKey] = skillPageCache.mounted ? Date.now() : 0;
        }
      };
      void pending.then(completed, completed);
    }
    return pending;
  };
  const reloadLocal = () => {
    const state = localReload.current;
    if (state.pending) { state.dirty = true; return state.pending; }
    state.pending = (async () => {
      try {
        do {
          state.dirty = false;
          const pending = skillPageCache.localPending;
          if (pending) {
            try { await pending; }
            catch (error) { if (mounted.current) await notify("Skills", String(error), "failed"); }
            if (!mounted.current) return;
          }
          state.dirty = false;
          const request = ++dataRequest.current;
          try {
            const result = await requestPayload("list_installed_skills");
            if (!mounted.current) return;
            if (request === dataRequest.current) applyCurrentPayload(request, result);
            setLoadError(ok(result) ? "" : result.message);
            if (!ok(result)) await notify("Skills", result.message, "failed");
          } catch (error) {
            if (mounted.current) { setLoadError(String(error)); await notify("Skills", String(error), "failed"); }
          }
        } while (state.dirty && mounted.current);
      } finally {
        state.pending = null;
      }
    })();
    return state.pending;
  };
  const applyCurrentPayload = (request: number, payload: SkillPayload, reloadIfStale = true) => {
    if (!mounted.current) return;
    if (request !== dataRequest.current) { if (reloadIfStale) void reloadLocal(); return; }
    setDataState((current) => {
      if (!mounted.current || request !== dataRequest.current || request < current.version) return current;
      const previous = current.value;
      const value = previous ? {
        ...payload,
        ...(!payload.inventoryOk && { skills: previous.skills, repos: previous.repos,
          repositorySkills: previous.repositorySkills, repoCounts: previous.repoCounts }),
        ...(!payload.backupsOk && { backups: previous.backups }),
      } : payload;
      return { version: request, value };
    });
  };
  const load = async (command = "refresh_skill_catalog", automatic = false) => {
    const local = command === "list_installed_skills";
    const pendingKey = local ? "localPending" : "catalogPending";
    const startedKey = local ? "localStartedAt" : "catalogStartedAt";
    if (automatic && local && skillPageCache.value && skillBatchRegistry.getSnapshot().length) return;
    if (automatic && !skillPageCache[pendingKey] && skillPageCache.value
      && (local ? Date.now() - skillPageCache[startedKey] < 30_000 : skillPageCache.catalogStartedAt > 0)) return;
    if (local) {
      loadingRequests.current++;
      setLoading(true);
    } else {
      catalogRequests.current++;
      setRefreshingCatalog(true);
    }
    const request = ++dataRequest.current;
    try {
      const r = await requestPayload(command);
      if (local && !skillPageCache.localStartedAt) {
        if (mounted.current) void reloadLocal();
      } else if (local) {
        applyCurrentPayload(request, r, false);
      } else {
        startTransition(() => applyCurrentPayload(request, r, !local));
      }
      if (mounted.current) setLoadError(ok(r) ? "" : r.message);
      if (mounted.current && !ok(r)) await notify("Skills", r.message, "failed");
    } catch (e) {
      if (mounted.current) { setLoadError(String(e)); await notify("Skills", String(e), "failed"); }
    } finally {
      if (local) {
        loadingRequests.current--;
        if (mounted.current && !loadingRequests.current) setLoading(false);
      } else {
        catalogRequests.current--;
        if (mounted.current && !catalogRequests.current) setRefreshingCatalog(false);
      }
    }
  };
  useEffect(() => {
    let active = true;
    mounted.current = true;
    skillPageCache.mounted++;
    const catalogPending = skillPageCache.catalogPending;
    if (catalogPending) void load("refresh_skill_catalog", true);
    else setRefreshingCatalog(false);
    void (async () => {
      await load("list_installed_skills", true);
      const pending = skillPageCache.localPending;
      if (active && pending) {
        try { await pending; }
        catch (error) { if (mounted.current) await notify("Skills", String(error), "failed"); }
      }
      if (active && !catalogPending) setInitialLocalLoaded(true);
    })();
    return () => { active = false; mounted.current = false; skillPageCache.mounted--; dataRequest.current++; };
  }, []);
  useEffect(() => {
    if (!initialLocalLoaded) return;
    // 本地结果提交并绘制后再启动首次发现，避免新请求使尚未提交的本地结果失效。
    let timer: ReturnType<typeof setTimeout> | undefined;
    const frame = requestAnimationFrame(() => {
      timer = setTimeout(() => void load("refresh_skill_catalog", true), 0);
    });
    return () => { cancelAnimationFrame(frame); clearTimeout(timer); };
  }, [initialLocalLoaded]);
  useEffect(() => skillBatchRegistry.subscribeSettled((owner, batchProgress) => {
    if (mounted.current && owner !== batchOwner.current && !batchProgress) return reloadLocal();
  }), []);
  return { data, setDataState, batchOwner, mounted, dataRequest, loading, refreshingCatalog, loadError, load, reloadLocal, applyCurrentPayload };
}

export function SkillsPage({ notify }: { notify: Notify }) {
  const { data, setDataState, batchOwner, mounted, dataRequest, loading, refreshingCatalog, loadError, load, reloadLocal, applyCurrentPayload } = useSkillsData(notify);
  const [tab, setTab] = useState<"discover" | "installed">("installed");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const batchOperations = useSyncExternalStore(skillBatchRegistry.subscribe, skillBatchRegistry.getSnapshot, skillBatchRegistry.getSnapshot);
  const updates = useSyncExternalStore(skillBatchRegistry.subscribeUpdates, skillBatchRegistry.getUpdates, skillBatchRegistry.getUpdates);
  const updateFailures = useSyncExternalStore(skillBatchRegistry.subscribeFailures, skillBatchRegistry.getFailures, skillBatchRegistry.getFailures);
  const [repoOpen, setRepoOpen] = useState(false);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [showLocations, setShowLocations] = useState(false);
  const [versionTask, setVersionTask] = useState<VersionTask | null>(null);
  const [detailDates, setDetailDates] = useState<{ id: string; dates: Record<string, number>; errors: Record<string, string> } | null>(null);
  const exportBackup = (backup: SkillBackup) => setVersionTask({ id: backup.skillId, mode: "export", backup: backup.id });
  const [matchTarget, setMatchTarget] = useState<({ id?: string; localPath?: string } & SkillMatchContext) | null>(null);
  const [duplicatesOpen, setDuplicatesOpen] = useState(false);
  const [backupsOpen, setBackupsOpen] = useState(false);
  const [bundleExportOpen, setBundleExportOpen] = useState(false);
  const [updateTarget, setUpdateTarget] = useState<{ id: string; repoKey: string; localPath?: string } | null>(null);
  const [repoSelection, setRepoSelection] = useState<string | null>(null);
  // 当前使用模式下隐藏的 Agent 不占列：只管理 Codex 时只有 Codex，只用增强功能时是 Codex、Claude Code。
  const focus = useAgentFocus();
  // 记住这一列数组：它是每个 SkillRow 的属性，每次渲染都新建会让行的 memo 失效。
  const agentColumns: McpAgentId[] = useMemo(() => MCP_AGENTS.filter((agent) => agentVisible(focus, agent)), [focus]);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  // 每组展开了多少行（默认 SKILL_GROUP_PAGE）。
  const [rowLimits, setRowLimits] = useState<Record<string, number>>({});
  const [storedGroups] = useState(() => readSkillGroupStorage(localStorage.getItem(SKILL_GROUP_STORAGE_KEY)));
  const [customGroupNames, setCustomGroupNames] = useState<string[]>(storedGroups.names);
  const [customGroupAssignments, setCustomGroupAssignments] = useState<Record<string, string>>(storedGroups.assignments);
  const [assignmentsByIdentity, setAssignmentsByIdentity] = useState<Record<string, string>>(storedGroups.assignmentsByIdentity);
  const [groupOrder, setGroupOrder] = useState<string[]>(storedGroups.order);
  const [groupSortDirection, setGroupSortDirection] = useState<SkillGroupSortDirection | null>(storedGroups.sortDirection);
  const [groupAliases] = useState<Record<string, string>>(storedGroups.aliases);
  useEffect(() => {
    const stored = readSkillGroupStorage(localStorage.getItem(SKILL_GROUP_STORAGE_KEY));
    localStorage.setItem(SKILL_GROUP_STORAGE_KEY, JSON.stringify({ ...stored, names: customGroupNames, assignments: customGroupAssignments, assignmentsByIdentity, order: groupOrder, sortDirection: groupSortDirection }));
  }, [customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupSortDirection]);
  const groupSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const openDetails = (skill: Skill, locations = false) => {
    setDetail(skill); setShowLocations(locations);
    void load("list_installed_skills", true);
  };
  // 从详情打开的子抽屉（关联来源、更新预览、仓库、受控源）关上后回到这个 Skill 的详情，不丢掉用户正在看的东西。
  const [returnDetail, setReturnDetail] = useState<{ skill: Skill; locations: boolean } | null>(null);
  const leaveDetail = () => {
    if (detail) setReturnDetail({ skill: detail, locations: showLocations });
    setDetail(null);
  };
  const backToDetail = () => {
    if (!returnDetail) return;
    setDetail(returnDetail.skill); setShowLocations(returnDetail.locations); setReturnDetail(null);
  };
  const run = async (command: string, args: Record<string, unknown>) => {
    const uninstallId = command === "uninstall_skill" && typeof args.id === "string" ? args.id : null;
    if (uninstallId && !skillBatchRegistry.reserveUninstall(uninstallId)) return false;
    setBusy(true);
    try {
      const r = await skillBatchRegistry.run(async () => {
        try {
          const request = ++dataRequest.current;
          const result = await invoke<SkillPayload>(command, args);
          applyCurrentPayload(request, result);
          return result;
        } finally { skillBatchRegistry.notifySettled(batchOwner.current); }
      });
      await notify("Skills", r.message, ok(r) ? "ok" : "failed");
      return ok(r);
    } catch (e) {
      await notify("Skills", String(e), "failed");
      return false;
    } finally {
      if (uninstallId) skillBatchRegistry.releaseUninstall(uninstallId);
      setBusy(false);
    }
  };
  const queueUpdate = (command: "install_skill" | "update_skill", args: { repoKey: string; id: string }) => {
    return skillBatchRegistry.enqueueUpdate(args.id, command, async () => {
      let outcome: { success: boolean; message: string };
      try {
        if (command === "update_skill") {
          const checkRequest = ++dataRequest.current;
          const current = await invoke<SkillPayload>("list_installed_skills");
          if (!current.inventoryOk) throw new Error(current.message);
          const installed = current.skills.filter((skill) => skill.id === args.id && skill.installed && !skill.bundled);
          if (installed.length !== 1 || installed[0].conflict) {
            applyCurrentPayload(checkRequest, current);
            throw new Error(t("同名 Skill 内容存在冲突，已跳过更新"));
          }
          if (!current.skills.some((skill) => skill.id === args.id && skill.repoKey === args.repoKey && skill.installed && !skill.bundled)) {
            applyCurrentPayload(checkRequest, current);
            throw new Error(t("Skill 已卸载或来源已变更，已跳过更新"));
          }
        }
        const request = ++dataRequest.current;
        const result = await invoke<SkillPayload>(command, args);
        applyCurrentPayload(request, result);
        outcome = { success: ok(result), message: result.message };
      } catch (error) {
        outcome = { success: false, message: String(error) };
      } finally {
        skillBatchRegistry.notifySettled(batchOwner.current);
      }
      skillBatchRegistry.recordUpdateOutcome(args.id, outcome, (summary) => {
        const message = tf("已完成 {0} 个 Skill 操作", [summary.completed]);
        void notify("Skills", summary.failed ? `${message}；${tf("{0} 个 Skill 失败", [summary.failed])}：${summary.details.join("；")}` : message, summary.failed ? "failed" : "ok");
      }, args.repoKey);
    }, args.repoKey);
  };
  const queueGroupUpdates = (skills: Skill[]) => {
    let skipped = 0;
    for (const skill of skills) if (!queueUpdate("update_skill", { repoKey: skill.repoKey, id: skill.id })) skipped++;
    if (skipped) void notify("Skills", tf("跳过 {0} 个已在操作中的 Skill", [skipped]), "ok");
  };
  const runBatch = async (group: SkillGroup<Skill>, agent: McpAgentId, enabled: boolean) => {
    const groupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
    if (skillBatchHasPending(skillBatchRegistry.getSnapshot(), groupItems, agent)) return;
    const targets = skillBatchTargets(group.items.map((skill) => batchSkill(skill, agent)), agent, enabled);
    if (!targets.length) {
      await notify("Skills", enabled ? t("该组没有可启用的 Skill") : t("该组没有可停用的 Skill"), "ok");
      return;
    }
    let operation: PendingSkillBatchOperation = { groupKey: group.key, agent, enabled, skillIds: targets.map((skill) => skill.id), settledSkillIds: [], successfulSkillIds: [], owner: batchOwner.current };
    skillBatchRegistry.add(operation);
    let completed = 0;
    let failed = 0;
    let acceptedInventory = false;
    const errors: string[] = [];
    const recordFailure = (skill: Skill, reason: string) => {
      failed++;
      if (errors.length >= 3) return;
      const detail = `${skill.name || skill.id}：${reason}`;
      errors.push(detail.length > 240 ? `${detail.slice(0, 239).replace(/[\uD800-\uDBFF]$/, "")}…` : detail);
    };
    try {
      for (const [index, skill] of targets.entries()) {
        const last = index === targets.length - 1;
        let success = false;
        try {
          const { request, result } = await skillBatchRegistry.run(async () => {
            const request = ++dataRequest.current;
            const result = await invoke<Result & Partial<SkillPayload>>("set_skill_enabled_for", {
              agent, id: skill.id, enabled, ...(!last && { includeInventory: false }),
            });
            return { request, result };
          });
          if (last && mounted.current && request === dataRequest.current && result.skills) {
            applyCurrentPayload(request, result as SkillPayload);
            acceptedInventory = Boolean(result.inventoryOk);
          } else if (mounted.current && request === dataRequest.current && ok(result)) {
            setDataState((current) => {
              if (!mounted.current || request < current.version || !current.value) return current;
              const skills = result.skills || current.value.skills.filter((item) => item.id === skill.id).map((item) => {
                const agents = skillAgents(item).filter((value) => value !== agent);
                if (enabled) agents.push(agent);
                return { ...item, agents, enabled: agent === "codex" ? enabled : item.enabled };
              });
              return { version: request, value: { ...current.value, skills: mergeSkillAgentResult(current.value.skills, skills, skill.id, agent) } };
            });
          } else if (mounted.current && request !== dataRequest.current) void reloadLocal();
          if (ok(result)) { completed++; success = true; }
          else recordFailure(skill, result.message);
        } catch (error) { recordFailure(skill, String(error)); }
        finally {
          const previous = operation;
          operation = { ...operation, settledSkillIds: [...operation.settledSkillIds, skill.id],
            successfulSkillIds: success ? [...operation.successfulSkillIds!, skill.id] : operation.successfulSkillIds };
          skillBatchRegistry.replace(previous, operation);
        }
      }
    } finally {
      try {
        await Promise.all([
          skillBatchRegistry.notifySettledAndWait(batchOwner.current),
          mounted.current && !acceptedInventory ? reloadLocal() : Promise.resolve(),
        ]);
        if (mounted.current && acceptedInventory) skillPageCache.localStartedAt = Date.now();
      } finally {
        skillBatchRegistry.remove(operation);
      }
    }
    const message = tf("已批量{0} {1} 个 Skill", [enabled ? t("启用") : t("停用"), completed]);
    await notify("Skills", failed ? `${message}；${tf("{0} 个 Skill 失败", [failed])}：${errors.join("；")}` : message, failed ? "failed" : "ok");
  };
  const saveRepo = async (repo: Repo, previousKey?: string) => {
    setBusy(true);
    try {
      const result = await skillBatchRegistry.run(async () => {
        try {
          const request = ++dataRequest.current;
          const result = await invoke<SkillPayload>("upsert_skill_repo", { repo, previousKey });
          applyCurrentPayload(request, result);
          return result;
        } finally { skillBatchRegistry.notifySettled(batchOwner.current); }
      });
      if (!ok(result)) await notify("Skills", result.message, "failed");
      return ok(result);
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return false;
    } finally {
      setBusy(false);
    }
  };
  // 从本地目录导入（#511）：可以选单个 Skill 目录（含 SKILL.md），也可以选放了多个 Skill 的父目录。
  const importLocalSkills = async () => {
    let picked: string | string[] | null;
    try {
      picked = await openDialog({ directory: true, multiple: false, title: t("选择 Skill 目录（含 SKILL.md）或其父目录") });
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return;
    }
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    // 导入的 Skill 是已安装状态，切到「已安装」才能看到。
    if (await run("import_local_skills", { path })) setTab("installed");
  };
  // 迁移包（#2852）：选中的分组连同 Skill 文件、分组、各 Agent 的启用状态导出成一个 zip。
  const exportSkillBundle = async (selected: ReadonlySet<string>) => {
    const request = skillBundleSelections(bundleGroups, selected);
    if (!request.skills.length) return;
    let path: string | null;
    try {
      path = await saveDialog({ title: t("导出 Skill 迁移包"), defaultPath: "codex-plus-skills.zip", filters: [{ name: "ZIP", extensions: ["zip"] }] });
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return;
    }
    if (!path) return;
    setBusy(true);
    try {
      const result = await invoke<Result>("export_skill_bundle", { path, skills: request.skills, groups: request.groups });
      await notify("Skills", result.message, ok(result) ? "ok" : "failed");
      if (ok(result)) setBundleExportOpen(false);
    } catch (error) {
      await notify("Skills", String(error), "failed");
    } finally {
      setBusy(false);
    }
  };
  // 导入迁移包：后端装文件、恢复各 Agent 的启用状态；分组存在本窗口里，按返回的清单写回。
  const importSkillBundle = async () => {
    let picked: string | string[] | null;
    try {
      picked = await openDialog({ directory: false, multiple: false, title: t("选择 Skill 迁移包"), filters: [{ name: "ZIP", extensions: ["zip"] }] });
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return;
    }
    const path = Array.isArray(picked) ? picked[0] : picked;
    if (!path) return;
    setBusy(true);
    try {
      const result = await skillBatchRegistry.run(async () => {
        try {
          const request = ++dataRequest.current;
          const response = await invoke<SkillBundleImportResult>("import_skill_bundle", { path });
          applyCurrentPayload(request, response);
          return response;
        } finally { skillBatchRegistry.notifySettled(batchOwner.current); }
      });
      const bundle = result.bundle;
      if (bundle?.skills.length) {
        // 三项各自只依赖自己的旧值，分开用函数式更新，不覆盖期间发生的其它分组改动。
        const blank = { names: [], assignmentsByIdentity: {}, order: [] };
        setCustomGroupNames((old) => applySkillBundleGroups(bundle, result.skills, { ...blank, names: old }).names);
        setAssignmentsByIdentity((old) => applySkillBundleGroups(bundle, result.skills, { ...blank, assignmentsByIdentity: old }).assignmentsByIdentity);
        setGroupOrder((old) => applySkillBundleGroups(bundle, result.skills, { ...blank, order: old }).order);
        setTab("installed");
      }
      await notify("Skills", result.message, ok(result) ? "ok" : "failed");
    } catch (error) {
      await notify("Skills", String(error), "failed");
      void reloadLocal();
    } finally {
      setBusy(false);
    }
  };
  const deleteRepo = async (repo: Repo) => { await run("delete_skill_repo", { key: skillRepoKey(repo) }); };
  const inspectRepo = async (repo: Repo) => {
    const result = await invoke<RepoInspection>("inspect_skill_repo", { repo });
    if (ok(result)) await reloadLocal();
    return result;
  };
  const addMatchingRepo = async (repo: Repo) => {
    try {
      const result = await inspectRepo(repo);
      if (!ok(result)) { await notify("Skills", result.message, "failed"); return null; }
      return await saveRepo(result.repo) ? result.repo : null;
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return null;
    }
  };
  const applySourceMatches = async (selections: SourceSelection[]): Promise<Result> => {
    setBusy(true);
    try {
      const result = await skillBatchRegistry.run(async () => {
        try {
          const request = ++dataRequest.current;
          const response = await invoke<SourceApplyResult>("apply_skill_repository_matches", { selections });
          if (response.skills) applyCurrentPayload(request, response);
          else void reloadLocal();
          return response;
        } finally { skillBatchRegistry.notifySettled(batchOwner.current); }
      });
      if (result.applied?.length) setAssignmentsByIdentity((old) => {
        const changes = result.applied.map((item) => ({
          ...item, localPath: item.localPath || selections.find((selection) => selection.localId === item.localId)?.localPath,
        }));
        const next = migrateSkillGroupAssignments(old, changes.filter((item) => !item.localPath).map((item) => ({
          id: item.localId, fromRepoKey: item.fromRepoKey, toRepoKey: item.toRepoKey,
        })));
        for (const change of changes) {
          if (!change.localPath) continue;
          const includesPath = (skill: Skill) => skill.id === change.localId
            && skill.locations?.some((location) => location.originalPath === change.localPath || location.associationPath === change.localPath);
          const previous = data?.skills.find(includesPath);
          const current = result.skills?.find(includesPath);
          if (!previous || !current) continue;
          const from = skillGroupIdentityKey(previous);
          const to = skillGroupIdentityKey(current);
          const value = Object.prototype.hasOwnProperty.call(next, from) ? next[from]
            : skillGroupAssignment(previous, customGroupNames, customGroupAssignments, next);
          if (from !== to && (Object.prototype.hasOwnProperty.call(next, from) || value)
            && !Object.prototype.hasOwnProperty.call(next, to)) next[to] = value;
        }
        return next;
      });
      await notify("Skills", result.message, ok(result) ? "ok" : "failed");
      return result;
    } catch (error) {
      const message = String(error);
      await notify("Skills", message, "failed");
      void reloadLocal();
      return { status: "failed", message };
    } finally { setBusy(false); }
  };
  const restoreBackup = async (backup: SkillBackup) => {
    const destination = backup.restorePath ? `\n${backup.restorePath}` : "";
    if (!(await appConfirm(`${t("恢复此备份？")}${destination}`, { title: t("恢复 Skill 备份"), confirmText: t("恢复") }))) return;
    await run("restore_skill_backup", { backupId: backup.id });
  };
  const deleteBackup = async (backup: SkillBackup) => {
    const path = backup.backupPath ? `\n${backup.backupPath}` : "";
    const warning = backup.deleteWarning ? `\n\n${backup.deleteWarning}` : "";
    const message = backup.missing
      ? `${tf("清除已丢失备份“{0}”的记录？", [backup.name || backup.skillId])}${path}\n\n${t("此操作不会删除其他文件。")}${warning}`
      : `${tf("删除 Skill 备份“{0}”？", [backup.name || backup.skillId])}${path}${warning}`;
    if (!(await appConfirm(message, { title: t(backup.missing ? "清除备份记录" : "删除 Skill 备份"), confirmText: t(backup.missing ? "清除记录" : "删除"), danger: true }))) return;
    await run("delete_skill_backup", { backupId: backup.id, missingOnly: !!backup.missing });
  };
  const openRepo = (key: string) => { setRepoSelection(key); setDetail(null); setRepoOpen(true); void load("list_installed_skills", true); };
  const deferredQuery = useDeferredValue(query);
  const conflictIds = useMemo(() => {
    const result = new Set<string>();
    for (const skill of data?.skills || []) {
      if (skill.conflict) result.add(skill.id);
    }
    return result;
  }, [data]);
  const guardedIds = useMemo(() => {
    const counts = new Map<string, number>();
    const result = new Set(conflictIds);
    for (const skill of data?.skills || []) {
      if (skill.installed && !skill.bundled) counts.set(skill.id, (counts.get(skill.id) || 0) + 1);
    }
    for (const [id, count] of counts) if (count > 1) result.add(id);
    return result;
  }, [data, conflictIds]);
  const knownSkills = useMemo(() => data?.skills || [], [data]);
  const canToggleSharedCodex = (skill: Skill) => Boolean(skill.locations?.some((location) => location.rootKind === "shared")
    && knownSkills.filter((item) => item.id === skill.id && item.installed
      && item.locations?.some((location) => location.rootKind === "codex" || location.rootKind === "shared")).length === 1);
  const canToggleCodex = (skill: Skill) => canToggleSharedCodex(skill)
    || (skill.displayInstances || [skill]).some((instance) => instance.safeToggleAgents?.includes("codex"));
  const batchSkill = (skill: Skill, agent: McpAgentId): Skill => {
    if (skill.managedSourceStatus === "needs-sync" && !skillAgents(skill).includes(agent)) return {...skill, conflict: t("请先同步工作副本")};
    if (!guardedIds.has(skill.id)) return skill;
    return { ...skill, conflict: agent === "codex" && canToggleCodex(skill) ? null : skill.conflict || t("请按安装位置管理") };
  };
  useEffect(() => {
    setAssignmentsByIdentity((old) => migrateSkillInstanceAssignments(old, customGroupAssignments, knownSkills, customGroupNames));
  }, [knownSkills, customGroupAssignments, customGroupNames]);
  const knownRepositories = useMemo(() => data?.repos || [], [data]);
  const builtinGroupLabel = t("内置");
  const userGroupLabel = t("用户");
  const localGroupLabel = t("本地导入");
  const allGroups = useMemo(() => {
    const result = groupSkills(knownSkills, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, true, groupAliases, assignmentsByIdentity, knownRepositories)
      .map((group) => ({ ...group, items: aggregateSkillGroupItems(group.items) }));
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupSortDirection, groupAliases, builtinGroupLabel, userGroupLabel, localGroupLabel]);
  const bundleGroups = useMemo(() => skillBundleExportGroups(allGroups), [allGroups]);
  // 当前视图的分组：在 allGroups 上按页签和搜索词筛选，不再把全部 Skill 再分组、合并一遍。
  // 只依赖 deferredQuery，输入时先更新输入框，列表在空闲时再跟上。
  const groups = useMemo(() => {
    const searching = Boolean(deferredQuery.trim());
    const inTab = (skill: Skill) => (tab === "installed" ? skill.installed : !skill.installed || skill.bundled);
    return allGroups.map((group) => ({ ...group, items: group.items.filter((skill) => inTab(skill)
      && (skill.displayInstances || [skill]).some((item) => matchesManagementQuery(
        [item.id, item.name, item.description, item.repoKey, sourceLabel(item), ...(item.locations || []).map((location) => location.manifestPath)], deferredQuery))) }))
      .filter((group) => (tab !== "installed" || group.kind !== "repo" || group.items.length > 0)
        && (!searching || group.items.length));
  }, [allGroups, deferredQuery, tab]);
  const openUrl = async (url: string) => {
    try { await invoke("open_external_url", { url }); }
    catch (e) { await notify("Skills", String(e), "failed"); }
  };
  const createCustomGroup = async () => {
    const normalized = (await appPrompt(t("新建 Skill 分组"), { placeholder: t("分组名称") }))?.trim();
    if (!normalized || customGroupNames.includes(normalized)) return;
    setCustomGroupNames((old) => [...old, normalized]);
  };
  const renameCustomGroup = async (oldName: string) => {
    const normalized = (await appPrompt(t("重命名 Skill 分组"), { defaultValue: oldName, placeholder: t("分组名称") }))?.trim();
    if (!normalized || normalized === oldName || customGroupNames.includes(normalized)) return;
    setCustomGroupNames((old) => old.map((item) => item === oldName ? normalized : item));
    setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).map(([id, group]) => {
      const name = legacySkillCustomGroupName(group, customGroupNames);
      return [id, name === oldName ? normalized : name ?? group];
    })));
    setAssignmentsByIdentity((old) => Object.fromEntries(Object.entries(old).map(([key, group]) => [key,
      group === customSkillGroupKey(oldName) ? customSkillGroupKey(normalized) : group])));
    setGroupOrder((old) => old.map((key) => key === customSkillGroupKey(oldName) ? customSkillGroupKey(normalized) : key));
    setExpandedGroups((old) => {
      const next = new Set(old);
      if (next.delete(customSkillGroupKey(oldName))) next.add(customSkillGroupKey(normalized));
      return next;
    });
  };
  const deleteCustomGroup = async (name: string) => {
    if (!(await appConfirm(tf("删除分组“{0}”？其中的 Skill 将回到来源分组。", [name]), { title: t("删除分组"), confirmText: t("删除"), danger: true }))) return;
    setCustomGroupNames((old) => old.filter((item) => item !== name));
    setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).filter(([, group]) =>
      legacySkillCustomGroupName(group, customGroupNames) !== name)));
    setAssignmentsByIdentity((old) => Object.fromEntries(Object.entries(old).map(([key, group]) => [key,
      group === customSkillGroupKey(name) ? "" : group])));
    setGroupOrder((old) => old.filter((key) => key !== customSkillGroupKey(name)));
    setExpandedGroups((old) => { const next = new Set(old); next.delete(customSkillGroupKey(name)); return next; });
  };
  const assignSkillGroup = (skill: Skill, group: string) => {
    setAssignmentsByIdentity((old) => {
      const next = { ...old };
      for (const item of skill.displayInstances || [skill]) next[skillGroupIdentityKey(item)] = group ? customSkillGroupKey(group) : "";
      return next;
    });
  };
  const handleGroupDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id || query.trim() || busy) return;
    const movable = new Set(groups.filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key));
    if (!movable.has(String(active.id)) || !movable.has(String(over.id))) return;
    setGroupOrder(moveSkillGroup(allGroups, String(active.id), String(over.id)));
    setGroupSortDirection(null);
  };
  // 每个 Agent 已启用的 Skill 数只在数据变化时数一次（分组批量开关的悬停说明用），不再每组每次渲染都扫全部 Skill。
  const agentCounts = useMemo(() => {
    const counts = {} as Record<McpAgentId, number>;
    for (const agent of MCP_AGENTS) counts[agent] = new Set((data?.skills || []).filter((s) =>
      s.installed && (agent !== "codex" || !skillCodexStatusError(s)) && skillAgents(s).includes(agent)).map((s) => s.id)).size;
    return counts;
  }, [data]);
  // 行里的操作都经由这个引用调用最新的实现：传给 SkillRow 的 actions 对象本身不变，memo 才能生效。
  const rowActionImpl = useRef<SkillRowActions | null>(null);
  rowActionImpl.current = {
    openDetails,
    openRepo,
    setVersionTask,
    assignGroup: assignSkillGroup,
    install: (skill) => { queueUpdate("install_skill", { repoKey: skill.repoKey, id: skill.id }); },
    update: (skill, inPlace) => {
      if (inPlace) setUpdateTarget({ id: skill.id, repoKey: skill.repoKey, localPath: skill.originalDir || undefined });
      else queueUpdate("update_skill", { repoKey: skill.repoKey, id: skill.id });
    },
    syncCopies: async (skill) => {
      if (!await appConfirm(t(SYNC_COPIES_HINT), { title: t("同步工作副本") })) return false;
      try {
        const rows = await invoke<LocalSkillVersion[]>("list_skill_versions", {id: skill.id});
        if (rows.some((v) => v.comparisonError)) throw new Error(t("部分安装位置无法比较，请先处理后重试。"));
        return await run("manage_skill_version", {action: {kind: "syncCopies", id: skill.id, revisions: Object.fromEntries(rows.map(v => [v.root, v.revision]))}});
      } catch (e) { await notify("Skills", String(e), "failed"); return false; }
    },
    run,
  };
  const rowActions = useMemo<SkillRowActions>(() => ({
    openDetails: (...args) => rowActionImpl.current!.openDetails(...args),
    openRepo: (...args) => rowActionImpl.current!.openRepo(...args),
    setVersionTask: (...args) => rowActionImpl.current!.setVersionTask(...args),
    assignGroup: (...args) => rowActionImpl.current!.assignGroup(...args),
    install: (...args) => rowActionImpl.current!.install(...args),
    update: (...args) => rowActionImpl.current!.update(...args),
    syncCopies: (...args) => rowActionImpl.current!.syncCopies(...args),
    run: (...args) => rowActionImpl.current!.run(...args),
  }), []);
  const discoverCount = (data?.skills || []).filter((s) => !s.installed || s.bundled).length;
  const installedCount = aggregateSkillGroupItems((data?.skills || []).filter((s) => s.installed)).length;
  const pageLoading = tab === "installed" ? loading : refreshingCatalog || (!data && loading);
  const sortLabel = `${t("按名称排序")} (${groupSortDirection === "asc" ? "A-Z" : groupSortDirection === "desc" ? "Z-A" : "A-Z / Z-A"})`;
  const currentDetail = detail ? allGroups.flatMap((group) => group.items).find((skill) => skillIdentityKey(skill) === skillIdentityKey(detail))
    || (detail.installed || guardedIds.has(detail.id) ? null : detail) : null;
  // 详情「更新」一节的直接更新：和行里的「更新」按钮同一条件，只给由 Codex++ 管理更新的 Skill（原位维护的走预览）。
  const detailQueueUpdate = Boolean(currentDetail && currentDetail.installed && !currentDetail.bundled && currentDetail.updateAvailable
    && !currentDetail.conflict && !conflictIds.has(currentDetail.id) && !guardedIds.has(currentDetail.id)
    && !(currentDetail.originalDir && currentDetail.updateOwner !== "manager")
    && ((!currentDetail.originalDir && !(currentDetail.updateOwner === "" && currentDetail.installChannel)) || currentDetail.updateOwner === "manager")
    && !currentDetail.locations?.some((location) => location.rootKind === "shared")
    && !updates.some((item) => item.id === currentDetail.id));
  // 空状态分开说：读取中、读取失败、搜索无结果、还没有 Skill，各配合适的按钮。
  const visibleCount = groups.reduce((sum, group) => sum + group.items.length, 0);
  const readFailed = !data ? Boolean(loadError) : !data.inventoryOk && !data.skills.length;
  const refreshCommand = tab === "installed" ? "list_installed_skills" : "refresh_skill_catalog";
  const openRepoWorkspace = () => { setRepoOpen(true); void load("list_installed_skills", true); };
  const emptyState = visibleCount ? null
    : !data && pageLoading ? <CxEmpty title={t("正在读取 Skills…")} />
    : readFailed ? <CxEmpty title={t("Skills 读取失败")} detail={loadError || data?.message}><Button onClick={() => void load(refreshCommand)} disabled={pageLoading}>{t("重试")}</Button></CxEmpty>
    : deferredQuery.trim() ? <CxEmpty title={t("没有匹配的 Skill")} detail={tab === "installed" ? t("只搜索了已安装的 Skill，可以到「发现」里找。") : undefined}><Button onClick={() => setQuery("")}>{t("清除搜索")}</Button></CxEmpty>
    : pageLoading ? <CxEmpty title={t("正在扫描…")} />
    : tab === "installed"
      ? <CxEmpty title={t("还没有安装 Skill")} detail={t("从仓库安装，或导入本地已有的 Skill 目录。")}><Button primary onClick={openRepoWorkspace}>{t("管理仓库")}</Button><Button onClick={() => void importLocalSkills()} disabled={busy}>{t("从本地目录导入")}</Button></CxEmpty>
      : <CxEmpty title={t("还没有可安装的 Skill")} detail={t("添加 Skill 仓库后，仓库里的 Skill 会出现在这里。")}><Button primary onClick={openRepoWorkspace}>{t("管理仓库")}</Button></CxEmpty>;
  return <div className="mg-page cx-page mg-skills-page">
    <PageHeader icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load(refreshCommand)} disabled={busy || pageLoading}><RefreshCw size={14}/>{tab === "installed" ? t(pageLoading ? "刷新中…" : "刷新已安装") : t(pageLoading ? "刷新目录中" : "刷新目录")}</Button>
      <Menu>
        <MenuItem onClick={() => void importLocalSkills()} disabled={busy}>{t("从本地目录导入")}</MenuItem>
        <MenuItem onClick={() => void run("import_ccs_skills", {})} disabled={busy}>{t("从 cc-switch 导入")}</MenuItem>
        <MenuItem onClick={() => void importSkillBundle()} disabled={busy} title={t("导入另一台电脑导出的迁移包，恢复 Skill、分组和各 Agent 的启用状态")}>{t("导入迁移包")}</MenuItem>
        <MenuItem onClick={() => setBundleExportOpen(true)} disabled={busy || !data} title={t("把分组里的 Skill 连同分组、各 Agent 的启用状态导出成一个 zip，便于换电脑")}>{t("导出迁移包")}</MenuItem>
        <MenuItem onClick={() => setDuplicatesOpen(true)} disabled={busy}>{t("预览重复副本")}</MenuItem>
        <MenuItem onClick={() => setBackupsOpen(true)} disabled={busy}>{t("查看 Skill 备份")}</MenuItem>
      </Menu>
    </>} />
    <CxToolbar query={query} onQuery={setQuery} placeholder={t("搜索 Skills 或仓库")}>
      <button type="button" className="cx-primary" onClick={openRepoWorkspace}><Link size={15}/>{t("管理仓库")}</button>
    </CxToolbar>
    <CxFilters label="Skills" value={tab} onChange={(next) => { setTab(next); if (next === "installed" && tab !== next) void load("list_installed_skills", true); }}
      items={[{ id: "installed", label: t("已安装"), count: installedCount }, { id: "discover", label: t("发现"), count: discoverCount }]}
      trailing={<>
        <button type="button" className="cx-secondary cx-skill-match" title={t("为本地 Skill 找到对应的 GitHub 仓库来源")} aria-label={t("匹配仓库")}
          disabled={busy || !data} onClick={() => setMatchTarget({})}><GitBranch size={14} aria-hidden="true"/>{t("匹配仓库")}</button>
        <button type="button" className="mg-icon-button" title={t("新建分组")} aria-label={t("新建分组")}
          onClick={createCustomGroup}><FolderPlus size={16}/></button>
        <button type="button" className="mg-icon-button" disabled={busy || !data}
          aria-label={sortLabel}
          title={`${t("按名称排序")} (${groupSortDirection === "asc" ? "Z-A" : "A-Z"})`} onClick={() => {
            const direction = nextSkillGroupSortDirection(groupSortDirection);
            setGroupOrder(sortSkillGroupKeys(allGroups, direction));
            setGroupSortDirection(direction);
          }}>{groupSortDirection === "asc" ? <ArrowDownAZ size={16}/> : groupSortDirection === "desc" ? <ArrowDownZA size={16}/> : <ArrowDownUp size={16}/>}</button>
        {groups.length > 1 ? <>
          <button type="button" className="mg-icon-button" title={t("全部折叠")} aria-label={t("全部折叠")}
            onClick={() => setExpandedGroups((old) => { const next = new Set(old); groups.forEach((group) => next.delete(group.key)); return next; })}><ChevronsUp size={16}/></button>
          <button type="button" className="mg-icon-button" title={t("全部展开")} aria-label={t("全部展开")}
            onClick={() => setExpandedGroups(new Set(allGroups.map((group) => group.key)))}><ChevronsDown size={16}/></button>
        </> : null}
      </>} />
    {updateFailures && <CxBanner tone="warn" icon={<X />} title={<span role="status" aria-live="polite">{tf("{0} 次 Skill 操作失败", [updateFailures.count])}</span>}
      detail={<ul className="cx-skill-failures" tabIndex={0} aria-label={t("失败的 Skill 列表")}>{updateFailures.details.map((detail) => <li key={JSON.stringify([detail.repoKey, detail.id])}><span>{detail.repoKey}</span> · <strong>{detail.id}</strong>：{detail.message}</li>)}</ul>}>
      <button type="button" className="mg-icon-button" aria-label={t("关闭 Skill 失败摘要")} title={t("关闭 Skill 失败摘要")} onClick={skillBatchRegistry.clearFailures}><X size={16}/></button>
    </CxBanner>}
    {data?.repoErrors?.length ? <CxBanner tone="warn" icon={<Link />} title={t(data.inventoryOk && data.backupsOk ? "部分仓库读取失败" : "Skill 数据读取失败")} detail={<span className="cx-skill-repo-errors">{data.repoErrors.map((e) => <span key={e}>{e}</span>)}</span>} /> : null}
    <div className="cx-skill-groups" aria-busy={pageLoading} data-columns={agentColumns.length}>
      {emptyState ? <CxCard>{emptyState}</CxCard> : null}
      {groups.length ?
        <DndContext sensors={groupSensors} collisionDetection={closestCenter} onDragEnd={handleGroupDragEnd}>
          <SortableContext items={groups.map((group) => group.key)} strategy={verticalListSortingStrategy}>{groups.map((group) => {
        const collapsed = !expandedGroups.has(group.key) && !deferredQuery.trim();
        const shownItems = group.items.slice(0, rowLimits[group.key] ?? SKILL_GROUP_PAGE);
        const hiddenRows = group.items.length - shownItems.length;
        const allGroupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
        const updateTargets = allGroupItems.filter((skill) => skill.installed && !skill.bundled && !guardedIds.has(skill.id) && !skill.conflict
          && (!skill.originalDir || skill.updateOwner === "manager") && !(skill.updateOwner === "" && skill.installChannel)
          && !skill.locations?.some((location) => location.rootKind === "shared") && skill.updateAvailable);
        const pendingUpdates = allGroupItems.filter((skill) => skill.installed && !skill.bundled && updates.some((item) => item.command === "update_skill" && item.id === skill.id && item.repoKey === skill.repoKey)).length;
        const hasUpdates = pendingUpdates > 0 || updateTargets.length > 0;
        const updateLabel = `${group.label} · ${tf("更新组内可更新 Skill（{0}）", [updateTargets.length])}${pendingUpdates ? ` · ${tf("进行中或排队 {0} 项", [pendingUpdates])}` : ""}`;
        const movable = group.kind === "repo" || group.kind === "custom";
        return <SortableSkillGroup group={group} key={group.key} disabled={!movable || !!query.trim() || busy}>{(handle) => <>
          <header className="cx-skill-group-head">
            {handle}
            <button type="button" className="cx-skill-group-toggle" aria-expanded={!collapsed} onClick={() => setExpandedGroups((old) => { const next = new Set(old); next.has(group.key) ? next.delete(group.key) : next.add(group.key); return next; })}>
              <ChevronDown size={14} className={collapsed ? "mg-chevron-collapsed" : ""}/><strong title={group.label}>{group.label}</strong><span className="cx-section-count">{group.items.length}</span>
            </button>
            <div className="cx-skill-group-aside">
              {pendingUpdates > 0 && <small role="status">{tf("更新中 · {0}", [pendingUpdates])}</small>}
              {hasUpdates && <button type="button" className="mg-icon-button" aria-label={updateLabel} title={updateLabel} disabled={!updateTargets.length || pendingUpdates > 0}
                onClick={() => queueGroupUpdates(updateTargets)}><CloudDownload size={15}/></button>}
              <div className="cx-agent-toggles" role="group" aria-label={`${group.label} · ${t("全选")}`}>
                {agentColumns.map((agent) => <GroupAgentSelect group={group} agent={agent} key={agent}
                  busy={busy || skillBatchHasPending(batchOperations, allGroupItems, agent)} operations={batchOperations}
                  prepareSkill={batchSkill}
                  count={agentCounts[agent]}
                  onChange={(enabled) => { void runBatch(group, agent, enabled).catch((error) => notify("Skills", String(error), "failed")); }}/>)}
              </div>
              {group.kind === "custom" ? <Menu label={tf("{0} 的更多操作", [group.label])}>
                <MenuItem disabled={busy} onClick={() => renameCustomGroup(group.label)}>{t("重命名")}</MenuItem>
                <MenuItem danger disabled={busy} onClick={() => deleteCustomGroup(group.label)}>{t("删除分组")}</MenuItem>
              </Menu> : <span className="cx-skill-menu-spacer" aria-hidden="true" />}
            </div>
          </header>
          {/* 空分组只留标题（数量 0），不再画一张空卡片。 */}
          {!collapsed && group.items.length ? <CxCard>
            {shownItems.map((s) => <SkillRow key={skillIdentityKey(s)} skill={s} busy={busy} actions={rowActions} agentColumns={agentColumns}
              update={s.bundled ? undefined : updates.find((item) => item.id === s.id)}
              conflict={Boolean(s.conflict) || conflictIds.has(s.id)} guarded={Boolean(s.conflict) || conflictIds.has(s.id) || guardedIds.has(s.id)}
              codexToggleSafe={canToggleCodex(s)} batchOperations={batchOperations} customGroupNames={customGroupNames}
              assignment={skillGroupAssignment(s, customGroupNames, customGroupAssignments, assignmentsByIdentity)}
              agentSkillsDirs={data?.agentSkillsDirs} />)}
            {hiddenRows > 0 ? <button type="button" className="cx-session-more" onClick={() => setRowLimits((old) => ({ ...old, [group.key]: shownItems.length + SKILL_GROUP_PAGE }))}>{tf("再显示 {0} 个", [hiddenRows])}</button> : null}
          </CxCard> : null}
        </>}</SortableSkillGroup>;
      })}</SortableContext></DndContext> : null}
    </div>
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} initialKey={repoSelection} busy={busy} updates={updates} conflictIds={guardedIds} onClose={() => { setRepoOpen(false); backToDetail(); }} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(command, args) => { queueUpdate(command, args); }} />}
    {currentDetail && <SkillDetailDrawer skill={conflictIds.has(currentDetail.id) && !currentDetail.conflict ? { ...currentDetail, conflict: t("同名 Skill 内容存在冲突，请先确认各安装位置。") } : currentDetail}
      dates={detailDates?.id === currentDetail.id ? detailDates.dates : undefined} comparisonErrors={detailDates?.id === currentDetail.id ? detailDates.errors : undefined}
      onExpand={() => { const id = currentDetail.id;
        void invoke<LocalSkillVersion[]>("list_skill_versions", { id }).then(rows => setDetailDates({ id,
          dates: Object.fromEntries(rows.flatMap(row => row.modifiedAtMs === null ? [] : [[row.root, row.modifiedAtMs]])),
          errors: Object.fromEntries(rows.flatMap(row => row.comparisonError ? [[row.root, row.comparisonError]] : [])),
        }), e => notify("Skills", String(e), "failed")); }}
      showLocations={showLocations} onArchive={() => setVersionTask({ id: currentDetail.id, mode: "archive" })} onExport={exportBackup}
      onLocal={() => setVersionTask({ id: currentDetail.id, mode: "local" })}
      count={data?.repoCounts?.[currentDetail.repoKey]} backups={(data?.backups || []).filter((backup) => backup.skillId === currentDetail.id)} busy={busy}
      onClose={() => { setDetail(null); setReturnDetail(null); }} onOpenUrl={(url) => void openUrl(url)}
      onOpenRepo={() => { if (repoOpen) setDetail(null); else { leaveDetail(); openRepo(currentDetail.repoKey); } }}
      onManageSource={() => { leaveDetail(); setDuplicatesOpen(true); }}
      onMatch={(localPath, context) => { leaveDetail(); setMatchTarget({ id: currentDetail.id, localPath: localPath || (currentDetail.conflictKind === "independent" ? currentDetail.originalDir || undefined : undefined), ...context }); }}
      onRestore={(backup) => void restoreBackup(backup)}
      onDeleteBackup={(backup) => void deleteBackup(backup)}
      onUpdate={(repoKey, localPath) => { setUpdateTarget({ id: currentDetail.id, repoKey, localPath }); leaveDetail(); }}
      onQueueUpdate={detailQueueUpdate ? () => queueUpdate("update_skill", { repoKey: currentDetail.repoKey, id: currentDetail.id }) : undefined}
      />}
    {matchTarget && <SkillSourceMatchDrawer repos={data?.repos || []} id={matchTarget.id} localPath={matchTarget.localPath}
      agentLabel={matchTarget.agentLabel} currentRepoKey={matchTarget.currentRepoKey} currentRepoPath={matchTarget.currentRepoPath}
      localSkillIds={knownSkills.filter((skill) => skill.installed && !skill.bundled
        && (!skill.repoPath || !skill.repoKey.includes("/"))).map((skill) => skill.id)}
      onClose={() => { setMatchTarget(null); backToDetail(); }} onAddRepo={addMatchingRepo} onApply={applySourceMatches}/>}
    {duplicatesOpen && <SkillDuplicatesDrawer onClose={() => { setDuplicatesOpen(false); backToDetail(); }} onArchive={(previewId, id, kind) => run("archive_skill_duplicate", { previewId, id, kind })}/>}
    {bundleExportOpen && <SkillBundleExportDrawer groups={bundleGroups} busy={busy} onClose={() => setBundleExportOpen(false)} onExport={(selected) => void exportSkillBundle(selected)}/>}
    {backupsOpen && <SkillBackupsDrawer backups={data?.backups || []} busy={busy} onClose={() => setBackupsOpen(false)} onRestore={(backup) => void restoreBackup(backup)} onDelete={(backup) => void deleteBackup(backup)} onExport={exportBackup}/>}
    {versionTask && <SkillVersionPicker task={versionTask} roots={{ ...data?.agentSkillsDirs, ...(data?.codexSkillsDir && { codex: data.codexSkillsDir }) }} onClose={() => setVersionTask(null)} onRun={(action) => run("manage_skill_version", { action })}/>}
    {updateTarget && <SkillInPlaceUpdateDrawer id={updateTarget.id} repoKey={updateTarget.repoKey} localPath={updateTarget.localPath} onClose={() => { setUpdateTarget(null); backToDetail(); }} onExecute={(previewId) => run("execute_skill_in_place_update", { previewId })}/>}
  </div>;
}

