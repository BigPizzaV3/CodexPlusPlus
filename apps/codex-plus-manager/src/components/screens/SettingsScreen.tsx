import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { useTranslation } from "react-i18next";
import { Actions, CardHead, Field, Panel, Toolbar } from "./shared";
import { codexExtraArgsToInput, inputToCodexExtraArgs } from "./utils";
import { DEFAULT_RELAY_TEST_MODEL, BackendSettings, SettingsResult } from "./model";
import { Theme } from "@tauri-apps/api/window";
import { CardContent } from "../ui/card";
import { Textarea } from "../ui/textarea";
export function SettingsScreen({ settings, theme, form, onFormChange, actions, }: {
    settings: SettingsResult | null;
    theme: Theme;
    form: BackendSettings;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    return (<>
      <Panel>
        <CardHead title={t("settings.title")} detail={settings?.settings_path ?? ""}/>
        <CardContent>
          <div className="theme-row">
            <div>
              <strong>{t("settings.theme")}</strong>
              <span>{theme === "dark" ? t("settings.themeDark") : t("settings.themeLight")}</span>
            </div>
            <Button variant="secondary" onClick={actions.toggleTheme}>{t("settings.switchTheme")}</Button>
          </div>
          <Field label={t("settings.relayTestModel")}>
            <Input value={form.relayTestModel} onChange={(event) => onFormChange({ ...form, relayTestModel: event.currentTarget.value })} placeholder={t("settings.relayTestModelPlaceholder", { model: DEFAULT_RELAY_TEST_MODEL })}/>
          </Field>
          <label className="check-row">
            <input checked={form.cliWrapperEnabled} onChange={(event) => onFormChange({ ...form, cliWrapperEnabled: event.currentTarget.checked })} type="checkbox"/>
            <span>{t("settings.cliWrapper")}</span>
          </label>
          <div className="form-row">
            <Field label={t("settings.wrapperBaseUrl")}>
              <Input value={form.cliWrapperBaseUrl} onChange={(event) => onFormChange({ ...form, cliWrapperBaseUrl: event.currentTarget.value })}/>
            </Field>
            <Field label={t("settings.wrapperApiKeyEnv")}>
              <Input value={form.cliWrapperApiKeyEnv} onChange={(event) => onFormChange({ ...form, cliWrapperApiKeyEnv: event.currentTarget.value })}/>
            </Field>
          </div>
          <Field label={t("settings.wrapperApiKey")}>
            <Input type="password" value={form.cliWrapperApiKey} onChange={(event) => onFormChange({ ...form, cliWrapperApiKey: event.currentTarget.value })}/>
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>{t("settings.save")}</Button>
            <Button variant="secondary" onClick={() => void actions.resetSettings()}>
              {t("settings.reset")}
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("settings.launchArgs")} detail={t("settings.launchArgsDetail")}/>
        <CardContent>
          <Field label={t("settings.extraArgs")}>
            <Textarea className="launch-args-input" placeholder={t("settings.extraArgsPlaceholder")} spellCheck={false} value={codexExtraArgsToInput(form.codexExtraArgs)} onChange={(event) => onFormChange({
            ...form,
            codexExtraArgs: inputToCodexExtraArgs(event.currentTarget.value),
        })}/>
          </Field>
          <p className="field-hint">{t("settings.extraArgsHint")}</p>
          <Toolbar>
            <Button onClick={() => void actions.saveSettings()}>{t("settings.save")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>);
}
