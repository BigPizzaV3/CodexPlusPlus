import { Info, RefreshCw, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { DEFAULT_CODEX_SESSIONS_DB_PATH, providerSyncTargetLabel, type BackendSettings, type LocalSessionsResult, type ProviderSyncProgress, type ProviderSyncTargetsResult, type SettingsResult } from "./model";
import { formatTime } from "./utils";
import { Actions, Badge, CardHead, Field, Metric, Panel, Toolbar } from "./shared";
export function SessionsScreen({ settings, form, sessions, providerSyncProgress, providerSyncTargets, selectedProviderSyncTarget, onFormChange, actions, }: {
    settings: SettingsResult | null;
    form: BackendSettings;
    sessions: LocalSessionsResult | null;
    providerSyncProgress: ProviderSyncProgress;
    providerSyncTargets: ProviderSyncTargetsResult | null;
    selectedProviderSyncTarget: string;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const items = sessions?.sessions ?? [];
    const activeCount = items.filter((item) => !item.archived).length;
    const archivedCount = items.length - activeCount;
    return (<>
      <Panel>
        <CardHead title={t("sessions.title")} detail={t("sessions.detail", { count: items.length, fixSize: 0, dbPath: DEFAULT_CODEX_SESSIONS_DB_PATH })}/>
        <CardContent>
          <div className="metric-list">
            <Metric label={t("sessions.total")} value={`${items.length}`}/>
            <Metric label={t("sessions.active")} value={`${activeCount}`}/>
            <Metric label={t("sessions.archived")} value={`${archivedCount}`}/>
            <Metric label={t("sessions.database")} value={sessions?.dbPath ?? DEFAULT_CODEX_SESSIONS_DB_PATH}/>
          </div>
          <div className="form-row">
            <Field label={t("sessions.syncTarget")}>
              <select className="select-input" disabled={providerSyncProgress.active || !(providerSyncTargets?.targets ?? []).length} value={selectedProviderSyncTarget} onChange={(event) => actions.setProviderSyncTarget(event.currentTarget.value)}>
                {(providerSyncTargets?.targets ?? []).map((target) => (<option key={target.id} value={target.id}>
                    {target.id}（{providerSyncTargetLabel(target)}）
                  </option>))}
                {!(providerSyncTargets?.targets ?? []).length ? <option value="">{t("sessions.syncTarget")}</option> : null}
              </select>
            </Field>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshLocalSessions()}>
              <RefreshCw className="h-4 w-4"/>
              {t("sessions.refresh")}
            </Button>
            <Button disabled={providerSyncProgress.active} onClick={() => void actions.syncProvidersNow()} variant="outline">
              <RefreshCw className="h-4 w-4"/>
              {providerSyncProgress.active ? t("sessions.fixing") : t("sessions.fixNow")}
            </Button>
          </Toolbar>
          <div className="provider-sync-progress" data-active={providerSyncProgress.active}>
            <div className="provider-sync-progress-head">
              <strong>{providerSyncProgress.active ? t("sessions.syncProgress") : t("sessions.syncProgressIdle")}</strong>
              <span>{providerSyncProgress.percent}%</span>
            </div>
            <div aria-valuemax={100} aria-valuemin={0} aria-valuenow={providerSyncProgress.percent} className="provider-sync-progress-bar" role="progressbar">
              <div className="provider-sync-progress-fill" style={{ width: `${providerSyncProgress.percent}%` }}/>
            </div>
            <small>{providerSyncProgress.message}</small>
          </div>
          <div className="hint-line">
            <Info className="h-4 w-4"/>
            <span>{t("sessions.deleteHint")}</span>
          </div>
          <label className="switch-row">
            <input checked={form.providerSyncEnabled} onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })} type="checkbox"/>
            <span>
              <strong>{t("sessions.autoFix")}</strong>
              <small>{t("sessions.autoFixDetail")}</small>
            </span>
          </label>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>{t("sessions.saveAutoFix")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("sessions.localSessions")} detail={items.length ? t("sessions.localSessionsDetail") : t("sessions.noSessions", { dbPath: DEFAULT_CODEX_SESSIONS_DB_PATH })}/>
        <CardContent>
          {items.length ? (<div className="session-list">
              {items.map((session) => (<div className="session-row" key={session.id}>
                  <div className="session-main">
                    <strong>{session.title || t("sessions.unnamed")}</strong>
                    <span>{session.id}</span>
                    <small>{session.cwd || t("sessions.noProject")}</small>
                  </div>
                  <div className="session-meta">
                    <Badge status={session.archived ? "archived" : "ok"}/>
                    <span>{session.modelProvider || t("sessions.noProvider")}</span>
                    <span>{formatTime(session.updatedAtMs ?? 0)}</span>
                  </div>
                  <Button variant="outline" onClick={() => void actions.deleteLocalSession(session)}>
                    <Trash2 className="h-4 w-4"/>
                    {t("sessions.delete")}
                  </Button>
                </div>))}
            </div>) : (<div className="empty">{t("sessions.noSessions", { dbPath: DEFAULT_CODEX_SESSIONS_DB_PATH })}</div>)}
        </CardContent>
      </Panel>
    </>);
}
