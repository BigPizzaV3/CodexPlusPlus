import { Download, ExternalLink, RefreshCw } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { SCRIPT_MARKET_REPOSITORY_URL, type ScriptMarketItem, type ScriptMarketResult, type SettingsResult } from "./model";
import { Actions, CardHead, Metric, Panel, ScriptRow, Toolbar } from "./shared";
export function UserScriptsScreen({ settings, market, actions }: {
    settings: SettingsResult | null;
    market: ScriptMarketResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const inventory = settings?.user_scripts;
    const scripts = inventory?.scripts ?? [];
    const marketScripts = market?.market.scripts ?? [];
    const installedCount = marketScripts.filter((script) => script.installed).length;
    return (<>
      <Panel>
        <CardHead title={t("scripts.marketTitle")} detail={t("scripts.marketDetail", { total: marketScripts.length, installed: installedCount, state: inventory?.enabled === false ? t("scripts.off") : t("scripts.on") })}/>
        <CardContent>
          <div className="metric-list">
            <Metric label={t("scripts.marketStatus")} value={market?.market.message ?? t("scripts.statusNotInstalled")}/>
            <Metric label={t("scripts.remoteScripts")} value={`${marketScripts.length}`}/>
            <Metric label={t("scripts.installed")} value={`${installedCount}`}/>
            <Metric label={t("scripts.localOverall")} value={inventory?.enabled === false ? t("scripts.off") : t("scripts.on")}/>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.refreshScriptMarket()}>
              <RefreshCw className="h-4 w-4"/>
              {t("scripts.refreshMarket")}
            </Button>
            <Button onClick={() => void actions.openExternalUrl(SCRIPT_MARKET_REPOSITORY_URL)} variant="secondary">
              <ExternalLink className="h-4 w-4"/>
              {t("scripts.contribute")}
            </Button>
            <Button onClick={() => void actions.refreshCurrent()} variant="secondary">
              <RefreshCw className="h-4 w-4"/>
              {t("scripts.refreshLocal")}
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("scripts.marketScripts")} detail={market?.market.updatedAt ? t("scripts.updatedAt", { time: market.market.updatedAt }) : t("scripts.loadFromGitHub")}/>
        <CardContent>
          {marketScripts.length ? (<div className="script-market-grid">
              {marketScripts.map((script) => (<MarketScriptCard key={script.id} script={script} actions={actions}/>))}
            </div>) : (<div className="empty">{t("scripts.clickRefresh")}</div>)}
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("scripts.localScripts")} detail={t("scripts.localScriptsDetail")}/>
        <CardContent>
          <div className="table">
            {scripts.length ? scripts.map((script) => <ScriptRow key={script.key} script={script} actions={actions}/>) : <div className="empty">{t("scripts.noScripts")}</div>}
          </div>
        </CardContent>
      </Panel>
    </>);
}
export function MarketScriptCard({ script, actions }: {
    script: ScriptMarketItem;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const status = script.updateAvailable ? t("marketScript.updatable") : script.installed ? t("marketScript.installed", { version: script.installedVersion }) : t("marketScript.notInstalled");
    return (<div className="script-market-card">
      <div className="script-market-title">
        <div>
          <strong>{script.name}</strong>
          <span>{script.author || t("marketScript.unknownAuthor")}</span>
        </div>
        <UiBadge variant={script.updateAvailable ? "default" : script.installed ? "secondary" : "outline"}>{status}</UiBadge>
      </div>
      <p className="script-market-description">{script.description || t("marketScript.noDescription")}</p>
      <div className="script-market-tags">
        <span className="script-market-tag">v{script.version}</span>
        {script.tags.map((tag) => (<span className="script-market-tag" key={tag}>{tag}</span>))}
      </div>
      <div className="script-market-actions">
        <Button onClick={() => void actions.installMarketScript(script.id)} size="sm">
          <Download className="h-4 w-4"/>
          {script.updateAvailable ? t("marketScript.update") : script.installed ? t("marketScript.reinstall") : t("marketScript.install")}
        </Button>
        {script.homepage ? (<Button onClick={() => void actions.openExternalUrl(script.homepage)} size="sm" variant="secondary">
            <ExternalLink className="h-4 w-4"/>
            {t("marketScript.homepage")}
          </Button>) : null}
      </div>
    </div>);
}
