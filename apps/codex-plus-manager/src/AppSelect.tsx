/**
 * 下拉选择：全应用替代原生下拉框。按钮 + portal 浮层（外点关闭、滚动跟随由 usePopoverLayer 负责），
 * 样式在 styles.css 的 `.app-select*`。选项可带一行说明（hint）。
 */
import { useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { CheckCircle2, ChevronDown } from "lucide-react";
import { usePopoverLayer } from "./use-popover-layer";

export type AppSelectOption<T extends string> = {
  value: T;
  label: ReactNode;
  disabled?: boolean;
  title?: string;
  /** 选项下面的一行小字说明。 */
  hint?: ReactNode;
};

export function AppSelect<T extends string>({
  value,
  options,
  onChange,
  disabled = false,
  className = "",
  title = "",
  ariaLabel,
  label,
  placeholder,
  id,
  size = "md",
  menuWidth,
}: {
  value: T;
  options: AppSelectOption<T>[];
  onChange: (value: T) => void;
  disabled?: boolean;
  className?: string;
  title?: string;
  ariaLabel?: string;
  /** 同 ariaLabel。 */
  label?: string;
  /** 当前值不在选项里时显示的文字（如「全部设为…」这类动作菜单）；不给时显示第一项。 */
  placeholder?: ReactNode;
  id?: string;
  /** sm：放在一行小控件里的紧凑尺寸。 */
  size?: "sm" | "md";
  /** 浮层最小宽度（默认 180）。 */
  menuWidth?: number;
}) {
  const [open, setOpen] = useState(false);
  const matched = options.find((option) => option.value === value);
  const selected = matched ?? (placeholder === undefined ? options[0] : undefined);
  const selectOption = (option: AppSelectOption<T>) => {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
  };
  const { rootRef, menuRef, menuStyle } = usePopoverLayer({
    open: open && !disabled,
    onOpenChange: setOpen,
    minWidth: menuWidth ?? 180,
    maxHeight: 300,
  });
  const sizeClass = size === "sm" ? "app-select-sm text-xs" : "";
  return (
    <div
      className={`app-select ${sizeClass} ${open ? "open" : ""} ${disabled ? "disabled" : ""} ${className}`.trim()}
      ref={rootRef}
    >
      <button
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={ariaLabel ?? label}
        className="app-select-trigger"
        disabled={disabled}
        id={id}
        onClick={() => setOpen((current) => !current)}
        title={title}
        type="button"
      >
        <span className={selected ? undefined : "app-select-placeholder"}>{selected ? selected.label : placeholder}</span>
        <ChevronDown className="h-4 w-4" />
      </button>
      {open && !disabled ? createPortal(
        <div
          className={`app-select-menu app-select-menu-portal ${sizeClass} ${className}`.trim()}
          ref={menuRef}
          role="listbox"
          style={menuStyle}
        >
          {options.map((option) => (
            <button
              aria-selected={option.value === value}
              className={`app-select-option ${option.value === value ? "selected" : ""} ${option.hint ? "has-hint" : ""}`.trim()}
              disabled={option.disabled}
              key={option.value}
              onClick={() => selectOption(option)}
              onMouseDown={(event) => {
                event.preventDefault();
                selectOption(option);
              }}
              role="option"
              title={option.title}
              type="button"
            >
              {option.value === value ? <CheckCircle2 className="h-4 w-4" /> : <span className="app-select-option-spacer" />}
              {option.hint ? (
                <span className="app-select-option-text">
                  <span>{option.label}</span>
                  <small>{option.hint}</small>
                </span>
              ) : (
                <span>{option.label}</span>
              )}
            </button>
          ))}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}
