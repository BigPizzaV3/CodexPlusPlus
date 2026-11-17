import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { FolderOpen, Trash2 } from "lucide-react";
import { open } from "@tauri-apps/plugin-dialog";
import { ManagementDrawer } from "./ManagementPages";
import { appConfirm } from "./app-dialog";
import { t } from "./i18n";

type Version = { root: string; path: string; modifiedAtMs: number; revision: string; linked: boolean; managed: boolean };
export type VersionTask = { id: string; mode: "archive" | "enable" | "resolve" | "local"; agent?: string } | { id: string; mode: "export"; backup: string };
const names: Record<string, string> = { source: "工作源", shared: "共享", codex: "Codex", claude: "Claude Code", grok: "Grok" };

export function SkillVersionPicker({ task, roots, onRun, onClose }: { task: VersionTask; roots: Record<string, string>; onRun: (action: Record<string, unknown>) => Promise<boolean>; onClose: () => void }) {
  const [versions, setVersions] = useState<Version[]>([]);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(task.mode !== "export");
  useEffect(() => {
    if (task.mode === "export") return;
    let active = true;
    invoke<Version[]>("list_skill_versions", { id: task.id }).then((v) => { if (active) setVersions(v); },
      (e) => { if (active) setError(String(e)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [task]);
  const apply = async (action: Record<string, unknown>) => {
    setBusy(true);
    try { if (await onRun(action)) onClose(); } finally { setBusy(false); }
  };
  const select = async (v: Version) => {
    if (task.mode === "local") {
      if (await appConfirm(`${t("仅保留本地版本，不再跟随 GitHub 更新？")}\n${v.path}`, {title: t("仅本地管理")})) await apply({kind: "keepLocal", id: task.id, revisions: { [v.root]: v.revision }});
      return;
    }
    const kind = task.mode === "resolve" ? "reconcile" : task.mode;
    if (kind !== "enable" && !await appConfirm(`${t(kind === "archive" ? "将此版本移入备份？" : "保留此版本？其他独立安装将移入备份并停用；同一受控源的副本会在备份后同步。" )}\n${v.path}`, { title: t("确认版本操作") })) return;
    await apply({ kind, id: task.id, root: v.root, revision: v.revision, ...(task.mode === "enable" && { agent: task.agent }),
      ...(kind === "reconcile" && { revisions: Object.fromEntries(versions.map((item) => [item.root, item.revision])) }) });
  };
  const title = t(task.mode === "export" ? "备份另存为" : task.mode === "enable" ? "选择启用的版本" : task.mode === "resolve" ? "解决版本冲突" : task.mode === "local" ? "仅本地管理" : "卸载本地版本");
  return <ManagementDrawer title={title} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}>
    <strong>{task.id}</strong>
    {error && <p role="alert">{error}</p>}{loading && <p role="status">{t("正在读取…")}</p>}
    {task.mode === "export" ? <div className="mg-version-list">{Object.entries(roots).map(([agent, directory]) => <button type="button" key={agent} disabled={busy} onClick={() => void apply({ kind: "export", backup: task.backup, directory })}><strong>{names[agent] || agent}</strong><small>{directory}</small></button>)}
      <button type="button" disabled={busy} onClick={async () => { try { const directory = await open({ directory: true, multiple: false }); if (directory) await apply({ kind: "export", backup: task.backup, directory }); } catch (e) { setError(String(e)); } }}><FolderOpen size={15}/>{t("选择其他目录")}</button></div>
    : <div className="mg-version-list">{versions.filter((v) => v.root !== "source").map((v) => <button type="button" key={v.root} disabled={busy || (task.mode === "local" || task.mode === "archive") && v.linked} onClick={() => void select(v)}>
      <strong>{t(names[v.root] || v.root)}</strong><time>{new Date(v.modifiedAtMs).toLocaleString()}</time><small>{v.path}</small>{task.mode === "archive" && <Trash2 size={14}/>}</button>)}
      {task.mode === "resolve" && <button type="button" disabled={busy || loading || !!error} onClick={async () => {
        const physical = versions.filter(v => v.root !== "source" && !v.linked && v.managed);
        if (!physical.length) { onClose(); return; }
        if (await appConfirm(t("保留各安装位置的内容，改为独立的本地版本？之后可分别关联 GitHub。"), { title })) await apply({ kind: "keepLocal", id: task.id, revisions: Object.fromEntries(physical.map(v => [v.root, v.revision])) });
      }}>{t("保留各位置的版本")}</button>}
    </div>}
  </ManagementDrawer>;
}
