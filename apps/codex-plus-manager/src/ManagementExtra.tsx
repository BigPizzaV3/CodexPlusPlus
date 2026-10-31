import { startTransition, useDeferredValue, useEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { AlertTriangle, ArrowDownAZ, ArrowDownUp, ArrowDownZA, ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, ChevronRight, ChevronsDown, ChevronsUp, CloudDownload, Copy, Edit3, ExternalLink, FolderOpen, FolderPlus, GripVertical, History, Link, MoreHorizontal, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { t, tf } from "./i18n";
import { appConfirm, appPrompt } from "./app-dialog";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { AgentColumn, ManagementDrawer, MatrixCell, SearchField } from "./ManagementPages";
import { MCP_AGENTS, type McpAgentId } from "./agent-mcp-model";
import "./ManagementPages.css";
import { parseSkillRepository } from "./skill-repository";
import { SkillListViewport } from "./SkillListViewport";
import { customSkillGroupKey, defaultSkillGroupKey, groupSkills, legacySkillCustomGroupName, moveSkillGroup, nextSkillGroupSortDirection, readSkillGroupStorage, skillGroupAssignment, skillIdentityKey, sortSkillGroupKeys, sortSkillGroupsByName, type SkillGroup, type SkillGroupSortDirection } from "./skill-groups";
import { mergeSkillAgentResult, skillAgents, skillBatchHasPending, skillBatchState, skillBatchTargets, type SkillBatchOperation } from "./skill-agent-batch";
import { checkRepositories, SkillTaskQueue } from "./skill-task-queue";
import { usePopoverLayer } from "./use-popover-layer";
import { useEscapeLayer } from "./use-escape-layer";
import { AppSelect } from "./AppSelect";
import { TopbarActions } from "./TopbarActions";
import { SkillRepositoryMatchDrawer } from "./SkillRepositoryMatchDrawer";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean;
  /** 已经启用到哪些 Agent（codex / claude / grok）。 */
  agents?: string[];
  /** Codex++ 的受控源里有它；只在某个 Agent 自己目录里的 Skill 为 false。 */
  managed?: boolean; codexConflict?: boolean; importSource?: string };
type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type SkillPayload = Result & { skills: Skill[]; repos: Repo[]; pendingImportedRepos?: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string; agentSkillsDirs?: Record<string, string> };
type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
type PendingSkillBatchOperation = SkillBatchOperation & { settledSkillIds: string[] };
type RepoCheck = { state: "queued" | "running" | "done" | "failed"; message?: string };
type SkillImportMatch = { id: string; name: string; reasons: string[]; snapshot: string };
type SkillImportCandidate = { id: string; name: string; description: string; sourceHash: string; repoKey: string; repoPath: string; matches: SkillImportMatch[]; duplicateCandidates: string[]; error: string | null };
type SkillImportPreview = Result & { candidates: SkillImportCandidate[]; warning: string };
type ImportRequest = { source: "local" | "cc-switch"; path: string | null; preview: SkillImportPreview; revision: number };
const skillUpdateQueue = new SkillTaskQueue<SkillPayload>();
const SKILL_GROUP_STORAGE_KEY = "codex-plus-plus.skill-groups.v1";
const ok = (r: Result) => r.status === "ok";
function Button({ children, onClick, disabled, primary }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean }) { return <button type="button" className={"mg-button " + (primary ? "mg-primary" : "")} onClick={onClick} disabled={disabled}>{children}</button>; }
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
function importChannel(skill: Skill) { return skill.importSource === "cc-switch" ? "cc-switch" : skill.importSource === "local" ? t("本地导入") : ""; }
function repoUrl(skill: Skill) {
  if (!skill.repoKey || skill.repoKey === "cc-switch" || skill.repoKey === LOCAL_IMPORT_REPO_KEY) return "";
  const parts = skill.repoKey.split("@")[0].split("/");
  if (parts.length < 2) return "";
  return `https://github.com/${parts.join("/")}${skill.repoPath ? `/tree/${skill.repoKey.split("@")[1]?.split(":")[0] || "main"}/${skill.repoPath}` : ""}`;
}
function SkillRepoDrawer({ repos, pendingRepos, catalog, counts, checks, initialKey, busy, onClose, onSave, onDelete, onInspect, onOpenUrl, onOpenSkill, onRun }: {
  repos: Repo[]; catalog: Skill[]; counts: Record<string, number>; initialKey: string | null; busy: boolean; onClose: () => void;
  pendingRepos: Repo[];
  checks: Record<string, RepoCheck>;
  onSave: (repo: Repo, previousKey?: string) => Promise<boolean>; onDelete: (repo: Repo) => void;
  onInspect: (repo: Repo) => Promise<RepoInspection>; onOpenUrl: (url: string) => void;
  onOpenSkill: (skill: Skill) => void; onRun: (command: string, args: Record<string, unknown>) => void;
}) {
  const allRepos = useMemo(() => [...repos, ...pendingRepos.filter((pending) => !repos.some((repo) => repoKey(repo) === repoKey(pending)))], [repos, pendingRepos]);
  const [draft, setDraft] = useState<Repo>({ owner: "", name: "", branch: "main", subdir: "", enabled: true });
  const [input, setInput] = useState("");
  const [parseError, setParseError] = useState("");
  const [editing, setEditing] = useState<Repo | null>(null);
  const [selected, setSelected] = useState<Repo | null>(() => allRepos.find((r) => repoKey(r) === initialKey) || repoFromKey(initialKey) || allRepos[0] || null);
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
  useEffect(() => { if (!selected && allRepos[0]) view(allRepos[0]); }, [allRepos, selected]);
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
  const active = allRepos.find((r) => selected && repoKey(r) === repoKey(selected))
    || allRepos.find((r) => selected && r.owner === selected.owner && r.name === selected.name && r.subdir === selected.subdir)
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
      {!repos.length && !pendingRepos.length && <p className="mg-muted">{t("还没有添加仓库。")}</p>}
      {allRepos.map((repo) => <div className="mg-repo-row" data-selected={active && repoKey(active) === repoKey(repo)} key={repoKey(repo)}>
        <button type="button" className="mg-repo-select" onClick={() => view(repo)}><strong title={`${repo.owner}/${repo.name}`}>{repo.owner}/{repo.name}</strong><small title={checks[repoKey(repo)]?.message || `${repo.branch}${repo.subdir ? ` · ${repo.subdir}` : ""}`}>{repo.branch}{repo.subdir ? ` · ${repo.subdir}` : ""}<span>{checks[repoKey(repo)]?.state === "running" ? t("正在检查…") : checks[repoKey(repo)]?.state === "queued" ? t("等待检查") : checks[repoKey(repo)]?.state === "failed" ? t("检查失败") : counts[repoKey(repo)] === undefined ? t("待扫描") : `${counts[repoKey(repo)]} Skills`}{!repo.enabled ? ` · ${t("已停用")}` : ""}</span></small></button>
        <div className="mg-repo-row-actions">{!pendingRepos.some((pending) => repoKey(pending) === repoKey(repo)) && <button className={"mg-switch " + (repo.enabled ? "is-on" : "")} type="button" role="switch" aria-checked={repo.enabled} disabled={busy} aria-label={`${repo.owner}/${repo.name} ${t("启用仓库")}`} onClick={() => void onSave({ ...repo, enabled: !repo.enabled })}><span /></button>}
        <button type="button" className="mg-icon-button" title={t("编辑仓库")} aria-label={`${t("编辑仓库")} ${repo.owner}/${repo.name}`} onClick={() => edit(repo)}><Edit3 size={14}/></button>
        <button className="mg-icon-button" title={t("删除仓库")} aria-label={`${t("删除仓库")} ${repo.owner}/${repo.name}`} disabled={busy} onClick={() => { cancelScan(); onDelete(repo); if (active && repoKey(active) === repoKey(repo)) { setSelected(allRepos.find((r) => repoKey(r) !== repoKey(repo)) || null); setPreview(null); } }}><Trash2 size={14}/></button></div></div>)}
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
          renderRow={(s) => <div className="mg-repo-skill" key={s.repoKey + ":" + s.id}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={[s.repoPath, s.description].filter(Boolean).join(" · ")}>{[s.repoPath, s.description].filter(Boolean).join(" · ")}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => repoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <Button primary disabled={busy} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id, expectedRepoPath: s.repoPath ?? null })}>{t("安装")}</Button>)}</div></div></div>} />
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
function SkillDetailDrawer({ skill, count, group, originGroup, groups, onAssign, onClose, onOpenUrl, onOpenRepo, onEditSource, onMatch }: {
  skill: Skill; count?: number; group?: SkillGroup<Skill>; originGroup?: SkillGroup<Skill>; groups: SkillGroup<Skill>[];
  onAssign: (group: string) => void; onClose: () => void; onOpenUrl: (url: string) => void; onOpenRepo: () => void; onEditSource: () => void; onMatch: () => void;
}) {
  const url = repoUrl(skill);
  const canMatch = skill.installed && !skill.bundled;
  return <ManagementDrawer dismissOnBackdrop={false} title={skill.name || skill.id} onClose={onClose}>
    <dl className="mg-detail"><dt>{t("所属分组")}</dt><dd>{group && skill.installed && !skill.bundled
      ? <AppSelect value={group.key} className="mg-skill-long-select" title={group.label} ariaLabel={`${t("切换分组")} ${skill.name || skill.id}`} onChange={onAssign}
          options={[...groups.filter((item) => item.kind !== "builtin").map((item) => ({ value: item.key, label: item.label, title: item.label })),
            ...(originGroup ? [{ value: "", label: `${t("按来源自动分组")} · ${originGroup.label}`, title: `${t("按来源自动分组")} · ${originGroup.label}` }] : [])]} />
      : group?.label || t("未安装")}</dd>
      <dt>{t("来源")}</dt><dd>{url ? <button className="mg-link-button mg-skill-repo-link" type="button" onClick={onOpenRepo}>{sourceLabel(skill)}{count === undefined ? "" : ` · ${count} Skills`}<ArrowRight size={13}/></button> : sourceLabel(skill)}</dd>
      {importChannel(skill) && <><dt>{t("导入渠道")}</dt><dd>{importChannel(skill)}</dd></>}
      <dt>{t("Skill ID")}</dt><dd>{skill.id}</dd><dt>{t("仓库路径")}</dt><dd>{skill.repoPath || "—"}</dd>
      <dt>{t("介绍")}</dt><dd>{skill.description || t("暂无描述")}</dd>
      <dt>{t("GitHub 仓库")}</dt><dd className="mg-skill-detail-repository">{url ? <button className="mg-link-button" type="button" onClick={() => onOpenUrl(url)}><ExternalLink size={13}/>{url}</button> : <span className="mg-muted">{t("未关联")}</span>}
        {canMatch && <><button type="button" className="mg-icon-button" aria-label={t("修改仓库来源")} title={t("修改仓库来源")} onClick={onEditSource}><Edit3 size={15}/></button>
          <button type="button" className="mg-icon-button" aria-label={t("匹配仓库来源")} title={t("匹配仓库来源")} onClick={onMatch}><Search size={15}/></button></>}
      </dd></dl>
  </ManagementDrawer>;
}
function SkillImportDrawer({ request, busy, onClose, onRefresh, onApply }: { request: ImportRequest; busy: boolean; onClose: () => void; onRefresh: () => void;
  onApply: (selections: { id: string; sourceHash: string; repoKey: string; repoPath: string; targetId: string | null; targetSnapshot: string | null }[]) => void }) {
  const candidates = request.preview.candidates;
  const [selected, setSelected] = useState<Set<string>>(() => new Set(candidates.filter((item) => !item.error && !item.matches.length && !item.duplicateCandidates.length).map((item) => item.id)));
  const [targets, setTargets] = useState<Record<string, string>>(() => Object.fromEntries(candidates.filter((item) => !item.error && item.matches.length === 1).map((item) => [item.id, item.matches[0].id])));
  const chosen = candidates.filter((item) => !item.error && selected.has(item.id));
  const invalidTargets = chosen.some((item) => item.matches.length > 0 && !item.matches.some((match) => match.id === targets[item.id]));
  const duplicateIncoming = chosen.some((item) => item.duplicateCandidates.some((id) => selected.has(id)));
  const targetList = chosen.map((item) => targets[item.id]).filter(Boolean);
  const duplicateTargets = new Set(targetList).size !== targetList.length;
  const submit = () => onApply(chosen.map((item) => {
    const match = item.matches.find((entry) => entry.id === targets[item.id]);
    return { id: item.id, sourceHash: item.sourceHash, repoKey: item.repoKey, repoPath: item.repoPath, targetId: match?.id || null, targetSnapshot: match?.snapshot || null };
  }));
  return <ManagementDrawer wide dismissOnBackdrop={false} title={t("选择要导入的 Skills")} onClose={busy ? () => {} : onClose}
    footer={<><Button onClick={onRefresh} disabled={busy}><RefreshCw size={14}/>{t("重新检查")}</Button><Button onClick={onClose} disabled={busy}>{t("取消")}</Button><Button primary onClick={submit} disabled={busy || !chosen.length || invalidTargets || duplicateIncoming || duplicateTargets}>{tf("导入所选 {0} 项", [chosen.length])}</Button></>}>
    {request.preview.warning && <p className="mg-notice mg-warning">{request.preview.warning}</p>}
    <p className="mg-skill-import-note">{t("覆盖时会同步已启用的 Agent 副本，并保留原版本备份。")}</p>
    <div className="mg-skill-import-toolbar"><span>{tf("共 {0} 项，已选 {1} 项", [candidates.length, chosen.length])}</span><div className="mg-row-actions">
      <Button disabled={busy} onClick={() => setSelected(new Set(candidates.filter((item) => !item.error && !item.matches.length && !item.duplicateCandidates.length).map((item) => item.id)))}>{t("选择无重复项")}</Button>
      <Button disabled={busy} onClick={() => setSelected(new Set(candidates.filter((item) => !item.error).map((item) => item.id)))}>{t("选择全部含覆盖")}</Button>
      <Button disabled={busy} onClick={() => setSelected(new Set())}>{t("清空选择")}</Button>
    </div></div>
    {(invalidTargets || duplicateIncoming || duplicateTargets) && <p className="mg-notice mg-warning" role="alert">{duplicateIncoming ? t("所选导入项互相重复，请只保留其中一个。") : duplicateTargets ? t("多个导入项不能覆盖同一个已有 Skill。") : t("请为每个重复项选择要覆盖的 Skill。")}</p>}
    <div className="mg-skill-import-list">{candidates.map((item) => {
      const targetOptions = item.matches.map((match) => {
        const label = `${match.name} (${match.id})`;
        return { value: match.id, label, title: label };
      });
      return <div className="mg-skill-import-row" key={item.id}>
      <label><input type="checkbox" checked={!item.error && selected.has(item.id)} disabled={busy || !!item.error} onChange={(event) => { const checked = event.currentTarget.checked; setSelected((old) => { const next = new Set(old); checked ? next.add(item.id) : next.delete(item.id); return next; }); }}/>
        <span><strong>{item.name || item.id}</strong><small title={item.description}>{item.id}{item.repoKey ? ` · ${item.repoKey.split("@", 1)[0]}` : ` · ${t("用户")}`}</small></span></label>
      {item.error ? <small className="mg-skill-import-error" role="alert">{item.error}</small> : item.matches.length ? <div className="mg-skill-import-target"><span>{t("覆盖已有")}</span><AppSelect value={targets[item.id] || ""} className="mg-skill-long-select" title={targetOptions.find((option) => option.value === targets[item.id])?.label || ""} ariaLabel={`${t("选择覆盖目标")} ${item.name || item.id}`} placeholder={t("选择覆盖目标")}
        disabled={!selected.has(item.id) || busy} options={targetOptions}
        onChange={(target) => setTargets((old) => ({ ...old, [item.id]: target }))}/></div> : item.duplicateCandidates.length ? <small className="mg-skill-import-duplicate">{t("导入清单中重复")}</small> : <small>{t("新增")}</small>}
    </div>; })}</div>
  </ManagementDrawer>;
}
function SortableSkillGroup({ group, disabled, children }: { group: SkillGroup<Skill>; disabled: boolean; children: (handle: ReactNode) => ReactNode }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: group.key, disabled });
  const handle = disabled ? <span className="mg-skill-drag-placeholder" aria-hidden="true" /> :
    <button type="button" className="mg-icon-button mg-skill-drag" title={t("拖动分组排序")} aria-label={`${t("拖动分组排序")} ${group.label}`} {...attributes} {...listeners}><GripVertical size={15}/></button>;
  return <section className="mg-skill-group" ref={setNodeRef} data-dragging={isDragging}
    style={{ transform: CSS.Transform.toString(transform), transition }}>{children(handle)}</section>;
}
function GroupAgentSelect({ group, agent, busy, operations, onChange }: { group: SkillGroup<Skill>; agent: McpAgentId; busy: boolean; operations: PendingSkillBatchOperation[]; onChange: (enabled: boolean) => void }) {
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
  return <label className="mg-skill-batch-cell" title={`${group.label} · ${name} · ${t("全选")}`}>
    <input type="checkbox" aria-label={`${group.label} · ${name} · ${t("全选")}`} checked={state.checked}
      ref={(element) => { if (element) element.indeterminate = state.mixed; }} disabled={busy || !state.total}
      onChange={(event) => onChange(event.target.checked)} />
  </label>;
}
function SkillConflictDialog({ skill, busy, onClose, onResolve }: { skill: Skill; busy: boolean; onClose: () => void; onResolve: (keepCodex: boolean) => void }) {
  const [keepCodex, setKeepCodex] = useState(true);
  const close = () => { if (!busy) onClose(); };
  useEscapeLayer(true, close);
  return <div className="modal-backdrop app-dialog-backdrop" role="dialog" aria-modal="true" aria-labelledby="skill-conflict-title"
    onMouseDown={(event) => { if (event.target === event.currentTarget) close(); }}>
    <div className="modal-card confirm-modal app-dialog mg-skill-conflict-dialog">
      <div className="modal-head"><h2 id="skill-conflict-title">{t("Skill 版本不一致")}</h2><button type="button" className="toast-close" aria-label={t("关闭")} disabled={busy} onClick={close}>×</button></div>
      <div className="confirm-modal-body"><p>{tf("“{0}”的 Codex 副本与 Codex++ 管理库不同，请选择保留的版本。", [skill.name || skill.id])}</p>
        <label className="mg-skill-version-choice"><input type="radio" name="skill-version" checked={keepCodex} disabled={busy} onChange={() => setKeepCodex(true)}/><span><strong>{t("保留 Codex 中修改后的版本")}</strong><small>{t("使用 Codex 副本更新管理库")}</small></span></label>
        <label className="mg-skill-version-choice"><input type="radio" name="skill-version" checked={!keepCodex} disabled={busy} onChange={() => setKeepCodex(false)}/><span><strong>{t("保留 Codex++ 管理库版本")}</strong><small>{t("使用管理库替换 Codex 副本")}</small></span></label>
        <small>{t("被替换的版本会保留在备份中。")}</small>
      </div>
      <div className="app-dialog-actions"><button type="button" className="mg-button" disabled={busy} onClick={close}>{t("稍后处理")}</button><button type="button" className="mg-button mg-primary" disabled={busy} onClick={() => onResolve(keepCodex)}>{t("保留所选版本")}</button></div>
    </div>
  </div>;
}
function SkillActionMenu({ skill, groups, group, originGroup, assignment, busy, canUninstall, onDetail, onAssign, onUninstall }: {
  skill: Skill; groups: SkillGroup<Skill>[]; group?: SkillGroup<Skill>; originGroup?: SkillGroup<Skill>; assignment?: string; busy: boolean; canUninstall: boolean;
  onDetail: () => void; onAssign: (group: string) => void; onUninstall: () => void;
}) {
  const [open, setOpen] = useState(false);
  const [submenu, setSubmenu] = useState(false);
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({ open, onOpenChange: setOpen, align: "end", menuWidth: 216, maxHeight: 340 });
  useEffect(() => { if (!open) setSubmenu(false); }, [open]);
  const select = (action: () => void) => { setOpen(false); action(); };
  return <div ref={rootRef} className="mg-menu-wrap">
    <button type="button" className="mg-icon-button" aria-label={`${t("更多操作")} ${skill.name || skill.id}`} title={t("更多操作")}
      aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen((value) => !value)}><MoreHorizontal size={17}/></button>
    {open && createPortal(<div ref={menuRef} className="mg-skill-action-menu" role="menu" style={menuStyle}>
      {submenu ? <>
        <button type="button" role="menuitem" className="mg-skill-menu-back" onClick={() => setSubmenu(false)}><ArrowLeft size={14}/><span>{t("切换分组")}</span></button>
        {group && <button type="button" role="menuitemradio" aria-checked="true" title={group.label} onClick={() => setOpen(false)}><span>{group.label}</span><Check size={14}/></button>}
        {groups.filter((item) => item.kind !== "builtin" && item.key !== group?.key).map((item) =>
          <button type="button" role="menuitemradio" aria-checked="false" key={item.key} title={item.label} onClick={() => select(() => onAssign(item.key))}><span>{item.label}</span></button>)}
        {assignment && originGroup && <><div className="mg-skill-menu-divider"/><button type="button" role="menuitem" title={originGroup.label} onClick={() => select(() => onAssign(""))}><span>{t("按来源自动分组")} · {originGroup.label}</span></button></>}
      </> : <>
        <button type="button" role="menuitem" onClick={() => select(onDetail)}><BookOpen size={14}/><span>{t("查看详情")}</span></button>
        {skill.installed && !skill.bundled && <button type="button" role="menuitem" aria-haspopup="menu" onClick={() => setSubmenu(true)}><FolderOpen size={14}/><span>{t("切换分组")}</span><ChevronRight size={14}/></button>}
        {canUninstall && <><div className="mg-skill-menu-divider"/><button type="button" role="menuitem" className="mg-danger" disabled={busy} onClick={() => select(onUninstall)}><Trash2 size={14}/><span>{t("卸载")}</span></button></>}
      </>}
    </div>, document.body)}
  </div>;
}
export function SkillsPage({ notify }: { notify: Notify }) {
  const [data, setData] = useState<SkillPayload | null>(null);
  const [tab, setTab] = useState<"discover" | "installed">("installed");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [batchOperations, setBatchOperations] = useState<PendingSkillBatchOperation[]>([]);
  const batchOperationsRef = useRef<PendingSkillBatchOperation[]>([]);
  const [repoOpen, setRepoOpen] = useState(false);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [conflict, setConflict] = useState<Skill | null>(null);
  const [importRequest, setImportRequest] = useState<ImportRequest | null>(null);
  const [matchRequest, setMatchRequest] = useState<{ id?: string; initialManual?: boolean } | null>(null);
  const importRevision = useRef(0);
  const dismissedConflicts = useRef(new Set<string>());
  const [repoSelection, setRepoSelection] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [repoChecks, setRepoChecks] = useState<Record<string, RepoCheck>>({});
  const updateTasks = useSyncExternalStore(skillUpdateQueue.subscribe, skillUpdateQueue.getSnapshot, skillUpdateQueue.getSnapshot);
  const handledUpdates = useRef(new Set<number>());
  const mounted = useRef(false);
  const dataRequest = useRef(0);
  const checkGeneration = useRef(0);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [storedGroups] = useState(() => readSkillGroupStorage(localStorage.getItem(SKILL_GROUP_STORAGE_KEY)));
  const [customGroupNames, setCustomGroupNames] = useState<string[]>(storedGroups.names);
  const [customGroupAssignments, setCustomGroupAssignments] = useState<Record<string, string>>(storedGroups.assignments);
  const [assignmentsByIdentity, setAssignmentsByIdentity] = useState<Record<string, string>>(storedGroups.assignmentsByIdentity);
  const [groupOrder, setGroupOrder] = useState<string[]>(storedGroups.order);
  const [groupSortDirection, setGroupSortDirection] = useState<SkillGroupSortDirection | null>(storedGroups.sortDirection);
  const [groupAliases, setGroupAliases] = useState<Record<string, string>>(storedGroups.aliases);
  useEffect(() => {
    const stored = readSkillGroupStorage(localStorage.getItem(SKILL_GROUP_STORAGE_KEY));
    localStorage.setItem(SKILL_GROUP_STORAGE_KEY, JSON.stringify({ ...stored, names: customGroupNames, assignments: customGroupAssignments, assignmentsByIdentity, order: groupOrder, aliases: groupAliases, sortDirection: groupSortDirection }));
  }, [customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupAliases, groupSortDirection]);
  const groupSensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 8 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }),
  );
  const reloadLocal = async () => {
    const request = ++dataRequest.current;
    try {
      const result = await invoke<SkillPayload>("list_installed_skills");
      if (mounted.current && request === dataRequest.current) {
        startTransition(() => setData(result));
      }
      return result;
    } catch (e) {
      if (mounted.current) await notify("Skills", String(e), "failed");
      return null;
    }
  };
  const applyCurrentPayload = (request: number, payload: SkillPayload) => {
    if (!mounted.current) return;
    if (request === dataRequest.current) setData(payload);
    else void reloadLocal();
  };
  const checkAllRepos = (repos: Repo[], pending: Repo[] = []) => {
    const generation = ++checkGeneration.current;
    const enabled = [...repos.filter((repo) => repo.enabled), ...pending.filter((repo) => !repos.some((existing) => repoKey(existing) === repoKey(repo)))];
    setRepoChecks(Object.fromEntries(enabled.map((repo) => [repoKey(repo), { state: "queued" as const }])));
    void checkRepositories(enabled, 3, async (repo) => {
      const key = repoKey(repo);
      if (!mounted.current || generation !== checkGeneration.current) return;
      setRepoChecks((old) => ({ ...old, [key]: { state: "running" } }));
      try {
        const result = await invoke<SkillPayload>("refresh_skill_repo", { repoKey: key });
        if (!mounted.current || generation !== checkGeneration.current) return;
        setRepoChecks((old) => ({ ...old, [key]: { state: ok(result) ? "done" : "failed", message: result.message } }));
        void reloadLocal();
      } catch (error) {
        if (mounted.current && generation === checkGeneration.current)
          setRepoChecks((old) => ({ ...old, [key]: { state: "failed", message: String(error) } }));
      }
    });
  };
  const load = async () => {
    setLoading(true);
    const result = await reloadLocal();
    if (mounted.current) {
      setLoading(false);
      if (result && ok(result)) checkAllRepos(result.repos, result.pendingImportedRepos);
      else if (result) void notify("Skills", result.message, "failed");
    }
  };
  useEffect(() => {
    mounted.current = true;
    void load();
    const onFocus = () => { if (mounted.current) void reloadLocal(); };
    window.addEventListener("focus", onFocus);
    return () => { mounted.current = false; checkGeneration.current++; dataRequest.current++; window.removeEventListener("focus", onFocus); };
  }, []);
  useEffect(() => {
    if (conflict) return;
    const next = data?.skills.find((skill) => skill.codexConflict && !dismissedConflicts.current.has(skill.id));
    if (next) setConflict(next);
  }, [data, conflict]);
  useEffect(() => {
    for (const task of updateTasks) {
      if ((task.state !== "done" && task.state !== "failed") || handledUpdates.current.has(task.sequence)) continue;
      handledUpdates.current.add(task.sequence);
      void reloadLocal();
      if (task.state === "failed" || (task.result && !ok(task.result)))
        void notify("Skills", task.error || task.result?.message || t("更新失败"), "failed");
    }
  }, [updateTasks]);
  const enqueueUpdate = (skill: Skill) => {
    const key = `${skill.repoKey}\0${skill.id}`;
    const expectedRepoPath = skill.repoPath ?? null;
    skillUpdateQueue.enqueue(key, async () => {
      const preview = await invoke<Result & { snapshot: string | null }>("preview_skill_update", { id: skill.id });
      if (!ok(preview)) throw new Error(preview.message);
      if (preview.snapshot) {
        const confirmed = await appConfirm(tf("“{0}”包含本地修改，或无法确认原始版本。更新会替换当前内容，原版本会保留在备份中。", [skill.name || skill.id]),
          { title: t("确认更新 Skill"), confirmText: t("备份并更新"), cancelText: t("保留当前版本"), danger: true });
        if (!confirmed) return invoke<SkillPayload>("list_installed_skills");
        return invoke<SkillPayload>("update_skill_confirmed", { repoKey: skill.repoKey, id: skill.id, expectedRepoPath, expectedSnapshot: preview.snapshot });
      }
      return invoke<SkillPayload>("update_skill", { repoKey: skill.repoKey, id: skill.id, expectedRepoPath });
    });
  };
  const run = async (command: string, args: Record<string, unknown>) => {
    setBusy(true);
    const request = ++dataRequest.current;
    try {
      const r = await invoke<SkillPayload>(command, args);
      applyCurrentPayload(request, r);
      const differing = !ok(r) ? r.skills?.find((skill) => skill.codexConflict && skill.id === args.id) : undefined;
      if (differing) setConflict(differing);
      else await notify("Skills", r.message, ok(r) ? "ok" : "failed");
      return ok(r);
    } catch (e) {
      await notify("Skills", String(e), "failed");
      return false;
    } finally {
      setBusy(false);
    }
  };
  const runBatch = async (group: SkillGroup<Skill>, agent: McpAgentId, enabled: boolean) => {
    const groupItems = allGroups.find((item) => item.key === group.key)?.items || group.items;
    if (batchOperationsRef.current.some((operation) => operation.groupKey === group.key && operation.agent === agent)
      || skillBatchHasPending(batchOperationsRef.current, groupItems, agent)) return;
    const targets = skillBatchTargets(group.items, agent, enabled);
    if (!targets.length) {
      await notify("Skills", enabled ? t("该组没有可启用的 Skill") : t("该组没有可停用的 Skill"), "ok");
      return;
    }
    let operation: PendingSkillBatchOperation = { groupKey: group.key, agent, enabled, skillIds: targets.map((skill) => skill.id), settledSkillIds: [] };
    batchOperationsRef.current = [...batchOperationsRef.current, operation];
    setBatchOperations(batchOperationsRef.current);
    let completed = 0;
    const errors: string[] = [];
    try {
      for (const skill of targets) {
        try {
          const request = ++dataRequest.current;
          const result = await invoke<SkillPayload>("set_skill_enabled_for", { agent, id: skill.id, enabled });
          if (mounted.current && request === dataRequest.current) {
            setData((current) => current && result.skills
              ? { ...current, skills: mergeSkillAgentResult(current.skills, result.skills, skill.id, agent) } : current);
          } else if (mounted.current) {
            void reloadLocal();
          }
          if (ok(result)) completed++;
          else errors.push(`${skill.name || skill.id}：${result.message}`);
        } catch (error) { errors.push(`${skill.name || skill.id}：${String(error)}`); }
        finally {
          const previous = operation;
          operation = { ...operation, settledSkillIds: [...operation.settledSkillIds, skill.id] };
          batchOperationsRef.current = batchOperationsRef.current.map((item) => item === previous ? operation : item);
          if (mounted.current) setBatchOperations(batchOperationsRef.current);
        }
      }
    } finally {
      batchOperationsRef.current = batchOperationsRef.current.filter((item) => item !== operation);
      if (mounted.current) {
        setBatchOperations(batchOperationsRef.current);
        void reloadLocal();
      }
    }
    const message = tf("已批量{0} {1} 个 Skill", [enabled ? t("启用") : t("停用"), completed]);
    await notify("Skills", errors.length ? `${message}；${errors.join("；")}` : message, errors.length ? "failed" : "ok");
  };
  const saveRepo = async (repo: Repo, previousKey?: string) => {
    setBusy(true);
    const request = ++dataRequest.current;
    try {
      const result = await invoke<SkillPayload>("upsert_skill_repo", { repo, previousKey });
      applyCurrentPayload(request, result);
      if (!ok(result)) await notify("Skills", result.message, "failed");
      return ok(result);
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return false;
    } finally {
      setBusy(false);
    }
  };
  const previewImport = async (source: "local" | "cc-switch", path: string | null) => {
    setBusy(true);
    try {
      const preview = await invoke<SkillImportPreview>("preview_skill_import", { source, path });
      if (ok(preview)) setImportRequest({ source, path, preview, revision: ++importRevision.current });
      else await notify("Skills", preview.message, "failed");
    } catch (error) { await notify("Skills", String(error), "failed"); }
    finally { setBusy(false); }
  };
  // 选单个 Skill 或其父目录，统一先预览重复项。
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
    await previewImport("local", path);
  };
  const deleteRepo = async (repo: Repo) => { await run("delete_skill_repo", { key: repoKey(repo) }); };
  const inspectRepo = async (repo: Repo) => {
    const result = await invoke<RepoInspection>("inspect_skill_repo", { repo });
    if (ok(result)) {
      const request = ++dataRequest.current;
      const payload = await invoke<SkillPayload>("list_installed_skills");
      applyCurrentPayload(request, payload);
    }
    return result;
  };
  const openRepo = (key: string) => { setRepoSelection(key); setDetail(null); setRepoOpen(true); };
  const deferredQuery = useDeferredValue(query);
  const items = useMemo(() => (data?.skills || [])
    .filter((s) => (tab === "installed" ? s.installed : !s.installed || s.bundled))
    .filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoKey, s.repoPath || "", sourceLabel(s)], deferredQuery)), [data, tab, deferredQuery]);
  const knownSkills = useMemo(() => (data?.skills || []).filter((skill) => skill.installed), [data]);
  const knownRepositories = useMemo(() => data?.repos || [], [data]);
  const builtinGroupLabel = t("内置");
  const userGroupLabel = t("用户");
  const localGroupLabel = t("本地导入");
  const allGroups = useMemo(() => {
    const result = groupSkills(knownSkills, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, true, groupAliases, assignmentsByIdentity, knownRepositories);
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, groupAliases, groupSortDirection, builtinGroupLabel, userGroupLabel, localGroupLabel]);
  const groupsByKey = useMemo(() => new Map(allGroups.map((group) => [group.key, group])), [allGroups]);
  const groupsBySkillIdentity = useMemo(() => new Map(allGroups.flatMap((group) => group.items.map((skill) => [skillIdentityKey(skill), group] as const))), [allGroups]);
  const groups = useMemo(() => {
    const result = groupSkills(items, knownSkills, customGroupNames, customGroupAssignments, groupOrder,
      { builtin: builtinGroupLabel, user: userGroupLabel, source: sourceLabel }, !query.trim(), groupAliases, assignmentsByIdentity, knownRepositories);
    return groupSortDirection ? sortSkillGroupsByName(result, groupSortDirection) : result;
  }, [items, knownSkills, knownRepositories, customGroupNames, customGroupAssignments, assignmentsByIdentity, groupOrder, tab, groupAliases, groupSortDirection, builtinGroupLabel, userGroupLabel, localGroupLabel, query]);
  const openUrl = async (url: string) => {
    try { await invoke("open_external_url", { url }); }
    catch (e) { await notify("Skills", String(e), "failed"); }
  };
  const createCustomGroup = async () => {
    const normalized = (await appPrompt(t("新建 Skill 分组"), { placeholder: t("分组名称") }))?.trim();
    if (!normalized) return;
    if (allGroups.some((group) => group.label.toLocaleLowerCase() === normalized.toLocaleLowerCase())) {
      await notify("Skills", t("分组名称已存在。"), "failed"); return;
    }
    setCustomGroupNames((old) => [...old, normalized]);
  };
  const renameSkillGroup = async (group: SkillGroup<Skill>) => {
    const normalized = (await appPrompt(t("重命名 Skill 分组"), { defaultValue: group.label, placeholder: t("分组名称") }))?.trim();
    if (!normalized || normalized === group.label) return;
    if (allGroups.some((other) => other.key !== group.key && other.label.toLocaleLowerCase() === normalized.toLocaleLowerCase())) {
      await notify("Skills", t("分组名称已存在。"), "failed"); return;
    }
    if (group.kind === "repo") {
      setGroupAliases((old) => ({ ...old, [group.key]: normalized }));
      return;
    }
    if (group.kind !== "custom") return;
    const oldName = group.key.slice(7);
    const nextKey = customSkillGroupKey(normalized);
    setCustomGroupNames((old) => old.map((item) => item === oldName ? normalized : item));
    setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).map(([id, group]) => {
      const name = legacySkillCustomGroupName(group, customGroupNames);
      return [id, name === oldName ? normalized : name ?? group];
    })));
    setAssignmentsByIdentity((old) => Object.fromEntries(Object.entries(old).map(([key, group]) => [key,
      group === customSkillGroupKey(oldName) ? nextKey : group])));
    setGroupOrder((old) => old.map((key) => key === group.key ? nextKey : key));
    setExpandedGroups((old) => {
      const next = new Set(old);
      if (next.delete(group.key)) next.add(nextKey);
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
  };
  const assignSkillGroup = (skill: Skill, group: string) => {
    setGroupOrder(allGroups.filter((item) => item.kind === "repo" || item.kind === "custom").map((item) => item.key));
    setAssignmentsByIdentity((old) => ({ ...old, [skillIdentityKey(skill)]: group }));
  };
  const handleGroupDragEnd = ({ active, over }: DragEndEvent) => {
    if (!over || active.id === over.id || query.trim() || busy || loading || tab !== "installed") return;
    const movable = new Set(groups.filter((group) => group.kind === "repo" || group.kind === "custom").map((group) => group.key));
    if (!movable.has(String(active.id)) || !movable.has(String(over.id))) return;
    setGroupOrder(moveSkillGroup(allGroups, String(active.id), String(over.id)));
    setGroupSortDirection(null);
  };
  const agentCount = (agent: McpAgentId) => (data?.skills || []).filter((s) => s.installed && skillAgents(s).includes(agent)).length;
  // 与 MCP 页同一张表：名称 | 来源 | 每个 Agent 一列勾选 | 操作。分组、详情、卸载收进行末的「更多」菜单。
  const renderSkillRow = (s: Skill) => {
    // 只卸载 Codex++ 管理的或 Codex 目录里的；Claude / Grok 自己目录里的 Skill 不动。
    const canUninstall = s.installed && !s.bundled && (s.managed || skillAgents(s).includes("codex") || s.agents === undefined);
    const task = updateTasks.find((item) => item.key === `${s.repoKey}\0${s.id}` && (item.state === "queued" || item.state === "running"));
    const assignment = skillGroupAssignment(s, customGroupNames, customGroupAssignments, assignmentsByIdentity);
    return <div className="mg-matrix-row" key={skillIdentityKey(s)}>
      <button type="button" className="mg-row-main" onClick={() => setDetail(s)}>
        <div><strong title={s.name || s.id}>{s.name || s.id}</strong>{s.updateAvailable ? <span className="mg-badge">{t("可更新")}</span> : null}{s.codexConflict ? <span className="mg-badge mg-skill-conflict-badge">{t("版本不一致")}</span> : null}</div>
        <small title={[s.repoPath, s.description].filter(Boolean).join(" · ")}>{[s.repoPath, s.description].filter(Boolean).join(" · ") || t("暂无描述")}</small>
      </button>
      <div className="mg-skill-source" title={[s.repoKey || sourceLabel(s), importChannel(s)].filter(Boolean).join(" · ")}>
        {repoUrl(s) ? <button className="mg-link-button" onClick={() => openRepo(s.repoKey)} type="button">{sourceLabel(s)}</button> : <small>{sourceLabel(s)}</small>}
        {repoUrl(s) && importChannel(s) ? <small>{importChannel(s)}</small> : null}
      </div>
      {MCP_AGENTS.map((agent) => <MatrixCell agent={agent} key={agent} name={s.name || s.id}
        enabled={s.installed && skillAgents(s).includes(agent)}
        disabled={busy || !s.installed || s.bundled || skillBatchHasPending(batchOperations, [s], agent)}
        hint={!s.installed ? t("未安装") : s.bundled ? t("Codex 自带，只读") : undefined}
        onToggle={() => void run("set_skill_enabled_for", { agent, id: s.id, enabled: !skillAgents(s).includes(agent) })} />)}
      <div className="mg-row-actions">
        {s.codexConflict ? <button type="button" className="mg-icon-button mg-skill-conflict-action" title={t("处理版本不一致")} aria-label={`${t("处理版本不一致")} ${s.name || s.id}`} onClick={() => setConflict(s)}><AlertTriangle size={16}/></button> : null}
        {!s.installed && !s.bundled ? <Button primary onClick={() => void run("install_skill", { repoKey: s.repoKey, id: s.id, expectedRepoPath: s.repoPath ?? null })} disabled={busy}>{t("安装")}</Button> : null}
        {s.installed && (s.updateAvailable || task) ? <Button onClick={() => enqueueUpdate(s)} disabled={!!task}>{task?.state === "queued" ? t("排队中") : task?.state === "running" ? t("更新中") : t("更新")}</Button> : null}
        <SkillActionMenu skill={s} groups={allGroups} group={groupsBySkillIdentity.get(skillIdentityKey(s))}
          originGroup={groupsByKey.get(defaultSkillGroupKey(s))} assignment={assignment} busy={busy} canUninstall={canUninstall}
          onDetail={() => setDetail(s)} onAssign={(group) => assignSkillGroup(s, group)} onUninstall={() => void run("uninstall_skill", { id: s.id })}/>
      </div>
    </div>;
  };
  const discoverCount = (data?.skills || []).filter((s) => !s.installed || s.bundled).length;
  const installedCount = (data?.skills || []).filter((s) => s.installed).length;
  const tabTotal = tab === "discover" ? discoverCount : installedCount;
  return <div className="mg-page mg-skills-page">
    <Header icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load()} disabled={loading}><RefreshCw size={16}/>{t("刷新目录")}</Button>
      <Button onClick={() => void importLocalSkills()} disabled={busy}><FolderOpen size={16}/>{t("从本地目录导入")}</Button>
      <Button onClick={() => void previewImport("cc-switch", null)} disabled={busy}><BookOpen size={16}/>{t("从 cc-switch 导入")}</Button>
      <Button onClick={() => setRepoOpen(true)}><Link size={16}/>{t("管理仓库")}</Button>
    </>} />
    <div className="mg-toolbar">
      <div className="mg-segmented">
        <button type="button" data-active={tab === "discover"} onClick={() => setTab("discover")}>{t("发现")} · {discoverCount}</button>
        <button type="button" data-active={tab === "installed"} onClick={() => setTab("installed")}>{t("已安装")} · {installedCount}</button>
      </div>
      <SearchField value={query} onChange={setQuery} placeholder={t("搜索 Skills 或仓库")} />
      <span>{items.length === tabTotal ? `${tabTotal} ${t("个 Skill")}` : `${items.length} / ${tabTotal}`}</span>
      <div className="mg-push mg-toolbar-actions">
        <button type="button" className="mg-icon-button" aria-label={t("匹配仓库来源")} title={t("匹配仓库来源")} onClick={() => setMatchRequest({})} disabled={!data}><Search size={17}/></button>
        <button type="button" className="mg-icon-button" aria-label={t("新建分组")} title={t("新建分组")} onClick={() => void createCustomGroup()}><FolderPlus size={17}/></button>
        <button type="button" className="mg-icon-button" aria-label={`${t("按名称排序")} (${groupSortDirection === "asc" ? "A-Z" : groupSortDirection === "desc" ? "Z-A" : "A-Z / Z-A"})`} title={`${t("按名称排序")} (${groupSortDirection === "asc" ? "Z-A" : "A-Z"})`} onClick={() => {
          const direction = nextSkillGroupSortDirection(groupSortDirection);
          setGroupOrder(sortSkillGroupKeys(allGroups, direction));
          setGroupSortDirection(direction);
        }}>{groupSortDirection === "asc" ? <ArrowDownAZ size={17}/> : groupSortDirection === "desc" ? <ArrowDownZA size={17}/> : <ArrowDownUp size={17}/>}</button>
        <button type="button" className="mg-icon-button" aria-label={t("全部折叠")} title={t("全部折叠")} onClick={() => setExpandedGroups(new Set())}><ChevronsUp size={17}/></button>
        <button type="button" className="mg-icon-button" aria-label={t("全部展开")} title={t("全部展开")} onClick={() => setExpandedGroups(new Set(groups.map((group) => group.key)))}><ChevronsDown size={17}/></button>
      </div>
    </div>
    <div className="mg-content">
      <div className="mg-matrix mg-agent-matrix mg-skill-matrix" aria-busy={loading}>
        <div className="mg-matrix-head"><span>{t("技能")}</span><span>{t("来源")}</span>{MCP_AGENTS.map((agent) => <AgentColumn agent={agent} count={agentCount(agent)} key={agent} title={data?.agentSkillsDirs?.[agent]} />)}<span className="mg-actions-label">{t("操作")}</span></div>
        {!groups.length ? <div className="mg-empty"><h2>{loading ? t("正在扫描…") : t("没有匹配的 Skill")}</h2></div> : <DndContext sensors={groupSensors} collisionDetection={closestCenter} onDragEnd={handleGroupDragEnd}>
        <SortableContext items={groups.map((group) => group.key)} strategy={verticalListSortingStrategy}>{groups.map((group) => {
          const groupItems = group.items;
          const collapsed = !expandedGroups.has(group.key) && !query.trim();
          const movable = group.kind === "custom" || group.kind === "repo";
          const allGroupItems = allGroups.find((item) => item.key === group.key)?.items || groupItems;
          const updateItems = tab === "installed"
            ? allGroupItems.filter((skill) => skill.installed && !skill.bundled && skill.updateAvailable)
            : [];
          const updates = updateItems.length;
          const hasUnqueuedUpdate = updateItems.some((skill) => !updateTasks.some((task) =>
            task.key === `${skill.repoKey}\0${skill.id}` && (task.state === "queued" || task.state === "running")));
          return <SortableSkillGroup group={group} key={group.key} disabled={!movable || tab !== "installed" || !!query.trim() || busy || loading}>{(handle) => <>
            <div className="mg-skill-group-row">
              <div className="mg-skill-group-name">{handle}<button type="button" className="mg-group-head" aria-expanded={!collapsed} onClick={() => setExpandedGroups((old) => { const next = new Set(old); next.has(group.key) ? next.delete(group.key) : next.add(group.key); return next; })}>
                <ChevronDown size={15} className={collapsed ? "mg-chevron-collapsed" : ""}/><strong title={group.label}>{group.label}</strong><small>{updates ? `${updates} ${t("可更新")} · ` : ""}{groupItems.length} {t("个 Skill")}</small>
              </button></div>
              <span className="mg-skill-batch-label">{t("全选")}</span>
              {MCP_AGENTS.map((agent) => <GroupAgentSelect group={group} agent={agent} key={agent}
                busy={busy || batchOperations.some((operation) => operation.groupKey === group.key && operation.agent === agent) || skillBatchHasPending(batchOperations, allGroupItems, agent)}
                operations={batchOperations}
                onChange={(enabled) => void runBatch(group, agent, enabled)}/>)}
              <div className="mg-row-actions">
                {updates ? <button type="button" className="mg-icon-button" disabled={!hasUnqueuedUpdate}
                  title={t("更新分组内全部 Skill")} aria-label={`${t("更新分组内全部 Skill")} ${group.label}`}
                  onClick={() => updateItems.forEach(enqueueUpdate)}><CloudDownload size={14}/></button> : null}
                {movable ? <button type="button" className="mg-icon-button" disabled={busy} title={t("重命名分组")} aria-label={`${t("重命名分组")} ${group.label}`} onClick={() => void renameSkillGroup(group)}><Edit3 size={14}/></button> : null}
                {group.kind === "custom" ? <button type="button" className="mg-icon-button" disabled={busy} title={t("删除分组")} aria-label={`${t("删除分组")} ${group.label}`} onClick={() => void deleteCustomGroup(group.key.slice(7))}><Trash2 size={14}/></button> : null}
              </div>
            </div>
            {!collapsed ? groupItems.map(renderSkillRow) : null}
          </>}</SortableSkillGroup>;
        })}</SortableContext></DndContext>}
      </div>
      {Object.entries(repoChecks).some(([, status]) => status.state === "failed") ? <div className="mg-notice mg-warning" role="status">{Object.entries(repoChecks).filter(([, status]) => status.state === "failed").map(([key, status]) => <p key={key}>{status.message || key}</p>)}</div> : null}
    </div>
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} pendingRepos={data?.pendingImportedRepos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} checks={repoChecks} initialKey={repoSelection} busy={busy} onClose={() => setRepoOpen(false)} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(cmd, args) => void run(cmd, args)} />}
    {detail && <SkillDetailDrawer skill={detail} count={data?.repoCounts?.[detail.repoKey]} groups={allGroups}
      group={groupsBySkillIdentity.get(skillIdentityKey(detail))} originGroup={groupsByKey.get(defaultSkillGroupKey(detail))}
      onAssign={(group) => assignSkillGroup(detail, group)} onClose={() => setDetail(null)} onOpenUrl={(url) => void openUrl(url)}
      onOpenRepo={() => repoOpen ? setDetail(null) : openRepo(detail.repoKey)}
      onEditSource={() => setMatchRequest({ id: detail.id, initialManual: true })} onMatch={() => setMatchRequest({ id: detail.id })}/>}
    {matchRequest && <SkillRepositoryMatchDrawer repos={data?.repos || []} id={matchRequest.id} initialManual={matchRequest.initialManual} onClose={() => setMatchRequest(null)} notify={notify}
      onUpdated={() => { void reloadLocal().then((result) => {
        if (!result) return;
        setDetail((current) => {
          if (!current || current !== detail) return current;
          return result.skills.find((item) => item.id === current.id && item.installed)
            || (!current.installed ? result.skills.find((item) => item.id === current.id && item.repoKey === current.repoKey && item.repoPath === current.repoPath) : undefined)
            || null;
        });
      }); }}/>}
    {importRequest && <SkillImportDrawer key={importRequest.revision} request={importRequest} busy={busy}
      onClose={() => setImportRequest(null)} onRefresh={() => void previewImport(importRequest.source, importRequest.path)}
      onApply={(selections) => { void (async () => { if (await run("apply_skill_import", { source: importRequest.source, path: importRequest.path, selections })) { setImportRequest(null); setTab("installed"); void load(); } })(); }}/>}
    {conflict && <SkillConflictDialog key={conflict.id} skill={conflict} busy={busy} onClose={() => { dismissedConflicts.current.add(conflict.id); setConflict(null); }}
      onResolve={(keepCodex) => { void (async () => { if (await run("resolve_skill_conflict", { id: conflict.id, keepCodex })) { dismissedConflicts.current.delete(conflict.id); setConflict(null); } })(); }}/>}
  </div>;
}
export function SessionPage({ sessions, onRefresh, onDelete, onDeleteMany, notify, repairPanel }: { sessions: { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number } | null; onRefresh: (offset?: number) => Promise<void>; onDelete: (s: ManagedSession) => Promise<void>; onDeleteMany: (s: ManagedSession[]) => Promise<void>; notify: Notify; repairPanel?: ReactNode }) { const [query, setQuery] = useState(""); const [mode, setMode] = useState<"project" | "time">("project"); const [expanded, setExpanded] = useState<Set<string>>(new Set()); const [selection, setSelection] = useState(false); const [selected, setSelected] = useState<Set<string>>(new Set()); const [detail, setDetail] = useState<ManagedSession | null>(null); const all = sessions?.sessions || []; const visible = all.filter((s) => matchesManagementQuery([s.title, s.cwd, s.id, s.modelProvider], query)); const groups = sessionGroups(visible, mode); const selectionScope = visible.map((s) => s.id).join("\0"); useEffect(() => { const ids = new Set(selectionScope.split("\0")); setSelected((old) => { const next = new Set([...old].filter((id) => ids.has(id))); return next.size === old.size ? old : next; }); }, [selectionScope]); const selectedRows = visible.filter((s) => selected.has(s.id)); const toggle = (id: string) => setSelected((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; }); return <div className="mg-page mg-session-page"><Header icon={<History size={20}/>} title={t("会话")} actions={<><Button onClick={() => void onRefresh()}><RefreshCw size={14}/>{t("刷新列表")}</Button><Button onClick={() => { setSelection(!selection); setSelected(new Set()); }}>{t("选择多个…")}</Button></>} />{repairPanel}<div className="mg-session-controls"><SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} /><div className="mg-segmented"><button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button><button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button></div>{selection && <><Button onClick={() => setSelected(new Set(visible.map((s) => s.id)))}>{t("全选当前列表")}</Button><Button disabled={!selectedRows.length} onClick={() => setSelected(new Set())}>{t("取消全部选择")}</Button><Button primary disabled={!selectedRows.length} onClick={() => void onDeleteMany(selectedRows)}><Trash2 size={14}/>{t("删除已选")}</Button></>}</div><div className="mg-content"><div className="mg-session-list">{!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((g) => { const open = query.trim() || expanded.has(g.key) || groups.length === 1; return <section key={g.key}><button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(g.key) ? next.delete(g.key) : next.add(g.key); return next; })}><ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"}/><strong>{g.label}</strong><span>{g.path}</span><small>{g.sessions.length} {t("个会话")}</small></button>{open && g.sessions.map((s) => <article className="mg-session-row" key={s.id}><div className="mg-session-check">{selection && <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)}/>}</div><button type="button" className="mg-session-main" onClick={() => setDetail(s)}><strong>{s.title || t("未命名会话")}</strong><small>{s.cwd || t("未记录项目路径")}</small><code>{s.id}</code></button><div className="mg-session-meta"><span>{s.modelProvider || "Codex"}</span><time>{s.updatedAtMs ? new Date(s.updatedAtMs).toLocaleString() : t("未知时间")}</time></div><div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(s)} aria-label={t("删除会话")}><Trash2 size={15}/></button></div></article>)}</section>; })}</div></div><div className="mg-pagination"><Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - (sessions?.limit || 50)))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14}/>{t("上一页")}</Button><span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [Math.floor(sessions.offset / sessions.limit) + 1, sessions.totalCount]) : "—"}</span><Button onClick={() => void onRefresh((sessions?.offset || 0) + (sessions?.limit || 50))} disabled={!sessions?.hasMore}><ArrowRight size={14}/>{t("下一页")}</Button></div>{detail && <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14}/>{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14}/>{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer>}</div>; }
