import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { ManagementDrawer } from "./ManagementPages";
import { t, tf } from "./i18n";
import "./SkillSourceMatchDrawer.css";

/** 目录重映射：旧目录下的全部会话搬到新目录；批量移动：选中的会话改到指定目录。 */
export type SessionRelocationRequest =
  | { kind: "remap"; from: string; to: string }
  | { kind: "move"; threadIds: string[]; to: string };
type Item = { threadId: string; title: string; oldCwd: string; newCwd: string; archived: boolean };
type Preview = {
  status: string; message: string; items: Item[]; projectRoots: Array<{ oldPath: string; newPath: string }>;
  targetProject?: string | null; skipped: string[]; targetExists: boolean; codexRunning: boolean;
};
type Result = { status: string; message: string };

/**
 * 会话目录重定位的预览与确认（#1085）。打开时先向后端要一份预览：哪些会话会改、旧新目录各是什么；
 * 确认后才执行。Codex 在运行或目标文件夹不存在时不能执行。
 */
export function SessionRelocateDrawer({ request, onClose, onDone }: {
  request: SessionRelocationRequest;
  onClose: () => void;
  onDone: (message: string, ok: boolean) => void;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [revision, setRevision] = useState(0);
  useEffect(() => {
    let active = true;
    setLoading(true); setPreview(null); setError("");
    void invoke<Preview>("preview_session_relocation", { request }).then((result) => {
      if (!active) return;
      if (result.status === "ok") setPreview(result);
      else setError(result.message);
    }, (reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [request, revision]);
  const apply = async () => {
    setBusy(true);
    try {
      const result = await invoke<Result>("apply_session_relocation", { request });
      onDone(result.message, result.status === "ok");
    } catch (reason) {
      setError(String(reason));
    } finally {
      setBusy(false);
    }
  };
  const count = preview?.items.length ?? 0;
  const blocked = !preview || preview.codexRunning || !preview.targetExists || (!count && !preview.projectRoots.length);
  return <ManagementDrawer title={request.kind === "remap" ? t("重新定位项目目录") : t("移动会话")} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}
    footer={<>
      <button type="button" className="cx-secondary" disabled={busy || loading} onClick={() => setRevision((old) => old + 1)}><RefreshCw size={14}/>{t("重新检查")}</button>
      <button type="button" className="cx-secondary" disabled={busy} onClick={onClose}>{t("取消")}</button>
      <button type="button" className="cx-primary" disabled={busy || loading || blocked} onClick={() => void apply()}>{busy ? t("正在移动…") : tf("确认移动 {0} 个会话", [count])}</button>
    </>}>
    <dl className="mg-detail">
      {request.kind === "remap" ? <><dt>{t("旧目录")}</dt><dd>{request.from}</dd></> : null}
      <dt>{t("新目录")}</dt><dd>{request.to}</dd>
    </dl>
    <p className="mg-muted">{t("会同时更新会话索引、侧边栏记录和会话文件里记录的工作目录，恢复会话时使用新目录；对话内容不变。改动前的记录会先备份。")}</p>
    {loading && <p className="mg-muted" role="status">{t("正在检查…")}</p>}
    {error && <p className="mg-warning" role="alert">{error}</p>}
    {preview?.codexRunning && <p className="mg-warning" role="alert">{t("Codex 正在运行。请先完全退出 Codex，再点「重新检查」。")}</p>}
    {preview && !preview.targetExists && <p className="mg-warning" role="alert">{t("新目录在这台电脑上不存在，请确认文件夹已经移到这里。")}</p>}
    {preview?.targetProject && <p className="mg-muted">{tf("新目录是 Codex 项目「{0}」的文件夹，这些会话会归到这个项目下。", [preview.targetProject])}</p>}
    {preview?.projectRoots.length ? <p className="mg-muted">{t("Codex 里这个项目的文件夹位置也会一起改到新目录。")}</p> : null}
    {preview && !count && <p className="mg-muted" role="status">{request.kind === "remap" ? t("旧目录下没有会话。") : t("没有需要移动的会话。")}</p>}
    {count ? <div className="mg-source-list mg-bundle-list">
      {preview!.items.map((item) => <div className="mg-source-row mg-bundle-row" key={item.threadId}>
        <div><strong>{item.title || t("未命名会话")}</strong>{item.archived ? <small>{t("已归档")}</small> : null}<small>{item.oldCwd} → {item.newCwd}</small></div>
      </div>)}
    </div> : null}
    {preview?.skipped.length ? <details className="mg-version-section">
      <summary>{tf("{0} 个会话不处理", [preview.skipped.length])}</summary>
      <ul className="mg-muted">{preview.skipped.map((reason) => <li key={reason}>{reason}</li>)}</ul>
    </details> : null}
  </ManagementDrawer>;
}
