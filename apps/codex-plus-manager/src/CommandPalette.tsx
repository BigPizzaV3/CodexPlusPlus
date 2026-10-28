/** ⌘K 命令面板：跳到任意页面、切到某个 Agent 的页面、执行几个常用操作。 */
import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { CornerDownLeft, Search } from "lucide-react";
import { t } from "./i18n";
import { filterCommands, type PaletteCommand } from "./command-palette-model";
import { useEscapeLayer } from "./use-escape-layer";

export type { PaletteCommand };

export function CommandPalette({ open, commands, onClose }: { open: boolean; commands: PaletteCommand[]; onClose: () => void }) {
  const [query, setQuery] = useState("");
  const [highlight, setHighlight] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const results = useMemo(() => filterCommands(commands, query), [commands, query]);
  // 登记进 Esc 栈：面板盖在弹窗上时，Esc 只关面板，不关下面的弹窗。
  useEscapeLayer(open, onClose);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setHighlight(0);
    window.setTimeout(() => input.current?.focus(), 0);
  }, [open]);

  useEffect(() => {
    list.current?.querySelector(`[data-index="${highlight}"]`)?.scrollIntoView({ block: "nearest" });
  }, [highlight]);

  if (!open) return null;

  const choose = (command: PaletteCommand | undefined) => {
    if (!command) return;
    onClose();
    command.run();
  };

  let lastGroup = "";
  return createPortal(
    <div className="palette-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onClose(); }}>
      <div aria-label={t("快速跳转")} aria-modal="true" className="palette" role="dialog">
        <div className="palette-input">
          <Search aria-hidden="true" className="h-4 w-4" />
          <input
            onChange={(event) => { setQuery(event.currentTarget.value); setHighlight(0); }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") { event.preventDefault(); setHighlight((value) => Math.min(results.length - 1, value + 1)); }
              if (event.key === "ArrowUp") { event.preventDefault(); setHighlight((value) => Math.max(0, value - 1)); }
              if (event.key === "Enter") { event.preventDefault(); choose(results[highlight]); }
            }}
            placeholder={t("跳到页面、Agent 或执行操作…")}
            ref={input}
            value={query}
          />
          <kbd>Esc</kbd>
        </div>
        <div className="palette-results" ref={list} role="listbox">
          {results.length === 0 ? <p className="palette-empty">{t("没有匹配项")}</p> : null}
          {results.map((command, index) => {
            const heading = command.group !== lastGroup ? command.group : null;
            lastGroup = command.group;
            return (
              <div key={command.id}>
                {heading ? <div className="palette-group">{heading}</div> : null}
                <button
                  aria-selected={index === highlight}
                  className="palette-item"
                  data-index={index}
                  onClick={() => choose(command)}
                  onMouseMove={() => setHighlight(index)}
                  role="option"
                  type="button"
                >
                  <span className="palette-icon">{command.icon}</span>
                  <span className="palette-label">{command.label}</span>
                  {index === highlight ? <CornerDownLeft aria-hidden="true" className="palette-enter" /> : null}
                </button>
              </div>
            );
          })}
        </div>
      </div>
    </div>,
    document.body,
  );
}
