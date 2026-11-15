export type ManagerNavigationIntent = {
  page: "settings" | "enhance" | "pluginMarket";
  /** 旧版悬浮球 / Codex 设置页发出的分节（stepwise / dictation）。服务配置已搬进 Codex，管理器忽略它。 */
  section?: string;
};

export function managerNavigationDestination(intent: ManagerNavigationIntent): {
  route: "settings" | "enhance" | "pluginMarket";
} {
  if (intent.page === "pluginMarket") return { route: "pluginMarket" };
  // 旧版悬浮球发出的 settings + section 仍落到增强页（那里提示服务配置在 Codex 里改）。
  return { route: intent.section ? "enhance" : intent.page };
}
