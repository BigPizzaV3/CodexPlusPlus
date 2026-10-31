import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw, Search } from "lucide-react";
import { Action, ManagementDrawer } from "./ManagementPages";
import { AppSelect } from "./AppSelect";
import { t, tf } from "./i18n";
import { checkRepositories } from "./skill-task-queue";
import { groupSourceMatches, preferredSourceMatch, selectedSourceMatches, sourceMatchKey, sourceMatchLabel, type SkillSourceMatch } from "./skill-source-match";
import { SkillRepositoryQuickAdd } from "./SkillRepositoryQuickAdd";
import "./SkillRepositoryMatchDrawer.css";

type Repository = { owner: string; name: string; branch: string; subdir: string; enabled: boolean };
type ScanResult = { status: string; message: string; matches: SkillSourceMatch[]; manualCandidates?: SkillSourceMatch[]; examined: number; warnings: string[] };
type ScanState = { state: "queued" | "running" | "done" | "failed"; message?: string };
const repoKey = (repo: Repository) => `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`;

export function SkillRepositoryMatchDrawer({ repos, id, initialManual = false, onClose, onUpdated, notify }: {
  repos: Repository[]; id?: string; initialManual?: boolean; onClose: () => void; onUpdated: () => void;
  notify: (title: string, message: string, status?: "ok" | "failed") => Promise<void>;
}) {
  const initialRepos = useRef(repos);
  const [revision, setRevision] = useState(0);
  const [scanning, setScanning] = useState(true);
  const [applying, setApplying] = useState(false);
  const [addingRepo, setAddingRepo] = useState(false);
  const applyingRef = useRef(false);
  const [matches, setMatches] = useState<SkillSourceMatch[]>([]);
  const [manualCandidates, setManualCandidates] = useState<SkillSourceMatch[]>([]);
  const [manual, setManual] = useState(Boolean(id && initialManual));
  const [query, setQuery] = useState("");
  const [selected, setSelected] = useState<Set<string>>(() => new Set());
  // 自动预选用 null 标记，手动选择的字符串不会随扫描结果改变。
  const [previews, setPreviews] = useState<Record<string, string | null>>({});
  const [manualPreviews, setManualPreviews] = useState<Record<string, string | null>>({});
  const [scans, setScans] = useState<Record<string, ScanState>>({});
  const [examined, setExamined] = useState(0);
  const [warnings, setWarnings] = useState<string[]>([]);
  useEffect(() => { initialRepos.current = repos; }, [repos]);
  useEffect(() => {
    let active = true;
    const list = initialRepos.current;
    setScanning(true); setMatches([]); setManualCandidates([]); setSelected(new Set()); setPreviews({}); setManualPreviews({}); setExamined(0); setWarnings([]);
    setScans(Object.fromEntries(list.map((repo) => [repoKey(repo), { state: "queued" as const }])));
    void checkRepositories(list, 3, async (repo) => {
      const key = repoKey(repo);
      if (!active) return;
      setScans((old) => ({ ...old, [key]: { state: "running" } }));
      try {
        const result = await invoke<ScanResult>("find_skill_repository_matches", { repoKey: key, id: id || null });
        if (!active) return;
        setScans((old) => ({ ...old, [key]: { state: result.status === "ok" ? "done" : "failed", message: result.message } }));
        if (result.status === "ok") {
          setMatches((old) => [...old, ...result.matches]);
          if (id) setManualCandidates((old) => [...old, ...(result.manualCandidates ?? []).filter((item) => item.localId === id)]);
          setExamined((old) => Math.max(old, result.examined));
          setWarnings((old) => [...new Set([...old, ...result.warnings])]);
        }
      } catch (error) {
        if (active) setScans((old) => ({ ...old, [key]: { state: "failed", message: String(error) } }));
      }
    }).finally(() => { if (active) setScanning(false); });
    return () => { active = false; };
  }, [id, revision]);
  const manualMode = Boolean(id && manual);
  const blocked = applying || addingRepo;
  const automaticGroups = useMemo(() => groupSourceMatches(matches), [matches]);
  const groups = useMemo(() => manualMode
    ? groupSourceMatches([...matches, ...manualCandidates].filter((item) => item.localId === id))
    : automaticGroups, [automaticGroups, id, manualCandidates, manualMode, matches]);
  const activePreviews = useMemo(() => manualMode ? manualPreviews
    : Object.fromEntries(Object.entries(previews).map(([skillId, value]) =>
      [skillId, value ?? preferredSourceMatch(automaticGroups.get(skillId) || [])])),
  [automaticGroups, manualMode, manualPreviews, previews]);
  const setActivePreviews = manualMode ? setManualPreviews : setPreviews;
  const chosen = useMemo(() => selectedSourceMatches(groups,
    Object.fromEntries([...selected].map((skillId) => [skillId, activePreviews[skillId] ?? ""]))), [activePreviews, groups, selected]);
  const canSubmit = Boolean(chosen?.length && (!manualMode || (chosen.length === 1 && chosen[0].localId === id)));
  const failed = Object.entries(scans).filter(([, state]) => state.state === "failed");
  const finished = Object.values(scans).filter((state) => state.state === "done" || state.state === "failed").length;
  const selectAll = () => {
    setSelected(new Set(groups.keys()));
    setActivePreviews((old) => {
      const next = { ...old };
      for (const skillId of groups.keys()) {
        if (!Object.prototype.hasOwnProperty.call(next, skillId)) next[skillId] = manualMode ? "" : null;
      }
      return next;
    });
  };
  const toggle = (skillId: string, checked: boolean) => {
    setSelected((old) => {
      const next = new Set(old);
      if (checked) next.add(skillId); else next.delete(skillId);
      return next;
    });
    if (checked) setActivePreviews((old) => Object.prototype.hasOwnProperty.call(old, skillId)
      ? old : { ...old, [skillId]: manualMode ? "" : null });
  };
  const switchMode = (nextManual: boolean) => {
    if (manualMode === nextManual) return;
    setManual(nextManual);
    setSelected(new Set());
  };
  const restartScan = () => {
    setSelected(new Set());
    setScanning(true);
    setRevision((value) => value + 1);
  };
  const hint = (item: SkillSourceMatch) => manualMode
    ? item.exact ? t("手动选择 · 内容一致") : t("手动选择 · 内容不一致")
    : item.exact ? t("内容一致") : t("名称相似，内容不同");
  const submit = async () => {
    if (applyingRef.current || addingRepo || !chosen || !canSubmit || scanning) return;
    applyingRef.current = true; setApplying(true);
    try {
      const selections = chosen.map(({ localId, localHash, snapshot, repoKey: key, repoPath, remoteHash }) => ({
        localId, localHash, snapshot, repoKey: key, repoPath, remoteHash, ...(manualMode ? { manual: true } : {}),
      }));
      const result = await invoke<{ status: string; message: string; repos?: Repository[] }>("apply_skill_repository_matches", { selections });
      if (result.repos) initialRepos.current = result.repos;
      onUpdated();
      await notify("Skills", result.message, result.status === "ok" ? "ok" : "failed");
      if (result.status === "ok") onClose();
      else restartScan();
    } catch (error) {
      onUpdated();
      await notify("Skills", String(error), "failed");
      restartScan();
    }
    finally { applyingRef.current = false; setApplying(false); }
  };
  return <ManagementDrawer title={id ? t("匹配 Skill 仓库来源") : t("批量匹配仓库来源")} dismissOnBackdrop={false} onClose={blocked ? () => {} : onClose}
    footer={<><Action disabled={blocked} onClick={restartScan}><RefreshCw size={14}/>{t("重新匹配")}</Action><Action disabled={blocked} onClick={onClose}>{t("取消")}</Action><Action primary disabled={blocked || scanning || !canSubmit} onClick={() => void submit()}>{tf("关联所选 {0} 项", [chosen?.length || 0])}</Action></>}>
    <div className="mg-source-match-progress" role="status">{scanning ? tf("正在检查仓库 {0}/{1}", [finished, initialRepos.current.length]) : tf("已检查 {0} 个仓库，{1} 个 Skill 有候选来源", [finished, groups.size])}</div>
    {id && <div className="mg-segmented" role="group" aria-label={t("匹配方式")}>
      <button type="button" data-active={!manualMode} aria-pressed={!manualMode} disabled={blocked} onClick={() => switchMode(false)}>{t("自动匹配")}</button>
      <button type="button" data-active={manualMode} aria-pressed={manualMode} disabled={blocked} onClick={() => switchMode(true)}>{t("手动选择")}</button>
    </div>}
    {manualMode && <><SkillRepositoryQuickAdd repos={initialRepos.current} disabled={blocked} onBusyChange={setAddingRepo}
      onAdded={(nextRepos) => { initialRepos.current = nextRepos; onUpdated(); restartScan(); }}/>
      <label className="mg-search mg-source-match-search"><Search size={15} aria-hidden="true"/><input type="search" aria-label={t("搜索仓库或 Skill 路径")} placeholder={t("搜索仓库或 Skill 路径")} value={query} disabled={blocked} onChange={(event) => setQuery(event.currentTarget.value)}/></label></>}
    {!initialRepos.current.length && <p className="mg-notice mg-warning">{t("尚未添加仓库。")}</p>}
    {!scanning && initialRepos.current.length > 0 && !groups.size && <p className="mg-muted">{manualMode ? t("没有可选择的仓库 Skill。") : examined ? t("未找到匹配来源。") : t("没有需要补充来源的 Skill。")}</p>}
    {failed.length > 0 && <details className="mg-source-match-errors"><summary>{tf("{0} 个仓库检查失败", [failed.length])}</summary>{failed.map(([key, state]) => <p key={key}>{key}：{state.message}</p>)}</details>}
    {warnings.length > 0 && <details className="mg-source-match-errors"><summary>{tf("{0} 项无法比较", [warnings.length])}</summary>{warnings.map((warning) => <p key={warning}>{warning}</p>)}</details>}
    {groups.size > 0 && <><div className="mg-source-match-toolbar"><span>{tf("已选 {0} 项", [selected.size])}</span><div><Action disabled={blocked || scanning} onClick={selectAll}>{t("全选")}</Action><Action disabled={blocked || !selected.size} onClick={() => setSelected(new Set())}>{t("清空选择")}</Action></div></div>
      {chosen === null && <p className="mg-notice mg-warning" role="alert">{t("请选择唯一的仓库路径；同一路径不能关联多个 Skill。")}</p>}
      <div className="mg-source-match-list">{[...groups].map(([skillId, candidates]) => {
        const checked = selected.has(skillId);
        const choice = candidates.find((item) => sourceMatchKey(item) === activePreviews[skillId]);
        const terms = manualMode ? query.trim().toLowerCase().split(/\s+/).filter(Boolean) : [];
        const filtered = candidates.filter((item) => terms.every((term) => `${sourceMatchLabel(item)} ${item.remoteName}`.toLowerCase().includes(term)));
        const options = choice && !filtered.includes(choice) ? [choice, ...filtered] : filtered;
        return <div className="mg-source-match-row" key={skillId}><label><input type="checkbox" checked={checked} disabled={blocked} onChange={(event) => toggle(skillId, event.currentTarget.checked)}/><span><strong>{candidates[0].localName || skillId}</strong><small>{skillId}</small></span></label>
          <div className="mg-source-match-target"><AppSelect className="mg-source-match-select" menuWidth={420} value={activePreviews[skillId] || ""} placeholder={t("选择仓库来源")} disabled={blocked || !options.length} ariaLabel={`${t("选择仓库来源")} ${skillId}`}
            options={options.map((item) => ({ value: sourceMatchKey(item), label: sourceMatchLabel(item), hint: hint(item) }))}
            onChange={(value) => setActivePreviews((old) => ({ ...old, [skillId]: value }))}/>
            {choice && <small data-exact={choice.exact}>{hint(choice)}</small>}
            {manualMode && terms.length > 0 && !filtered.length && <small role="status">{t("没有匹配的仓库路径。")}</small>}</div></div>;
      })}</div></>}
  </ManagementDrawer>;
}
