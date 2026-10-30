import { useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { Badge as UiBadge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { t } from "./i18n";

export type ExtensionCardStatus = { text: string; title?: string; highlight?: boolean };

/** 拓展头像：图标或作者头像，读不到时退回名字首字。 */
function ExtensionIcon({ src, name }: { src?: string; name: string }) {
  const [failed, setFailed] = useState(false);
  if (src && !failed) return <img alt="" className="community-ext-icon" onError={() => setFailed(true)} src={src} />;
  return <span aria-hidden className="community-ext-icon ext-card-letter">{(name.trim()[0] ?? "?").toUpperCase()}</span>;
}

/**
 * 拓展卡片：精选脚本、精选拓展包、社区拓展共用。
 * 列表：一行一张，头像 + 名字徽标 + 右侧操作，下面描述，再下面是仓库链接与事实行。
 * 板块：网格卡片，头像 + 名字徽标 + 副标题，描述、标签，操作在底部。
 */
export function ExtensionCard({
  view,
  icon,
  name,
  badges,
  meta,
  facts,
  status,
  description,
  tags = [],
  primary,
  link,
  secondary,
  channel,
  dimmed,
  onOpenUrl,
}: {
  view: "grid" | "list";
  icon?: string;
  name: string;
  badges?: ReactNode;
  /** 板块模式名字下的一行：作者 · 版本 · 时间。 */
  meta: ReactNode;
  /** 列表模式底部事实行（仓库链接之后）。 */
  facts: ReactNode;
  status?: ExtensionCardStatus | null;
  description?: string;
  tags?: string[];
  /** 安装 / 更新按钮。 */
  primary?: ReactNode;
  /** 仓库或主页。列表模式显示成事实行开头的链接，板块模式是一个按钮。 */
  link?: { url: string; text: string; button: ReactNode; title?: string } | null;
  /** 审核、卸载等次要操作。 */
  secondary?: ReactNode;
  channel?: string;
  dimmed?: boolean;
  onOpenUrl: (url: string) => void;
}) {
  if (view === "list") {
    return (
      <article className="community-ext-card ext-card" data-channel={channel} data-dimmed={dimmed || undefined}>
        <header>
          <ExtensionIcon name={name} src={icon} />
          <div className="community-ext-title">
            <strong>{name}</strong>
            {badges}
          </div>
          <div className="community-ext-actions">
            {secondary}
            {status ? <span className="community-ext-installed" data-highlight={status.highlight || undefined} title={status.title}>{status.text}</span> : null}
            {primary}
          </div>
        </header>
        {description ? <p className="community-ext-description">{description}</p> : null}
        <div className="community-ext-facts">
          {link ? (
            <button className="community-ext-repo" onClick={() => onOpenUrl(link.url)} title={link.title} type="button">
              <ExternalLink className="h-3 w-3" />
              {link.text}
            </button>
          ) : null}
          {facts}
          {tags.map((tag) => <span className="ext-card-fact-tag" key={tag}>#{tag}</span>)}
        </div>
      </article>
    );
  }
  // 标签最多露 3 个，其余收成「+N」，卡片高度才整齐。
  const visibleTags = tags.slice(0, 3);
  const hiddenTags = tags.slice(3);
  return (
    <div className="script-market-card ext-card" data-channel={channel} data-dimmed={dimmed || undefined} data-view="grid">
      <div className="script-market-title">
        <div className="ext-card-head">
          <ExtensionIcon name={name} src={icon} />
          <div>
            <div className="ext-card-name">
              <strong title={name}>{name}</strong>
              {badges}
            </div>
            <span>{meta}</span>
          </div>
        </div>
        {status ? <UiBadge className="script-market-status" title={status.title} variant={status.highlight ? "default" : "secondary"}>{status.text}</UiBadge> : null}
      </div>
      <p className="script-market-description">{description || t("暂无描述。")}</p>
      {tags.length ? (
        <div className="script-market-tags">
          {visibleTags.map((tag) => <span className="script-market-tag" key={tag}>{tag}</span>)}
          {hiddenTags.length ? <span className="script-market-tag script-market-tag-more" title={hiddenTags.join("、")}>+{hiddenTags.length}</span> : null}
        </div>
      ) : null}
      <div className="script-market-actions">
        {primary}
        {link ? (
          <Button onClick={() => onOpenUrl(link.url)} size="sm" title={link.title ?? link.text} variant="secondary">
            {link.button}
          </Button>
        ) : null}
        {secondary}
      </div>
    </div>
  );
}

/** GitHub 头像地址：`owner/repo` 或 GitHub 仓库主页。 */
export function githubAvatar(repoOrUrl: string): string {
  const match = /^(?:https?:\/\/github\.com\/)?([A-Za-z0-9-]+)\/[A-Za-z0-9._-]+/.exec(repoOrUrl.trim());
  return match ? `https://github.com/${match[1]}.png?size=64` : "";
}
