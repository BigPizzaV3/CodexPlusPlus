import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Plus, RefreshCw, Search } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { ManagementDrawer } from "./ManagementPages";
import { t, tf } from "./i18n";
import { parseSkillRepository, type SkillRepository } from "./skill-repository";
import "./SkillSourceMatchDrawer.css";

type Match = {
  previewId: string; localId: string; localName: string;
  repoKey: string; repoPath: string; remoteName: string; exact: boolean;
  conflict?: string | null; localPaths?: string[];
};
type Candidate = Match & { manual: boolean };
type ScanResult = { status: string; message: string; previewId: string; matches: Omit<Match, "previewId">[]; manualCandidates?: Omit<Match, "previewId">[]; examined: number; warnings: string[] };
type Evidence = { localId: string; repo: string; kind: string; path: string };
export type SourceSelection = Pick<Match, "previewId" | "localId" | "repoKey" | "repoPath"> & { manual?: boolean; localPath?: string };
type ApplyResult = { status: string; message: string };
const keyOfRepo = (repo: SkillRepository) => `${repo.owner}/${repo.name}@${repo.branch}${repo.subdir ? `:${repo.subdir}` : ""}`;
const keyOfCandidate = (candidate: Match) => JSON.stringify([candidate.repoKey, candidate.repoPath]);
const evidenceLabel = (kind: string) => {
  const labels: Record<string, string> = {
    "manifest-link": t("说明中的链接"),
    "script-link": t("脚本中的链接"),
    package: t("包元数据"),
    receipt: t("安装收据"),
    lock: t("锁文件"),
    "git-remote": "Git remote",
  };
  return labels[kind] || kind;
};

export function SkillSourceMatchDrawer({ repos, id, localPath, localSkillIds, onClose, onAddRepo, onApply }: {
  repos: SkillRepository[];
  id?: string;
  localPath?: string;
  localSkillIds: string[];
  onClose: () => void;
  onAddRepo: (repo: SkillRepository) => Promise<SkillRepository | null>;
  onApply: (selections: SourceSelection[]) => Promise<ApplyResult>;
}) {
  const [knownRepos, setKnownRepos] = useState(repos);
  const [revision, setRevision] = useState(0);
  const [scanning, setScanning] = useState(true);
  const [busy, setBusy] = useState(false);
  const [candidates, setCandidates] = useState<Candidate[]>([]);
  const [warnings, setWarnings] = useState<string[]>([]);
  const [progress, setProgress] = useState(0);
  const [selected, setSelected] = useState<Record<string, string>>({});
  const [input, setInput] = useState("");
  const [query, setQuery] = useState("");
  const [feedback, setFeedback] = useState("");
  const [evidence, setEvidence] = useState<Evidence[]>([]);
  const [evidenceError, setEvidenceError] = useState("");
  const [evidenceScanning, setEvidenceScanning] = useState(true);
  const pending = useRef(false);
  const evidenceIds = (id ? [id] : [...new Set(localSkillIds)].sort()).join("\0");

  useEffect(() => {
    let active = true;
    setEvidence([]);
    setEvidenceError("");
    setEvidenceScanning(true);
    void (async () => {
      for (const localId of evidenceIds ? evidenceIds.split("\0") : []) {
        try {
          const result = await invoke<{ status: string; message: string; evidence: Omit<Evidence, "localId">[] }>("discover_skill_source_evidence", { id: localId, localPath: localPath || null });
          if (!active) return;
          if (result.status === "ok") setEvidence((old) => [...old, ...result.evidence.map((item) => ({ ...item, localId }))]);
          else setEvidenceError((old) => [old, `${localId}：${result.message}`].filter(Boolean).join("；"));
        } catch (error) {
          if (!active) return;
          setEvidenceError((old) => [old, `${localId}：${String(error)}`].filter(Boolean).join("；"));
        }
      }
      if (active) setEvidenceScanning(false);
    })();
    return () => { active = false; };
  }, [evidenceIds, localPath, revision]);

  useEffect(() => {
    let active = true;
    setScanning(true);
    setCandidates([]);
    setWarnings([]);
    setSelected({});
    setProgress(0);
    void (async () => {
      for (const repo of knownRepos) {
        try {
          const result = await invoke<ScanResult>("find_skill_repository_matches", { repoKey: keyOfRepo(repo), id: id || null, localPath: localPath || null });
          if (!active) return;
          if (result.status !== "ok") setWarnings((old) => [...old, `${keyOfRepo(repo)}：${result.message}`]);
          else {
            setCandidates((old) => [...old, ...result.matches.map((match) => ({ ...match, previewId: result.previewId, manual: false })),
              ...(id ? (result.manualCandidates || []).filter((match) => match.localId === id).map((match) => ({ ...match, previewId: result.previewId, manual: true })) : [])]);
            setWarnings((old) => [...old, ...result.warnings]);
          }
        } catch (error) {
          if (!active) return;
          setWarnings((old) => [...old, `${keyOfRepo(repo)}：${String(error)}`]);
        }
        setProgress((old) => old + 1);
      }
      if (active) setScanning(false);
    })();
    return () => { active = false; };
  }, [id, localPath, knownRepos, revision]);

  const groups = useMemo(() => {
    const result = new Map<string, Candidate[]>();
    for (const candidate of candidates) {
      const group = result.get(candidate.localId) || [];
      if (!group.some((item) => keyOfCandidate(item) === keyOfCandidate(candidate))) group.push(candidate);
      result.set(candidate.localId, group);
    }
    for (const group of result.values()) group.sort((a, b) => Number(b.exact) - Number(a.exact) || keyOfCandidate(a).localeCompare(keyOfCandidate(b)));
    return result;
  }, [candidates]);
  const choices = Object.entries(selected).map(([localId, key]) => groups.get(localId)?.find((item) => keyOfCandidate(item) === key));
  const complete = choices.length > 0 && choices.every((item) => item && !item.conflict) && new Set(choices.map((item) => keyOfCandidate(item!))).size === choices.length;

  const add = async () => {
    const parsed = parseSkillRepository(input);
    if (!parsed) {
      setFeedback(t("无法识别仓库地址，请输入 GitHub 链接或 owner/repo。"));
      return;
    }
    if (knownRepos.some((repo) => keyOfRepo(repo).toLowerCase() === keyOfRepo(parsed).toLowerCase())) {
      setFeedback(t("该仓库已添加。"));
      return;
    }
    if (pending.current) return;
    pending.current = true; setBusy(true); setFeedback("");
    try {
      const added = await onAddRepo(parsed);
      if (added) { setKnownRepos((old) => [...old, added]); setInput(""); }
      else setFeedback(t("仓库未添加，请检查地址和网络。"));
    } finally { pending.current = false; setBusy(false); }
  };
  const apply = async () => {
    if (!complete || scanning || pending.current) return;
    pending.current = true; setBusy(true);
    try {
      const selections = choices.map((choice) => {
        const { previewId, localId, repoKey, repoPath, manual } = choice!;
        return { previewId, localId, repoKey, repoPath, ...(manual ? { manual: true } : {}), ...(localPath ? { localPath } : {}) };
      });
      const result = await onApply(selections);
      if (result.status === "ok") onClose();
      else setRevision((old) => old + 1);
    } finally { pending.current = false; setBusy(false); }
  };

  return <ManagementDrawer title={id ? t("重新关联仓库来源") : t("匹配仓库来源")} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}
    footer={<>
      <button type="button" className="cx-secondary" disabled={busy} onClick={() => setRevision((old) => old + 1)}><RefreshCw size={14}/>{t("重新匹配")}</button>
      <button type="button" className="cx-secondary" disabled={busy} onClick={onClose}>{t("取消")}</button>
      <button type="button" className="cx-primary" disabled={busy || scanning || !complete} onClick={() => void apply()}>{tf("关联所选 {0} 项", [choices.length])}</button>
    </>}>
    {localPath && <dl className="mg-detail"><dt>{t("所选安装位置")}</dt><dd>{localPath}</dd></dl>}
    <div className="mg-repo-quick mg-source-add">
      <input aria-label={t("GitHub 仓库地址")} placeholder="https://github.com/owner/repo" value={input} disabled={busy}
        onChange={(event) => { setInput(event.currentTarget.value); setFeedback(""); }}/>
      <button type="button" className="cx-secondary" disabled={busy || !input.trim()} onClick={() => void add()}><Plus size={14}/>{t("检查并添加")}</button>
    </div>
    {feedback && <p className="mg-warning" role="alert">{feedback}</p>}
    {evidenceError && <p className="mg-warning" role="alert">{evidenceError}</p>}
    {evidenceScanning && <p className="mg-muted" role="status">{t("正在检查本地来源线索")}</p>}
    {evidence.length > 0 && <section><strong>{t("本地发现的仓库候选")}</strong>{[...evidence].sort((a, b) =>
      Number(["manifest-link", "script-link"].includes(a.kind)) - Number(["manifest-link", "script-link"].includes(b.kind))
      || a.localId.localeCompare(b.localId) || a.repo.localeCompare(b.repo)).map((item, index) => <div className="mg-source-row" key={`${item.localId}:${item.repo}:${item.path}:${index}`}>
      <div><strong>{item.repo}</strong><small>{id ? evidenceLabel(item.kind) : `${item.localId} · ${evidenceLabel(item.kind)}`}</small></div>
      <div><small>{item.path}</small><button type="button" className="cx-secondary" disabled={busy} onClick={() => setInput(`https://github.com/${item.repo}`)}>{t("填入仓库地址")}</button></div>
    </div>)}</section>}
    <p className="mg-muted" role="status">{scanning ? tf("正在检查仓库 {0}/{1}", [progress, knownRepos.length]) : tf("已检查 {0} 个仓库，{1} 个 Skill 有候选来源", [progress, groups.size])}</p>
    {id && <label className="mg-search mg-source-search"><Search size={14}/><input aria-label={t("搜索候选来源")} placeholder={t("搜索候选来源")} value={query} onChange={(event) => setQuery(event.currentTarget.value)}/></label>}
    {!!warnings.length && <details className="mg-source-warnings"><summary>{tf("{0} 项检查提示", [warnings.length])}</summary>{warnings.map((warning, index) => <p key={`${index}:${warning}`}>{warning}</p>)}</details>}
    {!scanning && !groups.size && <p className="mg-muted">{t("没有找到候选来源，可添加 GitHub 仓库后重试。")}</p>}
    {groups.size > 0 && <div className="mg-source-list">{[...groups].map(([localId, options]) => {
      const filtered = options.filter((item) => `${item.repoKey} ${item.repoPath} ${item.remoteName}`.toLowerCase().includes(query.trim().toLowerCase()));
      const choice = options.find((item) => keyOfCandidate(item) === selected[localId]);
      const visible = choice && !filtered.includes(choice) ? [choice, ...filtered] : filtered;
      return <div className="mg-source-row" key={localId}>
        <label><input type="checkbox" checked={Object.prototype.hasOwnProperty.call(selected, localId)} disabled={busy || scanning}
          onChange={(event) => {
            const checked = event.currentTarget.checked;
            setSelected((old) => {
              const next = { ...old };
              if (!checked) delete next[localId];
              else {
                const exact = options.filter((item) => item.exact && !item.conflict);
                next[localId] = exact.length === 1 ? keyOfCandidate(exact[0]) : "";
              }
              return next;
            });
          }}/><span><strong>{options[0].localName || localId}</strong><small>{localId}</small></span></label>
        <div><AppSelect value={selected[localId] || ""} ariaLabel={`${t("选择仓库来源")} ${localId}`} placeholder={t("选择仓库来源")}
          disabled={busy || scanning || !Object.prototype.hasOwnProperty.call(selected, localId)} menuWidth={400}
          options={visible.map((item) => ({ value: keyOfCandidate(item), label: `${item.repoKey} / ${item.repoPath}`, disabled: !!item.conflict, hint: item.conflict || (item.exact ? t("内容一致") : item.manual ? t("手动选择，内容不同") : t("名称相似，内容不同")) }))}
          onChange={(value) => setSelected((old) => ({ ...old, [localId]: value }))}/>
          {choice && <small>{choice.exact ? t("内容一致") : choice.manual ? t("手动选择，内容不同") : t("名称相似，内容不同")}</small>}
          {options[0].localPaths?.map((path) => <small key={path}>{path}</small>)}
          {options.some((item) => item.conflict) && <small className="mg-warning">{options.find((item) => item.conflict)?.conflict}</small>}
        </div>
      </div>;
    })}</div>}
    {choices.length > 0 && !complete && <p className="mg-warning" role="alert">{t("请为每项选择不同的仓库路径。")}</p>}
  </ManagementDrawer>;
}
