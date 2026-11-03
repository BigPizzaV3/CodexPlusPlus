import { localScriptState, type LocalScriptState } from "./script-market-model.ts";

export type ExtensionHost = "codex" | "claude";

type HostExtension = {
  enabled: boolean;
  claude_enabled?: boolean | null;
  status: string;
  package?: { claudeDesktop?: Record<string, unknown> | null } | null;
};

export function extensionHostForTool(tool: string): ExtensionHost {
  return tool === "claude" ? "claude" : "codex";
}

export function extensionsForHost<T extends HostExtension>(scripts: readonly T[], host: ExtensionHost): T[] {
  return scripts.filter((script) => host === "codex" || Boolean(script.package?.claudeDesktop));
}

export function extensionEnabledForHost(script: HostExtension, host: ExtensionHost): boolean {
  return host === "claude" ? script.claude_enabled ?? script.enabled : script.enabled;
}

export function extensionStateForHost(
  script: HostExtension,
  host: ExtensionHost,
  globalEnabled: boolean,
): LocalScriptState {
  if (host === "codex") return localScriptState(script, globalEnabled);
  if (!extensionEnabledForHost(script, host)) return "off";
  if (!globalEnabled) return "globalOff";
  return "notLoaded";
}
