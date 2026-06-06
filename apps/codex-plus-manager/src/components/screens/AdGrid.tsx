import { useTranslation } from "react-i18next";
import type { Actions } from "./actions";
import { AdItem } from "./model";
import { ExternalLink } from "lucide-react";
export function AdGrid({ ads, empty, actions }: {
    ads: AdItem[];
    empty: string;
    actions: Actions;
}) {
    const { t } = useTranslation();
    if (!ads.length)
        return <div className="empty">{empty}</div>;
    return (<div className="ad-grid">
      {ads.map((ad) => (<button className="ad-card" key={ad.id || `${ad.type}-${ad.title}`} onClick={() => void actions.openExternalUrl(ad.url)} type="button">
          <div>
            <strong>{ad.title}</strong>
            <p>{ad.description}</p>
          </div>
          {ad.highlights?.length ? (<div className="ad-tags">
              {ad.highlights.map((item) => (<span key={item}>{item}</span>))}
            </div>) : null}
          <span className="ad-link">
            {t("adGrid.open")}
            <ExternalLink className="h-4 w-4"/>
          </span>
        </button>))}
    </div>);
}
