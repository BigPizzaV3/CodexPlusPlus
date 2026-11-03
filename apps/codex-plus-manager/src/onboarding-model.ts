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

// 顺序跟着侧栏从上往下走：万物皆拓展 → 聚合网关（总览、供应商池、Agent 接入）→ Agent 增强 → 搜索 → 设置。
export const TOUR_STEPS: TourStep[] = [
  {
    id: "welcome",
    title: "欢迎使用 Codex++",
    body: "Codex++ 给 Codex、Claude Code 等 Agent 加上拓展和增强，并让它们经同一个本地网关接入任意供应商。花半分钟认识一下侧栏。",
  },
  {
    id: "management",
    target: "management",
    title: "万物皆拓展",
    body: "Codex++ 的增强能力都以拓展开放：在这里启用内置拓展、安装社区拓展，或在 Codex 对话里说一句就造一个自己的。下面是 MCP、Skills 和会话。",
  },
  {
    id: "hub",
    target: "hub",
    title: "聚合网关",
    body: "任意 Agent 经 Codex++ 接入任意供应商：一个 Agent 可以同时用多家，按模型路由、出错自动换下一家。总览里一屏看全谁在用、走哪家。",
  },
  {
    id: "providers",
    target: "nav-gatewayProviders",
    title: "第一步：添加供应商",
    body: "把 API Key 或 ChatGPT、Copilot 等订阅加进供应商池，所有 Agent 共用这一个池子。",
  },
  {
    id: "agents-connect",
    target: "nav-gateway",
    title: "第二步：接入 Agent",
    body: "给 Agent 选一个默认模型、打开接入开关；供应商池里的模型就会出现在它自己的模型列表里。",
  },
  {
    id: "agents",
    target: "agents",
    title: "Agent 增强",
    body: "每个 Agent 一行，点开后它的页面列在下面：概览、连接与供应商、内置拓展、皮肤……Codex 行的播放键可以启动或停止 Codex。",
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
    body: "主题、版本更新、日志诊断和各项高级设置都在齿轮里。想再看一遍这个引导，在「设置 → 通用」里重新打开。",
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
      body: "Codex++ 给 {0} 加上拓展和增强，并让它经本地网关接入任意供应商。花半分钟认识一下侧栏。",
    },
    "agents-connect": {
      title: "第二步：让 {0} 走网关",
      body: "打开 {0} 的网关开关并选一个默认模型；供应商池里的模型就会出现在 {0} 自己的模型列表里。",
    },
    agents: {
      title: "{0} 的页面",
      body: "点开 {0} 后它的页面列在下面：概览、连接与供应商、内置拓展、皮肤……其他 Agent 的入口可在设置里切到「全部功能」后显示。",
    },
  };
  return TOUR_STEPS.map((step) => (override[step.id] ? { ...step, ...override[step.id], args: [soloName], plainTitle: !override[step.id].title } : step));
}

/**
 * 只用增强功能时的步骤：没有网关、供应商池和 Agent 接入，这三步去掉，其余换成不提供应商的说法。
 */
const ENHANCE_OVERRIDES: Record<string, Pick<TourStep, "body">> = {
  welcome: { body: "Codex++ 给 Codex 和 Claude Code 加上增强、皮肤、拓展等功能，供应商仍由你自己的工具管理。花半分钟认识一下侧栏。" },
  agents: { body: "点开 Codex 或 Claude Code，在它下面的页面里启用所需增强；Codex 行的播放键可以启动或停止 Codex。" },
  management: { body: "Codex++ 的增强能力都以拓展开放：在这里启用内置拓展、安装社区拓展，或在对话里造一个自己的。MCP、Skills 和会话就在它下面。" },
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
