/**
 * Agent 桌宠设置（与 Rust `AgentPetSettings` 对应，JSON 键 `agentPet`）。
 *
 * 图片不随应用打包：内置图库只是一组 id，图片由后端运行时联网拉取并缓存；
 * 自定义图片填 https 地址。目前只有 Codex 可以显示桌宠，其他 Agent 后续接入。
 */
export type AgentPetAgent = "codex" | "claude" | "grok";
export type AgentPetSoundSet = "duck" | "fx1";

export type AgentPetSettings = {
  enabled: boolean;
  agents: AgentPetAgent[];
  image: string;
  customImageUrl: string;
  size: number;
  soundEnabled: boolean;
  soundSet: AgentPetSoundSet;
  volume: number;
};

/** 已经接入的 Agent。其余的在页面上显示为「即将支持」。 */
export const AGENT_PET_SUPPORTED_AGENTS: readonly AgentPetAgent[] = ["codex"];
export const AGENT_PET_CUSTOM_IMAGE = "custom";
export const AGENT_PET_SIZE_MIN = 64;
export const AGENT_PET_SIZE_MAX = 240;
const AGENT_PET_DEFAULT_SIZE = 112;
const AGENT_PET_DEFAULT_VOLUME = 0.6;

export function defaultAgentPetSettings(): AgentPetSettings {
  return {
    enabled: false,
    agents: ["codex"],
    image: "whale",
    customImageUrl: "",
    size: AGENT_PET_DEFAULT_SIZE,
    soundEnabled: true,
    soundSet: "duck",
    volume: AGENT_PET_DEFAULT_VOLUME,
  };
}

const clamp = (value: number, min: number, max: number) => Math.min(max, Math.max(min, value));

/**
 * 规范化。`legacyWidgetEnabled` 是旧版「Codex 用量挂件」的开关：
 * 没有保存过桌宠设置、但旧挂件开着的用户，升级后桌宠默认保持开启。
 */
export function normalizeAgentPetSettings(raw: unknown, legacyWidgetEnabled = false): AgentPetSettings {
  const defaults = defaultAgentPetSettings();
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
    return { ...defaults, enabled: legacyWidgetEnabled === true };
  }
  const value = raw as Partial<Record<keyof AgentPetSettings, unknown>>;
  const agents = Array.isArray(value.agents)
    ? value.agents.filter((agent): agent is AgentPetAgent => AGENT_PET_SUPPORTED_AGENTS.includes(agent as AgentPetAgent))
    : defaults.agents;
  const size = typeof value.size === "number" && Number.isFinite(value.size)
    ? Math.round(clamp(value.size, AGENT_PET_SIZE_MIN, AGENT_PET_SIZE_MAX))
    : defaults.size;
  const volume = typeof value.volume === "number" && Number.isFinite(value.volume)
    ? clamp(value.volume, 0, 1)
    : defaults.volume;
  const image = typeof value.image === "string" && value.image.trim() ? value.image.trim() : defaults.image;
  return {
    enabled: value.enabled === true,
    agents: Array.from(new Set(agents)),
    image,
    customImageUrl: typeof value.customImageUrl === "string" ? value.customImageUrl.trim() : "",
    size,
    soundEnabled: value.soundEnabled !== false,
    soundSet: value.soundSet === "fx1" ? "fx1" : "duck",
    volume,
  };
}

export type AgentPetIssue = "customUrl";

/** 只有选了自定义图片时才校验地址：必须是不带账号信息的 https 地址。 */
export function agentPetSettingsIssue(settings: AgentPetSettings): AgentPetIssue | null {
  if (settings.image !== AGENT_PET_CUSTOM_IMAGE) return null;
  return isAgentPetImageUrl(settings.customImageUrl) ? null : "customUrl";
}

export function isAgentPetImageUrl(value: string): boolean {
  try {
    const url = new URL(value.trim());
    return url.protocol === "https:" && !url.username && !url.password && Boolean(url.hostname);
  } catch {
    return false;
  }
}
