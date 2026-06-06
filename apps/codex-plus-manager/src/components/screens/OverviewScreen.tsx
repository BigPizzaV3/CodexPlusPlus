import { useTranslation } from "react-i18next";
import { Actions } from "./actions";
import { OverviewResult } from "./model";
import { healthItems } from "./utils";
import { CardHead, Panel, Toolbar } from "./layout";
import { CardContent } from "../ui/card";
import { Badge, LatestLaunch } from "./statusWidgets";
import { Button } from "../ui/button";
import { Bell, CheckCircle2, RefreshCw, Rocket, Wrench } from "lucide-react";
export function OverviewScreen({ overview, actions, }: {
    overview: OverviewResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const health = healthItems(overview);
    return (<>
            <Panel>
                <CardHead title={t("overview.healthCheck")} detail={t("overview.healthDetail")}/>
                <CardContent>
                    <div className="health-grid">
                        <div className={`health-item ${overview?.codex_version ? "ok" : "needs-fix"}`}>
                            {overview?.codex_version ? <CheckCircle2 className="h-4 w-4"/> : <Bell className="h-4 w-4"/>}
                            <div>
                                <strong>{t("overview.codexVersion")}</strong>
                                <span>{overview?.codex_version ?? t("overview.codexVersionNotFound")}</span>
                            </div>
                            <Badge status={overview?.codex_version ? "ok" : "not_checked"}/>
                        </div>
                        {health.map((item) => (<div className={`health-item ${item.ok ? "ok" : "needs-fix"}`} key={item.title}>
                                {item.ok ? <CheckCircle2 className="h-4 w-4"/> : <Bell className="h-4 w-4"/>}
                                <div>
                                    <strong>{item.title}</strong>
                                    <span>{item.detail}</span>
                                </div>
                                <Badge status={item.status}/>
                            </div>))}
                    </div>
                    <Toolbar>
                        <Button onClick={() => void actions.checkHealth()}>
                            <RefreshCw className="h-4 w-4"/>
                            {t("overview.check")}
                        </Button>
                        <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>
                            <Wrench className="h-4 w-4"/>
                            {t("overview.repairEntry")}
                        </Button>
                        <Button variant="secondary" onClick={() => void actions.repairBackend()}>
                            {t("overview.repairBackend")}
                        </Button>
                    </Toolbar>
                </CardContent>
            </Panel>
            <Panel>
                <CardHead title={t("overview.recentLaunch")} detail={t("overview.recentLaunchDetail")}/>
                <CardContent>
                    <LatestLaunch status={overview?.latest_launch ?? null}/>
                    <Toolbar>
                        <Button onClick={() => void actions.launch()}>
                            <Rocket className="h-4 w-4"/>
                            {t("overview.launchCodex")}
                        </Button>
                        <Button variant="secondary" onClick={() => void actions.goLogs()}>
                            {t("overview.openAbout")}
                        </Button>
                    </Toolbar>
                </CardContent>
            </Panel>
        </>);
}
