import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ManagementDrawer } from "./ManagementPages";
import { t } from "./i18n";

type ReaPreview = {
  status: string; message: string; currentVersion: string; targetVersion: string;
  nodeVersion: string; nodeEngines: string; nodeCompatible: boolean; updateAvailable: boolean;
  prefix: string; clients: string[]; clientTargets: Array<{ client: string; path: string }>;
  skillPaths: string[]; skillVersions: Array<{ client: string; path: string; version: string | null }>; blockers: string[];
};

export function ReaUpdatePreviewDrawer({ onClose }: { onClose: () => void }) {
  const [preview, setPreview] = useState<ReaPreview | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  useEffect(() => {
    let active = true;
    void invoke<ReaPreview>("preview_rea_update").then((result) => {
      if (!active) return;
      if (result.status === "ok") setPreview(result);
      else setError(result.message);
    }, (reason) => { if (active) setError(String(reason)); }).finally(() => { if (active) setLoading(false); });
    return () => { active = false; };
  }, []);
  return <ManagementDrawer title={t("REA 安装器只读检查")} dismissOnBackdrop={false} onClose={onClose}
    footer={<button type="button" className="cx-secondary" onClick={onClose}>{t("关闭")}</button>}>
    {loading && <p className="mg-muted" role="status">{t("正在检查…")}</p>}
    {error && <p className="mg-warning" role="alert">{error}</p>}
    {preview && <>
      <dl className="mg-detail">
        <dt>{t("当前引擎")}</dt><dd>{preview.currentVersion || t("未记录")}</dd>
        <dt>{t("上游引擎")}</dt><dd>{preview.targetVersion || t("未记录")}</dd>
        <dt>{t("更新状态")}</dt><dd>{preview.updateAvailable ? t("发现新版本") : t("暂无更新")}</dd>
        <dt>Node</dt><dd>{preview.nodeVersion || "—"} · {preview.nodeCompatible ? t("兼容") : t("不兼容")} {preview.nodeEngines}</dd>
        <dt>Prefix</dt><dd>{preview.prefix}</dd>
        <dt>{t("已有客户端")}</dt><dd>{preview.clientTargets.length ? preview.clientTargets.map(({ client, path }) => <div key={client}>{client} · {path}</div>) : "—"}</dd>
        <dt>{t("技能版本与位置")}</dt><dd>{preview.skillVersions.length ? preview.skillVersions.map(({ client, path, version }) => <div key={`${client}:${path}`}>{client} · {version || t("未记录")} · {path}</div>) : "—"}</dd>
      </dl>
      {preview.blockers.length > 0 && <div className="mg-warning" role="alert"><strong>{t("暂不能更新")}</strong><ul>{preview.blockers.map((blocker) => <li key={blocker}>{blocker}</li>)}</ul></div>}
    </>}
  </ManagementDrawer>;
}
