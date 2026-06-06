import { Download, Link2, Settings, MessageCircle, ShieldCheck } from "lucide-react";
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { PROTOCOL_PROXY_ENDPOINT, defaultSettings, type BackendSettings, type RelayMode, type RelayProfile } from "./model";
import { applyRelayProfilePatchToFiles, configHasCodexGoalsFeature, relayProfileEditorStatus, relayProfileModeHelp, setCodexGoalsFeatureInConfig } from "./utils";
import { Actions, Field } from "./shared";
export function RelayProfileEditor({ profile, form, isNew = false, onProfileChange, onSwitch, actions, }: {
    profile: RelayProfile;
    form: BackendSettings;
    isNew?: boolean;
    onProfileChange: (value: RelayProfile) => void;
    onSwitch: () => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const showApiFields = profile.relayMode !== "official" || profile.officialMixApiKey;
    const [showAdvanced, setShowAdvanced] = useState(false);
    const updateDraft = (patch: Partial<RelayProfile>) => {
        onProfileChange(applyRelayProfilePatchToFiles(profile, patch, { allowGenerateFiles: isNew }));
    };
    return (<div className="relay-profile-editor">
      <div className="relay-editor-head">
        <div>
          <strong>{profile.name || t("profileEditor.unnamed")}</strong>
          <span>{relayProfileEditorStatus(profile, form, isNew)}</span>
        </div>
        {isNew ? null : (<Button disabled={!form.relayProfilesEnabled} onClick={onSwitch} title={!form.relayProfilesEnabled ? t("profileEditor.switchDisabled") : undefined} variant={profile.id === form.activeRelayId ? "secondary" : "default"}>
            {profile.id === form.activeRelayId ? t("profileEditor.inUse") : t("profileEditor.setCurrent")}
          </Button>)}
      </div>
      <div className="relay-fields">
        <Field className="relay-field-name" label={t("profileEditor.name")}>
          <Input value={profile.name} onChange={(event) => updateDraft({ name: event.currentTarget.value })}/>
        </Field>
        <Field className="relay-field-mode" label={t("profileEditor.mode")}>
          <select className="field-select" value={profile.relayMode} onChange={(event) => {
            const relayMode = event.currentTarget.value as RelayMode;
            updateDraft(relayMode === "official" ? { relayMode, officialMixApiKey: false } : { relayMode });
        }}>
            <option value="official">{t("profileEditor.official")}</option>
            <option value="pureApi">{t("profileEditor.pureApi")}</option>
          </select>
        </Field>
        <Field className="relay-field-config-model" label={t("profileEditor.configModel")}>
          <Input value={profile.model} onChange={(event) => updateDraft({ model: event.currentTarget.value })} placeholder={t("profileEditor.configModelPlaceholder")}/>
        </Field>
        <Field className="relay-field-goals" label={t("profileEditor.codexGoals")}>
          <label className="inline-check">
            <input checked={configHasCodexGoalsFeature(profile.configContents)} onChange={(event) => updateDraft({
            configContents: setCodexGoalsFeatureInConfig(profile.configContents, event.currentTarget.checked),
        })} type="checkbox"/>
            <span>{t("profileEditor.enableGoals")}</span>
          </label>
        </Field>
        <div className="relay-advanced-toggle">
          <Button aria-expanded={showAdvanced} onClick={() => setShowAdvanced((current) => !current)} size="sm" type="button" variant="secondary">
            <Settings className="h-4 w-4"/>
            {t("profileEditor.moreOptions")}
          </Button>
        </div>
        {showAdvanced ? (<div className="relay-advanced-fields">
            <Field className="relay-field-test-model" label={t("profileEditor.testModel")}>
              <Input value={profile.testModel} onChange={(event) => updateDraft({ testModel: event.currentTarget.value })} placeholder={t("profileEditor.testModelPlaceholder", { model: form.relayTestModel || defaultSettings.relayTestModel })}/>
            </Field>
            <Field className="relay-field-context-window" label={t("profileEditor.contextSize")}>
              <Input inputMode="numeric" value={profile.contextWindow} onChange={(event) => updateDraft({ contextWindow: event.currentTarget.value.replace(/[^\d]/g, "") })} placeholder={t("profileEditor.contextSizePlaceholder")}/>
            </Field>
            <Field className="relay-field-auto-compact" label={t("profileEditor.compressContext")}>
              <Input inputMode="numeric" value={profile.autoCompactLimit} onChange={(event) => updateDraft({ autoCompactLimit: event.currentTarget.value.replace(/[^\d]/g, "") })} placeholder={t("profileEditor.compressContextPlaceholder")}/>
            </Field>
          </div>) : null}
        {profile.relayMode === "official" ? (<Field className="relay-field-official-key" label={t("profileEditor.apiKey")}>
            <label className="inline-check">
              <input checked={profile.officialMixApiKey} onChange={(event) => updateDraft({ officialMixApiKey: event.currentTarget.checked })} type="checkbox"/>
              <span>{t("profileEditor.mixApiKey")}</span>
            </label>
          </Field>) : null}
        {showApiFields ? (<div className="relay-api-fields">
            <Field className="relay-field-base-url" label={t("profileEditor.baseUrl")}>
              <Input value={profile.baseUrl} onChange={(event) => updateDraft({ baseUrl: event.currentTarget.value })} placeholder={t("profileEditor.baseUrlPlaceholder")}/>
            </Field>
            <Field className="relay-field-key" label={t("profileEditor.key")}>
              <Input type="password" value={profile.apiKey} onChange={(event) => updateDraft({ apiKey: event.currentTarget.value })} placeholder={t("profileEditor.keyPlaceholder")}/>
            </Field>
            <Field className="relay-field-protocol" label={t("profileEditor.upstreamProtocol")}>
              <div className="protocol-options">
                <button className={`protocol-option ${profile.protocol === "responses" ? "active" : ""}`} onClick={() => updateDraft({ protocol: "responses" })} type="button">
                  Responses API
                </button>
                <button className={`protocol-option ${profile.protocol === "chatCompletions" ? "active" : ""}`} onClick={() => updateDraft({ protocol: "chatCompletions" })} type="button">
                  Chat Completions
                </button>
              </div>
            </Field>
          </div>) : null}
        {showApiFields ? (<Field className="relay-field-model-list" label={t("profileEditor.modelList")}>
            <div className="relay-model-list-tools">
              <Textarea value={profile.modelList} onChange={(event) => updateDraft({ modelList: event.currentTarget.value })} placeholder={t("profileEditor.modelListPlaceholder")}/>
              <Button onClick={async () => {
                const models = await actions.fetchRelayProfileModels(profile);
                if (models?.length)
                    updateDraft({ modelList: models.join("\n") });
            }} size="sm" type="button" variant="secondary">
                <Download className="h-4 w-4"/>
                {t("profileEditor.fetchUpstream")}
              </Button>
            </div>
          </Field>) : null}
        {showApiFields ? (<Field className="relay-field-user-agent" label={t("profileEditor.userAgent")}>
            <Input value={profile.userAgent} onChange={(event) => updateDraft({ userAgent: event.currentTarget.value })} placeholder={t("profileEditor.userAgentPlaceholder")}/>
          </Field>) : null}
      </div>
      {showApiFields && profile.protocol === "chatCompletions" ? (<div className="hint-line relay-protocol-hint">
          <MessageCircle className="h-4 w-4"/>
          <span>{t("profileEditor.responsesApiHint", { endpoint: PROTOCOL_PROXY_ENDPOINT })}</span>
        </div>) : null}
      <div className="hint-line relay-protocol-hint">
        <ShieldCheck className="h-4 w-4"/>
        <span>{relayProfileModeHelp(profile)}</span>
      </div>
      {profile.linkedCcsProviderId ? (<div className="hint-line relay-protocol-hint">
          <Link2 className="h-4 w-4"/>
          <span>
            {t("profileEditor.ccsLinkedHint", { source: profile.linkedCcsProviderId })}
          </span>
        </div>) : null}
    </div>);
}
