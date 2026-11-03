import { useEffect, type RefObject } from "react";

const scopes: HTMLElement[] = [];
const modalSelector = '[aria-modal="true"], [role="alertdialog"]';
const focusableSelector = 'a[href], button, input, select, textarea, [tabindex], [contenteditable="true"]';

/** 只约束使用此 hook 的弹层；新打开的应用内确认框仍可接管焦点。 */
export function useModalFocus(ref: RefObject<HTMLElement | null>, enabled = true) {
  useEffect(() => {
    const root = ref.current;
    if (!enabled || !root) return;
    const doc = root.ownerDocument;
    const win = doc.defaultView;
    const previous = doc.activeElement as HTMLElement | null;
    const existingModals = new Set(doc.querySelectorAll<HTMLElement>(modalSelector));
    let lastFocused: HTMLElement | null = null;
    scopes.push(root);

    const visible = (element: HTMLElement) => element.getClientRects().length > 0
      && win?.getComputedStyle(element).visibility !== "hidden";
    const controls = () => [...root.querySelectorAll<HTMLElement>(focusableSelector)].filter((element) =>
      element.tabIndex >= 0 && !element.matches(":disabled") && !element.closest("[hidden], [inert]") && visible(element));
    const active = () => scopes.at(-1) === root && ![...doc.querySelectorAll<HTMLElement>(modalSelector)].some((modal) =>
      !existingModals.has(modal) && !root.contains(modal) && !modal.contains(root) && visible(modal));
    const focusInside = () => {
      const available = controls();
      const preferred = available.find((element) => element.hasAttribute("data-autofocus"));
      const target = lastFocused && available.includes(lastFocused) ? lastFocused : preferred ?? available[0] ?? root;
      target.focus({ preventScroll: true });
    };
    const onFocus = (event: FocusEvent) => {
      if (!active()) return;
      if (root.contains(event.target as Node)) lastFocused = event.target as HTMLElement;
      else focusInside();
    };
    const onKey = (event: KeyboardEvent) => {
      if (!active() || event.defaultPrevented || event.key !== "Tab") return;
      const available = controls();
      const index = available.indexOf(doc.activeElement as HTMLElement);
      if (index < 0 || (event.shiftKey ? index === 0 : index === available.length - 1)) {
        event.preventDefault();
        (event.shiftKey ? available.at(-1) ?? root : available[0] ?? root).focus({ preventScroll: true });
      }
    };
    doc.addEventListener("focusin", onFocus);
    doc.addEventListener("keydown", onKey);
    focusInside();
    // 导览先量尺寸再显示；下一帧补上首次渲染时尚不可见的控件。
    const frame = win?.requestAnimationFrame(() => {
      if (active() && (doc.activeElement === root || !root.contains(doc.activeElement))) focusInside();
    });
    return () => {
      doc.removeEventListener("focusin", onFocus);
      doc.removeEventListener("keydown", onKey);
      if (frame !== undefined) win?.cancelAnimationFrame(frame);
      const wasTop = scopes.at(-1) === root;
      const index = scopes.lastIndexOf(root);
      if (index >= 0) scopes.splice(index, 1);
      if (wasTop && previous?.isConnected && (root.contains(doc.activeElement) || doc.activeElement === doc.body)) {
        previous.focus({ preventScroll: true });
      }
    };
  }, [ref, enabled]);
}
