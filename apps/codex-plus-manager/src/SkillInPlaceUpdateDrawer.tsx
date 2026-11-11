import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { ManagementDrawer } from "./ManagementPages";
import { t } from "./i18n";

type Preview = {
  status: string; message: string; previewId: string; id: string; repoKey: string; repoPath: string;
  targetPath: string; currentHash: string; nextHash: string; conflicts: string[];
  managedPaths?: string[]; followedPaths?: string[]; skippedPaths?: string[];
};

export function SkillInPlaceUpdateDrawer({ id, repoKey, localPath, onClose, onExecute }: {
  id: string; repoKey: string; localPath?: string; onClose: () => void; onExecute: (previewId: string) => Promise<boolean>;
}) {
  const [revision, setRevision] = useState(0);
  const [preview, setPreview] = useState<Preview | null>(null);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  useEffect(() => {
    let active = true;
    setLoading(true); setPreview(null); setError("");
    void invoke<Preview>("preview_skill_in_place_update", { id, repoKey, localPath: localPath || null }).then((result) => {
      if (!active) return;
      if (result.status === "ok") setPreview(result);
      else setError(result.message);
    }, (reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [id, repoKey, localPath, revision]);
  const execute = async () => {
    if (!preview || preview.conflicts.length) return;
    setBusy(true);
    try {
      if (await onExecute(preview.previewId)) setDone(true);
      else setRevision((old) => old + 1);
    } finally { setBusy(false); }
  };
  return <ManagementDrawer title={t("预览原位更新")} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}
    footer={<>
      {!done && <button type="button" className="cx-secondary" disabled={busy || loading} onClick={() => setRevision((old) => old + 1)}><RefreshCw size={14}/>{t("重新检查")}</button>}
      <button type="button" className="cx-secondary" disabled={busy} onClick={onClose}>{t("关闭")}</button>
      {!done && <button type="button" className="cx-primary" disabled={busy || loading || !preview || !!preview.conflicts.length} onClick={() => void execute()}>{t("确认接管并更新")}</button>}
    </>}>
    {loading && <p className="mg-muted" role="status">{t("正在检查更新…")}</p>}
    {error && <p className="mg-warning" role="alert">{error}</p>}
    {done && <p className="mg-notice" role="status">{t("原位更新已完成，原版本已保存到备份。")}</p>}
    {preview && <><dl className="mg-detail"><dt>{t("仓库")}</dt><dd>{preview.repoKey}</dd><dt>{t("仓库路径")}</dt><dd>{preview.repoPath || "/"}</dd><dt>{t("安装位置")}</dt><dd>{preview.targetPath}</dd><dt>{t("当前版本")}</dt><dd>{preview.currentHash || "—"}</dd><dt>{t("待更新版本")}</dt><dd>{preview.nextHash || "—"}</dd></dl>
      {!!preview.managedPaths?.length && <div><strong>{t("将同步的受控副本")}</strong><ul>{preview.managedPaths.map((path) => <li key={path}>{path}</li>)}</ul></div>}
      {!!preview.followedPaths?.length && <div><strong>{t("将随原件或受控副本更新的路径")}</strong><ul>{preview.followedPaths.map((path) => <li key={path}>{path}</li>)}</ul></div>}
      {!!preview.skippedPaths?.length && <div><strong>{t("独立或归属未确认的安装，保持不变")}</strong><ul>{preview.skippedPaths.map((path) => <li key={path}>{path}</li>)}</ul></div>}
      {preview.conflicts.length > 0 && <div className="mg-warning" role="alert"><strong>{t("存在冲突，无法更新")}</strong><ul>{preview.conflicts.map((conflict) => <li key={conflict}>{conflict}</li>)}</ul></div>}
    </>}
  </ManagementDrawer>;
}
