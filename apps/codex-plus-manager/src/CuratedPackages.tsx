import { Children, useEffect, useState, type ReactNode } from "react";
import { BadgeCheck, Download, Loader2, ShieldAlert, ShieldCheck } from "lucide-react";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxSection } from "./cx-layout";
import { MenuItem } from "./ManagementPages";
import { InstallDialog, errorText, request, type Prepared } from "./CommunityExtensionsPanel";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { CurationReviewDialog, curationRequest, useCurationStatus, type CuratedUpdate } from "./ExtensionCuration";
import { t, tf } from "./i18n";

// 与 crates/codex-plus-core/src/script_market.rs 的 market_packages_payload 对应。
export type CuratedPackage = {
  kind: "package";
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  tags: string[];
  homepage: string;
  icon: string;
  updatedAt: string;
  repo: string;
  tag: string | null;
  sha: string;
  official: boolean;
  channel: "official" | "curated";
  installed: boolean;
  /** 本机就是它的源码（自己发布的），不是从 GitHub 装的。 */
  localSource?: boolean;
  installedVersion: string;
  installedSha: string;
  updateAvailable: boolean;
};

export type BlockedInstall = { id: string; repo: string };

/**
 * 精选栏：拓展包（官方在前）和老脚本行（children）按「已安装」「可安装」分两节。拓展包安装走锁定提交的 preparePackage
 * （仓库与提交由后端从清单里查），确认框与社区栏共用，只是说明文字换成「已审核」。
 */
export function CuratedPackagesSection({
  packages,
  blockedInstalled,
  search,
  hasScripts,
  emptyText,
  installedScripts = [],
  children,
  onInstalled,
  onOpenUrl,
}: {
  packages: CuratedPackage[];
  blockedInstalled: BlockedInstall[];
  search: string;
  /** 清单里有老脚本（只是被搜索过滤掉了），用来区分「没有匹配」和「清单为空」。 */
  hasScripts: boolean;
  emptyText: string;
  /** 已安装的老脚本行，排在「已安装」一节。 */
  installedScripts?: ReactNode[];
  /** 未安装的老脚本行，排在「可安装」一节。 */
  children?: ReactNode;
  onInstalled: () => Promise<void>;
  onOpenUrl: (url: string) => void;
}) {
  const [preparing, setPreparing] = useState("");
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  // 维护者：查一次精选拓展的作者有没有发新版本，有就给「审核更新」。
  const curation = useCurationStatus();
  const [updates, setUpdates] = useState<Record<string, CuratedUpdate>>({});
  const [reviewing, setReviewing] = useState("");
  const packageKey = packages.map((item) => `${item.repo}@${item.sha}`).join(",");
  useEffect(() => {
    if (!curation?.maintainer || !packageKey) return;
    let alive = true;
    curationRequest<{ updates: CuratedUpdate[] }>("updates")
      .then((result) => { if (alive) setUpdates(Object.fromEntries(result.updates.map((item) => [item.repo.toLowerCase(), item]))); })
      .catch(() => undefined);
    return () => { alive = false; };
  }, [curation?.maintainer, packageKey]);
  const query = search.trim().toLocaleLowerCase();
  const visible = packages.filter((item) =>
    !query || [item.name, item.id, item.repo, item.description, item.author, ...item.tags].join(" ").toLocaleLowerCase().includes(query),
  );

  const prepare = async (item: CuratedPackage) => {
    setPreparing(item.id);
    setError("");
    setNotice("");
    try {
      setPrepared(await request<Prepared>("preparePackage", { id: item.id }));
    } catch (failure) {
      setError(tf("准备安装 {0} 失败：{1}", [item.name, errorText(failure)]));
    } finally {
      setPreparing("");
    }
  };

  const cancel = () => {
    if (prepared) void request("discard", { token: prepared.token }).catch(() => undefined);
    setPrepared(null);
  };

  const install = async (enable: boolean) => {
    if (!prepared) return;
    const target = prepared;
    setPrepared(null);
    setPreparing(target.id);
    try {
      const result = await request<{ hooksAdded?: boolean; hooksError?: string | null }>("install", { token: target.token, enable, replace: target.conflict !== null });
      const parts = [enable ? t("拓展已安装并启用。") : t("拓展已安装，尚未启用，可在「本地拓展」里启用。")];
      if (result.hooksError) parts.push(tf("Codex 钩子配置未能同步：{0}", [result.hooksError]));
      else if (result.hooksAdded) parts.push(t("Codex 配置里新增了「Codex++ 拓展钩子」，请在 Codex 的 Hooks 设置里信任一次，新开对话生效。"));
      setNotice(`${target.name} ${target.tag ?? target.sha.slice(0, 7)}：${parts.join("")}`);
      await onInstalled();
    } catch (failure) {
      setError(tf("安装 {0} 失败：{1}", [target.name, errorText(failure)]));
    } finally {
      setPreparing("");
    }
  };

  const row = (item: CuratedPackage) => {
    const update = updates[item.repo.toLowerCase()];
    const status = item.localSource
      ? { text: tf("本机源码 {0}", [item.installedVersion]), title: t("你在本机编写并发布的拓展，在「本地拓展」里管理") }
      : item.updateAvailable
      ? { text: t("可更新"), highlight: true }
      : item.installed
      ? { text: tf("已安装 {0}", [item.installedVersion || item.installedSha.slice(0, 7)]) }
      : null;
    const facts = [
      item.author || item.repo.split("/")[0],
      item.tag ?? `v${item.version}`,
      tf("锁定 {0}", [item.sha.slice(0, 7)]),
      item.updatedAt ? tf("更新于 {0}", [item.updatedAt.slice(0, 10)]) : "",
    ].filter(Boolean).join(" · ");
    // 已是审核版本时不再给按钮：状态标签已经说明。
    const showInstall = !item.localSource && (!item.installed || item.updateAvailable);
    return (
      <ExtensionCard
        badges={item.official ? <CxBadge tone="ok"><BadgeCheck size={12} />{t("官方")}</CxBadge> : null}
        description={item.description}
        icon={item.icon || githubAvatar(item.repo)}
        key={item.id}
        link={{ url: `https://github.com/${item.repo}`, text: item.repo }}
        menu={
          curation?.maintainer ? (
            <MenuItem disabled={reviewing !== ""} onClick={() => setReviewing(item.repo)}>
              <span className="ext-row-menu-item"><ShieldCheck size={14} />{update ? tf("审核更新 {0}", [update.latestTag ?? update.latestSha.slice(0, 7)]) : t("审核")}</span>
            </MenuItem>
          ) : null
        }
        meta={facts}
        name={item.name}
        onOpenUrl={onOpenUrl}
        primary={
          showInstall ? (
            <button className={item.updateAvailable ? "cx-secondary" : "cx-secondary ext-install"} disabled={preparing !== ""} onClick={() => void prepare(item)} type="button">
              {preparing === item.id ? <Loader2 className="animate-spin" /> : <Download />}
              {item.updateAvailable ? t("更新") : t("安装")}
            </button>
          ) : null
        }
        status={status}
        tags={item.tags}
      />
    );
  };

  const installedPackages = visible.filter((item) => item.installed || item.localSource).sort((a, b) => Number(b.updateAvailable) - Number(a.updateAvailable));
  const availablePackages = visible.filter((item) => !item.installed && !item.localSource);
  const scriptRows = Children.toArray(children);
  const installedCount = installedPackages.length + installedScripts.length;
  const availableCount = availablePackages.length + scriptRows.length;
  return (
    <>
      {blockedInstalled.length ? (
        <CxBanner icon={<ShieldAlert />} title={t("已安装的拓展被下架")} tone="warn" detail={tf("这些已安装的拓展已被 Codex++ 下架，建议在「本地拓展」里停用：{0}", [blockedInstalled.map((item) => `${item.id}（${item.repo}）`).join("、")])} />
      ) : null}
      {error ? <div className="community-ext-notice is-error">{error}</div> : null}
      {notice ? <div className="community-ext-notice">{notice}</div> : null}
      {installedCount ? (
        <CxSection count={installedCount} title={t("已安装")}>
          <CxCard>
            {installedPackages.map(row)}
            {installedScripts}
          </CxCard>
        </CxSection>
      ) : null}
      {availableCount ? (
        <CxSection count={availableCount} title={t("可安装")}>
          <CxCard>
            {availablePackages.map(row)}
            {scriptRows}
          </CxCard>
        </CxSection>
      ) : null}
      {!installedCount && !availableCount ? (
        <CxCard><CxEmpty title={packages.length || hasScripts ? t("没有匹配的拓展。") : emptyText} /></CxCard>
      ) : null}
      {prepared ? <InstallDialog onCancel={cancel} onInstall={(enable) => void install(enable)} prepared={prepared} /> : null}
      {reviewing ? (
        <CurationReviewDialog
          onApproved={(message) => {
            setReviewing("");
            setNotice(message);
            setUpdates((current) => Object.fromEntries(Object.entries(current).filter(([repo]) => repo !== reviewing.toLowerCase())));
            void onInstalled();
          }}
          onClose={() => setReviewing("")}
          onOpenUrl={onOpenUrl}
          repo={reviewing}
        />
      ) : null}
    </>
  );
}
