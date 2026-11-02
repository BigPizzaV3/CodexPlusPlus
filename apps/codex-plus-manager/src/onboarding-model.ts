/** 首次使用引导：步骤定义与「看过没有」的记录。页面只负责高亮与摆放说明卡。 */

export type TourStep = {
  id: string;
  /** 要高亮的元素：`[data-tour="..."]`；没有目标的步骤（欢迎、结束）显示在屏幕中央。 */
  target?: string;
  title: string;
  body: string;
  /** 有值时标题和正文是 tf 模板，按它填 {0}。 */
  args?: string[];
  /** 只有正文是模板、标题仍是普通文案。 */
  plainTitle?: boolean;
};

/** 导览版本；入门清单使用独立记录，不因内容更新打扰已完成引导的用户。 */
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
    body: "这里显示本地网关的运行状态和端口。结束导览后，从这里打开总览查看 Agent、供应商和最近请求。",
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
    body: "为 Agent 选择默认模型，再打开接入开关。接入后，供应商池里的模型会出现在 Agent 自己的模型列表里。",
  },
  {
    id: "agents",
    target: "agents",
    title: "每个 Agent 的专属设置",
    body: "每个 Agent 一行，进入后管理它的增强与皮肤。Codex 行的播放键可以启动或停止 Codex；拓展在「万物皆拓展」中统一管理。",
  },
  {
    id: "management",
    target: "management",
    title: "全局管理",
    body: "从「万物皆拓展」启用内置拓展或安装社区拓展；会话、用量、MCP 和 Skills 也在这里管理。",
  },
  {
    id: "palette",
    target: "palette",
    title: "快速跳转",
    body: "结束导览后，按 ⌘K 搜索页面和常用操作。",
  },
  {
    id: "settings",
    target: "settings",
    title: "设置与更新",
    body: "主题、配置备份、版本更新和日志都在这里。想再看一遍这个引导，也在设置里重新打开。",
  },
];

/**
 * 按使用模式调整的步骤。单 Agent 模式下不讲「多个 Agent 共用」，
 * 欢迎卡、接入、Agent 列表三步换成只针对这一个 Agent 的说法（文案里的 {0} 是 Agent 名字）。
 */
export function tourSteps(soloName: string | null, enhanceOnly = false): TourStep[] {
  if (enhanceOnly) return ENHANCE_TOUR_STEPS;
  if (!soloName) return TOUR_STEPS;
  const override: Record<string, Partial<Pick<TourStep, "title" | "body">>> = {
    welcome: {
      body: "Codex++ 让 {0} 经本地网关接入任意供应商。花半分钟认识一下各个入口。",
    },
    "agents-connect": {
      title: "第二步：让 {0} 走网关",
      body: "打开 {0} 的网关开关并选一个默认模型；供应商池里的模型就会出现在 {0} 自己的模型列表里。",
    },
    agents: {
      title: "{0} 的专属设置",
      body: "在 {0} 页面管理增强和皮肤，在「万物皆拓展」启用内置拓展或安装社区拓展。其他 Agent 的入口可在设置里切到「全部功能」后显示。",
    },
  };
  return TOUR_STEPS.map((step) => (override[step.id] ? { ...step, ...override[step.id], args: [soloName], plainTitle: !override[step.id].title } : step));
}

/**
 * 只用增强功能时的步骤：没有网关、供应商池和 Agent 接入，这三步去掉，其余换成不提供应商的说法。
 */
const ENHANCE_OVERRIDES: Record<string, Pick<TourStep, "body">> = {
  welcome: { body: "Codex++ 给 Codex 和 Claude Code 加上增强、皮肤、拓展等功能，供应商仍由你自己的工具管理。花半分钟认识一下各个入口。" },
  agents: { body: "进入 Codex 或 Claude 的增强设置，启用所需功能；Codex 行的播放键可以启动或停止 Codex。" },
  management: { body: "从「万物皆拓展」启用内置拓展或安装社区拓展；会话、MCP 和 Skills 也在这里管理。" },
};
const ENHANCE_HIDDEN_STEPS = new Set(["hub", "providers", "agents-connect"]);
const ENHANCE_TOUR_STEPS: TourStep[] = TOUR_STEPS
  .filter((step) => !ENHANCE_HIDDEN_STEPS.has(step.id))
  .map((step) => (ENHANCE_OVERRIDES[step.id] ? { ...step, ...ENHANCE_OVERRIDES[step.id] } : step));

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
