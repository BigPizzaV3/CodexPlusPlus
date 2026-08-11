import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { LinearIcon } from "./LinearIcon";

interface BoardSettingsMenuProps {
  showEmptyColumns: boolean;
  onShowEmptyColumnsChange: (show: boolean) => void;
}

export function BoardSettingsMenu({
  showEmptyColumns,
  onShowEmptyColumnsChange,
}: BoardSettingsMenuProps) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const menuRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0, ready: false });

  useLayoutEffect(() => {
    if (!open || !triggerRef.current || !menuRef.current) return;
    const trigger = triggerRef.current.getBoundingClientRect();
    const menu = menuRef.current.getBoundingClientRect();
    const left = Math.max(8, Math.min(trigger.right - menu.width, window.innerWidth - menu.width - 8));
    const top = trigger.bottom + 8 + menu.height <= window.innerHeight
      ? trigger.bottom + 8
      : Math.max(8, trigger.top - menu.height - 8);
    setPosition({ left, top, ready: true });
  }, [open]);

  useEffect(() => {
    if (!open) return;
    requestAnimationFrame(() => menuRef.current?.querySelector<HTMLButtonElement>("[role='switch']")?.focus());

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
      className="board-settings-menu"
      role="dialog"
      aria-label="看板设置"
      style={{
        left: position.left,
        top: position.top,
        visibility: position.ready ? "visible" : "hidden",
      }}
      onKeyDown={(event) => {
        if (event.key === "Tab") {
          event.preventDefault();
          const switches = [...(menuRef.current?.querySelectorAll<HTMLButtonElement>("[role='switch']") ?? [])]
            .filter((button) => !button.disabled);
          const currentIndex = switches.indexOf(document.activeElement as HTMLButtonElement);
          const offset = event.shiftKey ? -1 : 1;
          switches[(currentIndex + offset + switches.length) % switches.length]?.focus();
        }
      }}
    >
      <section className="board-settings-section" aria-labelledby="board-options-heading">
        <h2 id="board-options-heading">看板选项</h2>
        <div className="board-setting-row">
          <span>显示空列</span>
          <button
            type="button"
            className={`board-setting-switch${showEmptyColumns ? " is-on" : ""}`}
            role="switch"
            aria-checked={showEmptyColumns}
            onClick={() => onShowEmptyColumnsChange(!showEmptyColumns)}
          >
            <span aria-hidden="true" />
            <span className="sr-only">{showEmptyColumns ? "关闭显示空列" : "开启显示空列"}</span>
          </button>
        </div>
      </section>
    </div>,
    document.body,
  ) : null;

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className={`board-settings-trigger${open ? " is-open" : ""}${showEmptyColumns ? " is-active" : ""}`}
        aria-label="看板设置"
        aria-haspopup="dialog"
        aria-expanded={open}
        title="看板设置"
        onClick={() => {
          if (!open) {
            setPosition((current) => ({ ...current, ready: false }));
          }
          setOpen((current) => !current);
        }}
      >
        <LinearIcon name="displayOptions" />
        {showEmptyColumns && <span className="board-settings-active-dot" aria-hidden="true" />}
      </button>
      {menu}
    </>
  );
}
