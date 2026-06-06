import { Plus } from "lucide-react";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { CardContent } from "@/components/ui/card";
import { type BackendSettings, type RelayFilesResult, type RelayProfile, type SettingsResult } from "./model";
import { createRelayProfile, normalizeSettings } from "./utils";
import { Actions, CardHead, Panel } from "./shared";
import { RelayProfileDetail } from "./RelayProfileDetail";
import { RelayProfileList } from "./RelayProfileList";
export function RelayScreen({ settings: _settings, relayFiles, form, onFormChange, actions, }: {
    settings: SettingsResult | null;
    relayFiles: RelayFilesResult | null;
    form: BackendSettings;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const normalized = normalizeSettings(form);
    const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
    const [newProfileDraft, setNewProfileDraft] = useState<RelayProfile | null>(null);
    const detailProfile = newProfileDraft || (detailProfileId
        ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
        : null);
    const isNewProfile = !!newProfileDraft;
    const saveRelaySettings = async (next: BackendSettings, preserveLinkedProfiles = false) => {
        onFormChange(next);
        await actions.saveSettingsValue(next, true, preserveLinkedProfiles);
    };
    const editRelayProfile = async (profileId: string) => {
        let nextSettings = normalized;
        const profile = normalized.relayProfiles.find((item) => item.id === profileId);
        if (profile?.linkedCcsProviderId && normalized.ccsLinkEnabled) {
            const refreshed = await actions.refreshSettings(true);
            if (refreshed)
                nextSettings = normalizeSettings(refreshed);
        }
        setNewProfileDraft(null);
        setDetailProfileId(nextSettings.relayProfiles.some((item) => item.id === profileId) ? profileId : null);
    };
    useEffect(() => {
        if (!newProfileDraft && detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
            setDetailProfileId(null);
        }
    }, [detailProfileId, newProfileDraft, normalized.relayProfiles]);
    useEffect(() => {
        if (!newProfileDraft && detailProfileId === normalized.activeRelayId) {
            void actions.refreshRelayFiles();
        }
    }, [detailProfileId, newProfileDraft, normalized.activeRelayId]);
    if (detailProfile) {
        return (<RelayProfileDetail profile={detailProfile} relayFiles={!isNewProfile && detailProfile.id === normalized.activeRelayId ? relayFiles : null} form={normalized} isNew={isNewProfile} onBack={() => {
                setNewProfileDraft(null);
                setDetailProfileId(null);
            }} onFormChange={saveRelaySettings} onSaved={() => {
                setNewProfileDraft(null);
                setDetailProfileId(null);
            }} actions={actions}/>);
    }
    return (<>
      <Panel>
        <CardHead title={t("relayScreen.title")} detail={t("relayScreen.detail", { count: normalized.relayProfiles.length })}/>
        <CardContent>
          <label className="switch-row relay-master-switch">
            <input checked={normalized.relayProfilesEnabled} onChange={(event) => {
            const next = { ...normalized, relayProfilesEnabled: event.currentTarget.checked };
            void saveRelaySettings(next);
        }} type="checkbox"/>
            <span>
              <strong>{t("relayScreen.enableSwitch")}</strong>
              <small>{t("relayScreen.enableSwitchDetail")}</small>
            </span>
          </label>
          <label className="switch-row relay-link-switch">
            <input checked={normalized.ccsLinkEnabled} onChange={(event) => {
            if (event.currentTarget.checked) {
                void actions.importCcsProviders();
                return;
            }
            const next = { ...normalized, ccsLinkEnabled: false };
            void saveRelaySettings(next);
        }} type="checkbox"/>
            <span>
              <strong>{t("relayScreen.linkSwitch")}</strong>
              <small>{t("relayScreen.linkSwitchDetail")}</small>
            </span>
          </label>
          <div className="relay-add-row">
            <Button variant="secondary" onClick={() => {
            setNewProfileDraft(createRelayProfile(normalized));
            setDetailProfileId(null);
        }}>
              <Plus className="h-4 w-4"/>
              {t("relayScreen.addProvider")}
            </Button>
          </div>
          <RelayProfileList form={normalized} onEdit={(profileId) => void editRelayProfile(profileId)} onFormChange={saveRelaySettings} disabled={!normalized.relayProfilesEnabled} actions={actions}/>
        </CardContent>
      </Panel>
    </>);
}
