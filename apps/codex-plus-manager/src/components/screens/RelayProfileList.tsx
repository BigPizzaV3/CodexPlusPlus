import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { CheckCircle2, Copy, Edit3, GripVertical, TestTube, Trash2 } from "lucide-react";
import { type CSSProperties } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { type BackendSettings, type RelayProfile } from "./model";
import { duplicateRelayProfile, relayModeLabel, relayProfileConfigBrief, relayProfileSourceLabel, relayProtocolLabel, providerInitial, removeRelayProfile, reorderRelayProfiles, syncLegacyRelayFields } from "./utils";
import { Actions } from "./shared";
export function RelayProfileList({ form, onFormChange, onEdit, disabled = false, actions, }: {
    form: BackendSettings;
    onFormChange: (value: BackendSettings) => void;
    onEdit: (id: string) => void;
    disabled?: boolean;
    actions: Actions;
}) {
    const sensors = useSensors(useSensor(PointerSensor, {
        activationConstraint: { distance: 8 },
    }), useSensor(KeyboardSensor, {
        coordinateGetter: sortableKeyboardCoordinates,
    }));
    const handleDragEnd = (event: DragEndEvent) => {
        const { active, over } = event;
        if (!over || active.id === over.id)
            return;
        const next = reorderRelayProfiles(form, String(active.id), String(over.id));
        if (next !== form)
            onFormChange(next);
    };
    return (<DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={handleDragEnd}>
      <SortableContext items={form.relayProfiles.map((profile) => profile.id)} strategy={verticalListSortingStrategy}>
        <div className="relay-profile-list">
          {form.relayProfiles.map((profile, index) => (<SortableRelayProfileCard actions={actions} form={form} index={index} key={profile.id} onEdit={onEdit} onFormChange={onFormChange} disabled={disabled} profile={profile}/>))}
        </div>
      </SortableContext>
    </DndContext>);
}
export function SortableRelayProfileCard({ form, profile, index, onFormChange, onEdit, disabled = false, actions, }: {
    form: BackendSettings;
    profile: RelayProfile;
    index: number;
    onFormChange: (value: BackendSettings) => void;
    onEdit: (id: string) => void;
    disabled?: boolean;
    actions: Actions;
}) {
    const { t } = useTranslation();
    const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: profile.id });
    const active = profile.id === form.activeRelayId;
    const style: CSSProperties = {
        transform: CSS.Transform.toString(transform),
        transition,
    };
    return (<div className={`relay-profile-card ${active ? "active" : ""} ${isDragging ? "dragging" : ""}`} data-relay-profile-id={profile.id} key={profile.id} onKeyDown={(event) => {
            if (event.key === "Enter")
                onEdit(profile.id);
        }} ref={setNodeRef} style={style} tabIndex={0}>
      <button aria-label={t("sortableCard.dragSort")} className="relay-drag" title={t("sortableCard.dragSort")} type="button" {...attributes} {...listeners}>
        <GripVertical className="h-4 w-4"/>
      </button>
      <span className="relay-index" title={profile.name || t("sortableCard.unnamed")}>
        {providerInitial(profile.name)}
      </span>
      <span className="relay-summary">
        <strong>{profile.name || t("sortableCard.unnamed")}</strong>
        <small>{relayProfileSourceLabel(profile)} · {relayModeLabel(profile.relayMode)} · {relayProtocolLabel(profile.protocol)} · {relayProfileConfigBrief(profile)}</small>
      </span>
      <span className="relay-card-actions">
        <Button className={`relay-use-button ${active ? "active" : ""}`} disabled={disabled} onClick={(event) => {
            event.stopPropagation();
            if (disabled)
                return;
            const previousActiveRelayId = form.activeRelayId;
            const next = syncLegacyRelayFields({ ...form, activeRelayId: profile.id });
            void actions.switchRelayProfile(next, previousActiveRelayId);
        }} size="sm" title={disabled ? t("sortableCard.switchDisabled") : active ? t("sortableCard.inUse") : t("sortableCard.setCurrent")} variant={active ? "secondary" : "outline"}>
          <CheckCircle2 className="h-4 w-4"/>
          {active ? t("sortableCard.using") : t("sortableCard.use")}
        </Button>
        <span className="relay-card-extra">
          <Button onClick={(event) => {
            event.stopPropagation();
            void actions.testRelayProfile(profile);
        }} size="icon" title={t("sortableCard.sendHi")} variant="ghost">
            <TestTube className="h-4 w-4"/>
          </Button>
          <Button onClick={(event) => {
            event.stopPropagation();
            onEdit(profile.id);
        }} size="icon" title={t("sortableCard.edit")} variant="ghost">
            <Edit3 className="h-4 w-4"/>
          </Button>
          <Button onClick={(event) => {
            event.stopPropagation();
            onFormChange(duplicateRelayProfile(form, profile.id));
        }} size="icon" title={t("sortableCard.copy")} variant="ghost">
            <Copy className="h-4 w-4"/>
          </Button>
          <Button disabled={form.relayProfiles.length <= 1} onClick={(event) => {
            event.stopPropagation();
            onFormChange(removeRelayProfile(form, profile.id));
        }} size="icon" title={t("sortableCard.deleteProvider")} variant="ghost">
            <Trash2 className="h-4 w-4"/>
          </Button>
        </span>
      </span>
    </div>);
}
