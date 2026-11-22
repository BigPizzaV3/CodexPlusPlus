export type ManagerNavigationIntent = {
  page: "settings" | "enhance" | "pluginMarket" | "weixin" | "sessions";
  /** 旧版悬浮球 / Codex 设置页发出的分节（stepwise / dictation）。服务配置已搬进 Codex，管理器忽略它。 */
  section?: string;
};

export function managerNavigationDestination(intent: ManagerNavigationIntent): {
  route: ManagerNavigationIntent["page"];
} {
  if (intent.page === "pluginMarket") return { route: "pluginMarket" };
  // Codex 里「微信连接」页的「在 Codex++ 管理工具里打开更多设置」。
  if (intent.page === "weixin") return { route: "weixin" };
  // 旧版悬浮球发出的 settings + section 仍落到增强页（那里提示服务配置在 Codex 里改）。
  return { route: intent.section ? "enhance" : intent.page };
}
