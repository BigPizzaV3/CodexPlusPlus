import type { ReactNode } from "react";
import { t } from "./i18n";

/**
 * 拓展卡片：精选脚本、精选拓展包、社区拓展共用同一套卡片结构（.script-market-card），
 * 板块 / 列表两种排版下各栏位置一致。徽标跟在名字后面，状态（已安装 / 本机源码）在右上角。
 */
export function ExtensionCard({
  view,
  name,
  badges,
  meta,
  status,
  description,
  tags = [],
  actions,
  channel,
  dimmed,
}: {
  view: "grid" | "list";
  name: string;
  badges?: ReactNode;
  meta: ReactNode;
  status?: ReactNode;
  description?: string;
  tags?: string[];
  actions: ReactNode;
  channel?: string;
  dimmed?: boolean;
}) {
  // 标签最多露 3 个，其余收成「+N」，和精选脚本卡片一致。
  const visibleTags = tags.slice(0, 3);
  const hiddenTags = tags.slice(3);
  return (
    <div className="script-market-card ext-card" data-channel={channel} data-dimmed={dimmed || undefined} data-view={view}>
      <div className="script-market-title">
        <div>
          <div className="ext-card-name">
            <strong title={name}>{name}</strong>
            {badges}
          </div>
          <span>{meta}</span>
        </div>
        {status}
      </div>
      <p className="script-market-description">{description || t("暂无描述。")}</p>
      {tags.length ? (
        <div className="script-market-tags">
          {visibleTags.map((tag) => <span className="script-market-tag" key={tag}>{tag}</span>)}
          {hiddenTags.length ? <span className="script-market-tag script-market-tag-more" title={hiddenTags.join("、")}>+{hiddenTags.length}</span> : null}
        </div>
      ) : null}
      <div className="script-market-actions">{actions}</div>
    </div>
  );
}
