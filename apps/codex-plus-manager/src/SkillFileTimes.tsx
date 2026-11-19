import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ChevronLeft, ChevronRight, RefreshCw } from "lucide-react";
import { t } from "./i18n";

type SkillFileTime = { path: string; modifiedAtMs: number; isLink: boolean };

export function SkillFileTimes({ id, rootKind = "", backupId }: { id: string; rootKind?: string; backupId?: string }) {
  const [files, setFiles] = useState<SkillFileTime[] | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(false);
  const [page, setPage] = useState(0);
  const pending = useRef(false);
  const mounted = useRef(false);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  const load = () => {
    if (pending.current) return;
    pending.current = true;
    setLoading(true);
    setError("");
    void invoke<SkillFileTime[]>("list_skill_file_times", { id, rootKind, backupId: backupId || null })
      .then((result) => { if (mounted.current) { setFiles(result); setPage(0); } },
        (reason) => { if (mounted.current) setError(String(reason)); })
      .finally(() => { pending.current = false; if (mounted.current) setLoading(false); });
  };
  return <details className="mg-skill-file-times" onToggle={(event) => { if (event.currentTarget.open && !files) load(); }}>
    <summary>{t("文件修改时间")}</summary>
    {loading && <small role="status">{t("正在读取…")}</small>}
    {error && <small className="mg-warning" role="alert">{error}</small>}
    {(files || error) && <button type="button" className="mg-icon-button" onClick={load} disabled={loading} title={t("刷新")} aria-label={t("刷新")}><RefreshCw size={14}/></button>}
    {files && <><ul>{files.slice(page * 100, (page + 1) * 100).map((file) => <li key={file.path}><span>{file.path}{file.isLink && <> · {t("链接（未展开）")}</>}</span><time dateTime={new Date(file.modifiedAtMs).toISOString()}>{new Date(file.modifiedAtMs).toLocaleString()}</time></li>)}</ul>
      {files.length > 100 && <div className="mg-row-actions">
        <button type="button" className="mg-icon-button" disabled={page === 0} onClick={() => setPage((current) => current - 1)} title={t("上一页")} aria-label={t("上一页")}><ChevronLeft size={14}/></button>
        <small>{page + 1} / {Math.ceil(files.length / 100)}</small>
        <button type="button" className="mg-icon-button" disabled={(page + 1) * 100 >= files.length} onClick={() => setPage((current) => current + 1)} title={t("下一页")} aria-label={t("下一页")}><ChevronRight size={14}/></button>
      </div>}</>}
  </details>;
}
