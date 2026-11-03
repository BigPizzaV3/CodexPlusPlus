import { useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import { CxBadge, CxRow } from "./cx-layout";
import { Menu, MenuItem } from "./ManagementPages";
import { t, tf } from "./i18n";

export type ExtensionCardStatus = { text: string; title?: string; highlight?: boolean };

/** 拓展头像：图标或作者头像，读不到时退回名字首字。 */
function ExtensionIcon({ src, name }: { src?: string; name: string }) {
  const [failed, setFailed] = useState(false);
  if (src && !failed) return <img alt="" className="ext-row-avatar" onError={() => setFailed(true)} src={src} />;
  return <span aria-hidden className="ext-row-letter">{(name.trim()[0] ?? "?").toUpperCase()}</span>;
}

/**
 * 拓展的一行：精选脚本、精选拓展包、社区拓展、我发布的共用（参照 Cursor 的 Customize 列表）。
 * 左边头像，名字 + 小标签，一行说明（作者、版本、更新时间、标签放在悬停里）；
 * 右边是状态标签 + 主操作按钮，仓库链接和审核、卸载等次要操作收进三点菜单。
 */
export function ExtensionCard({
  icon,
  name,
  badges,
  meta,
  status,
  description,
  tags = [],
  primary,
  link,
  menu,
  dimmed,
  onOpenUrl,
}: {
  icon?: string;
  name: string;
  badges?: ReactNode;
  /** 作者 · 版本 · 更新时间等事实，一行文字；放在说明的悬停里，说明为空时直接显示。 */
  meta?: string;
  status?: ExtensionCardStatus | null;
  description?: string;
  tags?: string[];
  /** 安装 / 更新按钮。 */
  primary?: ReactNode;
  /** 仓库或主页，进三点菜单。 */
  link?: { url: string; text: string } | null;
  /** 审核、卸载等次要操作（MenuItem），进三点菜单。 */
  menu?: ReactNode;
  dimmed?: boolean;
  onOpenUrl: (url: string) => void;
}) {
  const tagText = tags.map((tag) => `#${tag}`).join(" ");
  const hover = [description, meta, tagText].filter(Boolean).join("\n");
  return (
    <CxRow
      badges={
        <>
          {badges}
          {status ? <CxBadge title={status.title} tone={status.highlight ? "warn" : "neutral"}>{status.text}</CxBadge> : null}
        </>
      }
      className="ext-row"
      detail={<span title={hover || undefined}>{description || meta || t("暂无描述。")}</span>}
      icon={<ExtensionIcon name={name} src={icon} />}
      muted={dimmed}
      title={name}
    >
      {primary}
      {link || menu ? (
        <Menu label={tf("{0} 的更多操作", [name])}>
          {link ? (
            <MenuItem onClick={() => onOpenUrl(link.url)}>
              <span className="ext-row-menu-item"><ExternalLink size={14} />{link.text}</span>
            </MenuItem>
          ) : null}
          {menu}
        </Menu>
      ) : null}
    </CxRow>
  );
}

/** GitHub 头像地址：`owner/repo` 或 GitHub 仓库主页。 */
export function githubAvatar(repoOrUrl: string): string {
  const match = /^(?:https?:\/\/github\.com\/)?([A-Za-z0-9-]+)\/[A-Za-z0-9._-]+/.exec(repoOrUrl.trim());
  return match ? `https://github.com/${match[1]}.png?size=64` : "";
}
