import { localScriptState, type LocalScriptState } from "./script-market-model.ts";

export type ExtensionHost = "codex" | "claude";

type HostExtension = {
  enabled: boolean;
  claude_enabled?: boolean | null;
  status: string;
  package?: { claudeDesktop?: Record<string, unknown> | null } | null;
};

/** Claude 页面上次加载一个拓展包的结果（页面里的拓展运行时回报的），按脚本 key。 */
export type ClaudePageStatus = { status: string; error: string; unsupported: string[]; loadedAt: string };

const declared = (value: unknown) => (Array.isArray(value) ? value.length > 0 : !!value && typeof value === "object" && Object.keys(value).length > 0);

/**
 * 一个拓展包在 Claude 里有哪两部分：`page` 是注入 Claude 页面的脚本和样式（Chat、Cowork、Code 页面都生效，
 * 刷新页面就按新的来），`mods` 是 Code 会话里的按钮、工具、事件（走 Claude Code Mods，重启 Claude 后加载）。
 */
export function claudeParts(script: Pick<HostExtension, "package">): { page: boolean; mods: boolean } {
  const claude = script.package?.claudeDesktop;
  return {
    page: !!claude?.page,
    mods: !!claude && (declared(claude.buttons) || declared(claude.events) || declared(claude.tools)),
  };
}

export function extensionHostForTool(tool: string): ExtensionHost {
  return tool === "claude" ? "claude" : "codex";
}

export function extensionsForHost<T extends HostExtension>(scripts: readonly T[], host: ExtensionHost): T[] {
  return scripts.filter((script) => host === "codex" || Boolean(script.package?.claudeDesktop));
}

export function extensionEnabledForHost(script: HostExtension, host: ExtensionHost): boolean {
  return host === "claude" ? script.claude_enabled ?? script.enabled : script.enabled;
}

/**
 * Claude 下不拿 Codex 的运行状态充数：有页面部分的包看 Claude 页面回报的结果（`pageStatus`），
 * 没回报过、或只有 Code 会话那部分的包一律算「还没加载」。
 */
export function extensionStateForHost(
  script: HostExtension,
  host: ExtensionHost,
  globalEnabled: boolean,
  pageStatus?: ClaudePageStatus | null,
): LocalScriptState {
  if (host === "codex") return localScriptState(script, globalEnabled);
  if (!extensionEnabledForHost(script, host)) return "off";
  if (!globalEnabled) return "globalOff";
  if (claudeParts(script).page && pageStatus) {
    if (pageStatus.status === "failed" || (pageStatus.status === "loaded" && pageStatus.error)) return "failed";
    if (pageStatus.status === "loaded") return "loaded";
  }
  return "notLoaded";
}
