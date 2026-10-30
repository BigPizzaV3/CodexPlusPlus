import { useState } from "react";
import { BadgeCheck, Download, ExternalLink, Loader2, ShieldAlert } from "lucide-react";
import { Button } from "@/components/ui/button";
import { InstallDialog, errorText, request, type Prepared } from "./CommunityExtensionsPanel";
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
  installedVersion: string;
  installedSha: string;
  updateAvailable: boolean;
};

export type BlockedInstall = { id: string; repo: string };

/**
 * 精选栏里的拓展包：官方排在前面。安装走锁定提交的 preparePackage（仓库与提交由后端从清单里查），
 * 确认框与社区栏共用，只是说明文字换成「已审核」。
 */
export function CuratedPackagesSection({
  packages,
  blockedInstalled,
  search,
  onInstalled,
  onOpenUrl,
}: {
  packages: CuratedPackage[];
  blockedInstalled: BlockedInstall[];
  search: string;
  onInstalled: () => Promise<void>;
  onOpenUrl: (url: string) => void;
}) {
  const [preparing, setPreparing] = useState("");
  const [prepared, setPrepared] = useState<Prepared | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
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

  if (!packages.length && !blockedInstalled.length) return null;
  return (
    <section className="curated-packages community-ext" aria-label={t("精选拓展包")}>
      {blockedInstalled.length ? (
        <div className="community-ext-notice is-error">
          <ShieldAlert className="h-4 w-4" />
          {tf("这些已安装的拓展已被 Codex++ 下架，建议在「本地拓展」里停用：{0}", [blockedInstalled.map((item) => `${item.id}（${item.repo}）`).join("、")])}
        </div>
      ) : null}
      {packages.length ? (
        <div className="curated-packages-head">
          {t("拓展包")}
          <span>{t("经 Codex++ 审核，锁定到审核过的版本")}</span>
        </div>
      ) : null}
      {error ? <div className="community-ext-notice is-error">{error}</div> : null}
      {notice ? <div className="community-ext-notice">{notice}</div> : null}
      {visible.length ? (
        <div className="community-ext-list">
          {visible.map((item) => (
            <article className="community-ext-card" data-channel={item.channel} key={item.id}>
              <header>
                {item.icon ? <img alt="" className="community-ext-icon" src={item.icon} /> : <span className="community-ext-icon" />}
                <div className="community-ext-title">
                  <strong>{item.name}</strong>
                  {item.official ? (
                    <span className="community-ext-badge is-official"><BadgeCheck className="h-3 w-3" />{t("官方")}</span>
                  ) : (
                    <span className="community-ext-badge is-curated">{t("精选")}</span>
                  )}
                </div>
                <div className="community-ext-actions">
                  {item.installed ? <span className="community-ext-installed">{tf("已安装 {0}", [item.installedVersion || item.installedSha.slice(0, 7)])}</span> : null}
                  <Button disabled={preparing !== "" || (item.installed && !item.updateAvailable)} onClick={() => void prepare(item)} size="sm" variant={item.installed && !item.updateAvailable ? "ghost" : "secondary"}>
                    {preparing === item.id ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                    {item.updateAvailable ? t("更新") : item.installed ? t("已是审核版本") : t("安装")}
                  </Button>
                </div>
              </header>
              {item.description ? <p className="community-ext-description">{item.description}</p> : null}
              <div className="community-ext-facts">
                <button className="community-ext-repo" onClick={() => onOpenUrl(`https://github.com/${item.repo}`)} type="button">
                  <ExternalLink className="h-3 w-3" />
                  {item.repo}
                </button>
                <span>{tf("审核版本 {0}", [item.tag ?? item.version ?? item.sha.slice(0, 7)])}</span>
                <span>{tf("锁定提交 {0}", [item.sha.slice(0, 7)])}</span>
                {item.author ? <span>{item.author}</span> : null}
                {item.updatedAt ? <span>{tf("更新于 {0}", [item.updatedAt.slice(0, 10)])}</span> : null}
              </div>
            </article>
          ))}
        </div>
      ) : packages.length ? (
        <div className="empty">{t("没有匹配的拓展包。")}</div>
      ) : null}
      {prepared ? <InstallDialog onCancel={cancel} onInstall={(enable) => void install(enable)} prepared={prepared} /> : null}
    </section>
  );
}
