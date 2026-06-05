import { useTranslation } from "react-i18next";
import type { LaunchMode } from "./model";
import type { Actions } from "./shared";
export function ModeSelector({ launchMode, actions }: {
    launchMode: LaunchMode;
    actions: Actions;
}) {
    const { t } = useTranslation();
    return (<div className="mode-grid">
      <button className={`mode-option ${launchMode === "relay" ? "active" : ""}`} onClick={() => void actions.setLaunchMode("relay")} type="button">
        <strong>{t("modeSelector.compat")}</strong>
        <span>{t("modeSelector.compatDetail")}</span>
      </button>
      <button className={`mode-option ${launchMode === "patch" ? "active" : ""}`} onClick={() => void actions.setLaunchMode("patch")} type="button">
        <strong>{t("modeSelector.full")}</strong>
        <span>{t("modeSelector.fullDetail")}</span>
      </button>
    </div>);
}
