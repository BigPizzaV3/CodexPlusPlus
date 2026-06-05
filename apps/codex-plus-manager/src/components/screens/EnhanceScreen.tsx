import { useTranslation } from "react-i18next";
import { BackendSettings } from "./model";
import { ModeSelector } from "./ModeSelector";
import { Actions } from "./actions";
import { CardHead, Panel, Toolbar } from "./layout";
import { CardContent } from "../ui/card";
import { Info, ShieldCheck } from "lucide-react";
import { FeatureToggle } from "./featureControls";
import { Button } from "../ui/button";
export function EnhanceScreen({ form, onFormChange, actions, }: {
    form: BackendSettings;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const setEnhanceFlag = (key: keyof BackendSettings, value: boolean) => onFormChange({ ...form, [key]: value });
    const masterEnabled = form.enhancementsEnabled;
    const patchMode = form.launchMode === "patch";
    return (<>
            <Panel>
                <CardHead title={t("enhance.title")} detail={t("enhance.detail")}/>
                <CardContent>
                    <label className="switch-row">
                        <input checked={form.enhancementsEnabled} onChange={(event) => onFormChange({ ...form, enhancementsEnabled: event.currentTarget.checked })} type="checkbox"/>
                        <span>
                            <strong>{t("enhance.masterSwitch")}</strong>
                            <small>{t("enhance.masterSwitchDetail")}</small>
                        </span>
                    </label>
                    <ModeSelector launchMode={form.launchMode} actions={actions}/>
                    {form.launchMode === "relay" ? (<div className="hint-line">
                            <ShieldCheck className="h-4 w-4"/>
                            <span>{t("enhance.compatHint")}</span>
                        </div>) : null}
                    <div className="feature-switch-grid">
                        <FeatureToggle title={t("enhance.pluginMarketplace")} detail={t("enhance.pluginMarketplaceDetail")} checked={form.codexAppPluginMarketplaceUnlock} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppPluginMarketplaceUnlock", value)}/>
                        <FeatureToggle title={t("enhance.forceUnlock")} detail={t("enhance.forceUnlockDetail")} checked={form.codexAppPluginEntryUnlock} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppPluginEntryUnlock", value)}/>
                        <FeatureToggle title={t("enhance.forcePluginInstall")} detail={t("enhance.forcePluginInstallDetail")} checked={form.codexAppForcePluginInstall} disabled={!masterEnabled || !patchMode} onChange={(value) => setEnhanceFlag("codexAppForcePluginInstall", value)}/>
                        <FeatureToggle title={t("enhance.modelWhitelist")} detail={t("enhance.modelWhitelistDetail")} checked={form.codexAppModelWhitelistUnlock} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppModelWhitelistUnlock", value)}/>
                        <FeatureToggle title={t("enhance.serviceTier")} detail={t("enhance.serviceTierDetail")} checked={form.codexAppServiceTierControls} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppServiceTierControls", value)}/>
                        <FeatureToggle title={t("enhance.sessionDelete")} detail={t("enhance.sessionDeleteDetail")} checked={form.codexAppSessionDelete} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppSessionDelete", value)}/>
                        <FeatureToggle title={t("enhance.markdownExport")} detail={t("enhance.markdownExportDetail")} checked={form.codexAppMarkdownExport} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppMarkdownExport", value)}/>
                        <FeatureToggle title={t("enhance.projectMove")} detail={t("enhance.projectMoveDetail")} checked={form.codexAppProjectMove} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppProjectMove", value)}/>
                        <FeatureToggle title={t("enhance.timeline")} detail={t("enhance.timelineDetail")} checked={form.codexAppConversationTimeline} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppConversationTimeline", value)}/>
                        <FeatureToggle title={t("enhance.conversationWidth")} detail={t("enhance.conversationWidthDetail")} checked={form.codexAppConversationView} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppConversationView", value)}/>
                        <FeatureToggle title={t("enhance.scrollRestore")} detail={t("enhance.scrollRestoreDetail")} checked={form.codexAppThreadScrollRestore} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppThreadScrollRestore", value)}/>
                        <FeatureToggle title={t("enhance.zedRemote")} detail={t("enhance.zedRemoteDetail")} checked={form.codexAppZedRemoteOpen} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppZedRemoteOpen", value)}/>
                        <FeatureToggle title={t("enhance.upstreamWorktree")} detail={t("enhance.upstreamWorktreeDetail")} checked={form.codexAppUpstreamWorktreeCreate} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppUpstreamWorktreeCreate", value)}/>
                        <FeatureToggle title={t("enhance.nativeMenu")} detail={t("enhance.nativeMenuDetail")} checked={form.codexAppNativeMenuPlacement} disabled={!masterEnabled} onChange={(value) => setEnhanceFlag("codexAppNativeMenuPlacement", value)}/>
                    </div>
                    <div className="hint-line">
                        <Info className="h-4 w-4"/>
                        <span>{t("enhance.generalHint")}</span>
                    </div>
                    <Toolbar>
                        <Button onClick={() => void actions.saveSettings()}>{t("enhance.save")}</Button>
                    </Toolbar>
                </CardContent>
            </Panel>
        </>);
}
