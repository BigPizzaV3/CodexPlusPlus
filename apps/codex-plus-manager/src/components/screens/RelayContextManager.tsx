import { useTranslation } from "react-i18next";
import { Actions } from "./actions";
import { BackendSettings, CodexContextEntries, CodexContextEntry, ContextKind, RelayFilesResult } from "./model";
import { contextEntriesByKind, contextEntriesWithLiveEntries, contextKindLabel, contextKindOptions, isSuccessStatus, setContextEntryEnabled } from "./utils";
import { useState } from "react";
import { Button } from "../ui/button";
import { Edit3, Plus, Save, Trash2 } from "lucide-react";
import { Field, Toolbar } from "./layout";
import { Input } from "../ui/input";
import { Textarea } from "../ui/textarea";
export function RelayContextManager({ form, liveEntries, relayFiles, onFormChange, actions, }: {
    form: BackendSettings;
    liveEntries: CodexContextEntries | null;
    relayFiles: RelayFilesResult | null;
    onFormChange: (value: BackendSettings) => void;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const entries = contextEntriesWithLiveEntries(form, liveEntries);
    const [activeKind, setActiveKind] = useState<ContextKind>("mcp");
    const [editor, setEditor] = useState<{
        kind: ContextKind;
        entry?: CodexContextEntry;
    } | null>(null);
    const visibleEntries = contextEntriesByKind(entries, activeKind);
    const label = contextKindLabel(activeKind);
    const saveEntry = async (kind: ContextKind, id: string, tomlBody: string) => {
        const next = await actions.upsertContextEntry(form, kind, id, tomlBody);
        if (!next)
            return;
        onFormChange(next);
        setEditor(null);
    };
    const toggleContextEntryEnabled = async (entry: CodexContextEntry) => {
        const nextBody = setContextEntryEnabled(entry.tomlBody, !entry.enabled);
        const next = await actions.upsertContextEntry(form, entry.kind, entry.id, nextBody);
        if (!next)
            return;
        onFormChange(next);
        const syncResult = await actions.syncLiveContextEntries(next, true);
        if (syncResult && isSuccessStatus(syncResult.status)) {
            void actions.refreshRelayFiles();
        }
    };
    const deleteEntry = async (entry: CodexContextEntry) => {
        const next = await actions.deleteContextEntry(form, entry.kind, entry.id);
        if (!next)
            return;
        onFormChange(next);
    };
    return (<div className="relay-context-panel">
      <div className="relay-context-head">
        <div>
          <strong>{t("contextManager.title")}</strong>
          <span>{t("contextManager.detail")}</span>
        </div>
        <div className="relay-context-head-actions">
          <Button onClick={() => setEditor({ kind: activeKind })} size="sm" variant="secondary">
            <Plus className="h-4 w-4"/>
            {t("contextManager.addNew", { label })}
          </Button>
        </div>
      </div>
      <div className="segmented">
        {contextKindOptions.map((option) => (<button className={activeKind === option.kind ? "active" : ""} key={option.kind} onClick={() => setActiveKind(option.kind)} type="button">
            <span>{contextKindLabel(option.kind)}</span>
            <small>{contextEntriesByKind(entries, option.kind).length}</small>
          </button>))}
      </div>
      <div className="relay-context-summary">
        {t("contextManager.summary", { count: visibleEntries.length, label })}
      </div>
      <div className="relay-context-list">
        {visibleEntries.length ? (visibleEntries.map((entry) => (<div className="relay-context-row" key={`${entry.kind}-${entry.id}`}>
              <strong className="context-title">{entry.title || entry.id}</strong>
              <div className="relay-context-actions">
                <button aria-checked={entry.enabled} aria-label={`contextEnabledSwitch-${entry.kind}-${entry.id}`} className={`context-enabled-switch ${entry.enabled ? "active" : ""}`} onClick={() => void toggleContextEntryEnabled(entry)} role="switch" title={entry.enabled ? t("contextManager.disableEntry") : t("contextManager.enableEntry")} type="button">
                  <span className="context-switch-track" aria-hidden="true">
                    <span className="context-switch-thumb"/>
                  </span>
                </button>
                <Button onClick={() => setEditor({ kind: entry.kind, entry })} size="icon" title={t("contextManager.editEntry")} variant="ghost">
                  <Edit3 className="h-4 w-4"/>
                </Button>
                <Button className="relay-context-delete" onClick={() => void deleteEntry(entry)} size="icon" title={t("contextManager.deleteEntry")} variant="ghost">
                  <Trash2 className="h-4 w-4"/>
                </Button>
              </div>
            </div>))) : (<div className="empty">{t("contextManager.empty", { label })}</div>)}
      </div>
      {editor ? (<ContextEntryEditor entry={editor.entry} kind={editor.kind} onCancel={() => setEditor(null)} onSave={(kind, id, tomlBody) => void saveEntry(kind, id, tomlBody)}/>) : null}
    </div>);
}
export function ContextEntryEditor({ kind, entry, onCancel, onSave, }: {
    kind: ContextKind;
    entry?: CodexContextEntry;
    onCancel: () => void;
    onSave: (kind: ContextKind, id: string, tomlBody: string) => void;
}) {
    const { t } = useTranslation();
    const [draftKind, setDraftKind] = useState<ContextKind>(entry?.kind ?? kind);
    const [id, setId] = useState(entry?.id ?? "");
    const [tomlBody, setTomlBody] = useState(entry?.tomlBody ?? "");
    const canSave = id.trim().length > 0;
    return (<div className="context-editor">
      <div className="context-editor-fields">
        <Field label={t("contextEntry.type")}>
          <select className="field-select" disabled={!!entry} value={draftKind} onChange={(event) => setDraftKind(event.currentTarget.value as ContextKind)}>
            {contextKindOptions.map((option) => (<option key={option.kind} value={option.kind}>{contextKindLabel(option.kind)}</option>))}
          </select>
        </Field>
        <Field label={t("contextEntry.id")}>
          <Input disabled={!!entry} value={id} onChange={(event) => setId(event.currentTarget.value.trim())} placeholder={t("contextEntry.idPlaceholder")}/>
        </Field>
      </div>
      <Field label={t("contextEntry.tomlBody")}>
        <Textarea className="context-editor-textarea" value={tomlBody} onChange={(event) => setTomlBody(event.currentTarget.value)} placeholder={t("contextEntry.tomlPlaceholder")} spellCheck={false}/>
      </Field>
      <Toolbar>
        <Button disabled={!canSave} onClick={() => onSave(draftKind, id.trim(), tomlBody)} size="sm">
          <Save className="h-4 w-4"/>
          {t("contextEntry.save")}
        </Button>
        <Button onClick={onCancel} size="sm" variant="secondary">{t("contextEntry.cancel")}</Button>
      </Toolbar>
    </div>);
}
