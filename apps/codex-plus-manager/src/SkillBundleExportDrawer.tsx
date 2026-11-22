import { useState } from "react";
import { Upload } from "lucide-react";
import { ManagementDrawer } from "./ManagementPages";
import { t, tf } from "./i18n";
import { skillBundleSelections, type SkillBundleExportGroup } from "./skill-groups";
import "./SkillSourceMatchDrawer.css";

/** 导出 Skill 迁移包（#2852）：勾选要带走的分组，分组名、成员和各 Agent 的启用状态一起写进包里。 */
export function SkillBundleExportDrawer({ groups, busy, onClose, onExport }: {
  groups: SkillBundleExportGroup[];
  busy: boolean;
  onClose: () => void;
  onExport: (selected: ReadonlySet<string>) => void;
}) {
  const [selected, setSelected] = useState<Set<string>>(() => new Set(groups.map((group) => group.key)));
  const count = skillBundleSelections(groups, selected).skills.length;
  const allSelected = groups.length > 0 && groups.every((group) => selected.has(group.key));
  const toggle = (key: string) => setSelected((old) => {
    const next = new Set(old);
    if (!next.delete(key)) next.add(key);
    return next;
  });
  return <ManagementDrawer title={t("导出 Skill 迁移包")} onClose={busy ? () => {} : onClose}
    footer={<>
      <button type="button" className="cx-secondary" disabled={busy || !groups.length}
        onClick={() => setSelected(allSelected ? new Set() : new Set(groups.map((group) => group.key)))}>{allSelected ? t("全不选") : t("全选")}</button>
      <button type="button" className="cx-primary" disabled={busy || !count} onClick={() => onExport(selected)}><Upload size={14}/>{tf("导出 {0} 个 Skill…", [count])}</button>
    </>}>
    <p className="mg-muted">{t("选中分组里已安装的 Skill 会连同文件、分组和各 Agent 的启用状态打成一个 zip；在另一台电脑的 Skills 页用「导入迁移包」恢复。内置 Skill 不导出。")}</p>
    {!groups.length && <p className="mg-muted" role="status">{t("没有可导出的 Skill。")}</p>}
    <div className="mg-source-list mg-bundle-list">
      {groups.map((group) => <div className="mg-source-row mg-bundle-row" key={group.key}>
        <label>
          <input type="checkbox" checked={selected.has(group.key)} disabled={busy} onChange={() => toggle(group.key)} />
          <span><strong>{group.label}</strong><small>{tf("{0} 个 Skill", [group.ids.length])}</small></span>
        </label>
      </div>)}
    </div>
  </ManagementDrawer>;
}
