/**
 * 弹层的 Esc 栈：弹窗、下拉、浮层打开时各自登记一层，按一次 Esc 只关最上面那层。
 *
 * 背景：以前每个弹层自己在 document / window 上挂 keydown，内层下拉、确认框和外层弹窗会被同一次 Esc
 * 一起关掉（价格弹窗里收下拉会丢掉整张表）。靠「下拉开着没有」去判断也不可靠：React 19 下，
 * 前一个监听器里的 setState 会在两个监听器之间以微任务提交，后一个监听器看到的已经是关掉后的 DOM。
 *
 * 所以只挂一个全局监听（window 捕获阶段，先于所有其它监听），由它交给栈顶那一层处理，
 * 并阻止这次按键继续传播。纯逻辑放在这里，方便用 node:test 覆盖；React 用法见 `use-escape-layer.ts`。
 */

export type EscapeKeyEvent = {
  key: string;
  isComposing?: boolean;
  preventDefault(): void;
  stopImmediatePropagation(): void;
};

type Layer = { close: () => void };

const layers: Layer[] = [];
let installed = false;

/** 登记一层（越晚登记越靠上），返回注销函数。同一层注销多次没有副作用。 */
export function pushEscapeLayer(close: () => void): () => void {
  const layer: Layer = { close };
  layers.push(layer);
  return () => {
    const index = layers.lastIndexOf(layer);
    if (index >= 0) layers.splice(index, 1);
  };
}

/** 处理一次按键：是 Esc 且有弹层时，只关栈顶那层并吞掉这次按键，返回是否处理了。 */
export function handleEscapeKey(event: EscapeKeyEvent): boolean {
  // 输入法组字时的 Esc 是取消组字，不关弹层。
  if (event.key !== "Escape" || event.isComposing) return false;
  const top = layers[layers.length - 1];
  if (!top) return false;
  event.preventDefault();
  event.stopImmediatePropagation();
  top.close();
  return true;
}

/** 在 window 捕获阶段挂唯一的监听（只挂一次）。 */
export function installEscapeListener(target: Pick<Window, "addEventListener"> | undefined = typeof window === "undefined" ? undefined : window) {
  if (installed || !target) return;
  installed = true;
  target.addEventListener("keydown", (event) => void handleEscapeKey(event as KeyboardEvent), true);
}

export function escapeLayerCount(): number {
  return layers.length;
}

/** 仅供测试：清空栈。 */
export function resetEscapeLayers() {
  layers.length = 0;
}
