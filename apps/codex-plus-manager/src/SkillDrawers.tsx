/** Skills 页的抽屉与共用的类型、工具函数：仓库管理、备份列表、Skill 详情。页面本身在 SkillsPage.tsx。 */
import { useDeferredValue, useEffect, useMemo, useRef, useState } from "react";
import { ArrowRight, CloudDownload, Edit3, ExternalLink, FolderOpen, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { t } from "./i18n";
import { matchesManagementQuery, skillRepoKey } from "./management-model";
import { Field, ManagementDrawer, PageButton as Button, SearchBox } from "./ManagementPages";
import "./ManagementPages.css";
import { parseSkillRepository } from "./skill-repository";
import { SkillListViewport } from "./SkillListViewport";
import { skillIdentityKey, type SkillBundleSkill } from "./skill-groups";
import type { SkillBatchRegistry } from "./skill-agent-batch";
import { SkillBackupUsage } from "./SkillBackupUsage";
import { SkillFileTimes } from "./SkillFileTimes";

export type Result = { status: string; message: string };
export type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
export type SkillLocation = { manifestPath: string; rootKind: string; managed: boolean; enabled: boolean; statusError?: string | null; originalPath?: string | null; selectable?: boolean;
  associationPath?: string | null; repoKey?: string; repoPath?: string; contentHash?: string; remoteHash?: string; updateAvailable?: boolean; updateOwner?: string };
export type SkillMatchContext = { agentLabel?: string; currentRepoKey?: string; currentRepoPath?: string };
export type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; displayGroupRepoKey?: string | null; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean;
  /** 已经启用到哪些 Agent（codex / claude / grok）。 */
  agents?: string[];
  /** Codex++ 的受控源里有它；只在某个 Agent 自己目录里的 Skill 为 false。 */
  managed?: boolean;
  managedSourceDir?: string | null;
  managedSourceStatus?: "stored" | "referenced" | "matching" | "different" | "needs-sync" | "unknown" | null;
  managedSourceError?: string | null;
  sourceVerified?: boolean;
  installChannel?: string;
  originalDir?: string | null;
  updateOwner?: string;
  instanceKey?: string;
  conflict?: string | null;
  conflictKind?: "independent" | "equal-independent" | "managed-modified" | null;
  safeDisableAgents?: string[];
  safeToggleAgents?: string[];
  locations?: SkillLocation[];
  displayInstances?: Skill[] };
export type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
export type SkillBackup = { id: string; skillId: string; name: string; backedUpAt: string; backupPath?: string | null; restorePath?: string | null; purpose?: string | null; missing?: boolean; restoreBlockedReason?: string | null; deleteWarning?: string | null };
export type SkillPayload = Result & { skills: Skill[]; inventoryOk: boolean; repos: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string; agentSkillsDirs?: Record<string, string>; backups?: SkillBackup[]; backupsOk: boolean };
export type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
export type SourceApplyResult = SkillPayload & { applied: Array<{ localId: string; fromRepoKey: string; toRepoKey: string; localPath?: string | null }> };
export type SkillBundleImportResult = SkillPayload & { bundle?: { groups: string[]; skills: SkillBundleSkill[] } };
export const ok = (r: Result) => r.status === "ok";
export function repoFromKey(key: string | null): Repo | null {
  const match = key?.match(/^([^/]+)\/([^@]+)@([^:]+)(?::(.*))?$/);
  return match ? { owner: match[1], name: match[2], branch: match[3], subdir: match[4] || "", enabled: false } : null;
}
export function parseRepoInput(value: string, current: Repo): Repo | null {
  const parsed = parseSkillRepository(value);
  return parsed ? { ...parsed, enabled: current.enabled } : null;
}
/** 后端 `LOCAL_IMPORT_REPO_KEY`：从本地目录导入的 Skill（#511）。 */
export const LOCAL_IMPORT_REPO_KEY = "local";
export function sourceLabel(skill: Skill) { if (skill.displayInstances) return t("多个 Agent 安装"); if (skill.bundled) return t("内置"); if (skill.repoKey === "cc-switch" || !skill.repoKey) return t("本地"); if (skill.repoKey === LOCAL_IMPORT_REPO_KEY) return t("本地导入"); return skill.repoKey.split("@", 1)[0]; }
export function skillChannelLabel(skill: Skill) {
  return skill.installChannel === "cc-switch" || (skill.repoKey === "cc-switch" && skill.sourceVerified)
    ? "cc-switch" : skill.installChannel === "github" ? "GitHub" : skill.installChannel === "system" ? t("内置") : skill.installChannel === "local" || skill.repoKey === "cc-switch" ? t("本地安装") : t("未记录");
}
export function repoUrl(skill: Skill) {
  if (!skill.repoKey || skill.repoKey === "cc-switch" || skill.repoKey === LOCAL_IMPORT_REPO_KEY) return "";
  const parts = skill.repoKey.split("@")[0].split("/");
  if (parts.length < 2) return "";
  return `https://github.com/${parts.join("/")}${skill.repoPath ? `/tree/${skill.repoKey.split("@")[1]?.split(":")[0] || "main"}/${skill.repoPath}` : ""}`;
}
export function SkillRepoDrawer({ repos, catalog, counts, initialKey, busy, updates, conflictIds, onClose, onSave, onDelete, onInspect, onOpenUrl, onOpenSkill, onRun }: {
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
  const [selected, setSelected] = useState<Repo | null>(() => repos.find((r) => skillRepoKey(r) === initialKey) || repoFromKey(initialKey) || repos[0] || null);
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
    if (counts[skillRepoKey(repo)] === undefined) void inspect(repo);
  };
  useEffect(() => { if (selected && counts[skillRepoKey(selected)] === undefined) void inspect(selected); }, []);
  useEffect(() => { if (!selected && repos[0]) view(repos[0]); }, [repos, selected]);
  const edit = (repo: Repo) => { setFormOpen(true); setEditing(repo); setDraft(repo); setInput(`https://github.com/${repo.owner}/${repo.name}/tree/${repo.branch}${repo.subdir ? `/${repo.subdir}` : ""}`); view(repo); };
  const save = async () => {
    const request = ++scanRequest.current;
    setScanning(false);
    if (await onSave(draft, editing ? skillRepoKey(editing) : undefined) && request === scanRequest.current) {
      setSelected(draft); setPreview(null); setFeedback(""); setFormOpen(false);
      setEditing(null);
      setDraft({ owner: "", name: "", branch: "main", subdir: "", enabled: true });
      setInput("");
      setParseError("");
    }
  };
  const active = repos.find((r) => selected && skillRepoKey(r) === skillRepoKey(selected))
    || repos.find((r) => selected && r.owner === selected.owner && r.name === selected.name && r.subdir === selected.subdir)
    || selected;
  const savedCatalog = active && counts[skillRepoKey(active)] !== undefined;
  const activeKey = active ? skillRepoKey(active) : "";
  const skills = useMemo(() => savedCatalog ? catalog.filter((s) => s.repoKey === activeKey) : preview?.skills || [], [savedCatalog, catalog, activeKey, preview]);
  const count = preview?.skillCount ?? (active ? counts[skillRepoKey(active)] : undefined);
  const deferredQuery = useDeferredValue(query);
  const filtered = useMemo(() => skills.filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoPath || ""], deferredQuery)), [skills, deferredQuery]);
  const locked = busy || scanning;
  return <ManagementDrawer modal="workspace" dismissOnBackdrop={false} title={t("管理 Skills 仓库")} onClose={onClose}>
    <div className="mg-repo-workspace" data-editing={formOpen}><aside className="mg-repo-sidebar">
    <div className="mg-repo-manager"><div className="mg-repo-manager-head"><strong>{t("已添加仓库")} <span className="mg-repo-total">{repos.length}</span></strong><button className="mg-icon-button" title={t("添加仓库")} aria-label={t("添加仓库")} onClick={() => { cancelScan(); setEditing(null); setFormOpen(true); setInput(""); setParseError(""); setDraft({ owner: "", name: "", branch: "main", subdir: "", enabled: true }); }}><Plus size={16}/></button></div>
      {!repos.length && <p className="mg-muted">{t("还没有添加仓库。")}</p>}
      {repos.map((repo) => <div className="mg-repo-row" data-selected={active && skillRepoKey(active) === skillRepoKey(repo)} key={skillRepoKey(repo)}>
        <button type="button" className="mg-repo-select" onClick={() => view(repo)}><strong title={`${repo.owner}/${repo.name}`}>{repo.owner}/{repo.name}</strong><small title={`${repo.branch}${repo.subdir ? ` · ${repo.subdir}` : ""}`}>{repo.branch}{repo.subdir ? ` · ${repo.subdir}` : ""}<span>{counts[skillRepoKey(repo)] === undefined ? t("待扫描") : `${counts[skillRepoKey(repo)]} Skills`}{!repo.enabled ? ` · ${t("已停用")}` : ""}</span></small></button>
        <div className="mg-repo-row-actions"><button className={"mg-switch " + (repo.enabled ? "is-on" : "")} type="button" role="switch" aria-checked={repo.enabled} disabled={busy} aria-label={`${repo.owner}/${repo.name} ${t("启用仓库")}`} onClick={() => void onSave({ ...repo, enabled: !repo.enabled })}><span /></button>
        <button type="button" className="mg-icon-button" title={t("编辑仓库")} aria-label={`${t("编辑仓库")} ${repo.owner}/${repo.name}`} onClick={() => edit(repo)}><Edit3 size={14}/></button>
        <button className="mg-icon-button" title={t("删除仓库")} aria-label={`${t("删除仓库")} ${repo.owner}/${repo.name}`} disabled={busy} onClick={() => { cancelScan(); onDelete(repo); if (active && skillRepoKey(active) === skillRepoKey(repo)) { setSelected(repos.find((r) => skillRepoKey(r) !== skillRepoKey(repo)) || null); setPreview(null); } }}><Trash2 size={14}/></button></div></div>)}
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
            return <div className="mg-repo-skill" key={skillIdentityKey(s)}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={s.description}>{s.description || s.repoPath}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => skillRepoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <span title={conflictIds.has(s.id) ? t("同名内容存在冲突") : pending ? state : undefined}><Button primary disabled={Boolean(pending) || conflictIds.has(s.id)} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id })}>{state}</Button></span>)}</div></div></div>;
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
export function SkillBackupList({ backups, busy, onRestore, onDelete, onExport }: {
  backups: SkillBackup[]; busy: boolean; onRestore: (backup: SkillBackup) => void; onDelete: (backup: SkillBackup) => void; onExport: (backup: SkillBackup) => void;
}) {
  return <>{backups.map((backup) => <div key={backup.id} className="mg-source-row">
    <div><strong>{backup.name || backup.skillId}</strong><small>{backup.backedUpAt}</small></div>
    <div><small>{backup.purpose === "duplicate" ? t("重复副本归档") : backup.purpose === "managed-source" ? t("受控源归档") : backup.purpose === "disabled" ? t("停用副本备份") : backup.purpose === "update" ? t("更新前备份") : t("技能备份")}</small>
      <small>{t("备份位置")}：{backup.backupPath || t("未知")}</small>
      <small>{t("恢复位置")}：{backup.restorePath || t("未记录")}</small>
      {backup.missing ? <small className="mg-warning">{t("备份文件已丢失")}</small> : <SkillFileTimes id={backup.skillId} backupId={backup.id}/>}
      {backup.restoreBlockedReason && <small className="mg-warning">{backup.restoreBlockedReason}</small>}
      <div className="mg-row-actions"><Button disabled={busy || !!backup.missing || !!backup.restoreBlockedReason} onClick={() => onRestore(backup)}>{t("恢复")}</Button>
        <button type="button" className="mg-icon-button" disabled={busy || !!backup.missing} title={t("备份另存为")} aria-label={t("备份另存为")} onClick={() => onExport(backup)}><FolderOpen size={14}/></button>
        <button type="button" className="mg-icon-button" disabled={busy} title={t(backup.missing ? "清除记录" : "删除备份")} aria-label={`${t(backup.missing ? "清除记录" : "删除备份")} ${backup.name || backup.skillId}`} onClick={() => onDelete(backup)}><Trash2 size={14}/></button></div></div>
  </div>)}</>;
}
export function SkillBackupsDrawer({ backups, busy, onClose, onRestore, onDelete, onExport }: {
  backups: SkillBackup[]; busy: boolean; onClose: () => void; onRestore: (backup: SkillBackup) => void; onDelete: (backup: SkillBackup) => void; onExport: (backup: SkillBackup) => void;
}) {
  return <ManagementDrawer title={t("Skill 备份")} dismissOnBackdrop={false} onClose={onClose}>
    <SkillBackupUsage backupRevision={JSON.stringify(backups.map((backup) => [backup.id, !!backup.missing]))}/>
    {backups.length ? <SkillBackupList backups={backups} busy={busy} onRestore={onRestore} onDelete={onDelete} onExport={onExport}/> : <p className="mg-muted">{t("暂无 Skill 备份。")}</p>}
  </ManagementDrawer>;
}
export function SkillDetailDrawer({ skill, count, backups, busy, onClose, onOpenUrl, onOpenRepo, onMatch, onRestore, onDeleteBackup, onUpdate, onManageSource, onExport, onArchive, onLocal, showLocations, dates, comparisonErrors, onExpand }: {
  dates?: Record<string, number>; comparisonErrors?: Record<string, string>; onExpand: () => void;
  showLocations: boolean; onLocal: () => void; onArchive: () => void; onExport: (backup: SkillBackup) => void;
  skill: Skill; count?: number; backups: SkillBackup[]; busy: boolean; onClose: () => void; onOpenUrl: (url: string) => void;
  onOpenRepo: () => void; onMatch: (localPath?: string, context?: SkillMatchContext) => void; onRestore: (backup: SkillBackup) => void; onDeleteBackup: (backup: SkillBackup) => void; onUpdate: (repoKey: string, localPath?: string) => void; onManageSource: () => void;
}) {
  const url = repoUrl(skill);
  const channel = skillChannelLabel(skill);
  const owner = skill.updateOwner === "in_place" ? t("Codex++ 原位维护") : skill.updateOwner === "manager" ? t("Codex++ 管理") : t("未接管");
  const selectableOriginal = skill.conflictKind === "independent" && !!skill.originalDir
    && skill.locations?.some((location) => !location.managed);
  const associatedOriginal = skill.conflictKind === "equal-independent" && !!skill.originalDir && !!url;
  const managedSourcePath = skill.managedSourceDir;
  const matchTargets = new Set(skill.locations?.flatMap((location) =>
    location.selectable && location.originalPath ? [location.originalPath]
      : location.associationPath ? [location.associationPath] : []) || []);
  const matchAtRepoPath = skill.installed && !skill.bundled && !skill.conflict && !skill.displayInstances
    && (matchTargets.size === 1 || (matchTargets.size === 0 && !!skill.managedSourceDir));
  const matchLabel = t(url ? "修改 GitHub 仓库关联" : "关联 GitHub 仓库");
  const previewUpdate = skill.installed && !skill.bundled && (!skill.conflict || selectableOriginal || associatedOriginal)
    && !!url && !!skill.originalDir && skill.updateOwner !== "manager";
  const rootLabel: Record<string, string> = { shared: t("共享"), codex: "Codex", claude: "Claude Code", grok: "Grok" };
  return <ManagementDrawer dismissOnBackdrop={false} title={skill.name || skill.id} onClose={onClose}>
    <div className="mg-skill-detail">
    {skill.conflict && <p className="mg-notice mg-warning" role="alert">{skill.displayInstances ? t("各 Agent 的安装内容或来源可能不同，请分别查看。") : skill.conflict}</p>}
    {skill.managedSourceStatus === "needs-sync" && <p className="mg-notice">{t("本地安装内容一致。请先同步工作副本，再更新或启用到其他 Agent。")}</p>}
    <dl className="mg-detail">
      <dt>{t(!skill.displayInstances && !url && !skill.bundled ? "来源" : "上游来源")}</dt><dd>{skill.displayInstances ? t("按安装位置查看") : url ? <button className="mg-link-button mg-skill-repo-link" type="button" onClick={onOpenRepo}>{sourceLabel(skill)}{count === undefined ? "" : ` · ${count} Skills`}<ArrowRight size={13}/></button> : skill.bundled ? t("内置") : t("本地")}{associatedOriginal && <small> · {t("仅关联所选安装位置")}</small>}</dd>
      {!skill.displayInstances && <><dt>{t("更新方式")}</dt><dd>{channel} · {owner}</dd></>}
      <dt>{t("仓库路径")}</dt><dd>{skill.repoPath || "—"}{matchAtRepoPath && <button type="button" className="mg-icon-button" title={matchLabel} aria-label={matchLabel}
        disabled={busy} onClick={() => onMatch(matchTargets.values().next().value, { currentRepoKey: skill.repoKey, currentRepoPath: skill.repoPath })}><Edit3 size={14}/></button>}</dd>
      <dt>{t("介绍")}</dt><dd>{skill.description || t("暂无描述")}</dd>
      {url && <><dt>GitHub</dt><dd><button className="mg-link-button" type="button" onClick={() => onOpenUrl(url)}><ExternalLink size={13}/>{url}</button></dd></>}
    </dl>
    <details className="mg-version-section" open={showLocations || undefined} onToggle={(e) => { if (e.currentTarget.open && !skill.bundled) onExpand(); }}><summary>{t("本地安装")}<small>{skill.locations?.length || 0}</small></summary><dl className="mg-detail">
      {skill.locations?.flatMap((location) => {
        const instance = skill.displayInstances?.find((item) => item.locations?.some((itemLocation) => itemLocation.manifestPath === location.manifestPath));
        const locationRepo = location.repoKey || instance?.repoKey || (!skill.displayInstances ? skill.repoKey : "");
        const locationRepoPath = location.repoPath || instance?.repoPath || (!skill.displayInstances ? skill.repoPath : "") || "";
        const locationOwner = location.updateOwner || instance?.updateOwner || (!skill.displayInstances ? skill.updateOwner : "");
        const hasRepo = !!repoFromKey(locationRepo);
        const associationPath = location.selectable && location.originalPath ? location.originalPath : location.associationPath;
        const sharedAssociation = associationPath && (skill.locations || []).filter((item) =>
          (item.selectable && item.originalPath ? item.originalPath : item.associationPath) === associationPath).length > 1;
        return [<dt key={`${location.manifestPath}:label`}>{rootLabel[location.rootKind] || location.rootKind}</dt>, <dd key={`${location.manifestPath}:value`}>{location.manifestPath}<br/>{location.managed ? t("Codex++ 受控") : t("本地安装")} · {location.statusError ? t("状态未知") : location.enabled ? t("已启用") : t("未启用")}
          {dates?.[location.rootKind] !== undefined && <small>{t("最近修改")} · <time>{new Date(dates[location.rootKind]).toLocaleString()}</time></small>}<SkillFileTimes id={skill.id} rootKind={location.rootKind}/>
          {comparisonErrors?.[location.rootKind] && <small role="alert">{t("无法比较")}：{t(comparisonErrors[location.rootKind])}</small>}
          {location.statusError && <><br/><span role="alert">{location.statusError}</span></>}
          {instance && <><br/>{t("安装渠道")}：{skillChannelLabel(instance)}</>}
          {(instance || associationPath) && <><br/>{t(hasRepo ? "上游来源" : "来源")}：{hasRepo ? `${locationRepo}${locationRepoPath ? ` / ${locationRepoPath}` : ""}` : t("本地")}
            {hasRepo && <><br/>{t("更新方式")}：{locationOwner === "in_place" ? t("Codex++ 原位维护") : locationOwner === "manager" ? t("Codex++ 管理") : t("未接管")}</>}
            {sharedAssociation && <><br/><small>{t("此来源由多个安装位置共用")}</small></>}
            {associationPath && !matchAtRepoPath && <div className="mg-row-actions"><Button disabled={busy} onClick={() => onMatch(associationPath, {
              agentLabel: rootLabel[location.rootKind] || location.rootKind, currentRepoKey: locationRepo, currentRepoPath: locationRepoPath,
            })}>{t(hasRepo ? "重新关联此安装位置" : "关联此安装位置")}</Button>
              {location.selectable && location.originalPath && (skill.displayInstances || skill.conflictKind === "equal-independent") && hasRepo && locationOwner !== "manager" && <Button disabled={busy} onClick={() => onUpdate(locationRepo, location.originalPath || undefined)}><CloudDownload size={14}/>{t("预览此位置更新")}</Button>}</div>}</>}</dd>];
      })}
    </dl>{skill.installed && !skill.bundled && !!skill.locations?.length && <div className="mg-row-actions"><Button disabled={busy} onClick={onLocal}>{t("仅本地管理")}</Button><button className="mg-icon-button" type="button" disabled={busy} title={t("卸载本地版本")} aria-label={t("卸载本地版本")} onClick={onArchive}><Trash2 size={14}/></button></div>}</details>
    <details className="mg-version-section"><summary>{t("备份")}<small>{backups.length + Number(!!managedSourcePath)}</small></summary>
      {managedSourcePath && <dl className="mg-detail">
        <dt>{t("受控源目录")}</dt><dd>{managedSourcePath}<SkillFileTimes id={skill.id} rootKind="source"/>
          <button type="button" className="mg-icon-button" disabled={busy} title={t("管理受控源")} aria-label={t("管理受控源")} onClick={onManageSource}><FolderOpen size={14}/></button>
          {skill.managedSourceStatus === "referenced" && <small>{t("正在被安装位置引用")}</small>}
          {skill.managedSourceStatus === "matching" && <small>{t("未被目录链接引用，内容与安装位置一致")}</small>}
          {skill.managedSourceStatus === "stored" && <small>{t("保留的工作源，可用于重新启用")}</small>}
          {skill.managedSourceStatus === "different" && (skill.displayInstances && !skill.conflict
            ? <small>{t("受控源与部分 Agent 的独立安装内容不同")}</small>
            : <small className="mg-warning">{t("未被目录链接引用，内容与安装位置不同；启用到其他 Agent 前需处理")}</small>)}
          {skill.managedSourceStatus === "unknown" && <small className="mg-warning">{t("无法比较受控源")}：{skill.managedSourceError}</small>}</dd>
      </dl>}
      {backups.length ? <SkillBackupList backups={backups} busy={busy} onRestore={onRestore} onDelete={onDeleteBackup} onExport={onExport}/> : !managedSourcePath && <p className="mg-muted">{t("暂无 Skill 备份。")}</p>}</details>
    {previewUpdate && <div className="mg-row-actions">
      <Button disabled={busy} onClick={() => onUpdate(skill.repoKey, skill.originalDir || undefined)}><CloudDownload size={14}/>{t("预览原位更新")}</Button>
    </div>}
    </div>
  </ManagementDrawer>;
}
