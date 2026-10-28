/**
 * Agent 桌宠的试听播放器：与 Codex 里 97-agent-pet.js 同一套衔接规则——
 * 按下播放按压音；点按时松开音排到按压音结束前 40ms，按住超过按压音时长则松开时立即播放。
 * 音效由后端 `agent_pet_sounds` 以 data URI 提供，这里解码成 AudioBuffer 缓存。
 */
import type { AgentPetSoundSet } from "./agent-pet-settings";

export type AgentPetSoundSources = Partial<Record<AgentPetSoundSet, { press?: string; release?: string }>>;
type Slot = "press" | "release";
type Playing = { source: AudioBufferSourceNode; startAt: number; duration: number };

export const AGENT_PET_RELEASE_LEAD_MS = 40;

/** 点按时松开音距现在的延迟（秒）：按压音结束前 lead 毫秒进场，已经过了就立即播放。 */
export function agentPetReleaseDelay(pressStartAt: number, pressDuration: number, now: number, leadMs = AGENT_PET_RELEASE_LEAD_MS): number {
  return Math.max(0, pressStartAt + pressDuration - leadMs / 1000 - now);
}

function dataUriBytes(uri: string): ArrayBuffer | null {
  const comma = uri.indexOf(",");
  if (comma < 0) return null;
  try {
    const binary = atob(uri.slice(comma + 1));
    const bytes = new Uint8Array(binary.length);
    for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
    return bytes.buffer;
  } catch {
    return null;
  }
}

export class AgentPetSoundPlayer {
  private context: AudioContext | null = null;
  private buffers = new Map<string, Promise<AudioBuffer | null>>();
  private press: Promise<Playing | null> | null = null;
  private release: Playing | null = null;

  // Node 的 strip-only 测试不支持参数属性，构造参数手动赋值。
  private sources: AgentPetSoundSources;

  constructor(sources: AgentPetSoundSources) {
    this.sources = sources;
  }

  setSources(sources: AgentPetSoundSources) {
    this.sources = sources;
    this.buffers.clear();
  }

  private audio(): AudioContext | null {
    if (this.context) return this.context;
    const AudioContextClass = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AudioContextClass) return null;
    try { this.context = new AudioContextClass(); } catch { return null; }
    return this.context;
  }

  private buffer(set: AgentPetSoundSet, slot: Slot): Promise<AudioBuffer | null> {
    const key = `${set}:${slot}`;
    const cached = this.buffers.get(key);
    if (cached) return cached;
    const context = this.audio();
    const uri = this.sources[set]?.[slot];
    const bytes = uri ? dataUriBytes(uri) : null;
    const pending = context && bytes ? context.decodeAudioData(bytes).catch(() => null) : Promise.resolve(null);
    this.buffers.set(key, pending);
    return pending;
  }

  /** 预解码一组音效，第一次按下也能立刻出声。 */
  warm(set: AgentPetSoundSet) {
    void this.buffer(set, "press");
    void this.buffer(set, "release");
  }

  private async play(set: AgentPetSoundSet, slot: Slot, volume: number, delaySeconds = 0): Promise<Playing | null> {
    const buffer = await this.buffer(set, slot);
    const context = this.context;
    if (!buffer || !context || volume <= 0) return null;
    if (context.state === "suspended") await context.resume().catch(() => undefined);
    const source = context.createBufferSource();
    const gain = context.createGain();
    gain.gain.value = volume;
    source.buffer = buffer;
    source.connect(gain).connect(context.destination);
    const startAt = context.currentTime + Math.max(0, delaySeconds);
    source.start(startAt);
    return { source, startAt, duration: buffer.duration };
  }

  private stop(playing: Playing | null) {
    try { playing?.source.stop(); } catch { /* 已经播完 */ }
  }

  async pressDown(set: AgentPetSoundSet, volume: number) {
    this.stop(this.release);
    this.release = null;
    this.press = this.play(set, "press", volume);
    await this.press;
  }

  async pressUp(set: AgentPetSoundSet, volume: number) {
    // 按压音可能还在解码：等它起播后再按它的起止时间排松开音。
    const press = await this.press;
    this.press = null;
    const delay = press && this.context ? agentPetReleaseDelay(press.startAt, press.duration, this.context.currentTime) : 0;
    this.release = await this.play(set, "release", volume, delay);
  }

  /** 试听按钮：模拟一次点按。 */
  async preview(set: AgentPetSoundSet, volume: number) {
    await this.pressDown(set, volume);
    await this.pressUp(set, volume);
  }

  dispose() {
    void this.press?.then((playing) => this.stop(playing));
    this.stop(this.release);
    void this.context?.close().catch(() => undefined);
    this.context = null;
    this.buffers.clear();
  }
}
