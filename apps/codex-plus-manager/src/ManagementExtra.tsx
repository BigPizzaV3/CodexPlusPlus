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
import { aggregateSkillGroupItems, customSkillGroupKey, groupSkills, legacySkillCustomGroupName, migrateSkillGroupAssignments, migrateSkillInstanceAssignments, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillGroupIdentityKey, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, type SkillGroup, type SkillGroupSortDirection } from "./skill-groups";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets, skillCodexStatusError, SkillBatchRegistry, type PendingSkillBatchOperation } from "./skill-agent-batch";
import { SkillSourceMatchDrawer, type SourceSelection } from "./SkillSourceMatchDrawer";
import { SkillDuplicatesDrawer } from "./SkillDuplicatesDrawer";
import { SkillInPlaceUpdateDrawer } from "./SkillInPlaceUpdateDrawer";
import { ReaUpdatePreviewDrawer } from "./ReaUpdatePreviewDrawer";
import { ImpeccableVersions } from "./ImpeccableVersions";
import { TopbarActions } from "./TopbarActions";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
type SkillLocation = { manifestPath: string; rootKind: string; managed: boolean; enabled: boolean; statusError?: string | null; originalPath?: string | null; selectable?: boolean;
  repoKey?: string; repoPath?: string; contentHash?: string; remoteHash?: string; updateAvailable?: boolean; updateOwner?: string };
type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean;
  /** 已经启用到哪些 Agent（codex / claude / grok）。 */
  agents?: string[];
  /** Codex++ 的受控源里有它；只在某个 Agent 自己目录里的 Skill 为 false。 */
  managed?: boolean;
  sourceVerified?: boolean;
  installChannel?: string;
  originalDir?: string | null;
  updateOwner?: string;
  instanceKey?: string;
  conflict?: string | null;
  conflictKind?: "independent" | "equal-independent" | "managed-modified" | null;
  safeDisableAgents?: string[];
  locations?: SkillLocation[];
  displayInstances?: Skill[] };
type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type SkillBackup = { id: string; skillId: string; name: string; backedUpAt: string; restorePath?: string | null; purpose?: string | null };
type SkillPayload = Result & { skills: Skill[]; repos: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string; agentSkillsDirs?: Record<string, string>; backups?: SkillBackup[] };
type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
type SourceApplyResult = SkillPayload & { applied: Array<{ localId: string; fromRepoKey: string; toRepoKey: string; localPath?: string | null }> };
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
function sourceLabel(skill: Skill) { if (skill.displayInstances) return t("多个 Agent 安装"); if (skill.bundled) return t("内置"); if (skill.repoKey === "cc-switch" || (skill.conflict && !skill.repoKey)) return t("本地（来源未确认）"); if (skill.repoKey === LOCAL_IMPORT_REPO_KEY) return t("本地导入"); return skill.repoKey ? skill.repoKey.split("@", 1)[0] : t("用户"); }
function skillChannelLabel(skill: Skill) {
  return skill.installChannel === "cc-switch" || (skill.repoKey === "cc-switch" && skill.sourceVerified)
    ? "cc-switch" : skill.installChannel === "github" ? "GitHub" : skill.installChannel === "system" ? t("内置") : skill.installChannel === "local" || skill.repoKey === "cc-switch" ? t("本地安装") : t("未记录");
}
function repoUrl(skill: Skill) {
  if (!skill.repoKey || skill.repoKey === "cc-switch" || skill.repoKey === LOCAL_IMPORT_REPO_KEY) return "";
  const parts = skill.repoKey.split("@")[0].split("/");
  if (parts.length < 2) return "";
  return `https://github.com/${parts.join("/")}${skill.repoPath ? `/tree/${skill.repoKey.split("@")[1]?.split(":")[0] || "main"}/${skill.repoPath}` : ""}`;
}
function SkillRepoDrawer({ repos, catalog, counts, initialKey, busy, updates, conflictIds, onClose, onSave, onDelete, onInspect, onOpenUrl, onOpenSkill, onRun }: {
  repos: Repo[]; catalog: Skill[]; counts: Record<string, number>; initialKey: string | null; busy: boolean; onClose: () => void;
  onSave: (repo: Repo, previousKey?: string) => Promise<boolean>; onDelete: (repo: Repo) => void;
  onInspect: (repo: Repo) => Promise<RepoInspection>; onOpenUrl: (url: string) => void;
  conflictIds: Set<string>;
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
            return <div className="mg-repo-skill" key={skillIdentityKey(s)}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={s.description}>{s.description || s.repoPath}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => repoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <span title={conflictIds.has(s.id) ? t("同名内容存在冲突") : pending ? state : undefined}><Button primary disabled={Boolean(pending) || conflictIds.has(s.id)} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id })}>{state}</Button></span>)}</div></div></div>;
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
function SkillBackupList({ backups, busy, onRestore }: {
  backups: SkillBackup[]; busy: boolean; onRestore: (backup: SkillBackup) => void;
}) {
  return <>{backups.map((backup) => <div key={backup.id} className="mg-source-row">
    <div><strong>{backup.name || backup.skillId}</strong><small>{backup.backedUpAt}</small></div>
    <div><small>{backup.purpose === "duplicate" ? t("重复副本归档") : backup.purpose === "update" ? t("更新前备份") : t("技能备份")}</small>
      <small>{backup.restorePath || t("恢复位置未记录")}</small>
      <Button disabled={busy} onClick={() => onRestore(backup)}>{t("恢复")}</Button></div>
  </div>)}</>;
}
function SkillBackupsDrawer({ backups, busy, onClose, onRestore }: {
  backups: SkillBackup[]; busy: boolean; onClose: () => void; onRestore: (backup: SkillBackup) => void;
}) {
  return <ManagementDrawer title={t("Skill 备份")} dismissOnBackdrop={false} onClose={onClose}>
    {backups.length ? <SkillBackupList backups={backups} busy={busy} onRestore={onRestore}/> : <p className="mg-muted">{t("暂无 Skill 备份。")}</p>}
  </ManagementDrawer>;
}
function SkillDetailDrawer({ skill, count, backups, busy, onClose, onOpenUrl, onOpenRepo, onMatch, onRestore, onUpdate, onReaPreview }: {
  skill: Skill; count?: number; backups: SkillBackup[]; busy: boolean; onClose: () => void; onOpenUrl: (url: string) => void;
  onOpenRepo: () => void; onMatch: (localPath?: string) => void; onRestore: (backup: SkillBackup) => void; onUpdate: (repoKey: string, localPath?: string) => void; onReaPreview: () => void;
}) {
  const url = repoUrl(skill);
  const channel = skillChannelLabel(skill);
  const owner = skill.id === "reverse-engineer-anything" ? t("需确认安装器") : skill.updateOwner === "in_place" ? t("Codex++ 原位维护") : skill.updateOwner === "manager" ? t("Codex++ 管理") : t("未接管");
  const selectableOriginal = skill.conflictKind === "independent" && !!skill.originalDir
    && skill.locations?.some((location) => !location.managed);
  const associatedOriginal = skill.conflictKind === "equal-independent" && !!skill.originalDir && !!url;
  const rootLabel: Record<string, string> = { shared: t("共享"), codex: "Codex", claude: "Claude Code", grok: "Grok" };
  return <ManagementDrawer dismissOnBackdrop={false} title={skill.name || skill.id} onClose={onClose}>
    {skill.conflict && <p className="mg-notice mg-warning" role="alert">{skill.displayInstances ? t("各 Agent 的安装内容或来源可能不同，请分别查看。") : skill.conflict}</p>}
    <dl className="mg-detail">
      <dt>{t("上游来源")}</dt><dd>{skill.displayInstances ? t("按安装位置查看") : url ? <button className="mg-link-button mg-skill-repo-link" type="button" onClick={onOpenRepo}>{sourceLabel(skill)}{count === undefined ? "" : ` · ${count} Skills`}<ArrowRight size={13}/></button> : skill.bundled ? t("内置") : t("来源未确认")}{associatedOriginal && <small> · {t("仅关联所选安装位置")}</small>}</dd>
      <dt>{t("安装渠道")}</dt><dd>{skill.displayInstances ? t("按安装位置查看") : channel}</dd>
      <dt>{t("更新方式")}</dt><dd>{skill.displayInstances ? t("按安装位置查看") : owner}</dd>
      <dt>{t("Skill ID")}</dt><dd>{skill.id}</dd>
      <dt>{t("仓库路径")}</dt><dd>{skill.repoPath || "—"}</dd>
      <dt>{t("介绍")}</dt><dd>{skill.description || t("暂无描述")}</dd>
      {url && <><dt>GitHub</dt><dd><button className="mg-link-button" type="button" onClick={() => onOpenUrl(url)}><ExternalLink size={13}/>{url}</button></dd></>}
      {skill.originalDir && <><dt>{t(associatedOriginal ? "关联来源的安装目录" : "原安装目录")}</dt><dd>{skill.originalDir}</dd></>}
      {skill.locations?.flatMap((location) => {
        const instance = skill.displayInstances?.find((item) => item.locations?.some((itemLocation) => itemLocation.manifestPath === location.manifestPath));
        const locationRepo = location.repoKey || instance?.repoKey || "";
        const locationRepoPath = location.repoPath || instance?.repoPath || "";
        const locationOwner = location.updateOwner || instance?.updateOwner;
        const hasRepo = !!repoFromKey(locationRepo);
        return [<dt key={`${location.manifestPath}:label`}>{rootLabel[location.rootKind] || location.rootKind}</dt>, <dd key={`${location.manifestPath}:value`}>{location.manifestPath}<br/>{location.managed ? t("Codex++ 受控") : t("本地安装")} · {location.statusError ? t("状态未知") : location.enabled ? t("已启用") : t("未启用")}
          {location.statusError && <><br/><span role="alert">{location.statusError}</span></>}
          {instance && <><br/>{t("安装渠道")}：{skillChannelLabel(instance)}</>}
          {(instance || (location.selectable && location.originalPath)) && <><br/>{t("上游来源")}：{hasRepo ? `${locationRepo}${locationRepoPath ? ` / ${locationRepoPath}` : ""}` : t("来源未确认")}
            {hasRepo && <><br/>{t("更新方式")}：{locationOwner === "in_place" ? t("Codex++ 原位维护") : locationOwner === "manager" ? t("Codex++ 管理") : t("未接管")}</>}
            {location.selectable && location.originalPath && (skill.displayInstances || skill.conflictKind === "equal-independent") && <div className="mg-row-actions"><Button disabled={busy} onClick={() => onMatch(location.originalPath || undefined)}>{t(hasRepo ? "重新关联此安装位置" : "关联此安装位置")}</Button>
              {hasRepo && locationOwner !== "manager" && skill.id !== "reverse-engineer-anything" && <Button disabled={busy} onClick={() => onUpdate(locationRepo, location.originalPath || undefined)}><CloudDownload size={14}/>{t("预览此位置更新")}</Button>}</div>}</>}</dd>];
      })}
    </dl>
    {skill.id === "impeccable" && (skill.displayInstances ? skill.displayInstances.map((instance) => <section key={skillIdentityKey(instance)}>
      <h3>{instance.locations?.map((location) => rootLabel[location.rootKind] || location.rootKind).join(" / ")}</h3>
      <ImpeccableVersions instanceKey={instance.instanceKey || instance.id}/>
    </section>) : <ImpeccableVersions instanceKey={skill.instanceKey || skill.id}/>)}
    {skill.installed && !skill.bundled && (!skill.conflict || selectableOriginal || associatedOriginal || skill.id === "reverse-engineer-anything") && <div className="mg-row-actions">
      {(!skill.conflict || selectableOriginal) && <Button disabled={busy} onClick={() => onMatch()}><Search size={14}/>{t("重新关联来源")}</Button>}
      {(!skill.conflict || selectableOriginal || associatedOriginal) && url && skill.originalDir && skill.updateOwner !== "manager" && skill.id !== "reverse-engineer-anything" && <Button disabled={busy} onClick={() => onUpdate(skill.repoKey, skill.originalDir || undefined)}><CloudDownload size={14}/>{t("预览原位更新")}</Button>}
      {skill.id === "reverse-engineer-anything" && <Button disabled={busy} onClick={onReaPreview}><CloudDownload size={14}/>{t("预览安装器更新")}</Button>}
    </div>}
    {backups.length > 0 && <section><h3>{t("备份")}</h3><SkillBackupList backups={backups} busy={busy} onRestore={onRestore}/></section>}
  </ManagementDrawer>;
}
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
    for (const id of operation.skillIds) if (!settled.has(id)) pendingById.set(id, operation.enabled);
  }
  const state = skillBatchState(group.items.map((skill) => {
    skill = prepareSkill(skill, agent);
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
  const [matchTarget, setMatchTarget] = useState<{ id?: string; localPath?: string } | null>(null);
  const [duplicatesOpen, setDuplicatesOpen] = useState(false);
  const [backupsOpen, setBackupsOpen] = useState(false);
  const [updateTarget, setUpdateTarget] = useState<{ id: string; repoKey: string; localPath?: string } | null>(null);
  const [reaPreviewOpen, setReaPreviewOpen] = useState(false);
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
            && skill.locations?.some((location) => location.originalPath === change.localPath);
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
  const openRepo = (key: string) => { setRepoSelection(key); setDetail(null); setRepoOpen(true); };
  const deferredQuery = useDeferredValue(query);
  const conflictIds = useMemo(() => {
    const counts = new Map<string, number>();
    const result = new Set<string>();
    for (const skill of data?.skills || []) {
      if (skill.conflict) result.add(skill.id);
      if (skill.installed && !skill.bundled) counts.set(skill.id, (counts.get(skill.id) || 0) + 1);
    }
    for (const [id, count] of counts) if (count > 1) result.add(id);
    return result;
  }, [data]);
  const knownSkills = useMemo(() => data?.skills || [], [data]);
  const canToggleSharedCodex = (skill: Skill) => Boolean(skill.locations?.some((location) => location.rootKind === "shared")
    && knownSkills.filter((item) => item.id === skill.id && item.installed
      && item.locations?.some((location) => location.rootKind === "codex" || location.rootKind === "shared")).length === 1);
  const batchSkill = (skill: Skill, agent: McpAgentId): Skill => {
    if (!conflictIds.has(skill.id)) return skill;
    return { ...skill, conflict: agent === "codex" && canToggleSharedCodex(skill) ? null : skill.conflict || t("同名 Skill 内容存在冲突") };
  };
  useEffect(() => {
    setAssignmentsByIdentity((old) => migrateSkillInstanceAssignments(old, customGroupAssignments, knownSkills, customGroupNames));
  }, [knownSkills, customGroupAssignments, customGroupNames]);
  const items = useMemo(() => knownSkills
    .filter((s) => (tab === "installed" ? s.installed : !s.installed || s.bundled)), [knownSkills, tab]);
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
  const groups = useMemo(() => {
    const result = groupSkills(items, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, !query.trim(), groupAliases, assignmentsByIdentity, knownRepositories)
      .map((group) => ({ ...group, items: aggregateSkillGroupItems(group.items).filter((skill) =>
        (skill.displayInstances || [skill]).some((item) => matchesManagementQuery(
          [item.id, item.name, item.description, item.repoKey, sourceLabel(item), ...(item.locations || []).map((location) => location.manifestPath)], deferredQuery))) }))
      .filter((group) => !query.trim() || group.items.length);
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [items, knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupSortDirection, groupAliases, builtinGroupLabel, userGroupLabel, localGroupLabel, query, deferredQuery]);
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
    if (!over || active.id === over.id || query.trim() || busy || loading) return;
    const movable = new Set(groups.filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key));
    if (!movable.has(String(active.id)) || !movable.has(String(over.id))) return;
    setGroupOrder(moveSkillGroup(allGroups, String(active.id), String(over.id)));
    setGroupSortDirection(null);
  };
  const agentCount = (agent: McpAgentId) => new Set((data?.skills || []).filter((s) =>
    s.installed && (agent !== "codex" || !skillCodexStatusError(s)) && skillAgents(s).includes(agent)).map((s) => s.id)).size;
  // 参照 MCP 页（Cursor 的 Customize）：每个 Skill 一行，右侧是各 Agent 的开关胶囊；分组是卡片外的小标题，
  // 标题右侧同一列位置是这一组的批量开关。详情、分组、卸载收进行末的三点菜单。
  const renderSkillRow = (s: Skill) => {
    const update = s.bundled ? undefined : updates.find((item) => item.id === s.id);
    const inPlaceUpdate = Boolean(s.originalDir && s.updateOwner !== "manager");
    const conflict = Boolean(s.conflict) || conflictIds.has(s.id);
    const statusError = skillCodexStatusError(s);
    const selectableOriginal = s.conflictKind === "independent" && !!s.originalDir
      && s.locations?.some((location) => !location.managed);
    const detail = conflict && s.locations?.length ? s.locations.map((location) => location.manifestPath).join(" · ") : s.description || t("暂无描述");
    // 共享原件由其他 Agent 共用，受控副本需通过重复预览处理。
    const canUninstall = s.installed && !s.bundled && !conflict && !s.locations?.some((location) => location.rootKind === "shared")
      && (s.managed || skillAgents(s).includes("codex") || s.agents === undefined);
    const assignment = skillGroupAssignment(s, customGroupNames, customGroupAssignments, assignmentsByIdentity);
    const on = s.installed && agentColumns.some((agent) => (agent !== "codex" || !statusError) && skillAgents(s).includes(agent));
    return <CxRow className="cx-skill-row" key={skillIdentityKey(s)}
      icon={<CxStatusIcon tone={on ? "ok" : "off"}><BookOpen /></CxStatusIcon>}
      title={s.name || s.id}
      badges={<>
        <small className="cx-skill-source" title={s.repoKey === "cc-switch" ? sourceLabel(s) : s.repoKey || sourceLabel(s)}>{sourceLabel(s)}</small>
        {statusError && <CxBadge tone="warn">{t("Codex 状态未知")}</CxBadge>}
        {conflict && !s.displayInstances ? <CxBadge tone="warn">{t(s.conflictKind === "equal-independent" ? "多个独立安装" : "同名冲突")}</CxBadge> : s.updateAvailable ? <CxBadge tone="warn">{t("可更新")}</CxBadge> : null}
      </>}
      detail={<span title={detail}>{detail}</span>}
      onClick={() => setDetail(s)}>
      {update ? <span title={update.repoKey !== s.repoKey ? t("同 ID 操作中") : undefined}><Button disabled onClick={() => {}}>{t(update.command === "uninstall_skill" ? "卸载排队中" : update.repoKey !== s.repoKey ? "同 ID 操作中" : update.state === "queued" ? "排队中" : update.command === "install_skill" ? "安装中" : "更新中")}</Button></span> : !s.installed && !s.bundled ? <Button disabled={conflict} onClick={() => { queueUpdate("install_skill", { repoKey: s.repoKey, id: s.id }); }}>{t("安装")}</Button> : s.installed && s.updateAvailable && !conflict ? <Button disabled={s.id === "reverse-engineer-anything"} onClick={() => { if (inPlaceUpdate) setUpdateTarget({ id: s.id, repoKey: s.repoKey, localPath: s.originalDir || undefined }); else queueUpdate("update_skill", { repoKey: s.repoKey, id: s.id }); }}>{t(s.id === "reverse-engineer-anything" ? "使用安装器更新" : "更新")}</Button> : null}
      <div className="cx-agent-toggles" role="group" aria-label={t("启用到 Agent")}>
        {agentColumns.map((agent) => {
          const safeDisable = conflict && s.conflictKind === "managed-modified"
            && s.safeDisableAgents?.includes(agent) && skillAgents(s).includes(agent);
          const safeCodex = conflict && agent === "codex" && canToggleSharedCodex(s);
          return <AgentToggle agent={agent} key={agent} name={s.name || s.id}
            enabled={s.installed && !(agent === "codex" && statusError) && skillAgents(s).includes(agent)}
            statusUnknown={agent === "codex" && !!statusError}
            disabled={busy || !s.installed || s.bundled || (agent === "codex" && !!statusError) || (conflict && !safeDisable && !safeCodex) || skillBatchHasPending(batchOperations, [s], agent)}
            title={data?.agentSkillsDirs?.[agent]}
            hint={agent === "codex" && statusError ? `${t("状态未知")}：${statusError}` : safeDisable ? t("停用并备份已修改的受控副本") : safeCodex ? t("只修改 Codex 的技能启用状态") : conflict ? s.conflict || t("同名 Skill 内容存在冲突") : !s.installed ? t("未安装") : s.bundled ? t("Codex 自带，只读") : undefined}
            onToggle={async () => {
              if (safeDisable && !(await appConfirm(t("此受控副本已修改，停用时将移入备份。"), {
                title: t("停用并备份"), confirmText: t("停用并备份"),
              }))) return;
              await run("set_skill_enabled_for", { agent, id: s.id, enabled: safeDisable ? false : !skillAgents(s).includes(agent) });
            }} />;
        })}
      </div>
      <Menu>
        <MenuItem onClick={() => setDetail(s)}>{t("查看详情")}</MenuItem>
        {s.installed && !s.bundled ? <MenuItem disabled={Boolean(s.displayInstances) || (conflict && !selectableOriginal)} onClick={() => setMatchTarget({ id: s.id, localPath: selectableOriginal ? s.originalDir || undefined : undefined })}>{t("重新关联来源")}</MenuItem> : null}
        {repoUrl(s) ? <MenuItem onClick={() => openRepo(s.repoKey)}>{t("管理仓库")}</MenuItem> : null}
        {!s.bundled && (!s.conflict || s.instanceKey) && customGroupNames.length ? <>
          <div className="mg-menu-label">{t("分组")}</div>
          <MenuItem checked={!assignment} onClick={() => assignSkillGroup(s, "")}>{t("按来源")}</MenuItem>
          {customGroupNames.map((group) => <MenuItem checked={assignment === customSkillGroupKey(group)} key={group} onClick={() => assignSkillGroup(s, group)}>{group}</MenuItem>)}
        </> : null}
        {canUninstall ? <MenuItem danger disabled={busy || Boolean(update)} onClick={() => void run("uninstall_skill", { id: s.id })}>{t("卸载")}</MenuItem> : null}
      </Menu>
    </CxRow>;
  };
  const discoverCount = (data?.skills || []).filter((s) => !s.installed || s.bundled).length;
  const installedCount = aggregateSkillGroupItems((data?.skills || []).filter((s) => s.installed)).length;
  const sortLabel = `${t("按名称排序")} (${groupSortDirection === "asc" ? "A-Z" : groupSortDirection === "desc" ? "Z-A" : "A-Z / Z-A"})`;
  const currentDetail = detail ? allGroups.flatMap((group) => group.items).find((skill) => skillIdentityKey(skill) === skillIdentityKey(detail))
    || (conflictIds.has(detail.id) ? null : detail) : null;
  return <div className="mg-page cx-page mg-skills-page">
    <Header icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load()} disabled={busy || loading}><RefreshCw size={14}/>{t("刷新目录")}</Button>
      <Menu>
        <MenuItem onClick={() => void importLocalSkills()} disabled={busy}>{t("从本地目录导入")}</MenuItem>
        <MenuItem onClick={() => void run("import_ccs_skills", {})} disabled={busy}>{t("从 cc-switch 导入")}</MenuItem>
        <MenuItem onClick={() => setDuplicatesOpen(true)} disabled={busy}>{t("预览重复副本")}</MenuItem>
        <MenuItem onClick={() => setBackupsOpen(true)} disabled={busy}>{t("查看 Skill 备份")}</MenuItem>
      </Menu>
    </>} />
    <CxToolbar query={query} onQuery={setQuery} placeholder={t("搜索 Skills 或仓库")}>
      <button type="button" className="cx-primary" onClick={() => setRepoOpen(true)}><Link size={15}/>{t("管理仓库")}</button>
    </CxToolbar>
    <CxFilters label="Skills" value={tab} onChange={setTab}
      items={[{ id: "installed", label: t("已安装"), count: installedCount }, { id: "discover", label: t("发现"), count: discoverCount }]}
      trailing={<>
        <button type="button" className="mg-icon-button" title={t("匹配仓库")} aria-label={t("匹配仓库")}
          disabled={busy || loading} onClick={() => setMatchTarget({})}><Search size={16}/></button>
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
      detail={<ul className="cx-skill-failures" tabIndex={0} aria-label={t("失败的 Skill 列表")}>{updateFailures.details.map((detail) => <li key={JSON.stringify([detail.repoKey, detail.id])}><span>{detail.repoKey}</span> · <strong>{detail.id}</strong>：{detail.message}</li>)}</ul>}>
      <button type="button" className="mg-icon-button" aria-label={t("关闭 Skill 失败摘要")} title={t("关闭 Skill 失败摘要")} onClick={skillBatchRegistry.clearFailures}><X size={16}/></button>
    </CxBanner>}
    {data?.repoErrors?.length ? <CxBanner tone="warn" icon={<Link />} title={t("部分仓库读取失败")} detail={<span className="cx-skill-repo-errors">{data.repoErrors.map((e) => <span key={e}>{e}</span>)}</span>} /> : null}
    <div className="cx-skill-groups" aria-busy={loading} data-columns={agentColumns.length}>
      {!groups.length ? <CxCard><CxEmpty title={loading ? t("正在扫描…") : t("没有匹配的 Skill")} /></CxCard> :
        <DndContext sensors={groupSensors} collisionDetection={closestCenter} onDragEnd={handleGroupDragEnd}>
          <SortableContext items={groups.map((group) => group.key)} strategy={verticalListSortingStrategy}>{groups.map((group) => {
        const collapsed = !expandedGroups.has(group.key) && !query.trim();
        const allGroupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
        const updateTargets = allGroupItems.filter((skill) => skill.installed && !skill.bundled && !conflictIds.has(skill.id) && skill.id !== "reverse-engineer-anything" && skill.updateAvailable && (!skill.originalDir || skill.updateOwner === "manager"));
        const pendingUpdates = allGroupItems.filter((skill) => skill.installed && !skill.bundled && updates.some((item) => item.command === "update_skill" && item.id === skill.id && item.repoKey === skill.repoKey)).length;
        const hasUpdates = pendingUpdates > 0 || updateTargets.length > 0;
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
                  prepareSkill={batchSkill}
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
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} initialKey={repoSelection} busy={busy} updates={updates} conflictIds={conflictIds} onClose={() => setRepoOpen(false)} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(command, args) => { queueUpdate(command, args); }} />}
    {currentDetail && <SkillDetailDrawer skill={conflictIds.has(currentDetail.id) && !currentDetail.conflict ? { ...currentDetail, conflict: t("同名 Skill 内容存在冲突，请先确认各安装位置。") } : currentDetail}
      count={data?.repoCounts?.[currentDetail.repoKey]} backups={(data?.backups || []).filter((backup) => backup.skillId === currentDetail.id)} busy={busy}
      onClose={() => setDetail(null)} onOpenUrl={(url) => void openUrl(url)} onOpenRepo={() => repoOpen ? setDetail(null) : openRepo(currentDetail.repoKey)}
      onMatch={(localPath) => { setDetail(null); setMatchTarget({ id: currentDetail.id, localPath: localPath || (currentDetail.conflictKind === "independent" ? currentDetail.originalDir || undefined : undefined) }); }}
      onRestore={(backup) => void restoreBackup(backup)}
      onUpdate={(repoKey, localPath) => { setUpdateTarget({ id: currentDetail.id, repoKey, localPath }); setDetail(null); }}
      onReaPreview={() => { setDetail(null); setReaPreviewOpen(true); }}/>}
    {matchTarget && <SkillSourceMatchDrawer repos={data?.repos || []} id={matchTarget.id} localPath={matchTarget.localPath}
      localSkillIds={knownSkills.filter((skill) => skill.installed && !skill.bundled
        && (!skill.repoPath || !skill.repoKey.includes("/"))).map((skill) => skill.id)}
      onClose={() => setMatchTarget(null)} onAddRepo={addMatchingRepo} onApply={applySourceMatches}/>}
    {duplicatesOpen && <SkillDuplicatesDrawer onClose={() => setDuplicatesOpen(false)} onArchive={(previewId, id) => run("archive_skill_duplicate", { previewId, id })}/>}
    {backupsOpen && <SkillBackupsDrawer backups={data?.backups || []} busy={busy} onClose={() => setBackupsOpen(false)} onRestore={(backup) => void restoreBackup(backup)}/>}
    {updateTarget && <SkillInPlaceUpdateDrawer id={updateTarget.id} repoKey={updateTarget.repoKey} localPath={updateTarget.localPath} onClose={() => setUpdateTarget(null)} onExecute={(previewId) => run("execute_skill_in_place_update", { previewId })}/>}
    {reaPreviewOpen && <ReaUpdatePreviewDrawer onClose={() => setReaPreviewOpen(false)}/>}
  </div>;
}
export function SessionPage({ sessions, onRefresh, onDelete, onDeleteMany, notify, repairPanel }: { sessions: { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number } | null; onRefresh: (offset?: number) => Promise<void>; onDelete: (s: ManagedSession) => Promise<void>; onDeleteMany: (s: ManagedSession[]) => Promise<void>; notify: Notify; repairPanel?: ReactNode }) { const [query, setQuery] = useState(""); const [mode, setMode] = useState<"project" | "time">("project"); const [expanded, setExpanded] = useState<Set<string>>(new Set()); const [selection, setSelection] = useState(false); const [selected, setSelected] = useState<Set<string>>(new Set()); const [detail, setDetail] = useState<ManagedSession | null>(null); const all = sessions?.sessions || []; const visible = all.filter((s) => matchesManagementQuery([s.title, s.cwd, s.id, s.modelProvider], query)); const groups = sessionGroups(visible, mode); const selectionScope = visible.map((s) => s.id).join("\0"); useEffect(() => { const ids = new Set(selectionScope.split("\0")); setSelected((old) => { const next = new Set([...old].filter((id) => ids.has(id))); return next.size === old.size ? old : next; }); }, [selectionScope]); const selectedRows = visible.filter((s) => selected.has(s.id)); const toggle = (id: string) => setSelected((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; }); return <div className="mg-page mg-session-page"><Header icon={<History size={20}/>} title={t("会话")} actions={<><Button onClick={() => void onRefresh()}><RefreshCw size={14}/>{t("刷新列表")}</Button><Button onClick={() => { setSelection(!selection); setSelected(new Set()); }}>{t("选择多个…")}</Button></>} />{repairPanel}<div className="mg-session-controls"><SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} /><div className="mg-segmented"><button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button><button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button></div>{selection && <><Button onClick={() => setSelected(new Set(visible.map((s) => s.id)))}>{t("全选当前列表")}</Button><Button disabled={!selectedRows.length} onClick={() => setSelected(new Set())}>{t("取消全部选择")}</Button><Button primary disabled={!selectedRows.length} onClick={() => void onDeleteMany(selectedRows)}><Trash2 size={14}/>{t("删除已选")}</Button></>}</div><div className="mg-content"><div className="mg-session-list">{!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((g) => { const open = query.trim() || expanded.has(g.key) || groups.length === 1; return <section key={g.key}><button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(g.key) ? next.delete(g.key) : next.add(g.key); return next; })}><ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"}/><strong>{g.label}</strong><span>{g.path}</span><small>{g.sessions.length} {t("个会话")}</small></button>{open && g.sessions.map((s) => <article className="mg-session-row" key={s.id}><div className="mg-session-check">{selection && <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)}/>}</div><button type="button" className="mg-session-main" onClick={() => setDetail(s)}><strong>{s.title || t("未命名会话")}</strong><small>{s.cwd || t("未记录项目路径")}</small><code>{s.id}</code></button><div className="mg-session-meta"><span>{s.modelProvider || "Codex"}</span><time>{s.updatedAtMs ? new Date(s.updatedAtMs).toLocaleString() : t("未知时间")}</time></div><div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(s)} aria-label={t("删除会话")}><Trash2 size={15}/></button></div></article>)}</section>; })}</div></div><div className="mg-pagination"><Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - (sessions?.limit || 50)))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14}/>{t("上一页")}</Button><span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [Math.floor(sessions.offset / sessions.limit) + 1, sessions.totalCount]) : "—"}</span><Button onClick={() => void onRefresh((sessions?.offset || 0) + (sessions?.limit || 50))} disabled={!sessions?.hasMore}><ArrowRight size={14}/>{t("下一页")}</Button></div>{detail && <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14}/>{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14}/>{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer>}</div>; }
