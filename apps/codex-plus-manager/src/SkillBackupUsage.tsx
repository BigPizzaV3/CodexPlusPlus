import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { RefreshCw } from "lucide-react";
import { t, tf } from "./i18n";

type BackupUsage = { backupCount: number; measuredCount: number; bytes: number; errors: string[] };
type CachedUsage = { usage: BackupUsage; revision: string; scannedAt: number };

let cachedUsage: CachedUsage | null = null;
let pendingUsage: Promise<CachedUsage> | null = null;
function requestBackupUsage(revision: string) {
  if (!pendingUsage) {
    pendingUsage = invoke<BackupUsage>("skill_backup_usage").then((usage) => {
      cachedUsage = { usage, revision, scannedAt: Date.now() };
      return cachedUsage;
    }, (error) => {
      if (cachedUsage) cachedUsage.scannedAt = 0;
      throw error;
    }).finally(() => { pendingUsage = null; });
  }
  return pendingUsage;
}

export function SkillBackupUsage({ backupRevision }: { backupRevision: string }) {
  const [result, setResult] = useState<CachedUsage | null>(cachedUsage);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const mounted = useRef(false);
  const pending = useRef(false);

  const load = () => {
    if (pending.current) return;
    pending.current = true;
    setLoading(true);
    setError("");
    void requestBackupUsage(backupRevision).then(
      (value) => { if (mounted.current) setResult(value); },
      (reason) => { if (mounted.current) setError(String(reason)); },
    ).finally(() => { pending.current = false; if (mounted.current) setLoading(false); });
  };
  useEffect(() => {
    mounted.current = true;
    if (pendingUsage || !cachedUsage || Date.now() - cachedUsage.scannedAt >= 60_000) load();
    return () => { mounted.current = false; };
  }, []);

  const usage = result?.usage;
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  const unit = usage?.bytes ? Math.min(Math.floor(Math.log(usage.bytes) / Math.log(1024)), units.length - 1) : 0;
  const size = usage ? `${(usage.bytes / (1024 ** unit)).toLocaleString(undefined, { maximumFractionDigits: unit ? 1 : 0 })} ${units[unit]}` : "";
  return <div>
    <div className="mg-row-actions">
      {loading && <small role="status">{t("正在统计备份…")}</small>}
      {usage && (loading || error) && <small>{t("上次统计")}：</small>}
      {usage && <small>{tf("备份数量：{0}", [usage.backupCount])} · {usage.errors.length
        ? tf("已统计 {0}/{1} 份，文件大小：{2}", [usage.measuredCount, usage.backupCount, size])
        : tf("文件大小：{0}", [size])}</small>}
      <button type="button" className="mg-icon-button" disabled={loading} onClick={load} title={t("刷新备份统计")} aria-label={t("刷新备份统计")}><RefreshCw size={14}/></button>
    </div>
    {result && result.revision !== backupRevision && <small className="mg-warning">{t("备份列表已变化，请刷新统计")}</small>}
    {error && <small className="mg-warning" role="alert">{error}</small>}
    {usage?.errors.map((item) => <small className="mg-warning" role="alert" key={item}>{item}</small>)}
  </div>;
}
