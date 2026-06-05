import { useTranslation } from "react-i18next";
import { APP_LINKS, AdsResult } from "./model";
import { isExpiredAd } from "./utils";
import { CardHead, Panel } from "./layout";
import { CardContent } from "../ui/card";
import { Button } from "../ui/button";
import { RefreshCw } from "lucide-react";
import { AdGrid } from "./AdGrid";
import { Actions } from "./actions";
export function RecommendationsScreen({ ads, actions }: {
    ads: AdsResult | null;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const items = (ads?.ads ?? []).filter((ad) => !isExpiredAd(ad));
    const sponsors = items.filter((ad) => ad.type === "sponsor");
    const normal = items.filter((ad) => ad.type === "normal");
    return (<>
      <Panel>
        <CardHead title={t("recommendations.title")} detail={t("recommendations.detail")}/>
        <CardContent>
          <div className="recommend-hero">
            <div>
              <strong>{ads ? t("recommendations.loaded", { count: items.length }) : t("recommendations.notLoaded")}</strong>
              <span>{t("recommendations.sourceDetail", { source: APP_LINKS.adListRepo })}</span>
            </div>
            <Button onClick={() => void actions.refreshAds()}>
              <RefreshCw className="h-4 w-4"/>
              {t("recommendations.refresh")}
            </Button>
          </div>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("recommendations.sponsors")} detail={`${sponsors.length}`}/>
        <CardContent>
          <AdGrid actions={actions} ads={sponsors} empty={t("recommendations.noSponsors")}/>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("recommendations.general")} detail={`${normal.length}`}/>
        <CardContent>
          <AdGrid actions={actions} ads={normal} empty={t("recommendations.noGeneral")}/>
        </CardContent>
      </Panel>
    </>);
}
