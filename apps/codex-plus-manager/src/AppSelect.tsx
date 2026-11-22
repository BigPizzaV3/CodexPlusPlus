/**
 * 下拉选择：全应用替代原生下拉框。按钮 + portal 浮层（外点关闭、滚动跟随由 usePopoverLayer 负责），
 * 样式在 styles.css 的 `.app-select*`。选项可带一行说明（hint）。
 *
 * 键盘：焦点始终留在按钮上（列表框的高亮项用 aria-activedescendant 指出），所以 Tab 照常走到下一个控件、
 * 菜单随之收起，不会掉进页面末尾的浮层里。↑/↓/Home/End 移动，Enter/空格选中，Esc 关闭（Esc 栈）。
 */
import { useEffect, useId, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import { initialActiveOption, isSelectNavKey, moveActiveOption } from "./app-select-model";
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
  const [activeIndex, setActiveIndex] = useState(-1);
  // 空格在按下时已经选中并关了菜单：松开时按钮的点击要吞掉，否则菜单又会打开。
  const swallowSpaceUp = useRef(false);
  const listboxId = useId();
  const optionId = (index: number) => `${listboxId}-option-${index}`;
  const matched = options.find((option) => option.value === value);
  const selected = matched ?? (placeholder === undefined ? options[0] : undefined);
  const disabledFlags = options.map((option) => Boolean(option.disabled));
  const openMenu = () => {
    setActiveIndex(initialActiveOption(disabledFlags, options.findIndex((option) => option.value === value)));
    setOpen(true);
  };
  const selectOption = (option: AppSelectOption<T>) => {
    if (option.disabled) return;
    onChange(option.value);
    setOpen(false);
  };
  const onTriggerKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (disabled) return;
    if (!open) {
      if (event.key === "ArrowDown" || event.key === "ArrowUp") {
        event.preventDefault();
        openMenu();
      }
      return;
    }
    if (isSelectNavKey(event.key)) {
      event.preventDefault();
      setActiveIndex((current) => moveActiveOption(disabledFlags, current, event.key as Parameters<typeof moveActiveOption>[2]));
    } else if (event.key === "Enter" || event.key === " ") {
      // 打开时 Enter / 空格选中高亮项，不再触发按钮的点击（那会把菜单关掉又打开）。
      event.preventDefault();
      if (event.key === " ") swallowSpaceUp.current = true;
      const option = options[activeIndex];
      if (option) selectOption(option);
      else setOpen(false);
    }
  };
  // 高亮项滚进可视区（选项多、菜单有滚动条时）。
  useEffect(() => {
    if (!open || activeIndex < 0) return;
    document.getElementById(optionId(activeIndex))?.scrollIntoView?.({ block: "nearest" });
    // optionId 只依赖稳定的 listboxId。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, activeIndex]);
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
        aria-activedescendant={open && activeIndex >= 0 ? optionId(activeIndex) : undefined}
        aria-controls={open ? listboxId : undefined}
        aria-expanded={open}
        aria-haspopup="listbox"
        aria-label={ariaLabel ?? label}
        className="app-select-trigger"
        disabled={disabled}
        id={id}
        onClick={() => (open ? setOpen(false) : openMenu())}
        onKeyDown={onTriggerKeyDown}
        onKeyUp={(event) => {
          if (event.key === " " && swallowSpaceUp.current) {
            swallowSpaceUp.current = false;
            event.preventDefault();
          }
        }}
        role="combobox"
        title={title}
        type="button"
      >
        <span className={selected ? undefined : "app-select-placeholder"}>{selected ? selected.label : placeholder}</span>
        <ChevronDown className="h-3.5 w-3.5" />
      </button>
      {open && !disabled ? createPortal(
        <div
          aria-label={ariaLabel ?? label}
          className={`app-select-menu app-select-menu-portal ${sizeClass} ${className}`.trim()}
          id={listboxId}
          ref={menuRef}
          role="listbox"
          style={menuStyle}
        >
          {options.map((option, index) => (
            <button
              aria-disabled={option.disabled || undefined}
              aria-selected={option.value === value}
              className={`app-select-option ${option.value === value ? "selected" : ""} ${option.hint ? "has-hint" : ""}`.trim()}
              data-active={index === activeIndex || undefined}
              disabled={option.disabled}
              id={optionId(index)}
              key={option.value}
              onClick={() => selectOption(option)}
              onMouseDown={(event) => {
                event.preventDefault();
                selectOption(option);
              }}
              onMouseEnter={() => { if (!option.disabled) setActiveIndex(index); }}
              role="option"
              tabIndex={-1}
              title={option.title}
              type="button"
            >
              {option.hint ? (
                <span className="app-select-option-text">
                  <span>{option.label}</span>
                  <small>{option.hint}</small>
                </span>
              ) : (
                <span className="app-select-option-label">{option.label}</span>
              )}
              {option.value === value ? <Check aria-hidden className="app-select-check" /> : null}
            </button>
          ))}
        </div>,
        document.body,
      ) : null}
    </div>
  );
}
