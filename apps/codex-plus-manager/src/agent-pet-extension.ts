/**
 * Agent 桌宠已改成随 Codex++ 发布的官方拓展包（脚本 key `bundled:agent-pet`）：设置存在这个拓展的配置里，
 * 开关就是拓展的启停。管理器「Agent 桌宠」页仍用原来的界面，这里负责两边的换算。
 */
import { normalizeAgentPetSettings, type AgentPetSettings } from "./agent-pet-settings.ts";
import { normalizeWhaleBalanceSettings, type WhaleBalanceSettings } from "./whale-settings.ts";

export const AGENT_PET_EXTENSION_KEY = "bundled:agent-pet";

export type AgentPetExtensionState = { pet: AgentPetSettings; whale: WhaleBalanceSettings };

/** 拓展配置 + 启停 → 页面用的桌宠设置与余额设置。 */
export function agentPetStateFromExtension(config: Record<string, unknown> | null | undefined, enabled: boolean): AgentPetExtensionState {
  const raw = config && typeof config === "object" ? config : {};
  return {
    pet: normalizeAgentPetSettings({ ...raw, enabled, agents: ["codex"] }),
    whale: normalizeWhaleBalanceSettings(raw as Partial<WhaleBalanceSettings>),
  };
}

/** 页面上的设置 → 拓展配置（只含清单 config 里声明过的键）。启停另走拓展开关。 */
export function agentPetExtensionConfig(state: AgentPetExtensionState): Record<string, unknown> {
  const { pet, whale } = state;
  return {
    kind: pet.kind,
    image: pet.image,
    customImageUrl: pet.customImageUrl,
    size: pet.size,
    soundEnabled: pet.soundEnabled,
    soundSet: pet.soundSet,
    customSoundPress: pet.customSoundPress,
    customSoundRelease: pet.customSoundRelease,
    volume: pet.volume,
    codexAppWhaleBalanceProtocol: whale.codexAppWhaleBalanceProtocol,
    codexAppWhaleBalancePath: whale.codexAppWhaleBalancePath,
    codexAppWhaleBalanceField: whale.codexAppWhaleBalanceField,
    codexAppWhaleBalanceCurrency: whale.codexAppWhaleBalanceCurrency,
    codexAppWhaleBalanceScale: whale.codexAppWhaleBalanceScale,
  };
}
