import { startTransition, useDeferredValue, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowDownAZ, ArrowDownZA, ArrowLeft, ArrowRight, BookOpen, ChevronDown, ChevronsDownUp, ChevronsUpDown, Copy, Edit3, ExternalLink, GripVertical, History, Link, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { t, tf } from "./i18n";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { ManagementDrawer } from "./ManagementPages";
import "./ManagementPages.css";
import { parseSkillRepository } from "./skill-repository";
import { SkillListViewport } from "./SkillListViewport";
import { moveSkillGroup, orderedSkillGroups, readSkillGroups } from "./skill-groups";
import { updatableSkills } from "./skill-updates";
import { createSkillUpdateQueue } from "./skill-update-queue";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; localPath?: string; installed: boolean; enabled: boolean; bundled: boolean; plugin?: boolean; updateAvailable: boolean; localDifference?: boolean; comparisonError?: string };
type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type SkillPayload = Result & { skills: Skill[]; repos: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string };
let skillPageView: {
  data: SkillPayload | null; tab: "discover" | "installed"; query: string;
  expandedGroups: Set<string>; nameSort: "asc" | "desc" | null;
} = { data: null, tab: "installed", query: "", expandedGroups: new Set(), nameSort: null };
let skillPageScrollTop = 0;
const skillGroupScroll = new Map<string, number>();
const skillUpdateQueue = createSkillUpdateQueue<SkillPayload>((skill) =>
  invoke<SkillPayload>("update_skill", { repoKey: skill.repoKey, id: skill.id }));
type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
type OriginCandidate = { repo: Repo; skill: { id: string; name: string; repoPath: string }; evidence: string; exactMatch?: boolean };
type OriginInspection = Result & { candidates: OriginCandidate[] };
const SKILL_GROUP_STORAGE_KEY = "codex-plus-plus.skill-groups.v1";
const ok = (r: Result) => r.status === "ok";
function Button({ children, onClick, disabled, primary }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean }) { return <button type="button" className={"mg-button " + (primary ? "mg-primary" : "")} onClick={onClick} disabled={disabled}>{children}</button>; }
function SearchBox({ value, onChange, placeholder }: { value: string; onChange: (v: string) => void; placeholder: string }) { return <div className="mg-search"><Search size={14} /><input aria-label={placeholder} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} /></div>; }
function Header({ icon, title, actions }: { icon: ReactNode; title: string; actions: ReactNode }) { return <header className="mg-header" data-tauri-drag-region><div className="mg-header-title">{icon}<h1>{title}</h1></div><div className="mg-header-actions">{actions}</div></header>; }
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
function sourceLabel(skill: Skill) { if (skill.plugin) return "Codex"; if (skill.bundled) return t("内置"); if (skill.repoKey === "cc-switch") return "cc-switch"; return skill.repoKey ? skill.repoKey.split("@", 1)[0] : "Codex"; }
function sourceGroup(skill: Skill) { return !skill.bundled && (!skill.repoKey || skill.repoKey === "cc-switch") ? t("本地") : sourceLabel(skill); }
function repoUrl(skill: Skill) {
  if (!skill.repoKey || skill.repoKey === "cc-switch") return "";
  const parts = skill.repoKey.split("@")[0].split("/");
  if (parts.length < 2) return "";
  return `https://github.com/${parts.join("/")}${skill.repoPath ? `/tree/${skill.repoKey.split("@")[1]?.split(":")[0] || "main"}/${skill.repoPath}` : ""}`;
}
function SkillRepoDrawer({ repos, catalog, counts, initialKey, busy, onClose, onSave, onDelete, onInspect, onOpenUrl, onOpenSkill, onRun }: {
  repos: Repo[]; catalog: Skill[]; counts: Record<string, number>; initialKey: string | null; busy: boolean; onClose: () => void;
  onSave: (repo: Repo, previousKey?: string) => Promise<boolean>; onDelete: (repo: Repo) => void;
  onInspect: (repo: Repo) => Promise<RepoInspection>; onOpenUrl: (url: string) => void;
  onOpenSkill: (skill: Skill) => void; onRun: (command: string, args: Record<string, unknown>) => void;
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
          renderRow={(s) => <div className="mg-repo-skill" key={s.repoKey + ":" + s.id}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={s.description}>{s.repoPath || s.description}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => repoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <Button primary disabled={busy} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id })}>{t("安装")}</Button>)}</div></div></div>} />
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
function SkillGroupDialog({ mode, name, names, onClose, onSave }: {
  mode: "create" | "rename" | "delete"; name: string; names: string[]; onClose: () => void; onSave: (name: string) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [value, setValue] = useState(name);
  const [submitted, setSubmitted] = useState(false);
  const normalized = value.trim();
  const error = !normalized ? t("请输入分组名称") : names.includes(normalized) && (mode === "create" || normalized !== name) ? t("分组名称已存在") : "";
  const title = mode === "create" ? t("新建分组") : mode === "rename" ? t("重命名分组") : t("删除分组");
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    ref.current?.showModal();
    ref.current?.querySelector<HTMLInputElement>("input")?.select();
    return () => { previous?.focus({ preventScroll: true }); };
  }, []);
  return <dialog ref={ref} className="mg-group-dialog" aria-labelledby="skill-group-dialog-title" onCancel={(event) => { event.preventDefault(); onClose(); }}
    onKeyDown={(event) => event.stopPropagation()}>
    <form onSubmit={(event) => { event.preventDefault(); setSubmitted(true); if (mode === "delete" || !error) onSave(normalized); }}>
      <header><h2 id="skill-group-dialog-title">{title}</h2><button type="button" className="mg-icon-button" aria-label={t("关闭窗口")} title={t("关闭窗口")} onClick={onClose}><X size={18}/></button></header>
      <div className="mg-group-dialog-body">
        {mode === "delete" ? <p>{tf("删除分组“{0}”？其中的 Skill 将回到来源分组。", [name])}</p> :
          <Field label={t("分组名称")}><input autoFocus maxLength={80} value={value} aria-label={t("分组名称")} aria-invalid={submitted && !!error} aria-describedby={submitted && error ? "skill-group-error" : undefined} onChange={(event) => setValue(event.target.value)} /></Field>}
        {submitted && error && mode !== "delete" && <p id="skill-group-error" className="mg-form-error" role="alert">{error}</p>}
      </div>
      <footer><Button onClick={onClose}>{t("取消")}</Button><button type="submit" className="mg-button mg-primary">{mode === "create" ? t("创建") : mode === "rename" ? t("保存") : t("删除分组")}</button></footer>
    </form>
  </dialog>;
}

function SortableSkillGroup({ name, children }: { name: string; children: (handle: ReactNode) => ReactNode }) {
  const sortable = useSortable({ id: name });
  return <section ref={sortable.setNodeRef} className="mg-skill-group" data-group={name} data-dragging={sortable.isDragging}
    style={{ transform: CSS.Translate.toString(sortable.transform), transition: sortable.transition, position: "relative", zIndex: sortable.isDragging ? 2 : undefined }}>
    {children(<button type="button" className="mg-icon-button mg-group-grip" ref={sortable.setActivatorNodeRef} {...sortable.attributes} {...sortable.listeners}
      aria-label={tf("拖动排序：{0}", [name])} title={t("拖动排序")}><GripVertical size={16}/></button>)}
  </section>;
}

function SkillVersionMenu({ label, disabled, onSelect }: { label: string; disabled: boolean; onSelect: (keepLocal: boolean) => void }) {
  const menu = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [keepLocal, setKeepLocal] = useState(true);
  useEffect(() => {
    const close = () => menu.current?.hidePopover();
    window.addEventListener("resize", close);
    return () => window.removeEventListener("resize", close);
  }, []);
  const choose = (local: boolean) => { setKeepLocal(local); menu.current?.hidePopover(); trigger.current?.focus(); };
  return <div className="mg-version-split">
    <button type="button" className="mg-button mg-version-apply" disabled={disabled} aria-label={`${t("确认")} ${label}`}
      title={keepLocal ? t("确认保留本地版本") : t("确认保留 Codex++ 版本")} onClick={() => onSelect(keepLocal)}>{keepLocal ? t("保留本地") : t("保留 Codex++")}</button>
    <button ref={trigger} type="button" className="mg-button mg-version-select" disabled={disabled} aria-label={`${t("选择")} ${label}`} aria-haspopup="menu" aria-expanded={open}
      onClick={() => {
        const node = menu.current;
        const rect = trigger.current?.getBoundingClientRect();
        if (!node || !rect) return;
        node.style.left = `${Math.max(8, Math.min(rect.right - 200, window.innerWidth - 208))}px`;
        node.style.top = `${Math.max(8, Math.min(rect.bottom + 4, window.innerHeight - 100))}px`;
        node.showPopover();
        node.querySelector<HTMLButtonElement>("button")?.focus();
      }}><ChevronDown size={13}/></button>
    <div ref={menu} popover="auto" className="mg-version-menu" role="menu" aria-label={label}
      onToggle={(event) => setOpen(event.newState === "open")}
      onKeyDown={(event) => {
        if (event.key === "Escape") trigger.current?.focus();
        if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
        event.preventDefault();
        const options = [...(menu.current?.querySelectorAll<HTMLButtonElement>("button") || [])];
        const current = options.indexOf(document.activeElement as HTMLButtonElement);
        options[(current + (event.key === "ArrowDown" ? 1 : options.length - 1)) % options.length]?.focus();
      }}>
      <button role="menuitemradio" aria-checked={keepLocal} type="button" onClick={() => choose(true)}>{t("保留本地版本")}<small>{keepLocal ? t("已选择") : t("默认")}</small></button>
      <button role="menuitemradio" aria-checked={!keepLocal} type="button" onClick={() => choose(false)}>{t("保留 Codex++ 版本")}{!keepLocal && <small>{t("已选择")}</small>}</button>
    </div>
  </div>;
}

function SkillOriginEditor({ skill, onAssociate }: { skill: Skill; onAssociate: (candidate: OriginCandidate) => Promise<boolean> }) {
  const [input, setInput] = useState("");
  const [candidates, setCandidates] = useState<OriginCandidate[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState("");
  const request = useRef(0);
  useEffect(() => () => { request.current += 1; }, []);
  const inspect = async (manual: boolean) => {
    const repo = manual ? parseSkillRepository(input) : null;
    setCandidates([]);
    setSelected(null);
    if (manual && !repo) { setMessage(t("请输入有效的 GitHub 仓库地址")); return; }
    const token = ++request.current;
    setBusy(true);
    setMessage("");
    try {
      const result = await invoke<OriginInspection>("inspect_skill_origin", { id: skill.id, repo: repo ? { ...repo, enabled: true } : null, branchExplicit: manual && /\/tree\//.test(input) });
      if (token !== request.current) return;
      if (!ok(result)) { setMessage(result.message); return; }
      setCandidates(result.candidates);
      if (!result.candidates.length) setMessage(t("未找到匹配来源，请输入 GitHub 仓库地址验证。"));
    } catch (error) {
      if (token === request.current) setMessage(String(error));
    } finally {
      if (token === request.current) setBusy(false);
    }
  };
  const associate = async () => {
    if (selected === null || !candidates[selected]) return;
    setBusy(true);
    try {
      if (await onAssociate(candidates[selected])) {
        setCandidates([]);
        setSelected(null);
        setMessage(t("来源已关联"));
      }
    } finally { setBusy(false); }
  };
  return <section className="mg-origin-editor" aria-label={t("关联来源")}>
    <div className="mg-origin-heading"><h3>{t("来源仓库")}</h3><Button disabled={busy} onClick={() => void inspect(false)}><Search size={14}/>{busy ? t("正在验证…") : t("匹配来源")}</Button></div>
    <form className="mg-origin-form" onSubmit={(event) => { event.preventDefault(); if (!busy) void inspect(true); }}>
      <input aria-label={t("Skill 来源仓库地址")} placeholder="https://github.com/owner/repo" value={input} disabled={busy} onChange={(event) => { setInput(event.currentTarget.value); setCandidates([]); setSelected(null); setMessage(""); }}/>
      <button type="submit" className="mg-button" disabled={busy || !input.trim()}><Link size={14}/>{t("验证仓库")}</button>
    </form>
    {message && <p className="mg-origin-message" role="status">{message}</p>}
    {candidates.length > 0 && <>
      <fieldset className="mg-origin-candidates"><legend>{t("选择来源")}</legend>{candidates.map((candidate, index) => <label key={`${repoKey(candidate.repo)}:${candidate.skill.repoPath}`}>
        <input type="radio" name={`skill-origin-${skill.id}`} checked={selected === index} disabled={busy} onChange={() => setSelected(index)}/>
        <span><strong>{candidate.repo.owner}/{candidate.repo.name}</strong><small>{candidate.repo.branch} · {candidate.skill.repoPath || "/"}</small><small>{candidate.evidence}</small></span>
      </label>)}</fieldset>
      <div className="mg-origin-footer"><Button primary disabled={busy || selected === null} onClick={() => void associate()}>{t("确认关联")}</Button></div>
    </>}
  </section>;
}
function SkillDetailDrawer({ skill, count, onClose, onOpenUrl, onOpenRepo, groups, group, onAssignGroup, onAssociate }: { skill: Skill; count?: number; onClose: () => void; onOpenUrl: (url: string) => void; onOpenRepo: () => void; groups?: string[]; group?: string; onAssignGroup?: (group: string) => void; onAssociate?: (candidate: OriginCandidate) => Promise<boolean> }) {
  const url = repoUrl(skill);
  return <ManagementDrawer dismissOnBackdrop={false} title={skill.name || skill.id} onClose={onClose}><dl className="mg-detail"><dt>{t("来源")}</dt><dd>{url ? <button className="mg-link-button mg-skill-repo-link" type="button" onClick={onOpenRepo}>{sourceLabel(skill)}{count === undefined ? "" : ` · ${count} Skills`}<ArrowRight size={13}/></button> : sourceLabel(skill)}</dd><dt>{t("分组")}</dt><dd>{onAssignGroup ? <select className="mg-detail-group-select" aria-label={t("分配 Skill 分组")} value={group || ""} onChange={(event) => onAssignGroup(event.currentTarget.value)}><option value="">{sourceGroup(skill)} · {t("默认分组")}</option>{(groups || []).filter((name) => name !== sourceGroup(skill)).map((name) => <option key={name} value={name}>{name}</option>)}</select> : group || sourceGroup(skill)}</dd><dt>{t("Skill ID")}</dt><dd>{skill.id}</dd><dt>{t("仓库路径")}</dt><dd>{skill.repoPath || "—"}</dd><dt>{t("介绍")}</dt><dd>{skill.description || t("暂无描述")}</dd>{url && <><dt>GitHub</dt><dd><button className="mg-link-button" type="button" onClick={() => onOpenUrl(url)}><ExternalLink size={13}/>{url}</button></dd></>}</dl>
    {skill.localPath && <dl className="mg-detail"><dt>{t("安装位置")}</dt><dd className="mg-skill-local-path">{skill.localPath}</dd></dl>}
    {onAssociate && skill.installed && !skill.bundled && !skill.plugin && <SkillOriginEditor key={skill.id} skill={skill} onAssociate={onAssociate}/>}
  </ManagementDrawer>;
}
export function SkillsPage({ notify }: { notify: Notify }) {
  const [data, setData] = useState<SkillPayload | null>(skillPageView.data);
  const [tab, setTab] = useState<"discover" | "installed">(skillPageView.tab);
  const [query, setQuery] = useState(skillPageView.query);
  const [mutationBusy, setBusy] = useState(false);
  const updateSnapshot = useSyncExternalStore(skillUpdateQueue.subscribe, skillUpdateQueue.getSnapshot);
  const busy = mutationBusy || updateSnapshot.active;
  const currentUpdate = updateSnapshot.tasks.find((task) => task.state === "running");
  const waitingUpdates = updateSnapshot.tasks.filter((task) => task.state === "queued").length;
  const updateFailures = updateSnapshot.tasks.filter((task) => task.state === "failed");
  useEffect(() => {
    if (updateSnapshot.result) setData(updateSnapshot.result);
  }, [updateSnapshot.result]);
  const [repoOpen, setRepoOpen] = useState(false);
  const [detail, setDetail] = useState<Skill | null>(null);
  const detailRef = useRef(detail);
  detailRef.current = detail;
  const [repoSelection, setRepoSelection] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [refreshing, setRefreshing] = useState(false);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(() => new Set(skillPageView.expandedGroups));
  const [storedGroups] = useState(() => readSkillGroups(localStorage.getItem(SKILL_GROUP_STORAGE_KEY)));
  const [customGroupNames, setCustomGroupNames] = useState(storedGroups.names);
  const [customGroupAssignments, setCustomGroupAssignments] = useState(storedGroups.assignments);
  const [groupOrder, setGroupOrder] = useState(storedGroups.order);
  const [nameSort, setNameSort] = useState<"asc" | "desc" | null>(skillPageView.nameSort);
  const contentRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    skillPageView = { data, tab, query, expandedGroups, nameSort };
  }, [data, tab, query, expandedGroups, nameSort]);
  useLayoutEffect(() => {
    if (contentRef.current) contentRef.current.scrollTop = skillPageScrollTop;
  }, []);
  const [groupDialog, setGroupDialog] = useState<{ mode: "create" | "rename" | "delete"; name: string } | null>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));
  useEffect(() => {
    localStorage.setItem(SKILL_GROUP_STORAGE_KEY, JSON.stringify({ names: customGroupNames, assignments: customGroupAssignments, order: groupOrder }));
  }, [customGroupNames, customGroupAssignments, groupOrder]);
  const load = async (command = "refresh_skill_catalog") => {
    const setPending = command === "refresh_skill_catalog" ? setRefreshing : setLoading;
    setPending(true);
    const previousUpdateResult = skillUpdateQueue.getSnapshot().result;
    try {
      const r = await invoke<SkillPayload>(command);
      if (skillUpdateQueue.getSnapshot().result === previousUpdateResult) {
        startTransition(() => setData(r));
      } else if (command === "refresh_skill_catalog") {
        const beforeRead = skillUpdateQueue.getSnapshot().result;
        const latest = await invoke<SkillPayload>("list_installed_skills");
        if (skillUpdateQueue.getSnapshot().result === beforeRead) {
          startTransition(() => setData({ ...latest, repoErrors: r.repoErrors }));
        }
      }
      if (!ok(r)) await notify("Skills", r.message, "failed");
    } catch (e) {
      await notify("Skills", String(e), "failed");
    } finally {
      setPending(false);
    }
  };
  useEffect(() => {
    let active = true;
    void (async () => {
      await load("list_installed_skills");
      if (active && !skillUpdateQueue.getSnapshot().active) await load();
    })();
    return () => { active = false; };
  }, []);
  const run = async (command: string, args: Record<string, unknown>) => {
    setBusy(true);
    try {
      const r = await invoke<SkillPayload>(command, args);
      setData(r);
      await notify("Skills", r.message, ok(r) ? "ok" : "failed");
      return ok(r);
    } catch (e) {
      await notify("Skills", String(e), "failed");
      return false;
    } finally {
      setBusy(false);
    }
  };
  const runBatch = async (skills: Skill[], enabled: boolean) => {
    const targets = skills.filter((skill) => skill.installed && !skill.bundled && !skill.plugin && skill.enabled !== enabled);
    if (!targets.length) {
      await notify("Skills", enabled ? t("该组没有可启用的 Skill") : t("该组没有可停用的 Skill"), "ok");
      return;
    }
    setBusy(true);
    let latest: SkillPayload | null = null;
    try {
      for (const skill of targets) {
        latest = await invoke<SkillPayload>("set_skill_enabled", { id: skill.id, enabled });
        if (!ok(latest)) break;
      }
      if (latest) setData(latest);
      const failed = latest ? !ok(latest) : false;
      const message = failed && latest
        ? latest.message
        : tf("已批量{0} {1} 个 Skill", [enabled ? t("启用") : t("停用"), targets.length]);
      await notify("Skills", message, failed ? "failed" : "ok");
    } catch (e) {
      await notify("Skills", String(e), "failed");
    } finally {
      setBusy(false);
    }
  };
  const updateGroup = (source: string, skills: Skill[]) => skillUpdateQueue.enqueue(source, updatableSkills(skills));
  const resolveVersions = async (versionSkills: Skill[], keepLocal: boolean) => {
    setBusy(true);
    try {
      for (const skill of versionSkills) {
        const result = await invoke<SkillPayload>("resolve_skill_version", { id: skill.id, keepLocal });
        setData(result);
        if (!ok(result)) {
          await notify("Skills", result.message, "failed");
          return;
        }
      }
      await notify("Skills", t("版本选择已保存，原文件已备份。"), "ok");
    } catch (error) {
      await notify("Skills", String(error), "failed");
    } finally { setBusy(false); }
  };
  const saveRepo = async (repo: Repo, previousKey?: string) => {
    setBusy(true);
    try {
      const result = await invoke<SkillPayload>("upsert_skill_repo", { repo, previousKey });
      setData(result);
      if (!ok(result)) await notify("Skills", result.message, "failed");
      return ok(result);
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return false;
    } finally {
      setBusy(false);
    }
  };
  const deleteRepo = async (repo: Repo) => { await run("delete_skill_repo", { key: repoKey(repo) }); };
  const inspectRepo = async (repo: Repo) => {
    const result = await invoke<RepoInspection>("inspect_skill_repo", { repo });
    if (ok(result)) {
      const payload = await invoke<SkillPayload>("list_installed_skills");
      startTransition(() => setData(payload));
    }
    return result;
  };
  const openRepo = (key: string) => { setRepoSelection(key); setDetail(null); setRepoOpen(true); };
  const associateOrigin = async (candidate: OriginCandidate) => {
    if (!detail) return false;
    setBusy(true);
    try {
      const result = await invoke<SkillPayload>("associate_skill_origin", { id: detail.id, repo: candidate.repo, repoPath: candidate.skill.repoPath });
      if (!ok(result)) { await notify("Skills", result.message, "failed"); return false; }
      setData(result);
      const updated = result.skills.find((skill) => skill.id === detail.id);
      if (updated && detailRef.current?.id === detail.id) setDetail(updated);
      setCustomGroupAssignments((old) => { const next = { ...old }; delete next[detail.id]; return next; });
      setExpandedGroups((old) => { const next = new Set(old); if (updated) next.add(sourceGroup(updated)); return next; });
      await notify("Skills", result.message, "ok");
      return true;
    } catch (error) {
      await notify("Skills", String(error), "failed");
      return false;
    } finally { setBusy(false); }
  };
  const deferredQuery = useDeferredValue(query);
  const items = useMemo(() => (data?.skills || [])
    .filter((s) => (tab === "installed" ? s.installed : !s.installed || s.bundled))
    .filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoKey, sourceLabel(s)], deferredQuery)), [data, tab, deferredQuery]);
  const allGroupNames = useMemo(() => orderedSkillGroups([
    ...customGroupNames, ...(data?.skills || []).map(sourceGroup), ...Object.values(customGroupAssignments),
  ], groupOrder), [customGroupNames, data, customGroupAssignments, groupOrder]);
  const ambiguousSkillNames = useMemo(() => {
    const counts = new Map<string, number>();
    for (const skill of data?.skills || []) {
      const key = `${skill.repoKey}:${skill.name.toLowerCase()}`;
      counts.set(key, (counts.get(key) || 0) + 1);
    }
    return new Set([...counts].filter(([, count]) => count > 1).map(([key]) => key));
  }, [data]);
  const groups = useMemo(() => {
    const grouped = new Map<string, Skill[]>();
    for (const name of customGroupNames) {
      if (!deferredQuery.trim() || matchesManagementQuery([name], deferredQuery)) grouped.set(name, []);
    }
    for (const skill of items) {
      const source = customGroupAssignments[skill.id] || sourceGroup(skill);
      const list = grouped.get(source) || [];
      list.push(skill);
      grouped.set(source, list);
    }
    return allGroupNames.filter((name) => grouped.has(name)).map((name): [string, Skill[]] => [name, grouped.get(name)!]);
  }, [items, customGroupAssignments, customGroupNames, allGroupNames, deferredQuery]);
  const openUrl = async (url: string) => {
    try { await invoke("open_external_url", { url }); }
    catch (e) { await notify("Skills", String(e), "failed"); }
  };
  const createCustomGroup = () => {
    setGroupDialog({ mode: "create", name: "" });
  };
  const saveGroup = (normalized: string) => {
    if (!groupDialog) return;
    const { mode, name } = groupDialog;
    if (mode === "create") {
      setCustomGroupNames((old) => [...old, normalized]);
      setGroupOrder([normalized, ...allGroupNames]);
      setQuery("");
      setTab("installed");
      contentRef.current?.scrollTo({ top: 0 });
    } else if (mode === "rename") {
      setCustomGroupNames((old) => old.map((item) => item === name ? normalized : item));
      setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).map(([id, group]) => [id, group === name ? normalized : group])));
      setGroupOrder(allGroupNames.map((item) => item === name ? normalized : item));
    } else {
      setCustomGroupNames((old) => old.filter((item) => item !== name));
      setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).filter(([, group]) => group !== name)));
      setGroupOrder(allGroupNames.filter((item) => item !== name));
    }
    setExpandedGroups((old) => {
      const next = new Set(old);
      next.delete(name);
      if (mode !== "delete") next.add(normalized);
      return next;
    });
    setGroupDialog(null);
  };
  const renderSkillRow = (s: Skill) => {
    const task = updateSnapshot.tasks.find((task) => task.id === s.id && task.repoKey === s.repoKey);
    const pending = task?.state === "running" || task?.state === "queued";
    return <div className="mg-skill-row" key={s.repoKey + ":" + s.id}>
    <button type="button" className="mg-skill-name" onClick={() => setDetail(s)}>
      <strong title={s.name || s.id}>{s.name || s.id}</strong>
      <small title={s.localPath || s.description}>{ambiguousSkillNames.has(`${s.repoKey}:${s.name.toLowerCase()}`) ? s.localPath || s.repoPath || s.description : s.description || t("暂无描述")}</small>
    </button>
    <div className="mg-skill-source" title={s.repoKey || sourceLabel(s)}>
      {repoUrl(s) ? <button className="mg-link-button" onClick={() => openRepo(s.repoKey)}>{sourceLabel(s)}</button> : sourceLabel(s)}
    </div>
    <div className="mg-skill-status">
      <div className="mg-status-toggle">
        {s.bundled || s.plugin ? <span className="mg-readonly" title={s.plugin ? t("由 Codex 插件管理") : undefined}>{s.plugin ? t("插件") : t("只读")}</span> :
          <button className={"mg-switch " + (s.enabled ? "is-on" : "")} type="button" role="switch"
            aria-checked={s.enabled} aria-label={`${s.name || s.id} ${t("启用技能")}`}
            disabled={!s.installed || busy}
            onClick={() => void run("set_skill_enabled", { id: s.id, enabled: !s.enabled })}><span /></button>}
      </div>
    </div>
    <div className="mg-row-actions">
      {s.installed ? <>
        {s.localDifference && <SkillVersionMenu label={tf("保留版本：{0}", [s.name || s.id])} disabled={busy} onSelect={(keepLocal) => void resolveVersions([s], keepLocal)}/>}
        {s.comparisonError && <span className="mg-warning" title={s.comparisonError}>{t("比较失败")}</span>}
        {(s.updateAvailable || pending) && <Button onClick={() => updateGroup(customGroupAssignments[s.id] || sourceGroup(s), [s])} disabled={mutationBusy || !!s.localDifference || !!s.comparisonError || pending}><RefreshCw size={13} className={task?.state === "running" ? "mg-skill-update-spin" : undefined}/>{task?.state === "running" ? t("更新中") : task?.state === "queued" ? t("已排队") : t("更新")}</Button>}
        {!s.bundled && !s.plugin && <Button onClick={() => void run("uninstall_skill", { id: s.id })} disabled={busy}>{t("卸载")}</Button>}
      </> : !s.bundled && <Button primary onClick={() => void run("install_skill", { repoKey: s.repoKey, id: s.id })} disabled={busy}>{t("安装")}</Button>}
    </div>
  </div>;
  };
  return <div className="mg-page mg-skills-page">
    <Header icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load()} disabled={busy || loading || refreshing}><RefreshCw size={14} className={refreshing ? "mg-skill-update-spin" : undefined}/>{refreshing ? t("正在检查…") : t("刷新目录")}</Button>
      <Button onClick={() => void run("import_ccs_skills", {})} disabled={busy}><BookOpen size={14}/>{t("从 cc-switch 导入")}</Button>
      <Button onClick={() => setRepoOpen(true)}><Link size={14}/>{t("管理仓库")}</Button>
    </>} />
    <div className="mg-tabs-bar">
      <div className="mg-segmented">
        <button type="button" data-active={tab === "discover"} onClick={() => setTab("discover")}>{t("发现")} · {(data?.skills || []).filter((s) => !s.installed || s.bundled).length}</button>
        <button type="button" data-active={tab === "installed"} onClick={() => setTab("installed")}>{t("已安装")} · {(data?.skills || []).filter((s) => s.installed).length}</button>
      </div>
      <SearchBox value={query} onChange={setQuery} placeholder={t("搜索 Skills 或仓库")} />
      <Button onClick={createCustomGroup}><Plus size={14}/>{t("新建分组")}</Button>
      <button type="button" className="mg-icon-button" aria-label={t("按名称排序")} data-direction={nameSort || "none"} title={nameSort === "asc" ? t("按名称排序：Z–A") : t("按名称排序：A–Z")} disabled={allGroupNames.length < 2} onClick={() => { const direction = nameSort === "asc" ? "desc" : "asc"; setNameSort(direction); setGroupOrder([...allGroupNames].sort((a, b) => direction === "asc" ? a.localeCompare(b) : b.localeCompare(a))); }}>{nameSort === "desc" ? <ArrowDownZA size={16}/> : <ArrowDownAZ size={16}/>}</button>
      {groups.length > 1 ? <>
        <button type="button" className="mg-icon-button" aria-label={t("全部折叠")} title={t("全部折叠")} onClick={() => setExpandedGroups(new Set())}><ChevronsDownUp size={16}/></button>
        <button type="button" className="mg-icon-button" aria-label={t("全部展开")} title={t("全部展开")} onClick={() => setExpandedGroups(new Set(groups.map(([source]) => source)))}><ChevronsUpDown size={16}/></button>
      </> : null}
    </div>
    {updateSnapshot.tasks.length > 0 && <div className="mg-skill-update-summary">
      <div role="status" aria-live="polite">
        {currentUpdate ? <><RefreshCw size={13} className="mg-skill-update-spin"/><span>{tf("正在更新：{0}", [currentUpdate.name || currentUpdate.id])}</span></> : <span>{tf("更新完成：成功 {0} 个，失败 {1} 个", [updateSnapshot.tasks.filter((task) => task.state === "succeeded").length, updateFailures.length])}</span>}
        {waitingUpdates > 0 && <small>{tf("等待更新 {0} 个", [waitingUpdates])}</small>}
      </div>
      {updateFailures.length > 0 && <details className="mg-skill-update-errors"><summary>{tf("{0} 个更新失败", [updateFailures.length])}</summary>{updateFailures.map((task) => <p key={`${task.repoKey}:${task.id}`}><strong>{task.name || task.id}</strong> · {task.error}</p>)}</details>}
    </div>}
    <div className="mg-content" ref={contentRef} onScroll={(event) => { skillPageScrollTop = event.currentTarget.scrollTop; }}>
      <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={({ active, over }) => { if (over && active.id !== over.id) { setGroupOrder(moveSkillGroup(allGroupNames, String(active.id), String(over.id))); setNameSort(null); } }}>
      <SortableContext items={groups.map(([name]) => name)} strategy={verticalListSortingStrategy}>
      {!groups.length ? <div className="mg-skill-list"><div className="mg-empty"><h2>{loading || refreshing ? t("正在扫描…") : t("没有匹配的 Skill")}</h2></div></div> : groups.map(([source, groupItems]) => {
        const collapsed = !expandedGroups.has(source) && !query.trim();
        const viewportKey = `${tab}:${source}:${deferredQuery}`;
        const batchItems = groupItems.filter((skill) => skill.installed && !skill.bundled && !skill.plugin);
        const allEnabled = batchItems.length > 0 && batchItems.every((skill) => skill.enabled);
        const mixed = !allEnabled && batchItems.some((skill) => skill.enabled);
        const batchTitle = mixed ? t("部分启用，点击全部启用") : allEnabled ? t("全部停用") : t("全部启用");
        const custom = customGroupNames.includes(source);
        const differences = groupItems.filter((skill) => skill.localDifference).length;
        const updates = groupItems.filter((skill) => skill.updateAvailable).length;
        const updateTargets = updatableSkills(groupItems);
        const groupUpdates = updateSnapshot.tasks.filter((task) => task.group === source);
        const groupRunning = groupUpdates.some((task) => task.state === "running");
        const groupWaiting = groupUpdates.some((task) => task.state === "queued");
        const groupDone = groupUpdates.filter((task) => task.state === "succeeded" || task.state === "failed").length;
        const groupPending = groupRunning || groupWaiting;
        const queuedKeys = new Set(updateSnapshot.tasks.filter((task) => task.state === "queued" || task.state === "running").map((task) => `${task.repoKey}:${task.id}`));
        const canEnqueue = updateTargets.some((skill) => !queuedKeys.has(`${skill.repoKey}:${skill.id}`));
        const comparisonErrors = groupItems.filter((skill) => skill.comparisonError).length;
        return <SortableSkillGroup key={source} name={source}>{(handle) => <>
          <div className="mg-skill-group-head">
            {handle}
            <button type="button" className="mg-group-head" aria-expanded={!collapsed} onClick={() => setExpandedGroups((old) => { const next = new Set(old); next.has(source) ? next.delete(source) : next.add(source); return next; })}>
              <ChevronDown size={15} className={collapsed ? "mg-chevron-collapsed" : ""}/><strong>{source}</strong>
            </button>
            <span className="mg-skill-group-notice" role="status">{[
              differences ? tf("{0} 个本地版本不同", [differences]) : "",
              updates ? tf("{0} 个可更新", [updates]) : "",
              comparisonErrors ? tf("{0} 个比较失败", [comparisonErrors]) : "",
            ].filter(Boolean).join(" · ")}</span>
            {groupUpdates.length > 0 && <span className="mg-skill-update-progress" role="status"><span>{groupRunning ? tf("更新中 {0}/{1}", [groupDone, groupUpdates.length]) : groupWaiting ? t("等待更新") : tf("已完成 {0}/{1}", [groupDone, groupUpdates.length])}</span><progress aria-label={tf("更新进度：{0}", [source])} max={groupUpdates.length} value={groupDone}/></span>}
            <div className="mg-row-actions">
              {(updates > 0 || groupPending) && <Button onClick={() => updateGroup(source, groupItems)} disabled={mutationBusy || !canEnqueue}><RefreshCw size={13} className={groupRunning ? "mg-skill-update-spin" : undefined}/>{groupPending && !canEnqueue ? groupRunning ? t("更新中") : t("已排队") : t("更新全部")}</Button>}
              {custom ? <><button type="button" className="mg-icon-button" aria-label={tf("重命名分组：{0}", [source])} title={t("重命名分组")} onClick={() => setGroupDialog({ mode: "rename", name: source })}><Edit3 size={14}/></button><button type="button" className="mg-icon-button" aria-label={tf("删除分组：{0}", [source])} title={t("删除分组")} onClick={() => setGroupDialog({ mode: "delete", name: source })}><Trash2 size={14}/></button></> : null}
            </div>
            <small className="mg-skill-group-total">{groupItems.length} {t("个 Skill")}</small>
          </div>
          {!collapsed ? <SkillListViewport className="mg-skill-list" items={groupItems} rowHeight={68} resetKey={viewportKey} initialScrollTop={skillGroupScroll.get(viewportKey)} onScrollTopChange={(top) => skillGroupScroll.set(viewportKey, top)} busy={loading} header={groupItems.length ? <div className="mg-skill-head"><span>{t("技能")}</span><span>{t("来源")}</span><span>{t("状态")}</span><span className="mg-group-status-control">{batchItems.length > 0 && <button className={"mg-switch " + (allEnabled ? "is-on" : "")} data-mixed={mixed} type="button" role="switch" aria-checked={allEnabled} aria-label={tf("分组状态：{0}", [source])} title={batchTitle} disabled={busy || loading || refreshing} onClick={() => void runBatch(groupItems, !allEnabled)}><span /></button>}</span></div> : undefined} empty={<div className="mg-group-empty">{t("此分组暂无 Skill")}</div>} renderRow={renderSkillRow} /> : null}
        </>}</SortableSkillGroup>;
      })}
      </SortableContext>
      </DndContext>
      {data?.repoErrors?.length ? <div className="mg-notice mg-warning">{data.repoErrors.map((e) => <p key={e}>{e}</p>)}</div> : null}
    </div>
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} initialKey={repoSelection} busy={busy} onClose={() => setRepoOpen(false)} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(cmd, args) => void run(cmd, args)} />}
    {detail && <SkillDetailDrawer skill={detail} onAssociate={associateOrigin} count={data?.repoCounts?.[detail.repoKey]} onClose={() => setDetail(null)} onOpenUrl={(url) => void openUrl(url)} onOpenRepo={() => repoOpen ? setDetail(null) : openRepo(detail.repoKey)} groups={allGroupNames} group={customGroupAssignments[detail.id] === sourceGroup(detail) ? "" : customGroupAssignments[detail.id]} onAssignGroup={(group) => { setCustomGroupAssignments((old) => { const next = { ...old }; if (group) next[detail.id] = group; else delete next[detail.id]; return next; }); setExpandedGroups((old) => { const next = new Set(old); next.add(group || sourceGroup(detail)); return next; }); }} />}
    {groupDialog && <SkillGroupDialog {...groupDialog} names={allGroupNames} onClose={() => setGroupDialog(null)} onSave={saveGroup}/>}
  </div>;
}
export function SessionPage({ sessions, onRefresh, onDelete, onDeleteMany, notify, repairPanel }: { sessions: { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number } | null; onRefresh: (offset?: number) => Promise<void>; onDelete: (s: ManagedSession) => Promise<void>; onDeleteMany: (s: ManagedSession[]) => Promise<void>; notify: Notify; repairPanel?: ReactNode }) { const [query, setQuery] = useState(""); const [mode, setMode] = useState<"project" | "time">("project"); const [expanded, setExpanded] = useState<Set<string>>(new Set()); const [selection, setSelection] = useState(false); const [selected, setSelected] = useState<Set<string>>(new Set()); const [detail, setDetail] = useState<ManagedSession | null>(null); const all = sessions?.sessions || []; const visible = all.filter((s) => matchesManagementQuery([s.title, s.cwd, s.id, s.modelProvider], query)); const groups = sessionGroups(visible, mode); const selectionScope = visible.map((s) => s.id).join("\0"); useEffect(() => { const ids = new Set(selectionScope.split("\0")); setSelected((old) => { const next = new Set([...old].filter((id) => ids.has(id))); return next.size === old.size ? old : next; }); }, [selectionScope]); const selectedRows = visible.filter((s) => selected.has(s.id)); const toggle = (id: string) => setSelected((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; }); return <div className="mg-page mg-session-page"><Header icon={<History size={20}/>} title={t("会话")} actions={<><Button onClick={() => void onRefresh()}><RefreshCw size={14}/>{t("刷新列表")}</Button><Button onClick={() => { setSelection(!selection); setSelected(new Set()); }}>{t("选择多个…")}</Button></>} />{repairPanel}<div className="mg-session-controls"><SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} /><div className="mg-segmented"><button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button><button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button></div>{selection && <><Button onClick={() => setSelected(new Set(visible.map((s) => s.id)))}>{t("全选当前列表")}</Button><Button disabled={!selectedRows.length} onClick={() => setSelected(new Set())}>{t("取消全部选择")}</Button><Button primary disabled={!selectedRows.length} onClick={() => void onDeleteMany(selectedRows)}><Trash2 size={14}/>{t("删除已选")}</Button></>}</div><div className="mg-content"><div className="mg-session-list">{!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((g) => { const open = query.trim() || expanded.has(g.key) || groups.length === 1; return <section key={g.key}><button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(g.key) ? next.delete(g.key) : next.add(g.key); return next; })}><ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"}/><strong>{g.label}</strong><span>{g.path}</span><small>{g.sessions.length} {t("个会话")}</small></button>{open && g.sessions.map((s) => <article className="mg-session-row" key={s.id}><div className="mg-session-check">{selection && <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)}/>}</div><button type="button" className="mg-session-main" onClick={() => setDetail(s)}><strong>{s.title || t("未命名会话")}</strong><small>{s.cwd || t("未记录项目路径")}</small><code>{s.id}</code></button><div className="mg-session-meta"><span>{s.modelProvider || "Codex"}</span><time>{s.updatedAtMs ? new Date(s.updatedAtMs).toLocaleString() : t("未知时间")}</time></div><div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(s)} aria-label={t("删除会话")}><Trash2 size={15}/></button></div></article>)}</section>; })}</div></div><div className="mg-pagination"><Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - (sessions?.limit || 50)))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14}/>{t("上一页")}</Button><span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [Math.floor(sessions.offset / sessions.limit) + 1, sessions.totalCount]) : "—"}</span><Button onClick={() => void onRefresh((sessions?.offset || 0) + (sessions?.limit || 50))} disabled={!sessions?.hasMore}><ArrowRight size={14}/>{t("下一页")}</Button></div>{detail && <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14}/>{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14}/>{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer>}</div>; }
