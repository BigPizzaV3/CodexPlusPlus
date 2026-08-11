import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { LinearIcon } from "./LinearIcon";

interface ColumnVisibilityMenuProps {
  label: string;
  action: "hide" | "show";
  className: string;
  onAction: () => void;
}

export function ColumnVisibilityMenu({
  label,
  action,
  className,
  onAction,
}: ColumnVisibilityMenuProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0, ready: false });
  const actionLabel = action === "hide" ? "隐藏列" : "显示列";

  useLayoutEffect(() => {
    if (!open || !triggerRef.current || !menuRef.current) return;
    const trigger = triggerRef.current.getBoundingClientRect();
    const menu = menuRef.current.getBoundingClientRect();
    const left = Math.max(8, Math.min(trigger.right - menu.width, window.innerWidth - menu.width - 8));
    const top = trigger.bottom + 6 + menu.height <= window.innerHeight
      ? trigger.bottom + 6
      : Math.max(8, trigger.top - menu.height - 6);
    setPosition({ left, top, ready: true });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus());

    function closeFromOutside(event: PointerEvent) {
      if (
        !menuRef.current?.contains(event.target as Node)
        && !triggerRef.current?.contains(event.target as Node)
      ) {
        setOpen(false);
      }
    }

    function closeFromViewportChange() {
      setOpen(false);
    }

    function closeFromEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setOpen(false);
        triggerRef.current?.focus();
      }
    }

    document.addEventListener("pointerdown", closeFromOutside);
    document.addEventListener("keydown", closeFromEscape);
    window.addEventListener("blur", closeFromViewportChange);
    window.addEventListener("resize", closeFromViewportChange);
    window.addEventListener("scroll", closeFromViewportChange, true);
    return () => {
      document.removeEventListener("pointerdown", closeFromOutside);
      document.removeEventListener("keydown", closeFromEscape);
      window.removeEventListener("blur", closeFromViewportChange);
      window.removeEventListener("resize", closeFromViewportChange);
      window.removeEventListener("scroll", closeFromViewportChange, true);
    };
  }, [open]);

  const menu = open ? createPortal(
    <div
      ref={menuRef}
      className="task-context-menu column-visibility-menu"
      role="menu"
      aria-label={`${label}列选项`}
      style={{
        left: position.left,
        top: position.top,
        visibility: position.ready ? "visible" : "hidden",
      }}
      onKeyDown={(event) => {
        if (event.key === "Tab") {
          event.preventDefault();
          menuRef.current?.querySelector<HTMLButtonElement>("[role='menuitem']")?.focus();
        }
      }}
    >
      <button
        type="button"
        className="context-menu-item column-visibility-action"
        role="menuitem"
        onClick={() => {
          setOpen(false);
          onAction();
        }}
      >
        <span className="context-menu-label">{actionLabel}</span>
      </button>
    </div>,
    document.body,
  ) : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`${className}${open ? " is-open" : ""}`}
        aria-label={`${label}选项`}
        aria-haspopup="menu"
        aria-expanded={open}
        title="选项"
        onClick={() => {
          if (!open) setPosition((current) => ({ ...current, ready: false }));
          setOpen((current) => !current);
        }}
      >
        <LinearIcon name="more" />
      </button>
      {menu}
    </>
  );
}
