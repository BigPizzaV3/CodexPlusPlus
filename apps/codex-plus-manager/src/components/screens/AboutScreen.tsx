import { ExternalLink, MessageCircle } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { Textarea } from "@/components/ui/textarea";
import { APP_LINKS, PROJECT_REPO_DISPLAY, type DiagnosticsResult, type LogsResult, type OverviewResult, type UpdateResult } from "./model";
import { splitLogLines } from "./utils";
import { Actions, CardHead, Metric, Panel, Toolbar } from "./shared";
export function AboutScreen({ overview, update, logs, diagnostics, actions, }: {
    overview: OverviewResult | null;
    update: UpdateResult | null;
    logs: LogsResult | null;
    diagnostics: DiagnosticsResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    return (<>
      <Panel>
        <CardHead title={t("about.title")} detail={t("about.detail")}/>
        <CardContent>
          <div className="metric-list">
            <Metric label={t("about.version")} value={overview?.current_version ?? update?.currentVersion ?? "-"}/>
            <Metric label={t("about.codexVersion")} value={overview?.codex_version ?? t("about.notDetected")}/>
            <Metric label={t("about.projectUrl")} value={PROJECT_REPO_DISPLAY}/>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.openExternalUrl(APP_LINKS.projectRepo)} variant="secondary">
              <ExternalLink className="h-4 w-4"/>
              {t("about.openProject")}
            </Button>
            <Button onClick={() => void actions.openExternalUrl(APP_LINKS.projectIssues)} variant="secondary">
              <ExternalLink className="h-4 w-4"/>
              {t("about.feedback")}
            </Button>
            <Button onClick={() => void actions.openExternalUrl(APP_LINKS.discord)} variant="secondary">
              <MessageCircle className="h-4 w-4"/>
              {t("about.discord")}
            </Button>
            <Button onClick={() => void actions.openExternalUrl(APP_LINKS.telegram)} variant="secondary">
              <MessageCircle className="h-4 w-4"/>
              {t("about.telegram")}
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("about.updateTitle")} detail={t("about.updateDetail", { version: overview?.current_version ?? update?.currentVersion ?? "-" })}/>
        <CardContent>
          <div className="metric-list">
            <Metric label={t("about.status")} value={update?.status ?? t("about.notChecked")}/>
            <Metric label={t("about.latestVersion")} value={update?.latestVersion ?? t("about.notDetected")}/>
            <Metric label={t("about.asset")} value={update?.assetName ?? "-"}/>
            <Metric label={t("about.progress")} value={`${update?.progress ?? 0}%`}/>
          </div>
          <Textarea className="log-view" readOnly value={update?.releaseSummary || update?.message || t("about.updateNotChecked")}/>
          <Toolbar>
            <Button onClick={() => void actions.checkUpdate()}>{t("about.checkUpdate")}</Button>
            <Button variant="secondary" onClick={() => void actions.performUpdate()}>{t("about.downloadInstall")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <LogsPanel logs={logs} actions={actions}/>
      <DiagnosticsPanel diagnostics={diagnostics} actions={actions}/>
    </>);
}
export function LogsPanel({ logs, actions }: {
    logs: LogsResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const lines = splitLogLines(logs?.text ?? "");
    return (<Panel>
      <CardHead title={t("about.recentLogs")} detail={logs?.path ?? ""}/>
      <CardContent>
        <div className="log-lines">
          {lines.length ? (lines.map((line, index) => (<div className="log-line" key={`${index}-${line.slice(0, 12)}`}>
                <span>{index + 1}</span>
                <code>{line || " "}</code>
              </div>))) : (<div className="empty">{t("about.noLogs")}</div>)}
        </div>
        <Toolbar>
          <Button onClick={() => void actions.refreshLogs()}>{t("about.refreshLogs")}</Button>
          <Button variant="secondary" onClick={() => void actions.copyLogs()}>
            {t("about.copyLogs")}
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>);
}
export function DiagnosticsPanel({ diagnostics, actions }: {
    diagnostics: DiagnosticsResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    return (<Panel>
      <CardHead title={t("about.diagnostics")} detail={t("about.diagnosticsDetail")}/>
      <CardContent>
        <Textarea className="log-view tall" readOnly value={diagnostics?.report ?? t("about.noDiagnostics")}/>
        <Toolbar>
          <Button onClick={() => void actions.refreshDiagnostics()}>{t("about.regenerate")}</Button>
          <Button variant="secondary" onClick={() => void actions.copyDiagnostics()}>
            {t("about.copyReport")}
          </Button>
        </Toolbar>
      </CardContent>
    </Panel>);
}
