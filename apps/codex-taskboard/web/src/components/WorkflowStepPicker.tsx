import { useEffect, useMemo, useRef, useState } from "react";
import { LinearIcon } from "./LinearIcon";
import { WORKFLOW_GROUPS, type PaletteItem } from "./workflowCatalog";
import { WorkflowMark } from "./WorkflowMark";

interface WorkflowStepPickerProps {
  items: PaletteItem[];
  onSelect: (item: PaletteItem) => void;
  onClose: () => void;
}

export function WorkflowStepPicker({ items, onSelect, onClose }: WorkflowStepPickerProps) {
  const [query, setQuery] = useState("");
  const searchRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    searchRef.current?.focus();

    function handleKeyDown(event: globalThis.KeyboardEvent) {
      if (event.key !== "Escape") return;
      event.preventDefault();
      onClose();
    }

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [onClose]);

  const filteredItems = useMemo(() => {
    const normalizedQuery = query.trim().toLocaleLowerCase();
    if (!normalizedQuery) return items;

    return items.filter((item) => (
      `${item.group} ${item.title} ${item.description} ${item.data.title} ${item.data.description} ${item.data.meta}`
        .toLocaleLowerCase()
        .includes(normalizedQuery)
    ));
  }, [items, query]);

  const groups = useMemo(() => {
    const availableGroups = new Set(filteredItems.map((item) => item.group));
    return [
      ...WORKFLOW_GROUPS.filter((group) => availableGroups.has(group)),
      ...[...availableGroups].filter((group) => !WORKFLOW_GROUPS.includes(group as typeof WORKFLOW_GROUPS[number])),
    ];
  }, [filteredItems]);

  return (
    <div className="workflow-step-picker-backdrop" onMouseDown={(event) => {
      if (event.target === event.currentTarget) onClose();
    }}>
      <section
        className="workflow-step-picker"
        role="dialog"
        aria-label="添加流程步骤"
        aria-modal="true"
      >
        <header className="workflow-step-picker-header">
          <strong>添加流程步骤</strong>
          <button type="button" aria-label="关闭" onClick={onClose}>
            <LinearIcon name="close" />
          </button>
        </header>

        <label className="workflow-step-picker-search">
          <LinearIcon name="search" />
          <input
            ref={searchRef}
            type="search"
            value={query}
            placeholder="搜索应用或动作…"
            aria-label="搜索应用或动作"
            onChange={(event) => setQuery(event.target.value)}
          />
        </label>

        <div className="workflow-step-picker-groups">
          {groups.map((group) => (
            <section className="workflow-step-picker-group" key={group}>
              <h3>{group}</h3>
              <div className="workflow-step-picker-items">
                {filteredItems.filter((item) => item.group === group).map((item) => (
                  <button
                    className="workflow-step-picker-item"
                    type="button"
                    key={`${item.group}-${item.data.kind}`}
                    onClick={() => onSelect(item)}
                  >
                    <span className={`workflow-step-picker-mark tone-${item.data.tone}`}>
                      <WorkflowMark
                        icon={item.data.icon}
                        logo={item.data.logo}
                        logoMonochrome={item.data.logoMonochrome}
                      />
                    </span>
                    <span className="workflow-step-picker-copy">
                      <strong>{item.title}</strong>
                      <span>{item.description}</span>
                    </span>
                    <LinearIcon className="workflow-step-picker-chevron" name="chevronRight" />
                  </button>
                ))}
              </div>
            </section>
          ))}
          {filteredItems.length === 0 && (
            <p className="workflow-step-picker-empty">没有匹配的应用或动作</p>
          )}
        </div>
      </section>
    </div>
  );
}
