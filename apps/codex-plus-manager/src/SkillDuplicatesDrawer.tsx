import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { ManagementDrawer } from "./ManagementPages";
import { appConfirm } from "./app-dialog";
import { t } from "./i18n";
import "./SkillSourceMatchDrawer.css";

type Duplicate = { id: string; sharedPath: string; codexPath: string; status: "identical-managed" | "different" | "unmanaged" | "unsupported" | "enabled-only-copy"; detail?: string | null };
type Preview = { status: string; message: string; previewId: string; candidates: Duplicate[] };
const statusLabel = (status: Duplicate["status"]) => ({
  "identical-managed": t("内容一致，可归档受控副本"),
  different: t("内容不同，保留两份"),
  unmanaged: t("归属未确认，保留两份"),
  unsupported: t("无法比较，保留两份"),
  "enabled-only-copy": t("共享位置已停用，保留当前启用的副本"),
})[status];

export function SkillDuplicatesDrawer({ onClose, onArchive }: {
  onClose: () => void;
  onArchive: (previewId: string, id: string) => Promise<boolean>;
}) {
  const [preview, setPreview] = useState<Preview | null>(null);
  const [revision, setRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    setLoading(true); setPreview(null); setError("");
    void invoke<Preview>("preview_skill_duplicates").then((result) => {
      if (!active) return;
      if (result.status === "ok") setPreview(result);
      else setError(result.message);
    }, (reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, [revision]);
  const archive = async (item: Duplicate) => {
    if (!preview || item.status !== "identical-managed") return;
    if (!(await appConfirm(`${t("将受控副本移入备份？")}\n${item.codexPath}`, { title: t("归档重复副本"), confirmText: t("归档") }))) return;
    setBusy(true);
    try { if (await onArchive(preview.previewId, item.id)) setRevision((old) => old + 1); }
    finally { setBusy(false); }
  };
  return <ManagementDrawer title={t("重复副本预览")} dismissOnBackdrop={false} onClose={busy ? () => {} : onClose}
    footer={<><button type="button" className="cx-secondary" disabled={busy || loading} onClick={() => setRevision((old) => old + 1)}><RefreshCw size={14}/>{t("重新检查")}</button><button type="button" className="cx-secondary" disabled={busy} onClick={onClose}>{t("关闭")}</button></>}>
    {loading && <p className="mg-muted" role="status">{t("正在检查…")}</p>}
    {error && <p className="mg-warning" role="alert">{error}</p>}
    {preview && !preview.candidates.length && <p className="mg-muted">{t("没有发现可比较的重复副本。")}</p>}
    {preview?.candidates.map((item) => <section key={item.id} className="mg-source-row">
      <div><strong>{item.id}</strong><small>{statusLabel(item.status)}</small>{item.detail && <small>{item.detail}</small>}</div>
      <div><dl className="mg-detail"><dt>{t("共享位置")}</dt><dd>{item.sharedPath}</dd><dt>Codex</dt><dd>{item.codexPath}</dd></dl>
        {item.status === "identical-managed" && <button type="button" className="cx-secondary" disabled={busy} onClick={() => void archive(item)}>{t("归档副本")}</button>}
      </div>
    </section>)}
  </ManagementDrawer>;
}
