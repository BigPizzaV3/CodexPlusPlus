// 开关行只有开关本身可点：点标题、说明、行内空白都不切换。
//
// 开关行的写法是 `<label>` 包着隐藏的 checkbox、文字和开关图形（ToggleVisual / .toggle-switch-visual），
// 浏览器默认点 label 里任何位置都会切换 checkbox。这里在 document 捕获阶段统一拦掉这个默认行为，
// 所有页面、以后新增的开关行都按同一条规则走，不用每一行各自处理。
// 不带开关图形的 label（普通勾选框、表单字段）不受影响。

/** 行里本来就该响应点击的东西：开关本身，以及说明里的链接、按钮等。 */
const INTERACTIVE = "input, .toggle-switch-visual, a, button, select, textarea, [role='button']";

/** 这次点击落在开关行里、但不在开关（或行内其它可点的控件）上：要拦掉 label 的默认切换。 */
export function blocksSwitchRowClick(target: EventTarget | null): boolean {
  const element = target as Element | null;
  if (!element || typeof element.closest !== "function") return false;
  const label = element.closest("label");
  if (!label || !label.querySelector(".toggle-switch-visual")) return false;
  const control = element.closest(INTERACTIVE);
  return !(control && label.contains(control));
}

/** 应用启动时装一次（main.tsx）。键盘操作隐藏的 checkbox 不经过这里，照常切换。 */
export function installSwitchOnlyClicks(doc: Document = document) {
  doc.addEventListener("click", (event) => {
    if (blocksSwitchRowClick(event.target)) event.preventDefault();
  }, true);
}
