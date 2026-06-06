import { useTranslation } from "react-i18next";
import { Actions } from "./actions";
import { BackendSettings, CodexContextEntries, RelayFilesResult } from "./model";
import { RelayContextManager } from "./RelayContextManager";
import { CardHead, Panel } from "./layout";
import { CardContent } from "../ui/card";
import { normalizeSettings } from "./utils";
export function ContextScreen({ form, liveEntries, relayFiles, onFormChange, actions, }: {
    form: BackendSettings;
    liveEntries: CodexContextEntries | null;
    relayFiles: RelayFilesResult | null;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    return (<Panel fill>
      <CardHead title={t("context.title")} detail={t("context.detail")}/>
      <CardContent>
        <RelayContextManager form={normalizeSettings(form)} liveEntries={liveEntries} relayFiles={relayFiles} onFormChange={onFormChange} actions={actions}/>
      </CardContent>
    </Panel>);
}
