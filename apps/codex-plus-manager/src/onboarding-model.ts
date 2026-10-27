/** 首次使用引导：步骤定义与「看过没有」的记录。页面只负责高亮与摆放说明卡。 */

export type TourStep = {
  id: string;
  /** 要高亮的元素：`[data-tour="..."]`；没有目标的步骤（欢迎、结束）显示在屏幕中央。 */
  target?: string;
  title: string;
  body: string;
};

/** 引导版本：步骤大改时加一，老用户会再看到一次。 */
export const TOUR_VERSION = 1;
export const TOUR_STORAGE_KEY = "codex-plus-plus.onboarding";

export const TOUR_STEPS: TourStep[] = [
  {
    id: "welcome",
    title: "欢迎使用 Codex++",
    body: "Codex++ 让任意 Agent（Codex、Claude Code、Grok…）经同一个本地网关接入任意供应商。花半分钟认识一下各个入口。",
  },
  {
    id: "hub",
    target: "hub",
    title: "网关状态",
    body: "这里显示本地网关是否在运行和它的端口。点一下打开网关总览：谁在用、走哪家、是否健康，一屏看全。",
  },
  {
    id: "providers",
    target: "nav-gatewayProviders",
    title: "第一步：添加供应商",
    body: "在供应商池里添加 API Key，或直接登录 ChatGPT、Copilot 等订阅。所有 Agent 共用这一个池子。",
  },
  {
    id: "agents-connect",
    target: "nav-gateway",
    title: "第二步：接入 Agent",
    body: "选择哪些 Agent 经网关接入，并给每个 Agent 指定默认模型。打开「接入」后，模型会出现在 Agent 自己的模型列表里。",
  },
  {
    id: "agents",
    target: "agents",
    title: "每个 Agent 的专属设置",
    body: "每个 Agent 一行：绿点表示已接入网关。点进去管理它自己的增强、皮肤、插件等；Codex 行的播放键可以直接启动或停止 Codex。",
  },
  {
    id: "management",
    target: "management",
    title: "全局管理",
    body: "请求日志、会话、MCP 和 Skills 在这里统一管理，一次覆盖所有 Agent。",
  },
  {
    id: "palette",
    target: "palette",
    title: "快速跳转",
    body: "随时按 ⌘K 搜索页面和常用操作，不用一层层点。",
  },
  {
    id: "settings",
    target: "settings",
    title: "设置与更新",
    body: "主题、配置备份、版本更新和日志都在这里。想再看一遍这个引导，也在设置里重新打开。",
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
