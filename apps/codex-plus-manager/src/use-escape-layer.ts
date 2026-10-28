import { useEffect, useRef } from "react";
import { installEscapeListener, pushEscapeLayer } from "./escape-layer";

/**
 * 弹层打开期间登记进 Esc 栈：按 Esc 时只有最上面那层的 `onEscape` 会被调用。
 *
 * 只按 `active` 登记 / 注销：回调每次渲染都可能是新函数（父组件轮询刷新时尤其如此），
 * 用 ref 取最新值，避免重新登记把外层弹窗挪到栈顶、盖过后打开的下拉。
 * 注意：父子同时挂载时子组件的 effect 先跑，所以同一次渲染里同时打开的内外两层顺序会反过来；
 * 网关里的内层弹层都是在外层打开之后再由用户操作打开的。
 */
export function useEscapeLayer(active: boolean, onEscape: () => void) {
  const handler = useRef(onEscape);
  handler.current = onEscape;
  useEffect(() => {
    if (!active) return;
    installEscapeListener();
    return pushEscapeLayer(() => handler.current());
  }, [active]);
}
