/** 界面导览：步骤定义与「看过没有」的记录。页面只负责高亮与摆放说明卡。 */

export type TourStep = {
  id: string;
  /** 要高亮的元素：`[data-tour="..."]`；没有目标的步骤（欢迎）显示在屏幕中央。 */
  target?: string;
  title: string;
  body: string;
};

/** 导览版本；启动引导使用独立记录，不因内容更新打扰已完成引导的用户。 */
export const TOUR_VERSION = 1;
export const TOUR_STORAGE_KEY = "codex-plus-plus.onboarding";

// 顺序跟着侧栏从上往下走：万物皆拓展 → Agent → 搜索 → 设置。
// 本地服务（共享供应商、路由、Agent 接入）是侧栏第一组里的一个入口，在 Agent 一步里顺带提一句，不单独讲。
export const TOUR_STEPS: TourStep[] = [
  {
    id: "welcome",
    title: "欢迎使用 Codex++",
    body: "Codex++ 给 Codex、Claude Code 等 Agent 加上拓展和增强。花半分钟认识一下侧栏。",
  },
  {
    id: "management",
    target: "management",
    title: "万物皆拓展",
    body: "Codex++ 的增强能力都以拓展开放：在这里启用内置拓展、安装社区拓展，或在 Codex 对话里说一句就造一个自己的。下面是 MCP、Skills 和会话。",
  },
  {
    id: "agents",
    target: "agents",
    title: "Agent",
    body: "每个 Agent 一行，点开后它的页面列在下面：概览、连接与供应商、内置拓展、皮肤……想让几个 Agent 共用一份供应商，到侧栏的「本地服务」里配置。",
  },
  {
    id: "palette",
    target: "palette",
    title: "搜索页面和操作",
    body: "结束导览后，按 ⌘K 搜索页面、开关和常用操作。",
  },
  {
    id: "settings",
    target: "settings",
    title: "设置",
    body: "主题、版本更新、日志诊断和各项高级设置都在齿轮里。想再看一遍这个导览，在「设置 → 通用」里重新打开。",
  },
];

/** 是否需要自动弹出引导：没看过，或看的是旧版本。 */
export function shouldShowTour(stored: string | null): boolean {
  if (!stored) return true;
  const version = Number(stored);
  return !Number.isFinite(version) || version < TOUR_VERSION;
}

/** 说明卡放在目标的右侧；右边放不下就放左侧，再不行放到下方。返回卡片左上角坐标。 */
export function placeCard(
  target: { left: number; top: number; right: number; bottom: number },
  card: { width: number; height: number },
  viewport: { width: number; height: number },
  gap = 14,
): { left: number; top: number; side: "right" | "left" | "bottom" } {
  const clampTop = (top: number) => Math.max(12, Math.min(top, viewport.height - card.height - 12));
  const middle = (target.top + target.bottom) / 2 - card.height / 2;
  if (target.right + gap + card.width <= viewport.width - 12) return { left: target.right + gap, top: clampTop(middle), side: "right" };
  if (target.left - gap - card.width >= 12) return { left: target.left - gap - card.width, top: clampTop(middle), side: "left" };
  const left = Math.max(12, Math.min(target.left, viewport.width - card.width - 12));
  return { left, top: clampTop(target.bottom + gap), side: "bottom" };
}
