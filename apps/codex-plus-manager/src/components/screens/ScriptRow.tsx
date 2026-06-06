import { Power, PowerOff, Trash2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { type UserScriptInventory } from "./model";
import type { Actions } from "./actions";
export function ScriptRow({ script, actions }: {
    script: NonNullable<UserScriptInventory["scripts"]>[number];
    actions: Actions;
}) {
    const { t } = useTranslation();
    const source = script.market_id ? `${t("scriptRow.sourceMarket")} · ${script.version || t("scriptRow.sourceUnknown")}` : script.source === "builtin" ? t("scriptRow.sourceBuiltin") : t("scriptRow.sourceUser");
    const canDelete = script.source === "user";
    return (<div className="table-row">
      <span>{script.name}</span>
      <span>{source}</span>
      <span>{script.enabled ? t("scriptRow.enabled") : t("scriptRow.disabled")}</span>
      <span>{script.status}</span>
      <div className="script-row-actions">
        <Button onClick={() => void actions.setUserScriptEnabled(script.key, !script.enabled)} size="sm" variant="secondary">
          {script.enabled ? <PowerOff className="h-4 w-4"/> : <Power className="h-4 w-4"/>}
          {script.enabled ? t("scriptRow.disable") : t("scriptRow.enable")}
        </Button>
        {canDelete ? (<Button onClick={() => void actions.deleteUserScript(script.key)} size="sm" variant="outline">
            <Trash2 className="h-4 w-4"/>
            {t("scriptRow.delete")}
          </Button>) : null}
      </div>
    </div>);
}
