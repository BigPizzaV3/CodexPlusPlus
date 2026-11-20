/** 赞助商预设的来源：App 拉到的推荐清单写进来，两个「添加供应商」目录从这里读。规则在 presets.ts 的 sponsorPresets。 */
import { useMemo, useSyncExternalStore } from "react";
import { sponsorPresets, type ProviderPreset, type SponsorAdsPayload } from "./presets";

let adsPayload: SponsorAdsPayload = null;
const listeners = new Set<() => void>();

export function setSponsorAds(payload: SponsorAdsPayload) {
  adsPayload = payload;
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function useSponsorPresets(): ProviderPreset[] {
  const payload = useSyncExternalStore(subscribe, () => adsPayload);
  return useMemo(() => sponsorPresets(payload), [payload]);
}
