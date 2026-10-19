import { startTransition, useDeferredValue, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, BookOpen, ChevronDown, Copy, Edit3, ExternalLink, History, Link, Plus, RefreshCw, Search, Trash2, X } from "lucide-react";
import { t, tf } from "./i18n";
import { matchesManagementQuery, sessionGroups, type ManagedSession } from "./management-model";
import { ManagementDrawer } from "./ManagementPages";
import "./ManagementPages.css";
import { parseSkillRepository } from "./skill-repository";
import { SkillListViewport } from "./SkillListViewport";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
type Skill = { id: string; name: string; description: string; repoKey: string; repoPath?: string; installed: boolean; enabled: boolean; bundled: boolean; updateAvailable: boolean };
type Repo = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type SkillPayload = Result & { skills: Skill[]; repos: Repo[]; repositorySkills: Skill[]; repoCounts: Record<string, number>; repoErrors: string[]; skillsDir: string; codexSkillsDir: string };
type RepoInspection = Result & { repo: Repo; skills: Skill[]; skillCount: number };
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
function sourceLabel(skill: Skill) { if (skill.bundled) return t("内置"); if (skill.repoKey === "cc-switch") return "cc-switch"; return skill.repoKey ? skill.repoKey.split("@", 1)[0] : t("用户"); }
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
          renderRow={(s) => <div className="mg-repo-skill" key={s.repoKey + ":" + s.id}><button type="button" className="mg-skill-name" onClick={() => onOpenSkill(s)}><strong>{s.name || s.id}</strong><small title={s.description}>{s.description || s.repoPath}</small></button><div className="mg-repo-skill-actions"><Button onClick={() => onOpenSkill(s)}>{t("详情")}</Button><div className="mg-repo-install-slot">{repos.some((r) => repoKey(r) === s.repoKey) && (s.installed ? <span className="mg-repo-install-state">{t("已安装")}</span> : <Button primary disabled={busy} onClick={() => onRun("install_skill", { repoKey: s.repoKey, id: s.id })}>{t("安装")}</Button>)}</div></div></div>} />
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
export function SkillsPage({ notify }: { notify: Notify }) {
  const [data, setData] = useState<SkillPayload | null>(null);
  const [tab, setTab] = useState<"discover" | "installed">("discover");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState(false);
  const [repoOpen, setRepoOpen] = useState(false);
  const [detail, setDetail] = useState<Skill | null>(null);
  const [repoSelection, setRepoSelection] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  const [customGroupNames, setCustomGroupNames] = useState<string[]>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem(SKILL_GROUP_STORAGE_KEY) || "{}");
      return Array.isArray(raw.names) ? raw.names.filter((name: unknown): name is string => typeof name === "string" && name.trim().length > 0) : [];
    } catch { return []; }
  });
  const [customGroupAssignments, setCustomGroupAssignments] = useState<Record<string, string>>(() => {
    try {
      const raw = JSON.parse(localStorage.getItem(SKILL_GROUP_STORAGE_KEY) || "{}");
      return raw.assignments && typeof raw.assignments === "object" ? raw.assignments : {};
    } catch { return {}; }
  });
  useEffect(() => {
    localStorage.setItem(SKILL_GROUP_STORAGE_KEY, JSON.stringify({ names: customGroupNames, assignments: customGroupAssignments }));
  }, [customGroupNames, customGroupAssignments]);
  const load = async (command = "refresh_skill_catalog") => {
    setLoading(true);
    try {
      const r = await invoke<SkillPayload>(command);
      startTransition(() => setData(r));
      if (!ok(r)) await notify("Skills", r.message, "failed");
    } catch (e) {
      await notify("Skills", String(e), "failed");
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => {
    let active = true;
    void (async () => {
      await load("list_installed_skills");
      if (active) await load();
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
    const targets = skills.filter((skill) => skill.installed && !skill.bundled && skill.enabled !== enabled);
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
  const deferredQuery = useDeferredValue(query);
  const items = useMemo(() => (data?.skills || [])
    .filter((s) => (tab === "installed" ? s.installed : !s.installed || s.bundled))
    .filter((s) => matchesManagementQuery([s.id, s.name, s.description, s.repoKey, sourceLabel(s)], deferredQuery)), [data, tab, deferredQuery]);
  const groups = useMemo(() => {
    const grouped = new Map<string, Skill[]>();
    for (const skill of items) {
      const source = customGroupAssignments[skill.id] || sourceLabel(skill);
      const list = grouped.get(source) || [];
      list.push(skill);
      grouped.set(source, list);
    }
    return [...grouped.entries()].sort(([a], [b]) => a.localeCompare(b));
  }, [items, customGroupAssignments]);
  const openUrl = async (url: string) => {
    try { await invoke("open_external_url", { url }); }
    catch (e) { await notify("Skills", String(e), "failed"); }
  };
  const createCustomGroup = () => {
    const normalized = window.prompt(t("新建 Skill 分组"), "")?.trim();
    if (!normalized || customGroupNames.includes(normalized)) return;
    setCustomGroupNames((old) => [...old, normalized]);
  };
  const renameCustomGroup = (oldName: string) => {
    const normalized = window.prompt(t("重命名 Skill 分组"), oldName)?.trim();
    if (!normalized || normalized === oldName || customGroupNames.includes(normalized)) return;
    setCustomGroupNames((old) => old.map((item) => item === oldName ? normalized : item));
    setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).map(([id, group]) => [id, group === oldName ? normalized : group])));
  };
  const deleteCustomGroup = (name: string) => {
    if (!window.confirm(tf("删除分组“{0}”？其中的 Skill 将回到来源分组。", [name]))) return;
    setCustomGroupNames((old) => old.filter((item) => item !== name));
    setCustomGroupAssignments((old) => Object.fromEntries(Object.entries(old).filter(([, group]) => group !== name)));
  };
  const assignSkillGroup = (skillId: string, group: string) => {
    setCustomGroupAssignments((old) => {
      const next = { ...old };
      if (group) next[skillId] = group;
      else delete next[skillId];
      return next;
    });
  };
  const renderSkillRow = (s: Skill) => <div className="mg-skill-row" key={s.repoKey + ":" + s.id}>
    <button type="button" className="mg-skill-name" onClick={() => setDetail(s)}>
      <strong title={s.name || s.id}>{s.name || s.id}</strong>
      <small title={s.description}>{s.description || t("暂无描述")}</small>
    </button>
    <div className="mg-skill-source" title={s.repoKey || sourceLabel(s)}>
      {repoUrl(s) ? <button className="mg-link-button" onClick={() => openRepo(s.repoKey)}>{sourceLabel(s)}</button> : sourceLabel(s)}
      <select className="mg-skill-group-select" aria-label={t("分配 Skill 分组")} value={customGroupAssignments[s.id] || ""} onChange={(event) => assignSkillGroup(s.id, event.currentTarget.value)}>
        <option value="">{t("按来源")}</option>
        {customGroupNames.map((group) => <option key={group} value={group}>{group}</option>)}
      </select>
    </div>
    <div className="mg-skill-status">
      <div className="mg-status-toggle">
        {s.bundled ? <span className="mg-readonly">{t("只读")}</span> :
          <button className={"mg-switch " + (s.enabled ? "is-on" : "")} type="button" role="switch"
            aria-checked={s.enabled} aria-label={`${s.name || s.id} ${t("启用技能")}`}
            disabled={!s.installed || busy}
            onClick={() => void run("set_skill_enabled", { id: s.id, enabled: !s.enabled })}><span /></button>}
      </div>
      <span className="mg-badge">{s.installed ? (s.enabled ? t("已启用") : t("已停用")) : t("未安装")}</span>
    </div>
    <div className="mg-row-actions">
      {s.installed ? <>
        {s.updateAvailable && <Button onClick={() => void run("update_skill", { repoKey: s.repoKey, id: s.id })} disabled={busy}>{t("更新")}</Button>}
        {!s.bundled && <Button onClick={() => void run("uninstall_skill", { id: s.id })} disabled={busy}>{t("卸载")}</Button>}
      </> : !s.bundled && <Button primary onClick={() => void run("install_skill", { repoKey: s.repoKey, id: s.id })} disabled={busy}>{t("安装")}</Button>}
    </div>
  </div>;
  return <div className="mg-page mg-skills-page">
    <Header icon={<BookOpen size={20}/>} title="Skills" actions={<>
      <Button onClick={() => void load()} disabled={busy || loading}><RefreshCw size={14}/>{t("刷新目录")}</Button>
      <Button onClick={() => void run("import_ccs_skills", {})} disabled={busy}><BookOpen size={14}/>{t("从 cc-switch 导入")}</Button>
      <Button onClick={() => setRepoOpen(true)}><Link size={14}/>{t("管理仓库")}</Button>
    </>} />
    <div className="mg-tabs-bar">
      <div className="mg-segmented">
        <button type="button" data-active={tab === "discover"} onClick={() => setTab("discover")}>{t("发现")} · {(data?.skills || []).filter((s) => !s.installed || s.bundled).length}</button>
        <button type="button" data-active={tab === "installed"} onClick={() => setTab("installed")}>{t("已安装")} · {(data?.skills || []).filter((s) => s.installed).length}</button>
      </div>
      <SearchBox value={query} onChange={setQuery} placeholder={t("搜索 Skills 或仓库")} />
      <Button onClick={createCustomGroup}>{t("新建分组")}</Button>
      {groups.length > 1 ? <>
        <Button onClick={() => setCollapsedGroups(new Set(groups.map(([source]) => source)))}>{t("全部折叠")}</Button>
        <Button onClick={() => setCollapsedGroups(new Set())}>{t("全部展开")}</Button>
      </> : null}
    </div>
    <div className="mg-content">
      {!groups.length ? <div className="mg-skill-list"><div className="mg-empty"><h2>{loading ? t("正在扫描…") : t("没有匹配的 Skill")}</h2></div></div> : groups.map(([source, groupItems]) => {
        const collapsed = collapsedGroups.has(source) && !query.trim();
        const canBatch = groupItems.some((skill) => skill.installed && !skill.bundled);
        const custom = customGroupNames.includes(source);
        return <section className="mg-skill-group" key={source}>
          <div className="mg-skill-group-head">
            <button type="button" className="mg-group-head" onClick={() => setCollapsedGroups((old) => { const next = new Set(old); next.has(source) ? next.delete(source) : next.add(source); return next; })}>
              <ChevronDown size={15} className={collapsed ? "mg-chevron-collapsed" : ""}/><strong>{source}</strong><small>{groupItems.length} {t("个 Skill")}</small>
            </button>
            <div className="mg-row-actions">
              {custom ? <><Button disabled={busy} onClick={() => renameCustomGroup(source)}>{t("重命名")}</Button><Button disabled={busy} onClick={() => deleteCustomGroup(source)}>{t("删除分组")}</Button></> : null}
              {canBatch ? <><Button disabled={busy} onClick={() => void runBatch(groupItems, true)}>{t("全部启用")}</Button><Button disabled={busy} onClick={() => void runBatch(groupItems, false)}>{t("全部停用")}</Button></> : null}
            </div>
          </div>
          {!collapsed ? <SkillListViewport className="mg-skill-list" items={groupItems} rowHeight={68} resetKey={`${tab}:${source}:${deferredQuery}`} busy={loading} header={<div className="mg-skill-head"><span>{t("技能")}</span><span>{t("来源")}</span><span>{t("状态")}</span><span /></div>} empty={<div className="mg-empty"><h2>{t("没有匹配的 Skill")}</h2></div>} renderRow={renderSkillRow} /> : null}
        </section>;
      })}
      {data?.repoErrors?.length ? <div className="mg-notice mg-warning">{data.repoErrors.map((e) => <p key={e}>{e}</p>)}</div> : null}
    </div>
    {repoOpen && <SkillRepoDrawer repos={data?.repos || []} catalog={data?.repositorySkills || []} counts={data?.repoCounts || {}} initialKey={repoSelection} busy={busy} onClose={() => setRepoOpen(false)} onSave={saveRepo} onDelete={(r) => void deleteRepo(r)} onInspect={inspectRepo} onOpenUrl={(url) => void openUrl(url)} onOpenSkill={setDetail} onRun={(cmd, args) => void run(cmd, args)} />}
    {detail && <SkillDetailDrawer skill={detail} count={data?.repoCounts?.[detail.repoKey]} onClose={() => setDetail(null)} onOpenUrl={(url) => void openUrl(url)} onOpenRepo={() => repoOpen ? setDetail(null) : openRepo(detail.repoKey)} />}
  </div>;
}
export function SessionPage({ sessions, onRefresh, onDelete, onDeleteMany, notify, repairPanel }: { sessions: { sessions: ManagedSession[]; totalCount: number; hasMore: boolean; offset: number; limit: number } | null; onRefresh: (offset?: number) => Promise<void>; onDelete: (s: ManagedSession) => Promise<void>; onDeleteMany: (s: ManagedSession[]) => Promise<void>; notify: Notify; repairPanel?: ReactNode }) { const [query, setQuery] = useState(""); const [mode, setMode] = useState<"project" | "time">("project"); const [expanded, setExpanded] = useState<Set<string>>(new Set()); const [selection, setSelection] = useState(false); const [selected, setSelected] = useState<Set<string>>(new Set()); const [detail, setDetail] = useState<ManagedSession | null>(null); const all = sessions?.sessions || []; const visible = all.filter((s) => matchesManagementQuery([s.title, s.cwd, s.id, s.modelProvider], query)); const groups = sessionGroups(visible, mode); const selectionScope = visible.map((s) => s.id).join("\0"); useEffect(() => { const ids = new Set(selectionScope.split("\0")); setSelected((old) => { const next = new Set([...old].filter((id) => ids.has(id))); return next.size === old.size ? old : next; }); }, [selectionScope]); const selectedRows = visible.filter((s) => selected.has(s.id)); const toggle = (id: string) => setSelected((old) => { const next = new Set(old); next.has(id) ? next.delete(id) : next.add(id); return next; }); return <div className="mg-page mg-session-page"><Header icon={<History size={20}/>} title={t("会话")} actions={<><Button onClick={() => void onRefresh()}><RefreshCw size={14}/>{t("刷新列表")}</Button><Button onClick={() => { setSelection(!selection); setSelected(new Set()); }}>{t("选择多个…")}</Button></>} />{repairPanel}<div className="mg-session-controls"><SearchBox value={query} onChange={setQuery} placeholder={t("搜索标题、目录或会话 ID")} /><div className="mg-segmented"><button type="button" data-active={mode === "project"} onClick={() => setMode("project")}>{t("按项目")}</button><button type="button" data-active={mode === "time"} onClick={() => setMode("time")}>{t("按时间")}</button></div>{selection && <><Button onClick={() => setSelected(new Set(visible.map((s) => s.id)))}>{t("全选当前列表")}</Button><Button disabled={!selectedRows.length} onClick={() => setSelected(new Set())}>{t("取消全部选择")}</Button><Button primary disabled={!selectedRows.length} onClick={() => void onDeleteMany(selectedRows)}><Trash2 size={14}/>{t("删除已选")}</Button></>}</div><div className="mg-content"><div className="mg-session-list">{!visible.length ? <div className="mg-empty"><h2>{t("还没有匹配的会话")}</h2><Button onClick={() => void onRefresh()}>{t("刷新")}</Button></div> : groups.map((g) => { const open = query.trim() || expanded.has(g.key) || groups.length === 1; return <section key={g.key}><button type="button" className="mg-group-head" onClick={() => setExpanded((old) => { const next = new Set(old); next.has(g.key) ? next.delete(g.key) : next.add(g.key); return next; })}><ChevronDown size={15} className={open ? "" : "mg-chevron-collapsed"}/><strong>{g.label}</strong><span>{g.path}</span><small>{g.sessions.length} {t("个会话")}</small></button>{open && g.sessions.map((s) => <article className="mg-session-row" key={s.id}><div className="mg-session-check">{selection && <input type="checkbox" checked={selected.has(s.id)} onChange={() => toggle(s.id)}/>}</div><button type="button" className="mg-session-main" onClick={() => setDetail(s)}><strong>{s.title || t("未命名会话")}</strong><small>{s.cwd || t("未记录项目路径")}</small><code>{s.id}</code></button><div className="mg-session-meta"><span>{s.modelProvider || "Codex"}</span><time>{s.updatedAtMs ? new Date(s.updatedAtMs).toLocaleString() : t("未知时间")}</time></div><div className="mg-row-actions"><button type="button" className="mg-icon-button" onClick={() => void onDelete(s)} aria-label={t("删除会话")}><Trash2 size={15}/></button></div></article>)}</section>; })}</div></div><div className="mg-pagination"><Button onClick={() => void onRefresh(Math.max(0, (sessions?.offset || 0) - (sessions?.limit || 50)))} disabled={!sessions || !sessions.offset}><ArrowLeft size={14}/>{t("上一页")}</Button><span>{sessions ? tf("第 {0} 页 · 共 {1} 个会话", [Math.floor(sessions.offset / sessions.limit) + 1, sessions.totalCount]) : "—"}</span><Button onClick={() => void onRefresh((sessions?.offset || 0) + (sessions?.limit || 50))} disabled={!sessions?.hasMore}><ArrowRight size={14}/>{t("下一页")}</Button></div>{detail && <ManagementDrawer title={detail.title || t("未命名会话")} onClose={() => setDetail(null)} footer={<><Button onClick={() => navigator.clipboard.writeText(detail.id)}><Copy size={14}/>{t("复制会话 ID")}</Button><Button primary onClick={() => void onDelete(detail)}><Trash2 size={14}/>{t("删除会话")}</Button></>}><dl className="mg-detail"><dt>{t("项目目录")}</dt><dd>{detail.cwd || "—"}</dd><dt>{t("模型供应商")}</dt><dd>{detail.modelProvider || "Codex"}</dd><dt>{t("数据库")}</dt><dd>{detail.dbPath || "—"}</dd></dl></ManagementDrawer>}</div>; }
