/** 本地网关页各标签共用的小组件与类型。 */
import type { CSSProperties, ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { KeyRound, Shuffle, Waypoints } from "lucide-react";
import { t, tf } from "./i18n";
import {
  formatRemaining,
  maskAccountName,
  maskAccountText,
  quotaRemaining,
  quotaWindowLabel,
  type GatewayResult,
  type MemberStatus,
  type ModelView,
  type ProviderQuota,
} from "./gateway-model";
import { BRAND_ICON_URLS } from "./provider-brand-icons";
import { brandIconIsColor, brandIconKey } from "./provider-brands";
import { providerAvatarText } from "./provider-name";

export type Status = "ok" | "failed" | "warning" | "info";
export type Notify = (title: string, message: string, status?: Status) => Promise<void>;
/** `background`：不占全局 busy（订阅登录要等用户在浏览器里操作几分钟，不能把其它按钮都锁住）。 */
export type RunOptions = { background?: boolean };
export type Run = (
  key: string,
  command: string,
  args: Record<string, unknown> | undefined,
  title: string,
  silent?: boolean,
  options?: RunOptions,
) => Promise<boolean>;

/**
 * 网关页、总览共用的命令执行：占用 busy（`background` 时不占）、收下新数据、弹结果提示。
 * 提示文字里的账号名、邮箱打码；用户自己取消的（status 为 cancelled）不提示。
 */
export function makeRun(setBusy: (key: string | null) => void, accept: (result: GatewayResult) => void, notify: Notify): Run {
  return async (key, command, args, title, silent = false, options = {}) => {
    if (!options.background) setBusy(key);
    try {
      const result = await invoke<GatewayResult>(command, args);
      accept(result);
      if (result.status === "cancelled") return false;
      if (result.status !== "ok") {
        await notify(tf("{0}失败", [title]), maskAccountText(result.message), "failed");
        return false;
      }
      if (!silent) await notify(title, maskAccountText(result.message), "ok");
      return true;
    } catch (error) {
      await notify(t("调用失败"), maskAccountText(String(error)), "failed");
      return false;
    } finally {
      if (!options.background) setBusy(null);
    }
  };
}

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

/** 已知厂商用打包的图标（按名字、再按地址识别），其它供应商用按名字固定配色的字母头像。
 * 悬停提示里的名字打码（订阅账号名里带邮箱 / 用户名）。 */
export function Avatar({ name, baseUrl = "", size = "md", plain = false }: { name: string; baseUrl?: string; size?: "sm" | "md" | "lg"; plain?: boolean }) {
  const key = plain ? null : brandIconKey(name, baseUrl);
  const url = key ? BRAND_ICON_URLS[key] : undefined;
  if (key && url) {
    return (
      <span className="gw-avatar gw-avatar-brand" data-size={size} title={maskAccountName(name)}>
        {brandIconIsColor(key) ? <img alt="" src={url} /> : <i className="gw-mono-icon" style={{ "--gw-icon": `url("${url}")` } as CSSProperties} />}
      </span>
    );
  }
  return (
    <span className="gw-avatar" data-size={size} style={{ "--gw-hue": hueOf(name) } as CSSProperties} title={maskAccountName(name)}>
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

/** Key 数量：钥匙图标 + 数字。 */
export function KeyCount({ count }: { count: number }) {
  return (
    <span className="gw-key-count" title={tf("{0} 个 Key", [count])}>
      <KeyRound aria-hidden="true" />
      {count}
    </span>
  );
}

/** 订阅额度的剩余量：一根细条 + 百分比，取最紧的窗口。 */
export function QuotaBadge({ quota }: { quota: ProviderQuota }) {
  const left = Math.round(quotaRemaining(quota, Date.now()) * 100);
  const tone = left <= 10 ? "low" : left <= 30 ? "mid" : "ok";
  return (
    <span className="gw-quota-badge" data-tone={tone} title={t("剩余额度（取最紧的窗口）")}>
      <i style={{ "--gw-quota": `${left}%` } as CSSProperties} />
      {tf("剩 {0}%", [left])}
    </span>
  );
}

/** 订阅额度的明细：每个窗口一行，已用比例和重置时间。 */
export function QuotaWindows({ quota }: { quota: ProviderQuota }) {
  const now = Date.now();
  return (
    <span className="gw-quota-windows">
      {quota.windows.map((window, index) => {
        const reset = window.resetsAtMs !== null && window.resetsAtMs > now ? formatRemaining(window.resetsAtMs - now) : "";
        const label = quotaWindowLabel(window);
        const used = window.resetsAtMs !== null && window.resetsAtMs <= now ? 0 : Math.round(window.usedPercent);
        return (
          <span className="gw-quota-window" data-tone={used >= 90 ? "low" : used >= 70 ? "mid" : "ok"} key={index}>
            <b>{label.args.length ? tf(label.text, label.args) : t(label.text)}</b>
            <i style={{ "--gw-quota": `${Math.max(0, 100 - used)}%` } as CSSProperties} />
            <span>{tf("已用 {0}%", [used])}</span>
            {reset ? <small>{tf("{0} 后重置", [reset])}</small> : null}
          </span>
        );
      })}
    </span>
  );
}

/** 供应商状态：空闲时只显示 Key 数（图标 + 数字），其它状态显示文字。 */
export function MemberStatusLabel({ status }: { status: MemberStatus }) {
  if (status.text === "{0} 个 Key") return <KeyCount count={Number(status.args[0])} />;
  if (status.text === "{0} 个 Key · {1} 个冷却中") {
    return (
      <>
        <KeyCount count={Number(status.args[0])} />
        {tf("{0} 个冷却中", [status.args[1]])}
      </>
    );
  }
  return <>{status.args.length ? tf(status.text, status.args) : t(status.text)}</>;
}

/** 模型按钮前的小图标：模型组、多家自动切换各有图标，其余用供应商头像。 */
export function ModelViewGlyph({ view }: { view: ModelView }) {
  if (view.group) return <Waypoints className="gw-picker-glyph h-4 w-4" />;
  if (view.auto) return <Shuffle aria-label={t("多家自动切换")} className="gw-picker-glyph h-4 w-4" />;
  return <Avatar name={view.sub || view.title} size="sm" />;
}

/** 模型名后的「自动」小标签：多家自动切换时显示，悬停列出按顺序尝试的几家。 */
export function AutoTag({ view }: { view: ModelView }) {
  return view.auto ? <em className="gw-auto-tag" title={view.sub}>{t("自动")}</em> : null;
}
