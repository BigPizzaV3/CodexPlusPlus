// 临时占位组件：373b3e23 的 ComputerUsePage.tsx 引用了 ./ComputerUsePreview，
// 但该文件从未随提交进入仓库，导致 main 与所有 PR 的「PR build artifacts」
// 工作流（tsc --noEmit 与 vite build）自该提交起全部失败。
// 本文件只提供同名导出让构建通过，不渲染任何内容；真实的预览组件落地后
// 直接整体替换本文件即可（引用处无 props，替换零成本）。
export function ComputerUsePreview() {
  return null;
}
