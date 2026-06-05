import { useTranslation } from "react-i18next";
import { Actions } from "./actions";
import { DEFAULT_CODEX_AUTH_PATH, BackendSettings, RelayProfile } from "./model";
import { SyncedTextarea } from "./SyncedTextarea";
import { contextEntriesForProfile, deriveRelayProfileFromFiles, effectiveRelayConfigPreview, joinTomlSectionsRootFirst, relayCombinedCommonConfig, splitContextConfigText, stripCommonConfigTextFallback, stripContextEntriesFromConfig, syncLegacyRelayFields } from "./utils";
import { Button } from "../ui/button";
import { Download } from "lucide-react";
export function RelayFileEditors({ contextProfile, profile, form, isActive, profileId, onFormChange, onProfileChange, actions, }: {
    contextProfile: RelayProfile;
    profile: RelayProfile;
    form: BackendSettings;
    isActive: boolean;
    profileId: string;
    onFormChange: (value: BackendSettings) => void;
    onProfileChange: (value: RelayProfile) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const configPreview = effectiveRelayConfigPreview(profile, form, contextProfile);
    const entries = contextEntriesForProfile(form, contextProfile);
    return (<div className="relay-file-grid">
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>{t("configPreview.title")}</strong>
            <span>{isActive ? t("configPreview.activeDesc") : t("configPreview.inactiveDesc")}</span>
          </div>
        </div>
        <SyncedTextarea className="relay-file-textarea" value={configPreview} onValueChange={(value) => {
            const withoutCommon = stripCommonConfigTextFallback(value, relayCombinedCommonConfig(form));
            const configContents = stripContextEntriesFromConfig(withoutCommon, entries);
            onProfileChange(deriveRelayProfileFromFiles({
                ...profile,
                configContents,
            }));
        }}/>
      </div>
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>{t("commonConfig.title")}</strong>
            <span>{t("commonConfig.detail")}</span>
          </div>
          <Button onClick={async () => {
            const extracted = await actions.extractRelayCommonConfig(profile.configContents || "");
            if (!extracted)
                return;
            const split = splitContextConfigText(extracted.commonConfigContents || "");
            if (!split.common.trim() && !split.context.trim()) {
                await actions.showMessage(t("commonConfig.title"), t("commonConfig.noExtract"), "failed");
                return;
            }
            const promotedProfile = {
                ...profile,
                configContents: extracted.profileConfigContents,
            };
            const next = syncLegacyRelayFields({
                ...form,
                relayCommonConfigContents: split.common,
                relayContextConfigContents: joinTomlSectionsRootFirst([form.relayContextConfigContents || "", split.context]),
                relayProfiles: form.relayProfiles.map((item) => (item.id === profileId ? promotedProfile : item)),
            });
            onFormChange(next);
            onProfileChange(promotedProfile);
            await actions.saveSettingsValue(next, false);
        }} size="sm" type="button" variant="secondary">
            <Download className="h-4 w-4"/>
            {t("commonConfig.extract")}
          </Button>
        </div>
        <SyncedTextarea className="relay-file-textarea" value={form.relayCommonConfigContents} onValueChange={(value) => onFormChange({ ...form, relayCommonConfigContents: value })}/>
      </div>
      <div className="relay-file-panel">
        <div className="relay-file-head">
          <div>
            <strong>{t("authJson.title")}</strong>
            <span>{isActive ? t("authJson.activeDesc", { authPath: DEFAULT_CODEX_AUTH_PATH }) : t("authJson.inactiveDesc", { authPath: DEFAULT_CODEX_AUTH_PATH })}</span>
          </div>
        </div>
        <SyncedTextarea className="relay-file-textarea" value={profile.authContents} onValueChange={(value) => onProfileChange(deriveRelayProfileFromFiles({ ...profile, authContents: value }))}/>
      </div>
    </div>);
}
