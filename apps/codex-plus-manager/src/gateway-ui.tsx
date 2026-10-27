/** 本地网关页各标签共用的小组件与类型。 */
import type { CSSProperties, ReactNode } from "react";
import { BRAND_ICON_URLS } from "./provider-brand-icons";
import { brandIconIsColor, brandIconKey } from "./provider-brands";
import { providerAvatarText } from "./provider-name";

export type Status = "ok" | "failed" | "warning" | "info";
export type Notify = (title: string, message: string, status?: Status) => Promise<void>;
export type Run = (key: string, command: string, args: Record<string, unknown> | undefined, title: string, silent?: boolean) => Promise<boolean>;

/** 品牌图标随应用打包，来源和许可见 assets/agents/LICENSE.txt。 */
export const AGENT_ICONS: Record<string, string> = {
  claude: new URL("./assets/agents/claude.svg", import.meta.url).href,
  grok: new URL("./assets/agents/grok.svg", import.meta.url).href,
  codex: new URL("./assets/agents/chatgpt.svg", import.meta.url).href,
  codexApp: new URL("./assets/agents/codex.svg", import.meta.url).href,
};

/**
 * 网关里 Agent 的图标。Codex 用 Codex 应用自己的彩色图标；Claude Code、Grok 只有黑色单色图，
 * 直接当 <img> 在深色主题里看不见，所以用遮罩按当前文字颜色绘制，跟随主题。
 */
export function AgentIcon({ agent, className = "" }: { agent: string; className?: string }) {
  if (agent === "codex") return <img alt="" className={className} src={AGENT_ICONS.codexApp} />;
  const url = AGENT_ICONS[agent];
  return <i aria-hidden="true" className={`${className} gw-agent-mono`.trim()} style={url ? ({ "--gw-agent-icon": `url("${url}")` } as CSSProperties) : undefined} />;
}

/** 网关里 Agent 的图标：Codex 用 Codex 应用的图标（`codex` 键是 Hub 用的 OpenAI 单色图标）。 */
export function agentIcon(agent: string): string | undefined {
  return AGENT_ICONS[agent === "codex" ? "codexApp" : agent];
}

export const APP_LOGO_URL = new URL("./assets/codex-plus-logo.png", import.meta.url).href;

function hueOf(name: string): number {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + (char.codePointAt(0) ?? 0)) % 360;
  return hash;
}

export function avatarOf(name: string): string {
  return providerAvatarText(name, "P");
}

/** 已知厂商用打包的图标（按名字、再按地址识别），其它供应商用按名字固定配色的字母头像。 */
export function Avatar({ name, baseUrl = "", size = "md", plain = false }: { name: string; baseUrl?: string; size?: "sm" | "md" | "lg"; plain?: boolean }) {
  const key = plain ? null : brandIconKey(name, baseUrl);
  const url = key ? BRAND_ICON_URLS[key] : undefined;
  if (key && url) {
    return (
      <span className="gw-avatar gw-avatar-brand" data-size={size} title={name}>
        {brandIconIsColor(key) ? <img alt="" src={url} /> : <i className="gw-mono-icon" style={{ "--gw-icon": `url("${url}")` } as CSSProperties} />}
      </span>
    );
  }
  return (
    <span className="gw-avatar" data-size={size} style={{ "--gw-hue": hueOf(name) } as CSSProperties} title={name}>
      {avatarOf(name)}
    </span>
  );
}

export function Switch({ on, disabled, label, onToggle }: { on: boolean; disabled?: boolean; label: string; onToggle: () => void }) {
  return (
    <button aria-checked={on} aria-label={label} className={`gw-switch ${on ? "is-on" : ""}`} disabled={disabled} onClick={onToggle} role="switch" type="button">
      <span />
    </button>
  );
}

export function SectionLabel({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  return (
    <div className="gw-section-label">
      <span>{children}</span>
      {aside ? <div className="gw-section-aside">{aside}</div> : null}
    </div>
  );
}

/** 收藏的模型只存在本机浏览器存储里（每个查看者自己的偏好）。 */
const FAVORITES_KEY = "codexpp.gateway.favoriteModels";

export function loadFavorites(): string[] {
  try {
    const value = JSON.parse(window.localStorage.getItem(FAVORITES_KEY) ?? "[]");
    return Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : [];
  } catch {
    return [];
  }
}

export function saveFavorites(models: string[]) {
  try {
    window.localStorage.setItem(FAVORITES_KEY, JSON.stringify(models));
  } catch {
    // 存不了就只在本次会话里生效。
  }
}
