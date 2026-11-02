/**
 * 首次使用引导：压暗整个界面，只露出当前步骤的目标（`[data-tour="..."]`），旁边一张说明卡。
 * 目标找不到（例如所在分组被折叠）或步骤没有目标时，说明卡显示在屏幕中央。
 */
import { useCallback, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowLeft, ArrowRight, X } from "lucide-react";
import { t, tf } from "./i18n";
import { paletteShortcut } from "./command-palette-model";
import { placeCard, tourSteps } from "./onboarding-model";
import { providersHidden, singleAgent, type AgentFocus } from "./agent-focus-model";
import { agentName } from "./gateway-model";
import { useEscapeLayer } from "./use-escape-layer";
import { useModalFocus } from "./use-modal-focus";
import "./OnboardingTour.css";

type Rect = { left: number; top: number; right: number; bottom: number; width: number; height: number };

const PAD = 6;

export function OnboardingTour({ focus, onClose }: { focus: AgentFocus; onClose: (finished: boolean) => void }) {
  const solo = singleAgent(focus);
  const steps = tourSteps(solo ? agentName(solo) : null, providersHidden(focus));
  const text = (value: string, args?: string[]) => (args ? tf(value, args) : t(value));
  useEscapeLayer(true, () => onClose(false));
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<Rect | null>(null);
  const [cardPos, setCardPos] = useState<{ left: number; top: number } | null>(null);
  const [viewport, setViewport] = useState(() => ({ width: window.innerWidth, height: window.innerHeight }));
  const card = useRef<HTMLDivElement>(null);
  useModalFocus(card);
  const step = steps[index];
  const last = index === steps.length - 1;

  // 量目标位置；窗口变化、侧栏滚动时重新量。
  const measure = useCallback(() => {
    setViewport((current) => current.width === window.innerWidth && current.height === window.innerHeight
      ? current
      : { width: window.innerWidth, height: window.innerHeight });
    const element = step.target ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`) : null;
    const box = element?.getBoundingClientRect();
    if (!element || !box || box.width === 0 || box.height === 0) {
      setRect(null);
      return;
    }
    setRect({ left: box.left - PAD, top: box.top - PAD, right: box.right + PAD, bottom: box.bottom + PAD, width: box.width + PAD * 2, height: box.height + PAD * 2 });
  }, [step.target]);

  useLayoutEffect(() => {
    const element = step.target ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`) : null;
    element?.scrollIntoView({ block: "nearest" });
    measure();
    window.addEventListener("resize", measure);
    document.addEventListener("scroll", measure, true);
    return () => {
      window.removeEventListener("resize", measure);
      document.removeEventListener("scroll", measure, true);
    };
  }, [measure, step.target]);

  // 卡片尺寸要等渲染后才知道，所以摆放放在第二个 layout effect。
  useLayoutEffect(() => {
    const element = card.current;
    if (!element) return;
    const size = { width: element.offsetWidth, height: element.offsetHeight };
    setCardPos(rect
      ? placeCard(rect, size, viewport)
      : { left: Math.max(12, (viewport.width - size.width) / 2), top: Math.max(12, (viewport.height - size.height) / 2) });
  }, [rect, index, viewport]);

  return createPortal(
    <div className="tour-root" role="dialog" aria-modal="true" aria-labelledby="tour-title" aria-describedby="tour-description">
      {/* 有目标：用一圈超大阴影压暗其余区域，目标区域保持原样；没有目标：整屏压暗。 */}
      {rect ? (
        <div className="tour-spotlight" style={{ left: rect.left, top: rect.top, width: rect.width, height: rect.height }} />
      ) : (
        <div className="tour-dim" />
      )}
      <div
        className="tour-card"
        data-centered={rect ? undefined : "true"}
        ref={card}
        tabIndex={-1}
        onKeyDown={(event) => {
          // Enter / Space 留给当前按钮；方向键不干扰输入控件或输入法。
          if (event.defaultPrevented || event.nativeEvent.isComposing || event.altKey || event.ctrlKey || event.metaKey
            || (event.target as HTMLElement).closest('input, textarea, select, [contenteditable="true"], [role="slider"], [role="radiogroup"]')) return;
          if (event.key === "ArrowRight") {
            event.preventDefault();
            if (last) onClose(true);
            else setIndex((current) => Math.min(steps.length - 1, current + 1));
          } else if (event.key === "ArrowLeft") {
            event.preventDefault();
            setIndex((current) => Math.max(0, current - 1));
          }
        }}
        style={cardPos ? { left: cardPos.left, top: cardPos.top } : { visibility: "hidden" }}
      >
        <button aria-label={t("跳过引导")} className="tour-close" onClick={() => onClose(false)} title={t("跳过引导")} type="button">
          <X aria-hidden="true" size={14} />
        </button>
        <span className="tour-step">{tf("{0} / {1}", [index + 1, steps.length])}</span>
        <h3 id="tour-title">{text(step.title, step.plainTitle ? undefined : step.args)}</h3>
        {/* 文案里写的是 ⌘K，Windows / Linux 换成真实键位 Ctrl K。 */}
        <p id="tour-description">{text(step.body, step.args).replace("⌘K", paletteShortcut)}</p>
        <div className="tour-dots" aria-hidden="true">
          {steps.map((item, position) => <i data-active={position === index} key={item.id} />)}
        </div>
        <div className="tour-actions">
          {index === 0 ? (
            <button className="tour-ghost" onClick={() => onClose(false)} type="button">{t("跳过")}</button>
          ) : (
            <button className="tour-ghost" onClick={() => setIndex(index - 1)} type="button">
              <ArrowLeft aria-hidden="true" size={13} />
              {t("上一步")}
            </button>
          )}
          <button className="tour-primary" data-autofocus onClick={() => (last ? onClose(true) : setIndex(index + 1))} type="button">
            {index === 0 ? t("开始") : last ? t("完成") : t("下一步")}
            {last ? null : <ArrowRight aria-hidden="true" size={13} />}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
