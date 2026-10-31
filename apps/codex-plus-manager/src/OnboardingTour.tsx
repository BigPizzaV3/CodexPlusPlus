/**
 * 首次使用引导：压暗整个界面，只露出当前步骤的目标（`[data-tour="..."]`），旁边一张说明卡。
 * 目标找不到（例如所在分组被折叠）或步骤没有目标时，说明卡显示在屏幕中央。
 */
import { useCallback, useEffect, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ArrowLeft, ArrowRight, X } from "lucide-react";
import { t, tf } from "./i18n";
import { paletteShortcut } from "./command-palette-model";
import { placeCard, TOUR_STEPS } from "./onboarding-model";
import { useEscapeLayer } from "./use-escape-layer";
import "./OnboardingTour.css";

type Rect = { left: number; top: number; right: number; bottom: number; width: number; height: number };

const PAD = 6;

export function OnboardingTour({ onClose }: { onClose: (finished: boolean) => void }) {
  useEscapeLayer(true, () => onClose(false));
  const [index, setIndex] = useState(0);
  const [rect, setRect] = useState<Rect | null>(null);
  const [cardPos, setCardPos] = useState<{ left: number; top: number } | null>(null);
  const card = useRef<HTMLDivElement>(null);
  const step = TOUR_STEPS[index];
  const last = index === TOUR_STEPS.length - 1;

  // 量目标位置；窗口变化、侧栏滚动时重新量。
  const measure = useCallback(() => {
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
    const viewport = { width: window.innerWidth, height: window.innerHeight };
    setCardPos(rect
      ? placeCard(rect, size, viewport)
      : { left: (viewport.width - size.width) / 2, top: (viewport.height - size.height) / 2 });
  }, [rect, index]);

  useEffect(() => {
    // Esc 走 Esc 栈（下面的 useEscapeLayer），只关引导本身，不连带关掉下面的弹窗。
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "ArrowRight" || event.key === "Enter") {
        event.preventDefault();
        if (last) onClose(true);
        else setIndex((current) => current + 1);
      } else if (event.key === "ArrowLeft") setIndex((current) => Math.max(0, current - 1));
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [last, onClose]);

  return createPortal(
    <div className="tour-root" role="dialog" aria-modal="true" aria-labelledby="tour-title">
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
        style={cardPos ? { left: cardPos.left, top: cardPos.top } : { visibility: "hidden" }}
      >
        <button aria-label={t("跳过引导")} className="tour-close" onClick={() => onClose(false)} title={t("跳过引导")} type="button">
          <X aria-hidden="true" size={14} />
        </button>
        <span className="tour-step">{tf("{0} / {1}", [index + 1, TOUR_STEPS.length])}</span>
        <h3 id="tour-title">{t(step.title)}</h3>
        {/* 文案里写的是 ⌘K，Windows / Linux 换成真实键位 Ctrl K。 */}
        <p>{t(step.body).replace("⌘K", paletteShortcut)}</p>
        <div className="tour-dots" aria-hidden="true">
          {TOUR_STEPS.map((item, position) => <i data-active={position === index} key={item.id} />)}
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
          <button className="tour-primary" onClick={() => (last ? onClose(true) : setIndex(index + 1))} type="button">
            {index === 0 ? t("开始") : last ? t("完成") : t("下一步")}
            {last ? null : <ArrowRight aria-hidden="true" size={13} />}
          </button>
        </div>
      </div>
    </div>,
    document.body,
  );
}
