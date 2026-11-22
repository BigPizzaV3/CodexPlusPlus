import { useState } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { isTauri } from "@tauri-apps/api/core";
import { AppSelect } from "./AppSelect";
import { CxRow } from "./cx-layout";
import { t } from "./i18n";
import { loadDisplayPreferences, storeDisplayPreferences, zoomOptions, type DisplayPreferences } from "./display-preferences";

export async function applyManagerDisplay(value: DisplayPreferences) {
  if (isTauri()) await getCurrentWebview().setZoom(value.zoom / 100);
  else document.documentElement.style.zoom = String(value.zoom / 100);
  document.documentElement.dataset.managerDensity = value.density;
}

export function DisplaySettings() {
  const [value, setValue] = useState(loadDisplayPreferences);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const change = async (next: DisplayPreferences) => {
    setBusy(true);
    try {
      await applyManagerDisplay(next);
      setValue(next);
      storeDisplayPreferences(next);
      setError("");
    } catch { setError(t("无法应用显示设置，请重试。")); }
    finally { setBusy(false); }
  };
  return <>
    <CxRow title={t("管理器缩放")} detail={t("调整主窗口的文字和控件大小。")}>
      <AppSelect size="sm" ariaLabel={t("管理器缩放")} disabled={busy} value={String(value.zoom)}
        options={zoomOptions.map((zoom) => ({ value: String(zoom), label: `${zoom}%` }))}
        onChange={(zoom) => void change({ ...value, zoom: Number(zoom) })} />
    </CxRow>
    <CxRow title={t("列表密度")} detail={error || t("调整管理器列表和导航行的间距。")}>
      <AppSelect size="sm" ariaLabel={t("列表密度")} disabled={busy} value={value.density}
        options={[{ value: "compact", label: t("紧凑") }, { value: "standard", label: t("标准") }, { value: "comfortable", label: t("宽松") }]}
        onChange={(density) => void change({ ...value, density })} />
    </CxRow>
  </>;
}
