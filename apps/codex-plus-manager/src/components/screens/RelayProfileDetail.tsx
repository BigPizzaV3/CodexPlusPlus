import { useTranslation } from "react-i18next";
import { Actions } from "./actions";
import { BackendSettings, RelayFilesResult, RelayProfile } from "./model";
import { RelayFileEditors } from "./RelayFileEditors";
import { RelayProfileEditor } from "./RelayProfileEditor";
import { useEffect, useState } from "react";
import { addRelayProfile, deriveRelayProfileFromFiles, effectiveRelayConfigPreview, syncLegacyRelayFields, updateRelayProfile } from "./utils";
import { Toolbar } from "./layout";
import { Button } from "../ui/button";
import { ArrowLeft, Save } from "lucide-react";
export function RelayProfileDetail({ profile, relayFiles, form, isNew = false, onBack, onFormChange, onSaved, actions, }: {
    profile: RelayProfile;
    relayFiles: RelayFilesResult | null;
    form: BackendSettings;
    isNew?: boolean;
    onBack: () => void;
    onFormChange: (value: BackendSettings, preserveLinkedProfiles?: boolean) => void | Promise<void>;
    onSaved?: () => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const [draft, setDraft] = useState<RelayProfile>(profile);
    const isActive = !isNew && profile.id === form.activeRelayId;
    useEffect(() => {
        setDraft(deriveRelayProfileFromFiles(isActive && relayFiles
            ? {
                ...profile,
                configContents: relayFiles.configContents,
                authContents: relayFiles.authContents,
            }
            : profile));
    }, [profile.id, isActive, isNew, relayFiles?.configContents, relayFiles?.authContents]);
    const saveDraft = async () => {
        const normalizedDraft = deriveRelayProfileFromFiles(draft);
        const next = isNew
            ? addRelayProfile(form, normalizedDraft)
            : updateRelayProfile(form, profile.id, normalizedDraft);
        await onFormChange(next, !!normalizedDraft.linkedCcsProviderId);
        if (isActive) {
            await actions.saveRelayFile("config", effectiveRelayConfigPreview(normalizedDraft, form, normalizedDraft), true);
            await actions.saveRelayFile("auth", normalizedDraft.authContents, true);
        }
        onSaved?.();
    };
    const switchDraft = () => {
        if (isNew || !form.relayProfilesEnabled)
            return;
        const normalizedDraft = deriveRelayProfileFromFiles(draft);
        const previousActiveRelayId = form.activeRelayId;
        const next = syncLegacyRelayFields({
            ...form,
            relayProfiles: form.relayProfiles.map((item) => (item.id === profile.id ? normalizedDraft : item)),
            activeRelayId: profile.id,
        });
        void actions.switchRelayProfile(next, previousActiveRelayId);
    };
    return (<div className="relay-detail-page" key={profile.id}>
      <div className="relay-detail-sticky">
        <Toolbar>
          <Button onClick={onBack} variant="secondary">
            <ArrowLeft className="h-4 w-4"/>
            {t("profileDetail.back")}
          </Button>
          <Button onClick={() => void saveDraft()}>
            <Save className="h-4 w-4"/>
            {t("profileDetail.save")}
          </Button>
          <Button onClick={() => void saveDraft()}>
            <Save className="h-4 w-4"/>
            {t("profileDetail.save")}
          </Button>
        </Toolbar>
      </div>
      <RelayProfileEditor profile={draft} form={form} isNew={isNew} onProfileChange={setDraft} onSwitch={switchDraft} actions={actions}/>
      <RelayFileEditors contextProfile={profile} profile={draft} form={form} isActive={isActive} profileId={profile.id} onFormChange={onFormChange} onProfileChange={setDraft} actions={actions}/>
    </div>);
}
