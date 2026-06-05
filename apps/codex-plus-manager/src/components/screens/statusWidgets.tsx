import { useTranslation } from "react-i18next";
import { Badge as UiBadge } from "@/components/ui/badge";
import { type LaunchStatus } from "./model";
import { formatTime, statusClass, statusLabel } from "./utils";
export function StatusRow({ title, status = "unknown", path }: {
    title: string;
    status?: string;
    path?: string | null;
}) {
    const { t } = useTranslation();
    return (<div className="status-row">
      <span>{title}</span>
      <Badge status={status}/>
      <code>{path || t("statusRow.noPath")}</code>
    </div>);
}
export function Badge({ status }: {
    status: string;
}) {
    return <UiBadge className={statusClass(status)} variant="secondary">{statusLabel(status)}</UiBadge>;
}
export function LatestLaunch({ status }: {
    status: LaunchStatus | null;
}) {
    const { t } = useTranslation();
    if (!status)
        return <div className="empty">{t("latestLaunch.empty")}</div>;
    return (<div className="metric-list">
      <Metric label={t("latestLaunch.status")} value={status.status}/>
      <Metric label={t("latestLaunch.message")} value={status.message}/>
      <Metric label={t("latestLaunch.debugPort")} value={String(status.debug_port ?? "-")}/>
      <Metric label={t("latestLaunch.helperPort")} value={String(status.helper_port ?? "-")}/>
      <Metric label={t("latestLaunch.time")} value={formatTime(status.started_at_ms)}/>
    </div>);
}
export function Metric({ label, value }: {
    label: string;
    value: string;
}) {
    return (<div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>);
}
