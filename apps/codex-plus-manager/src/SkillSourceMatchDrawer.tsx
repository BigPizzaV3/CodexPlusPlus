import { useEffect, useMemo, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Pause, Play, Plus, RefreshCw, Search } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { ManagementDrawer } from "./ManagementPages";
import { t, tf } from "./i18n";
import { parseSkillRepository, type SkillRepository } from "./skill-repository";
import { discardSkillSourceScan, getSkillSourceScan, skillSourceRepoKey as keyOfRepo, type SkillSourceMatch as Match } from "./skill-source-scan";
import "./SkillSourceMatchDrawer.css";

export type SourceSelection = Pick<Match, "previewId" | "localId" | "repoKey" | "repoPath"> & { manual?: boolean; localPath?: string };
type ApplyResult = { status: string; message: string; applied?: { localId: string }[] };
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
  const scan = useMemo(() => getSkillSourceScan((command, args) => invoke(command, args), id, localPath), [id, localPath]);
  const [snapshot, setSnapshot] = useState(scan.getSnapshot);
  const [action, setAction] = useState<"add" | "apply" | null>(null);
  const [selected, setSelected] = useState<Record<string, string>>({});
  const [input, setInput] = useState("");
  const [query, setQuery] = useState("");
  const [feedback, setFeedback] = useState("");
  const pending = useRef(false);
  const evidenceIds = (id ? [id] : [...new Set(localSkillIds)].sort()).join("\0");
  const repoKeys = repos.map(keyOfRepo).join("\0");
  const { status, candidates, warnings, progress, currentRepo, evidence, evidenceErrors, evidenceScanning } = snapshot;
  const knownRepos = snapshot.repos;
  const scanning = status === "scanning" || status === "idle";
  const busy = action !== null;

  useEffect(() => {
    const unsubscribe = scan.subscribe(() => setSnapshot(scan.getSnapshot()));
    setSnapshot(scan.getSnapshot());
    scan.ensure(repos, evidenceIds ? evidenceIds.split("\0") : []);
    return unsubscribe;
  }, [scan, repoKeys, evidenceIds]);

  const groups = useMemo(() => {
    const result = new Map<string, typeof candidates>();
    for (const candidate of candidates) {
      const group = result.get(candidate.localId) || [];
      if (!group.some((item) => keyOfCandidate(item) === keyOfCandidate(candidate))) group.push(candidate);
      result.set(candidate.localId, group);
    }
    for (const group of result.values()) group.sort((a, b) => Number(b.exact) - Number(a.exact)
      || Number(a.manual) - Number(b.manual) || keyOfCandidate(a).localeCompare(keyOfCandidate(b)));
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
    pending.current = true; setAction("add"); setFeedback("");
    try {
      const added = await onAddRepo(parsed);
      if (added) { scan.addRepo(added); setInput(""); }
      else setFeedback(t("仓库未添加，请检查地址和网络。"));
    } catch (error) { setFeedback(String(error)); }
    finally { pending.current = false; setAction(null); }
  };
  const apply = async () => {
    if (!complete || scanning || pending.current) return;
    pending.current = true; setAction("apply"); setFeedback("");
    try {
      const selections = choices.map((choice) => {
        const { previewId, localId, repoKey, repoPath, manual } = choice!;
        return { previewId, localId, repoKey, repoPath, ...(manual ? { manual: true } : {}), ...(localPath ? { localPath } : {}) };
      });
      const result = await onApply(selections);
      if (result.status === "ok") {
        discardSkillSourceScan(scan, id, localPath);
        onClose();
      } else {
        if (result.applied?.length) {
          const applied = result.applied.map((item) => item.localId);
          scan.consume(applied);
          setSelected((old) => Object.fromEntries(Object.entries(old).filter(([localId]) => !applied.includes(localId))));
        }
        setFeedback(result.message);
      }
    } catch (error) { setFeedback(String(error)); }
    finally { pending.current = false; setAction(null); }
  };

  return <ManagementDrawer title={id ? t("重新关联仓库来源") : t("匹配仓库来源")} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}
    footer={<>
      {!scanning && <button type="button" className="mg-icon-button" title={t("重新匹配")} aria-label={t("重新匹配")} disabled={busy}
        onClick={() => { setSelected({}); setFeedback(""); scan.restart(knownRepos, evidenceIds ? evidenceIds.split("\0") : []); }}><RefreshCw size={14}/></button>}
      {status === "paused" && <button type="button" className="cx-secondary" disabled={busy} onClick={() => scan.resume()}><Play size={14}/>{t("继续扫描")}</button>}
      <button type="button" className="cx-secondary" disabled={busy} onClick={onClose}>{t("关闭")}</button>
      {scanning ? <button type="button" className="cx-primary" disabled={busy || status === "idle"} onClick={() => scan.pause()}><Pause size={14}/>{t("暂停扫描")}</button>
        : <button type="button" className="cx-primary" disabled={busy || !complete} onClick={() => void apply()}>{action === "apply" ? t("正在关联来源…") : tf("关联所选 {0} 项", [choices.length])}</button>}
    </>}>
    {localPath && <dl className="mg-detail"><dt>{t("所选安装位置")}</dt><dd>{localPath}</dd></dl>}
    <div className="mg-repo-quick mg-source-add">
      <input aria-label={t("GitHub 仓库地址")} placeholder="https://github.com/owner/repo" value={input} disabled={busy}
        onChange={(event) => { setInput(event.currentTarget.value); setFeedback(""); }}/>
      <button type="button" className="cx-secondary" disabled={busy || !input.trim()} onClick={() => void add()}><Plus size={14}/>{t(action === "add" ? "正在检查仓库…" : "检查并添加")}</button>
    </div>
    {feedback && <p className="mg-warning" role="alert">{feedback}</p>}
    {evidenceErrors.length > 0 && <p className="mg-warning" role="alert">{evidenceErrors.join("；")}</p>}
    {evidenceScanning && <p className="mg-muted" role="status">{t(status === "paused" ? "正在完成当前线索" : "正在检查本地来源线索")}</p>}
    {evidence.length > 0 && <section><strong>{t("本地发现的仓库候选")}</strong>{[...evidence].sort((a, b) =>
      Number(["manifest-link", "script-link"].includes(a.kind)) - Number(["manifest-link", "script-link"].includes(b.kind))
      || a.localId.localeCompare(b.localId) || a.repo.localeCompare(b.repo)).map((item, index) => <div className="mg-source-row" key={`${item.localId}:${item.repo}:${item.path}:${index}`}>
      <div><strong>{item.repo}</strong><small>{id ? evidenceLabel(item.kind) : `${item.localId} · ${evidenceLabel(item.kind)}`}</small></div>
      <div><small>{item.path}</small><button type="button" className="cx-secondary" disabled={busy} onClick={() => setInput(`https://github.com/${item.repo}`)}>{t("填入仓库地址")}</button></div>
    </div>)}</section>}
    <p className="mg-muted" role="status">{scanning
      ? `${tf("正在检查仓库 {0}/{1}", [progress, knownRepos.length])}${currentRepo ? `：${currentRepo}` : ""}`
      : status === "paused"
        ? `${tf("已暂停，已检查 {0}/{1} 个仓库", [progress, knownRepos.length])}${currentRepo ? ` · ${t("正在完成当前仓库")}：${currentRepo}` : ""}`
        : tf("已检查 {0} 个仓库，{1} 个 Skill 有候选来源", [progress, groups.size])}</p>
    {id && <label className="mg-search mg-source-search"><Search size={14}/><input aria-label={t("搜索候选来源")} placeholder={t("搜索候选来源")} value={query} onChange={(event) => setQuery(event.currentTarget.value)}/></label>}
    {!!warnings.length && <details className="mg-source-warnings"><summary>{tf("{0} 项检查提示", [warnings.length])}</summary>{warnings.map((warning, index) => <p key={`${index}:${warning}`}>{warning}</p>)}</details>}
    {!scanning && !groups.size && <p className="mg-muted">{status === "paused" && progress < knownRepos.length
      ? t("已检查的仓库中没有候选来源，可继续扫描。") : t("没有找到候选来源，可添加 GitHub 仓库后重试。")}</p>}
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
          {(localPath ? [localPath] : options[0].localPaths)?.map((path) => <small key={path}>{path}</small>)}
          {options.some((item) => item.conflict) && <small className="mg-warning">{options.find((item) => item.conflict)?.conflict}</small>}
        </div>
      </div>;
    })}</div>}
    {choices.length > 0 && !complete && <p className="mg-warning" role="alert">{t("请为每项选择不同的仓库路径。")}</p>}
  </ManagementDrawer>;
}
