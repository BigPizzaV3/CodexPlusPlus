/**
 * Agent 桌宠设置（与 Rust `AgentPetSettings` 对应，JSON 键 `agentPet`）。
 *
 * 图片不随应用打包：内置图库只是一组 id，图片由后端运行时联网拉取并缓存；
 * 自定义图片填 https 地址。目前只有 Codex 可以显示桌宠，其他 Agent 后续接入。
 *
 * 音效组：`duck` / `fx1` 随应用内嵌；其余 id 来自后端的可下载音效目录，用到时才下载；
 * `custom` 使用用户导入的本地音频（文件名存在 `customSoundPress` / `customSoundRelease`）。
 */
export type AgentPetAgent = "codex" | "claude" | "grok";
/** 内嵌音效组、可下载音效组 id 或 `custom`。 */
export type AgentPetSoundSet = string;
export type AgentPetSoundSlot = "press" | "release";

export type AgentPetSettings = {
  enabled: boolean;
  agents: AgentPetAgent[];
  image: string;
  customImageUrl: string;
  size: number;
  soundEnabled: boolean;
  soundSet: AgentPetSoundSet;
  customSoundPress: string;
  customSoundRelease: string;
  volume: number;
};

/** 已经接入的 Agent。其余的在页面上显示为「即将支持」。 */
export const AGENT_PET_SUPPORTED_AGENTS: readonly AgentPetAgent[] = ["codex"];
export const AGENT_PET_CUSTOM_IMAGE = "custom";
export const AGENT_PET_CUSTOM_SOUND = "custom";
/** 随应用内嵌、不用下载的音效组。 */
export const AGENT_PET_EMBEDDED_SOUND_SETS: readonly string[] = ["duck", "fx1"];
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
    customSoundPress: "",
    customSoundRelease: "",
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
    // 音效组是否存在由后端校验（下载目录在后端），这里只保证是个非空字符串。
    soundSet: typeof value.soundSet === "string" && value.soundSet.trim() ? value.soundSet.trim() : defaults.soundSet,
    customSoundPress: typeof value.customSoundPress === "string" ? value.customSoundPress.trim() : "",
    customSoundRelease: typeof value.customSoundRelease === "string" ? value.customSoundRelease.trim() : "",
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

/** 一组音效的缓存键，与后端 `agent_pet::load_sound` 返回的 key 同一规则：自定义音效带文件名。 */
export function agentPetSoundKey(settings: Pick<AgentPetSettings, "soundSet" | "customSoundPress" | "customSoundRelease">): string {
  return settings.soundSet === AGENT_PET_CUSTOM_SOUND
    ? `${AGENT_PET_CUSTOM_SOUND}:${settings.customSoundPress}:${settings.customSoundRelease}`
    : settings.soundSet;
}
