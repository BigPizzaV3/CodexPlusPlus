import { useTranslation } from "react-i18next";
import { OverviewResult, SettingsResult, WatcherResult } from "./model";
import { Actions } from "./actions";
import { CardHead, Field, Panel, Toolbar } from "./layout";
import { CardContent } from "../ui/card";
import { StatusRow } from "./statusWidgets";
import { Button } from "../ui/button";
import { Input } from "../ui/input";
export function MaintenanceScreen({ overview, watcher, settings, launchForm, onLaunchFormChange, removeOwnedData, onRemoveOwnedDataChange, actions, }: {
    overview: OverviewResult | null;
    watcher: WatcherResult | null;
    settings: SettingsResult | null;
    launchForm: {
        appPath: string;
        debugPort: string;
        helperPort: string;
    };
    onLaunchFormChange: (next: {
        appPath: string;
        debugPort: string;
        helperPort: string;
    }) => void;
    removeOwnedData: boolean;
    onRemoveOwnedDataChange: (value: boolean) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const savedCodexAppPath = settings?.settings.codexAppPath ?? "";
    return (<>
      <Panel>
        <CardHead title={t("maintenance.title")} detail={t("maintenance.detail")}/>
        <CardContent>
          <div className="status-table">
            <StatusRow title={t("maintenance.codexApp")} status={overview?.codex_app.status} path={overview?.codex_app.path}/>
            <StatusRow title={t("maintenance.silentEntry")} status={overview?.silent_shortcut.status} path={overview?.silent_shortcut.path}/>
            <StatusRow title={t("maintenance.managerEntry")} status={overview?.management_shortcut.status} path={overview?.management_shortcut.path}/>
            <StatusRow title={t("maintenance.watcher")} status={watcher?.enabled ? "ok" : "disabled"} path={watcher?.disabled_flag}/>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.checkHealth()}>{t("maintenance.check")}</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>{t("maintenance.repairShortcuts")}</Button>
            <Button variant="secondary" onClick={() => void actions.repairBackend()}>{t("maintenance.repairBackend")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("maintenance.entryManagement")} detail={t("maintenance.entryDetail")}/>
        <CardContent>
          <label className="check-row">
            <input checked={removeOwnedData} onChange={(event) => onRemoveOwnedDataChange(event.currentTarget.checked)} type="checkbox"/>
            <span>{t("maintenance.removeData")}</span>
          </label>
          <Toolbar>
            <Button onClick={() => void actions.installEntrypoints()}>{t("maintenance.installEntry")}</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallEntrypoints()}>{t("maintenance.uninstallEntry")}</Button>
            <Button variant="secondary" onClick={() => void actions.repairShortcuts()}>{t("maintenance.repairEntry")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("maintenance.autoTakeover")} detail={t("maintenance.watcherDetail")}/>
        <CardContent>
          <Toolbar>
            <Button variant="secondary" onClick={() => void actions.installWatcher()}>{t("maintenance.installWatcher")}</Button>
            <Button variant="secondary" onClick={() => void actions.uninstallWatcher()}>{t("maintenance.removeWatcher")}</Button>
            <Button variant="secondary" onClick={() => void actions.enableWatcher()}>{t("maintenance.enableWatcher")}</Button>
            <Button variant="secondary" onClick={() => void actions.disableWatcher()}>{t("maintenance.disableWatcher")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("maintenance.codexAppPath")} detail={t("maintenance.pathDetail")}/>
        <CardContent>
          <div className="status-table">
            <StatusRow title={t("maintenance.savedPath")} status={savedCodexAppPath ? "ok" : "not_checked"} path={savedCodexAppPath || null}/>
            <StatusRow title={t("maintenance.currentDetection")} status={overview?.codex_app.status} path={overview?.codex_app.path}/>
          </div>
          <Field label={t("maintenance.savedAppPath")}>
            <Input value={settings?.settings.codexAppPath ?? ""} placeholder={t("maintenance.pathDetail")} readOnly/>
          </Field>
          <Toolbar>
            <Button onClick={() => void actions.chooseCodexAppPath("folder")}>{t("maintenance.selectFolder")}</Button>
            <Button variant="secondary" onClick={() => void actions.chooseCodexAppPath("file")}>{t("maintenance.selectFile")}</Button>
            <Button variant="secondary" onClick={() => void actions.clearCodexAppPath()}>{t("maintenance.clearPath")}</Button>
          </Toolbar>
        </CardContent>
      </Panel>
      <Panel>
        <CardHead title={t("maintenance.manualLaunch")} detail={t("maintenance.manualLaunchDetail")}/>
        <CardContent>
          <Field label={t("maintenance.pathOverride")}>
            <Input value={launchForm.appPath} onChange={(event) => onLaunchFormChange({ ...launchForm, appPath: event.currentTarget.value })} placeholder={savedCodexAppPath || t("maintenance.pathDetail")}/>
          </Field>
          <div className="form-row">
            <Field label={t("maintenance.debugPort")}>
              <Input value={launchForm.debugPort} onChange={(event) => onLaunchFormChange({ ...launchForm, debugPort: event.currentTarget.value })}/>
            </Field>
            <Field label={t("maintenance.helperPort")}>
              <Input value={launchForm.helperPort} onChange={(event) => onLaunchFormChange({ ...launchForm, helperPort: event.currentTarget.value })}/>
            </Field>
          </div>
          <Toolbar>
            <Button onClick={() => void actions.launch()}>{t("maintenance.launchCodex")}</Button>
            <Button variant="secondary" onClick={() => void actions.saveManualCodexAppPath()}>
              {t("maintenance.saveAsDefault")}
            </Button>
          </Toolbar>
        </CardContent>
      </Panel>
    </>);
}
