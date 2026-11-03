import { startTransition, useDeferredValue, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { ArrowDownAZ, ArrowDownUp, ArrowDownZA, ArrowLeft, ArrowRight, BookOpen, ChevronDown, ChevronsDown, ChevronsUp, CloudDownload, Copy, Edit3, ExternalLink, FolderOpen, FolderPlus, GripVertical, History, Link, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { t, tf } from "./i18n";
import { appConfirm, appPrompt } from "./app-dialog";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { AgentToggle, ManagementDrawer, Menu, MenuItem } from "./ManagementPages";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxRow, CxStatusIcon, CxToolbar } from "./cx-layout";
import { AgentGlyph } from "./HubOverview";
import { MCP_AGENTS, type McpAgentId } from "./agent-mcp-model";
import { agentVisible } from "./agent-focus-model";
import { useAgentFocus } from "./agent-focus-store";
import "./ManagementPages.css";
import { parseSkillRepository } from "./skill-repository";
import { SkillListViewport } from "./SkillListViewport";
import { customSkillGroupKey, groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, type SkillGroup, type SkillGroupSortDirection } from "./skill-groups";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets, SkillBatchRegistry, type PendingSkillBatchOperation } from "./skill-agent-batch";
import { TopbarActions } from "./TopbarActions";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean;
  /** 已经启用到哪些 Agent（codex / claude / grok）。 */
  agents?: string[];
  /** Codex++ 的受控源里有它；只在某个 Agent 自己目录里的 Skill 为 false。 */
  managed?: boolean };
type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type SkillPayload = Result & { skills: Skill[]; repos: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string; agentSkillsDirs?: Record<string, string> };
type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
const SKILL_GROUP_STORAGE_KEY = "codex-plus-plus.skill-groups.v1";
const skillBatchRegistry = new SkillBatchRegistry();
const ok = (r: Result) => r.status === "ok";
function Button({ children, onClick, disabled, primary }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean }) { return <button type="button" className={primary ? "cx-primary" : "cx-secondary"} onClick={onClick} disabled={disabled}>{children}</button>; }
function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) { return <div className="mg-search"><Search size={14} /><input aria-label={placeholder} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} /></div>; }
// 标题由外壳顶栏显示；这里只把页面操作送进顶栏。
function Header({ actions }: { icon?: ReactNode; title?: string; actions: ReactNode }) { return <TopbarActions>{actions}</TopbarActions>; }
function Field({ label, children }: { label: string; children: ReactNode }) { return <label className="mg-field"><span>{label}</span>{children}</label>; }
function repoKey(repo: Repo) { return `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`; }
function repoFromKey(key: string | null): Repo | null {
  const match = key?.match(/^([^/]+)\/([^@]+)@([^:]+)(?::(.*))?$/);
  return match ? { owner: match[1], name: match[2], branch: match[3], subdir: match[4] || "", enabled: false } : null;
}
function parseRepoInput(value: string, current: Repo): Repo | null {
  const parsed = parseSkillRepository(value);
  return parsed ? { ...parsed, enabled: current.enabled } : null;
}
/** 后端 `LOCAL_IMPORT_REPO_KEY`：从本地目录导入的 Skill（#511）。 */
const LOCAL_IMPORT_REPO_KEY = "local";
function sourceLabel(skill: Skill) { if (skill.bundled) return t("内置"); if (skill.repoKey === "cc-switch") return "cc-switch"; if (skill.repoKey === LOCAL_IMPORT_REPO_KEY) return t("本地导入"); return skill.repoKey ? skill.repoKey.split("@", 1)[0] : t("用户"); }
function repoUrl(skill: Skill) {
  if (!skill.repoKey || skill.repoKey === "cc-switch" || skill.repoKey === LOCAL_IMPORT_REPO_KEY) return "";
  const parts = skill.repoKey.split("@")[0].split("/");
  if (parts.length < 2) return "";
  return `https://github.com/${parts.join("/")}${skill.repoPath ? `/tree/${skill.repoKey.split("@")[1]?.split(":")[0] || "main"}/${skill.repoPath}` : ""}`;
}
function SkillRepoDrawer({ repos, catalog, counts, initialKey, busy, updates, onClose, onSave, onDelete, onInspect, onOpenUrl, onOpenSkill, onRun }: {
  repos: Repo[]; catalog: Skill[]; counts: Record<string, number>; initialKey: string | null; busy: boolean; onClose: () => void;
  onSave: (repo: Repo, previousKey?: string) => Promise<boolean>; onDelete: (repo: Repo) => void;
  onInspect: (repo: Repo) => Promise<RepoInspection>; onOpenUrl: (url: string) => void;
  updates: ReturnType<SkillBatchRegistry["getUpdates"]>; onOpenSkill: (skill: Skill) => void; onRun: (command: "install_skill", args: { repoKey: string; id: string }) => void;
}) {
  const [draft, setDraft] = useState<Repo>({ owner: "", name: "", branch: "main", subdir: "", enabled: true });
  const [input, setInput] = useState("");
  const [parseError, setParseError] = useState("");
  const [editing, setEditing] = useState<Repo | null>(null);
  const [selected, setSelected] = useState<Repo | null>(() => repos.find((r) => repoKey(r) === initialKey) || repoFromKey(initialKey) || repos[0] || null);
  const [preview, setPreview] = useState<RepoInspection | null>(null);
  const [formOpen, setFormOpen] = useState(false);
  const editorRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (formOpen) editorRef.current?.querySelector<HTMLInputElement>("input")?.focus({ preventScroll: true });
  }, [formOpen, editing]);
  const [query, setQuery] = useState("");
  const [scanning, setScanning] = useState(false);
  const scanRequest = useRef(0);
  useEffect(() => () => { scanRequest.current++; }, []);
  const [feedback, setFeedback] = useState("");
  const cancelScan = () => { scanRequest.current++; setScanning(false); };
  const set = (v: Partial<Repo>) => setDraft((old) => ({ ...old, ...v }));
  const inspect = async (repo: Repo, updateDraft = false) => {
    const request = ++scanRequest.current;
    setScanning(true); setFeedback(""); setSelected(repo); setPreview(null); setQuery("");
    try {
      const result = await onInspect(repo);
      if (request !== scanRequest.current) return;
      if (ok(result)) {
        setSelected(result.repo); setPreview(result); setFeedback(result.message);
        if (updateDraft) setDraft(result.repo);
      } else setFeedback(result.message);
    } catch (e) { if (request === scanRequest.current) setFeedback(String(e)); }
    finally { if (request === scanRequest.current) setScanning(false); }
  };
  const parse = async () => {
    const parsed = parseRepoInput(input, draft);
    if (parsed) { setDraft(parsed); setParseError(""); await inspect(parsed, true); }
    else setParseError(t("无法识别仓库地址，请输入 GitHub 链接或 owner/repo。"));
  };
  const view = (repo: Repo) => {
    cancelScan();
    setSelected(repo); setPreview(null); setFeedback(""); setQuery("");
    if (counts[repoKey(repo)] === undefined) void inspect(repo);
  };
  useEffect(() => { if (selected && counts[repoKey(selected)] === undefined) void inspect(selected); }, []);
  useEffect(() => { if (!selected && repos[0]) view(repos[0]); }, [repos, selected]);
  const edit = (repo: Repo) => { setFormOpen(true); setEditing(repo); setDraft(repo); setInput(`https://github.com/${repo.owner}/${repo.name}/tree/${repo.branch}${repo.subdir ? `/${repo.subdir}` : ""}`); view(repo); };
  const save = async () => {
    const request = ++scanRequest.current;
    setScanning(false);
    if (await onSave(draft, editing ? repoKey(editing) : undefined) && request === scanRequest.current) {
      setSelected(draft); setPreview(null); setFeedback(""); setFormOpen(false);
      setEditing(null);
      setDraft({ owner: "", name: "", branch: "main", subdir: "", enabled: true });
      setInput("");
      setParseError("");
    }
  };
  const active = repos.find((r) => selected && repoKey(r) === repoKey(selected))
    || repos.find((r) => selected && r.owner === selected.owner && r.name === selected.name && r.subdir === selected.subdir)
    || selected;
  const savedCatalog = active && counts[repoKey(active)] !== undefined;
  const activeKey = active ? repoKey(active) : "";
  const skills = useMemo(() => savedCatalog ? catalog.filter((s) => s.repoKey === activeKey) : preview?.skills || [], [savedCatalog, catalog, activeKey, preview]);
  const count = preview?.skillCount ?? (active ? counts[repoKey(active)] : undefined);
  const deferredQuery = useDeferredValue(query);
  const filtered = useMemo(() => skills.filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoPath || ""], deferredQuery)), [skills, deferredQuery]);
  const locked = busy || scanning;
  return <ManagementDrawer wide dismissOnBackdrop={false} title={t("管理 Skills 仓库")} onClose={onClose}>
    <div className="mg-repo-workspace" data-editing={formOpen}><aside className="mg-repo-sidebar">
    <div className="mg-repo-manager"><div className="mg-repo-manager-head"><strong>{t("已添加仓库")} <span className="mg-repo-total">{repos.length}</span></strong><button className="mg-icon-button" title={t("添加仓库")} aria-label={t("添加仓库")} onClick={() => { cancelScan(); setEditing(null); setFormOpen(true); setInput(""); setParseError(""); setDraft({ owner: "", name: "", branch: "main", subdir: "", enabled: true }); }}><Plus size={16}/></button></div>
      {!repos.length && <p className="mg-muted">{t("还没有添加仓库。")}</p>}
      {repos.map((repo) => <div className="mg-repo-row" data-selected={active && repoKey(active) === repoKey(repo)} key={repoKey(repo)}>
        <button type="button" className="mg-repo-select" onClick={() => view(repo)}><strong title={`${repo.owner}/${repo.name}`}>{repo.owner}/{repo.name}</strong><small title={`${repo.branch}${repo.subdir ? ` · ${repo.subdir}` : ""}`}>{repo.branch}{repo.subdir ? ` · ${repo.subdir}` : ""}<span>{counts[repoKey(repo)] === undefined ? t("待扫描") : `${counts[repoKey(repo)]} Skills`}{!repo.enabled ? ` · ${t("已停用")}` : ""}</span></small></button>
        <div className="mg-repo-row-actions"><button className={"mg-switch " + (repo.enabled ? "is-on" : "")} type="button" role="switch" aria-checked={repo.enabled} disabled={busy} aria-label={`${repo.owner}/${repo.name} ${t("启用仓库")}`} onClick={() => void onSave({ ...repo, enabled: !repo.enabled })}><span /></button>
        <button type="button" className="mg-icon-button" title={t("编辑仓库")} aria-label={`${t("编辑仓库")} ${repo.owner}/${repo.name}`} onClick={() => edit(repo)}><Edit3 size={14}/></button>
        <button className="mg-icon-button" title={t("删除仓库")} aria-label={`${t("删除仓库")} ${repo.owner}/${repo.name}`} disabled={busy} onClick={() => { cancelScan(); onDelete(repo); if (active && repoKey(active) === repoKey(repo)) { setSelected(repos.find((r) => repoKey(r) !== repoKey(repo)) || null); setPreview(null); } }}><Trash2 size={14}/></button></div></div>)}
    </div>

    </aside><section className="mg-repo-browser">
      {active ? <>
        <div className="mg-repo-detail-head"><h3>{active.owner}/{active.name}</h3><div className="mg-row-actions"><Button disabled={locked} onClick={() => void inspect(active)}><RefreshCw size={14}/>{t("刷新仓库")}</Button><Button onClick={() => onOpenUrl(`https://github.com/${active.owner}/${active.name}`)}><ExternalLink size={14}/>GitHub</Button></div></div>
        <dl className="mg-detail"><dt>{t("分支")}</dt><dd>{active.branch}</dd><dt>{t("子目录")}</dt><dd>{active.subdir || "/"}</dd><dt>{t("Skill 数量")}</dt><dd className="mg-repo-count">{scanning ? t("正在扫描…") : count === undefined ? t("尚未扫描") : `${count} Skills`}</dd></dl>
        {feedback && <p className="mg-notice" role="status">{feedback}</p>}
        {count === 0 && <p className="mg-muted">{t("当前分支和目录中没有 SKILL.md。")}</p>}
        <SearchBox value={query} onChange={setQuery} placeholder={t("搜索此仓库的 Skills")} />
        <SkillListViewport className="mg-repo-results" items={filtered} rowHeight={64} resetKey={`${activeKey}:${deferredQuery}`} busy={scanning}
          empty={<p className="mg-muted" role="status">{scanning ? t("正在扫描…") : count ? t("没有匹配的 Skill") : t("暂无 Skills")}</p>}
          renderRow={(s) => {
            const pending = updates.find((item) => item.id === s.id);
            const state = pending && pending.repoKey !== s.repoKey ? t("同 ID 操作中") : pending ? t(pending.state === "running" ? "安装中" : "排队中") : t("安装");
            return <div className="mg-repo-skill" key={skillIdentityKey(s)}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={s.description}>{s.description || s.repoPath}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => repoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <span title={pending ? state : undefined}><Button primary disabled={Boolean(pending)} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id })}>{state}</Button></span>)}</div></div></div>;
          }} />
      </> : <p className="mg-muted">{t("还没有添加仓库。")}</p>}
    </section>
    {formOpen && <div className="mg-repo-form" ref={editorRef} role="region" aria-label={editing ? t("编辑仓库") : t("添加仓库")}
      onKeyDown={(event) => { if (event.key === "Escape") { event.stopPropagation(); event.preventDefault(); cancelScan(); setFormOpen(false); } }}>
      <div className="mg-repo-manager-head">
        <h3><Edit3 size={15}/>{editing ? t("编辑仓库") : t("添加仓库")}</h3>
        <button className="mg-icon-button" title={t("取消编辑")} aria-label={t("取消编辑")} onClick={() => { cancelScan(); setFormOpen(false); }}><X size={14}/></button>
      </div>
      <div className="mg-repo-form-content">
        <div className="mg-repo-quick">
          <input disabled={locked} value={input} onChange={(e) => { setInput(e.target.value); setParseError(""); }} aria-label={t("仓库地址")} placeholder="https://github.com/owner/repo" />
          <Button onClick={() => void parse()} disabled={locked || !input.trim()}><Search size={14}/>{scanning ? t("解析中…") : t("解析")}</Button>
        </div>
        {parseError && <p className="mg-form-error">{parseError}</p>}
        <fieldset disabled={locked} className="mg-form-grid">
          <Field label={t("所有者")}><input value={draft.owner} onChange={(e) => set({ owner: e.target.value })} /></Field>
          <Field label={t("仓库名")}><input value={draft.name} onChange={(e) => set({ name: e.target.value })} /></Field>
          <Field label={t("分支")}><input value={draft.branch} onChange={(e) => set({ branch: e.target.value })} /></Field>
          <Field label={t("子目录")}><input value={draft.subdir} onChange={(e) => set({ subdir: e.target.value })} placeholder={t("可选")} /></Field>
        </fieldset>
      </div>
      <div className="mg-repo-form-footer">
        <label className="mg-check-line"><input type="checkbox" disabled={locked} checked={draft.enabled} onChange={(e) => set({ enabled: e.target.checked })}/>{t("启用仓库")}</label>
        <div className="mg-row-actions">
          <Button disabled={locked || !draft.owner.trim() || !draft.name.trim()} onClick={() => void inspect(draft, true)}>{t("检查仓库")}</Button>
          <Button primary disabled={locked || !draft.owner.trim() || !draft.name.trim()} onClick={() => void save()}>{editing ? t("保存修改") : t("添加仓库")}</Button>
        </div>
      </div>
    </div>}
    </div>
  </ManagementDrawer>;
}
function SkillDetailDrawer({ skill, count, onClose, onOpenUrl, onOpenRepo }: { skill: Skill; count?: number; onClose: () => void; onOpenUrl: (url: string) => void; onOpenRepo: () => void }) {
  const url = repoUrl(skill);
  return <ManagementDrawer dismissOnBackdrop={false} title={skill.name || skill.id} onClose={onClose}><dl className="mg-detail"><dt>{t("来源")}</dt><dd>{url ? <button className="mg-link-button mg-skill-repo-link" type="button" onClick={onOpenRepo}>{sourceLabel(skill)}{count === undefined ? "" : ` · ${count} Skills`}<ArrowRight size={13}/></button> : sourceLabel(skill)}</dd><dt>{t("Skill ID")}</dt><dd>{skill.id}</dd><dt>{t("仓库路径")}</dt><dd>{skill.repoPath || "—"}</dd><dt>{t("介绍")}</dt><dd>{skill.description || t("暂无描述")}</dd>{url && <><dt>GitHub</dt><dd><button className="mg-link-button" type="button" onClick={() => onOpenUrl(url)}><ExternalLink size={13}/>{url}</button></dd></>}</dl></ManagementDrawer>;
}
function SortableSkillGroup({ group, disabled, children }: { group: SkillGroup<Skill>; disabled: boolean; children: (handle: ReactNode) => ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: group.key, disabled });
  const handle = disabled ? <span className="mg-skill-drag-placeholder" aria-hidden="true" /> :
    <button type="button" className="mg-icon-button mg-skill-drag" title={t("拖动分组排序")} aria-label={`${t("拖动分组排序")} ${group.label}`} {...attributes} {...listeners}><GripVertical size={15}/></button>;
  return <section className="mg-skill-group" ref={setNodeRef} data-dragging={isDragging}
    style={{ transform: CSS.Transform.toString(transform), transition }}>{children(handle)}</section>;
}
/** 分组标题上的批量开关：和行里的 Agent 胶囊同一列，一组全开是实心、部分开是半实心；点一下全开或全关。 */
function GroupAgentSelect({ group, agent, busy, operations, count, onChange }: { group: SkillGroup<Skill>; agent: McpAgentId; busy: boolean; operations: PendingSkillBatchOperation[]; count?: number; onChange: (enabled: boolean) => void }) {
  const pendingById = new Map<string, boolean>();
  for (const operation of operations) {
    if (operation.agent !== agent) continue;
    const settled = new Set(operation.settledSkillIds);
    for (const id of operation.skillIds) if (!settled.has(id)) pendingById.set(id, operation.enabled);
  }
  const state = skillBatchState(group.items.map((skill) => {
    const pending = pendingById.get(skill.id);
    if (pending === undefined) return skill;
    const agents = skillAgents(skill).filter((value) => value !== agent);
    if (pending) agents.push(agent);
    return { ...skill, agents };
  }), agent);
  const name = agent === "codex" ? "Codex" : agent === "claude" ? "Claude Code" : "Grok";
  const label = `${group.label} · ${name} · ${t("全选")}`;
  return <button type="button" className="cx-agent-toggle cx-skill-batch" aria-label={label}
    title={count === undefined ? label : `${label}（${tf("已启用 {0} 个 Skill", [count])}）`}
    aria-pressed={state.mixed ? "mixed" : state.checked} disabled={busy || !state.total}
    onClick={() => onChange(!state.checked)}>
    <AgentGlyph agent={agent} /><span>{name}</span>
  </button>;
}
export function SkillsPage({ notify }: { notify: Notify }) {
  const [dataState, setDataState] = useState<{ version: number; value: SkillPayload | null }>({ version: 0, value: null });
  const data = dataState.value;
  const [tab, setTab] = useState<"discover" | "installed">("installed");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const batchOperations = useSyncExternalStore(skillBatchRegistry.subscribe, skillBatchRegistry.getSnapshot, skillBatchRegistry.getSnapshot);
  const updates = useSyncExternalStore(skillBatchRegistry.subscribeUpdates, skillBatchRegistry.getUpdates, skillBatchRegistry.getUpdates);
  const updateFailures = useSyncExternalStore(skillBatchRegistry.subscribeFailures, skillBatchRegistry.getFailures, skillBatchRegistry.getFailures);
  const batchOwner = useRef(Symbol("skills-page"));
  const mounted = useRef(false);
  const dataRequest = useRef(0);
  const [repoOpen, setRepoOpen] = useState(false);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [repoSelection, setRepoSelection] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  // 当前使用模式下隐藏的 Agent 不占列：只管理 Codex 时只有 Codex，只用增强功能时是 Codex、Claude Code。
  const focus = useAgentFocus();
  const agentColumns: McpAgentId[] = MCP_AGENTS.filter((agent) => agentVisible(focus, agent));
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
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
  const reloadLocal = async () => {
    const request = ++dataRequest.current;
    try {
      const result = await invoke<SkillPayload>("list_installed_skills");
      if (!mounted.current || request !== dataRequest.current) return;
      startTransition(() => setDataState((current) => mounted.current && request === dataRequest.current && request >= current.version
        ? { version: request, value: result } : current));
    } catch (error) {
      if (mounted.current) await notify("Skills", String(error), "failed");
    }
  };
  const applyCurrentPayload = (request: number, payload: SkillPayload) => {
    if (!mounted.current) return;
    if (request !== dataRequest.current) { void reloadLocal(); return; }
    setDataState((current) => mounted.current && request === dataRequest.current && request >= current.version
      ? { version: request, value: payload } : current);
  };
  const load = async (command = "refresh_skill_catalog") => {
    setLoading(true);
    const request = ++dataRequest.current;
    try {
      const r = await invoke<SkillPayload>(command);
      startTransition(() => applyCurrentPayload(request, r));
      if (!ok(r)) await notify("Skills", r.message, "failed");
    } catch (e) {
      await notify("Skills", String(e), "failed");
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => {
    let active = true;
    mounted.current = true;
    void (async () => {
      await load("list_installed_skills");
      if (active) await load();
    })();
    return () => { active = false; mounted.current = false; dataRequest.current++; };
  }, []);
  useEffect(() => skillBatchRegistry.subscribeSettled((owner) => {
    if (mounted.current && owner !== batchOwner.current) void reloadLocal();
  }), []);
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
          if (!ok(current)) throw new Error(current.message);
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
      });
    }, args.repoKey);
  };
  const queueGroupUpdates = (skills: Skill[]) => {
    for (const skill of skills) queueUpdate("update_skill", { repoKey: skill.repoKey, id: skill.id });
  };
  const runBatch = async (group: SkillGroup<Skill>, agent: McpAgentId, enabled: boolean) => {
    const groupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
    if (skillBatchHasPending(skillBatchRegistry.getSnapshot(), groupItems, agent)) return;
    const targets = skillBatchTargets(group.items, agent, enabled);
    if (!targets.length) {
      await notify("Skills", enabled ? t("该组没有可启用的 Skill") : t("该组没有可停用的 Skill"), "ok");
      return;
    }
    let operation: PendingSkillBatchOperation = { groupKey: group.key, agent, enabled, skillIds: targets.map((skill) => skill.id), settledSkillIds: [], owner: batchOwner.current };
    skillBatchRegistry.add(operation);
    let completed = 0;
    let failed = 0;
    const errors: string[] = [];
    const recordFailure = (skill: Skill, reason: string) => {
      failed++;
      if (errors.length >= 3) return;
      const detail = `${skill.name || skill.id}：${reason}`;
      errors.push(detail.length > 240 ? `${detail.slice(0, 239).replace(/[\uD800-\uDBFF]$/, "")}…` : detail);
    };
    try {
      for (const skill of targets) {
        try {
          const { request, result } = await skillBatchRegistry.run(async () => {
            const request = ++dataRequest.current;
            const result = await invoke<SkillPayload>("set_skill_enabled_for", { agent, id: skill.id, enabled });
            return { request, result };
          });
          if (mounted.current && request === dataRequest.current) {
            setDataState((current) => {
              if (!mounted.current || request < current.version || !current.value || !result.skills) return current;
              return { version: request, value: { ...current.value, skills: mergeSkillAgentResult(current.value.skills, result.skills, skill.id, agent) } };
            });
          } else if (mounted.current) void reloadLocal();
          if (ok(result)) completed++;
          else recordFailure(skill, result.message);
        } catch (error) { recordFailure(skill, String(error)); }
        finally {
          const previous = operation;
          operation = { ...operation, settledSkillIds: [...operation.settledSkillIds, skill.id] };
          skillBatchRegistry.replace(previous, operation);
        }
      }
    } finally {
      skillBatchRegistry.remove(operation);
      if (mounted.current) void reloadLocal();
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
  const deleteRepo = async (repo: Repo) => { await run("delete_skill_repo", { key: repoKey(repo) }); };
  const inspectRepo = async (repo: Repo) => {
    const result = await invoke<RepoInspection>("inspect_skill_repo", { repo });
    if (ok(result)) await reloadLocal();
    return result;
  };
  const openRepo = (key: string) => { setRepoSelection(key); setDetail(null); setRepoOpen(true); };
  const deferredQuery = useDeferredValue(query);
  const items = useMemo(() => (data?.skills || [])
    .filter((s) => (tab === "installed" ? s.installed : !s.installed || s.bundled))
    .filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoKey, sourceLabel(s)], deferredQuery)), [data, tab, deferredQuery]);
  const knownSkills = useMemo(() => data?.skills || [], [data]);
  const knownRepositories = useMemo(() => data?.repos || [], [data]);
  const builtinGroupLabel = t("内置");
  const userGroupLabel = t("用户");
  const localGroupLabel = t("本地导入");
  const allGroups = useMemo(() => {
    const result = groupSkills(knownSkills, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, true, groupAliases, assignmentsByIdentity, knownRepositories);
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupSortDirection, groupAliases, builtinGroupLabel, userGroupLabel, localGroupLabel]);
  const groups = useMemo(() => {
    const result = groupSkills(items, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, !query.trim(), groupAliases, assignmentsByIdentity, knownRepositories);
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [items, knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupSortDirection, groupAliases, builtinGroupLabel, userGroupLabel, localGroupLabel, query]);
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
    setAssignmentsByIdentity((old) => ({ ...old, [skillIdentityKey(skill)]: group ? customSkillGroupKey(group) : "" }));
  };
  const handleGroupDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id || query.trim() || busy || loading) return;
    const movable = new Set(groups.filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key));
    if (!movable.has(String(active.id)) || !movable.has(String(over.id))) return;
    setGroupOrder(moveSkillGroup(allGroups, String(active.id), String(over.id)));
    setGroupSortDirection(null);
  };
  const agentCount = (agent: McpAgentId) => (data?.skills || []).filter((s) => s.installed && skillAgents(s).includes(agent)).length;
  // 参照 MCP 页（Cursor 的 Customize）：每个 Skill 一行，右侧是各 Agent 的开关胶囊；分组是卡片外的小标题，
  // 标题右侧同一列位置是这一组的批量开关。详情、分组、卸载收进行末的三点菜单。
  const renderSkillRow = (s: Skill) => {
    const update = s.bundled ? undefined : updates.find((item) => item.id === s.id);
    // 只卸载 Codex++ 管理的或 Codex 目录里的；Claude / Grok 自己目录里的 Skill 不动。
    const canUninstall = s.installed && !s.bundled && (s.managed || skillAgents(s).includes("codex") || s.agents === undefined);
    const assignment = skillGroupAssignment(s, customGroupNames, customGroupAssignments, assignmentsByIdentity);
    const on = s.installed && agentColumns.some((agent) => skillAgents(s).includes(agent));
    return <CxRow className="cx-skill-row" key={skillIdentityKey(s)}
      icon={<CxStatusIcon tone={on ? "ok" : "off"}><BookOpen /></CxStatusIcon>}
      title={s.name || s.id}
      badges={<>
        <small className="cx-skill-source" title={s.repoKey || sourceLabel(s)}>{sourceLabel(s)}</small>
        {s.updateAvailable ? <CxBadge tone="warn">{t("可更新")}</CxBadge> : null}
      </>}
      detail={<span title={s.description}>{s.description || t("暂无描述")}</span>}
      onClick={() => setDetail(s)}>
      {update ? <span title={update.repoKey !== s.repoKey ? t("同 ID 操作中") : undefined}><Button disabled onClick={() => {}}>{t(update.command === "uninstall_skill" ? "卸载排队中" : update.repoKey !== s.repoKey ? "同 ID 操作中" : update.state === "queued" ? "排队中" : update.command === "install_skill" ? "安装中" : "更新中")}</Button></span> : !s.installed && !s.bundled ? <Button onClick={() => { queueUpdate("install_skill", { repoKey: s.repoKey, id: s.id }); }}>{t("安装")}</Button> : s.installed && s.updateAvailable ? <Button onClick={() => { queueUpdate("update_skill", { repoKey: s.repoKey, id: s.id }); }}>{t("更新")}</Button> : null}
      <div className="cx-agent-toggles" role="group" aria-label={t("启用到 Agent")}>
        {agentColumns.map((agent) => <AgentToggle agent={agent} key={agent} name={s.name || s.id}
          enabled={s.installed && skillAgents(s).includes(agent)}
          disabled={busy || !s.installed || s.bundled || skillBatchHasPending(batchOperations, [s], agent)}
          title={data?.agentSkillsDirs?.[agent]}
          hint={!s.installed ? t("未安装") : s.bundled ? t("Codex 自带，只读") : undefined}
          onToggle={() => void run("set_skill_enabled_for", { agent, id: s.id, enabled: !skillAgents(s).includes(agent) })} />)}
      </div>
      <Menu>
        <MenuItem onClick={() => setDetail(s)}>{t("查看详情")}</MenuItem>
        {repoUrl(s) ? <MenuItem onClick={() => openRepo(s.repoKey)}>{t("管理仓库")}</MenuItem> : null}
        {!s.bundled && customGroupNames.length ? <>
          <div className="mg-menu-label">{t("分组")}</div>
          <MenuItem checked={!assignment} onClick={() => assignSkillGroup(s, "")}>{t("按来源")}</MenuItem>
          {customGroupNames.map((group) => <MenuItem checked={assignment === customSkillGroupKey(group)} key={group} onClick={() => assignSkillGroup(s, group)}>{group}</MenuItem>)}
        </> : null}
        {canUninstall ? <MenuItem danger disabled={busy || Boolean(update)} onClick={() => void run("uninstall_skill", { id: s.id })}>{t("卸载")}</MenuItem> : null}
      </Menu>
    </CxRow>;
  };
  const discoverCount = (data?.skills || []).filter((s) => !s.installed || s.bundled).length;
  const installedCount = (data?.skills || []).filter((s) => s.installed).length;
  const sortLabel = `${t("按名称排序")} (${groupSortDirection === "asc" ? "A-Z" : groupSortDirection === "desc" ? "Z-A" : "A-Z / Z-A"})`;
  return <div className="mg-page cx-page mg-skills-page">
    <Header icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load()} disabled={busy || loading}><RefreshCw size={14}/>{t("刷新目录")}</Button>
      <Menu>
        <MenuItem onClick={() => void importLocalSkills()} disabled={busy}>{t("从本地目录导入")}</MenuItem>
        <MenuItem onClick={() => void run("import_ccs_skills", {})} disabled={busy}>{t("从 cc-switch 导入")}</MenuItem>
      </Menu>
    </>} />
    <CxToolbar query={query} onQuery={setQuery} placeholder={t("搜索 Skills 或仓库")}>
      <button type="button" className="cx-primary" onClick={() => setRepoOpen(true)}><Link size={15}/>{t("管理仓库")}</button>
    </CxToolbar>
    <CxFilters label="Skills" value={tab} onChange={setTab}
      items={[{ id: "installed", label: t("已安装"), count: installedCount }, { id: "discover", label: t("发现"), count: discoverCount }]}
      trailing={<>
        <button type="button" className="mg-icon-button" title={t("新建分组")} aria-label={t("新建分组")}
          onClick={createCustomGroup}><FolderPlus size={16}/></button>
        <button type="button" className="mg-icon-button" disabled={busy || loading}
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
      detail={<ul className="cx-skill-failures" tabIndex={0} aria-label={t("失败的 Skill 列表")}>{updateFailures.details.map((detail) => <li key={detail.id}><strong>{detail.id}</strong>：{detail.message}</li>)}</ul>}>
      <button type="button" className="mg-icon-button" aria-label={t("关闭 Skill 失败摘要")} title={t("关闭 Skill 失败摘要")} onClick={skillBatchRegistry.clearFailures}><X size={16}/></button>
    </CxBanner>}
    {data?.repoErrors?.length ? <CxBanner tone="warn" icon={<Link />} title={t("部分仓库读取失败")} detail={<span className="cx-skill-repo-errors">{data.repoErrors.map((e) => <span key={e}>{e}</span>)}</span>} /> : null}
    <div className="cx-skill-groups" aria-busy={loading} data-columns={agentColumns.length}>
      {!groups.length ? <CxCard><CxEmpty title={loading ? t("正在扫描…") : t("没有匹配的 Skill")} /></CxCard> :
        <DndContext sensors={groupSensors} collisionDetection={closestCenter} onDragEnd={handleGroupDragEnd}>
          <SortableContext items={groups.map((group) => group.key)} strategy={verticalListSortingStrategy}>{groups.map((group) => {
        const collapsed = !expandedGroups.has(group.key) && !query.trim();
        const allGroupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
        const updateTargets = allGroupItems.filter((skill) => skill.installed && !skill.bundled && skill.updateAvailable && !updates.some((item) => item.id === skill.id));
        const pendingUpdates = allGroupItems.filter((skill) => skill.installed && !skill.bundled && updates.some((item) => item.command === "update_skill" && item.id === skill.id && item.repoKey === skill.repoKey)).length;
        const hasUpdates = pendingUpdates > 0 || allGroupItems.some((skill) => skill.installed && !skill.bundled && skill.updateAvailable);
        const updateLabel = `${group.label} · ${tf("更新组内可更新 Skill（{0}）", [updateTargets.length])}${pendingUpdates ? ` · ${tf("进行中或排队 {0} 项", [pendingUpdates])}` : ""}`;
        const movable = group.kind === "repo" || group.kind === "custom";
        return <SortableSkillGroup group={group} key={group.key} disabled={!movable || !!query.trim() || busy || loading}>{(handle) => <>
          <header className="cx-skill-group-head">
            {handle}
            <button type="button" className="cx-skill-group-toggle" aria-expanded={!collapsed} onClick={() => setExpandedGroups((old) => { const next = new Set(old); next.has(group.key) ? next.delete(group.key) : next.add(group.key); return next; })}>
              <ChevronDown size={14} className={collapsed ? "mg-chevron-collapsed" : ""}/><strong title={group.label}>{group.label}</strong><span className="cx-section-count">{group.items.length}</span>
            </button>
            <div className="cx-skill-group-aside">
              {hasUpdates && <button type="button" className="mg-icon-button" aria-label={updateLabel} title={updateLabel} disabled={!updateTargets.length}
                onClick={() => queueGroupUpdates(updateTargets)}><CloudDownload size={15}/></button>}
              <div className="cx-agent-toggles" role="group" aria-label={`${group.label} · ${t("全选")}`}>
                {agentColumns.map((agent) => <GroupAgentSelect group={group} agent={agent} key={agent}
                  busy={busy || skillBatchHasPending(batchOperations, allGroupItems, agent)} operations={batchOperations}
                  count={agentCount(agent)}
                  onChange={(enabled) => void runBatch(group, agent, enabled)}/>)}
              </div>
              {group.kind === "custom" ? <Menu label={tf("{0} 的更多操作", [group.label])}>
                <MenuItem disabled={busy} onClick={() => renameCustomGroup(group.label)}>{t("重命名")}</MenuItem>
                <MenuItem danger disabled={busy} onClick={() => deleteCustomGroup(group.label)}>{t("删除分组")}</MenuItem>
              </Menu> : <span className="cx-skill-menu-spacer" aria-hidden="true" />}
            </div>
          </header>
          {/* 空分组只留标题（数量 0），不再画一张空卡片。 */}
          {!collapsed && group.items.length ? <CxCard>{group.items.map(renderSkillRow)}</CxCard> : null}
        </>}</SortableSkillGroup>;
      })}</SortableContext></DndContext>}
    </div>
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} initialKey={repoSelection} busy={busy} updates={updates} onClose={() => setRepoOpen(false)} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(command, args) => { queueUpdate(command, args); }} />}
    {detail && <SkillDetailDrawer skill={detail} count={data?.repoCounts?.[detail.repoKey]} onClose={() => setDetail(null)} onOpenUrl={(url) => void openUrl(url)} onOpenRepo={() => repoOpen ? setDetail(null) : openRepo(detail.repoKey)} />}
  </div>;
}
export function SessionPage({ sessions, onRefresh, onDelete, onDeleteMany, notify, repairPanel }: { sessions: { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number } | null; onRefresh: (offset?: number) => Promise<void>; onDelete: (s: ManagedSession) => Promise<void>; onDeleteMany: (s: ManagedSession[]) => Promise<void>; notify: Notify; repairPanel?: ReactNode }) { const [query, setQuery] = useState(""); const [mode, setMode] = useState<"project" | "time">("project"); const [expanded, setExpanded] = useState<Set<string>>(new Set()); const [selection, setSelection] = useState(false); const [selected, setSelected] = useState<Set<string>>(new Set()); const [detail, setDetail] = useState<ManagedSession | null>(null); const all = sessions?.sessions || []; const visible = all.filter((s) => matchesManagementQuery([s.title, s.cwd, s.id, s.modelProvider], query)); const groups = sessionGroups(visible, mode); const selectionScope = visible.map((s) => s.id).join("\0"); useEffect(() => { const ids = new Set(selectionScope.split("\0")); setSelected((old) => { const next = new Set([...old].filter((id) => ids.has(id))); return next.size === old.size ? old : next; }); }, [selectionScope]); const selectedRows = visible.filter((s) => selected.has(s.id)); const toggle = (id: string) => setSelected((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; }); return <div className="mg-page mg-session-page"><Header icon={<History size={20}/>} title={t("会话")} actions={<><Button onClick={() => void onRefresh()}><RefreshCw size={14}/>{t("刷新列表")}</Button><Button onClick={() => { setSelection(!selection); setSelected(new Set()); }}>{t("选择多个…")}</Button></>} />{repairPanel}<div className="mg-session-controls"><SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} /><div className="mg-segmented"><button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button><button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button></div>{selection && <><Button onClick={() => setSelected(new Set(visible.map((s) => s.id)))}>{t("全选当前列表")}</Button><Button disabled={!selectedRows.length} onClick={() => setSelected(new Set())}>{t("取消全部选择")}</Button><Button primary disabled={!selectedRows.length} onClick={() => void onDeleteMany(selectedRows)}><Trash2 size={14}/>{t("删除已选")}</Button></>}</div><div className="mg-content"><div className="mg-session-list">{!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((g) => { const open = query.trim() || expanded.has(g.key) || groups.length === 1; return <section key={g.key}><button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(g.key) ? next.delete(g.key) : next.add(g.key); return next; })}><ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"}/><strong>{g.label}</strong><span>{g.path}</span><small>{g.sessions.length} {t("个会话")}</small></button>{open && g.sessions.map((s) => <article className="mg-session-row" key={s.id}><div className="mg-session-check">{selection && <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)}/>}</div><button type="button" className="mg-session-main" onClick={() => setDetail(s)}><strong>{s.title || t("未命名会话")}</strong><small>{s.cwd || t("未记录项目路径")}</small><code>{s.id}</code></button><div className="mg-session-meta"><span>{s.modelProvider || "Codex"}</span><time>{s.updatedAtMs ? new Date(s.updatedAtMs).toLocaleString() : t("未知时间")}</time></div><div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(s)} aria-label={t("删除会话")}><Trash2 size={15}/></button></div></article>)}</section>; })}</div></div><div className="mg-pagination"><Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - (sessions?.limit || 50)))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14}/>{t("上一页")}</Button><span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [Math.floor(sessions.offset / sessions.limit) + 1, sessions.totalCount]) : "—"}</span><Button onClick={() => void onRefresh((sessions?.offset || 0) + (sessions?.limit || 50))} disabled={!sessions?.hasMore}><ArrowRight size={14}/>{t("下一页")}</Button></div>{detail && <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14}/>{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14}/>{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer>}</div>; }
