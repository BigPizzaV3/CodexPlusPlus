import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Download, ExternalLink, Loader2, RefreshCw, Search, ShieldAlert, Star } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "./gateway-ui";
import { t, tf } from "./i18n";
import "./CommunityExtensionsPanel.css";

// 与 crates/codex-plus-core/src/extension_discovery.rs 对应。
export type Permissions = {
  backend: string | null;
  hooks: { event: string; matcher: string }[];
  gatewayRequest: number;
  gatewayResponse: number;
  tools: string[];
  routes: string[];
  renderer: string[];
};
// channel 为 "local"：本机就是这个仓库的源码（自己写的、从这里发布的），不是从 GitHub 装的。
type Installed = { version: string; tag: string | null; sha: string; fromDefaultBranch: boolean; channel?: Channel | "local" } | null;
/** 安装渠道：社区（未审核）、精选（锁定到审核过的提交）、官方。 */
export type Channel = "community" | "curated" | "official";
type Entry = {
  repo: string;
  url: string;
  owner: string;
  ownerAvatar: string;
  description: string;
  stars: number;
  license: string;
  pushedAt: string;
  id: string;
  name: string;
  version: string;
  author: string;
  homepage: string;
  icon: string;
  keywords: string[];
  engine: number;
  compatible: boolean;
  permissions: Permissions;
  installed: Installed;
  /** 精选清单给出的警示：与官方 / 精选拓展同 id、自称官方。 */
  warnings?: string[];
};
type Listing = {
  enabled: boolean;
  entries: Entry[];
  fetchedAt: number;
  stale: boolean;
  problems: string[];
  message: string;
};
export type Prepared = {
  token: string;
  repo: string;
  id: string;
  name: string;
  version: string;
  tag: string | null;
  sha: string;
  fromDefaultBranch: boolean;
  fromAsset: boolean;
  permissions: Permissions;
  channel?: Channel;
  conflict: { source: "local" | "github"; repo?: string; channel?: Channel } | null;
};
type Sort = "stars" | "updated" | "name";

export function request<T>(action: string, payload?: unknown): Promise<T> {
  return invoke<T>("extension_community_request", { action, payload: payload ?? null });
}

export function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}

/** 按风险从高到低列出声明的能力；空数组表示只是页面脚本以外什么也没有。 */
function permissionLines(permissions: Permissions): { risk: "high" | "medium" | "low"; text: string }[] {
  const lines: { risk: "high" | "medium" | "low"; text: string }[] = [];
  if (permissions.backend) lines.push({ risk: "high", text: tf("常驻后台进程 {0}：可读写本机文件、访问网络", [permissions.backend]) });
  for (const hook of permissions.hooks) {
    lines.push({ risk: "high", text: hook.matcher ? tf("Codex 钩子 {0}（匹配 {1}）：可拦截或改写工具调用", [hook.event, hook.matcher]) : tf("Codex 钩子 {0}：可拦截或改写工具调用", [hook.event]) });
  }
  if (permissions.gatewayRequest) lines.push({ risk: "high", text: tf("网关请求中间件 {0} 个：可读取并改写发往模型的完整请求", [permissions.gatewayRequest]) });
  if (permissions.gatewayResponse) lines.push({ risk: "high", text: tf("网关响应中间件 {0} 个：可读取并改写模型回复", [permissions.gatewayResponse]) });
  if (permissions.tools.length) lines.push({ risk: "medium", text: tf("模型工具：{0}", [permissions.tools.join("、")]) });
  for (const route of permissions.routes) lines.push({ risk: "medium", text: tf("本地路由 {0}", [route]) });
  if (permissions.renderer.length) lines.push({ risk: "low", text: tf("在 Codex 页面里运行脚本：{0}", [permissions.renderer.join("、")]) });
  return lines;
}

export function riskBadges(permissions: Permissions): string[] {
  const badges: string[] = [];
  if (permissions.backend) badges.push(t("后台进程"));
  if (permissions.hooks.length) badges.push(t("钩子"));
  if (permissions.gatewayRequest || permissions.gatewayResponse) badges.push(t("网关"));
  if (permissions.tools.length) badges.push(t("模型工具"));
  return badges;
}

function versionText(installed: NonNullable<Installed>): string {
  return installed.fromDefaultBranch ? tf("{0}（{1}）", [installed.version, installed.sha.slice(0, 7)]) : installed.tag || installed.version;
}

function minutesAgo(seconds: number): number {
  return Math.max(0, Math.floor((Date.now() / 1000 - seconds) / 60));
}

export function CommunityExtensionsPanel({ onInstalled, onOpenUrl }: { onInstalled: () => Promise<void>; onOpenUrl: (url: string) => void }) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [search, setSearch] = useState("");
  const [sort, setSort] = useState<Sort>("stars");
  const [preparing, setPreparing] = useState("");
  const [prepared, setPrepared] = useState<Prepared | null>(null);

  const load = useCallback(async (force: boolean) => {
    setLoading(true);
    setError("");
    try {
      setListing(await request<Listing>("list", { force }));
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load(false);
  }, [load]);

  const entries = useMemo(() => {
    const query = search.trim().toLocaleLowerCase();
    const list = (listing?.entries ?? []).filter((entry) =>
      !query || [entry.name, entry.id, entry.repo, entry.description, entry.author, ...entry.keywords].join(" ").toLocaleLowerCase().includes(query),
    );
    const sorted = [...list];
    if (sort === "updated") sorted.sort((a, b) => b.pushedAt.localeCompare(a.pushedAt));
    else if (sort === "name") sorted.sort((a, b) => a.name.localeCompare(b.name));
    else sorted.sort((a, b) => b.stars - a.stars);
    // 需要更新 Codex++ 的排到最后。
    return sorted.sort((a, b) => Number(b.compatible) - Number(a.compatible));
  }, [listing, search, sort]);

  const setEnabled = async (enabled: boolean) => {
    setError("");
    try {
      setListing(await request<Listing>("setEnabled", { enabled }));
      if (enabled) void load(false);
    } catch (failure) {
      setError(errorText(failure));
    }
  };

  const prepare = async (entry: Entry) => {
    setPreparing(entry.repo);
    setError("");
    setNotice("");
    try {
      setPrepared(await request<Prepared>("prepare", { repo: entry.repo }));
    } catch (failure) {
      setError(tf("准备安装 {0} 失败：{1}", [entry.repo, errorText(failure)]));
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
    setPreparing(target.repo);
    try {
      const result = await request<{ hooksAdded?: boolean; hooksError?: string | null }>("install", { token: target.token, enable, replace: target.conflict !== null });
      const parts = [enable ? t("拓展已安装并启用。") : t("拓展已安装，尚未启用，可在「本地拓展」里启用。")];
      if (result.hooksError) parts.push(tf("Codex 钩子配置未能同步：{0}", [result.hooksError]));
      else if (result.hooksAdded) parts.push(t("Codex 配置里新增了「Codex++ 拓展钩子」，请在 Codex 的 Hooks 设置里信任一次，新开对话生效。"));
      setNotice(`${target.name} ${target.tag ?? target.sha.slice(0, 7)}：${parts.join("")}`);
      await onInstalled();
      await load(false);
    } catch (failure) {
      setError(tf("安装 {0} 失败：{1}", [target.repo, errorText(failure)]));
    } finally {
      setPreparing("");
    }
  };

  const enabled = listing?.enabled ?? true;
  return (
    <div className="community-ext">
      <div className="community-ext-intro">
        <div className="community-ext-intro-head">
          <ShieldAlert className="h-4 w-4" />
          <strong>{t("社区拓展未经 Codex++ 审核")}</strong>
          <label className="community-ext-switch">
            <span>{t("显示社区拓展")}</span>
            <Switch label={t("显示社区拓展")} on={enabled} onToggle={() => void setEnabled(!enabled)} />
          </label>
        </div>
        <p>{t("这里列出 GitHub 上打了 codexpp-extension 标签的拓展包仓库。作者可以在新版本里加入任意代码，只安装你信任的作者的拓展。安装时锁定到作者发布的版本，并在安装前列出它要的权限。")}</p>
      </div>
      {enabled ? (
        <div className="script-market-toolbar community-ext-toolbar">
          <div className="script-market-search">
            <Search className="h-4 w-4" />
            <Input aria-label={t("搜索社区拓展")} onChange={(event) => setSearch(event.currentTarget.value)} placeholder={t("搜索名称、仓库、作者或描述")} value={search} />
          </div>
          <div className="community-ext-sort" role="group" aria-label={t("社区拓展排序")}>
            {(["stars", "updated", "name"] as Sort[]).map((value) => (
              <Button key={value} aria-pressed={sort === value} onClick={() => setSort(value)} size="sm" variant={sort === value ? "secondary" : "ghost"}>
                {value === "stars" ? t("星数") : value === "updated" ? t("最近更新") : t("名称")}
              </Button>
            ))}
          </div>
          <Button disabled={loading} onClick={() => void load(true)} size="sm" variant="ghost">
            {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : <RefreshCw className="h-4 w-4" />}
            {t("刷新")}
          </Button>
        </div>
      ) : null}
      {listing?.message ? <div className={`community-ext-notice${listing.stale ? " is-error" : ""}`}>{listing.message}</div> : null}
      {error ? <div className="community-ext-notice is-error">{error}</div> : null}
      {notice ? <div className="community-ext-notice">{notice}</div> : null}
      {enabled && listing?.fetchedAt ? (
        <div className="community-ext-meta">
          {tf("{0} 个拓展 · 列表更新于 {1} 分钟前", [listing.entries.length, minutesAgo(listing.fetchedAt)])}
          {listing.problems.length ? ` · ${tf("{0} 个仓库暂时读不到，下次刷新再试", [listing.problems.length])}` : ""}
        </div>
      ) : null}
      {enabled ? (
        entries.length ? (
          <div className="community-ext-list">
            {entries.map((entry) => (
              <article className="community-ext-card" data-compatible={entry.compatible} key={entry.repo}>
                <header>
                  {entry.icon || entry.ownerAvatar ? <img alt="" className="community-ext-icon" src={entry.icon || entry.ownerAvatar} /> : <span className="community-ext-icon" />}
                  <div className="community-ext-title">
                    <strong>{entry.name}</strong>
                    <span className="community-ext-badge is-unreviewed">{t("未审核")}</span>
                    {riskBadges(entry.permissions).map((badge) => <span className="community-ext-badge" key={badge}>{badge}</span>)}
                    {entry.warnings?.length ? <span className="community-ext-badge is-suspicious">{t("可疑")}</span> : null}
                  </div>
                  <div className="community-ext-actions">
                    {entry.installed?.channel === "local" ? (
                      // 从 GitHub 重装会覆盖本机源码，这里不给按钮。
                      <span className="community-ext-installed" title={t("你在本机编写并发布的拓展，在「本地拓展」里管理")}>{tf("本机源码 {0}", [entry.installed.version])}</span>
                    ) : (
                      <>
                        {entry.installed ? <span className="community-ext-installed">{tf("已安装 {0}", [versionText(entry.installed)])}</span> : null}
                        <Button disabled={!entry.compatible || preparing !== ""} onClick={() => void prepare(entry)} size="sm" variant={entry.installed ? "ghost" : "secondary"}>
                          {preparing === entry.repo ? <Loader2 className="h-4 w-4 animate-spin" /> : <Download className="h-4 w-4" />}
                          {entry.installed ? t("重新安装") : t("安装")}
                        </Button>
                      </>
                    )}
                  </div>
                </header>
                {entry.description ? <p className="community-ext-description">{entry.description}</p> : null}
                {entry.warnings?.map((warning) => <p className="community-ext-dialog-warning" key={warning}>{warning}</p>)}
                <div className="community-ext-facts">
                  <button className="community-ext-repo" onClick={() => onOpenUrl(entry.url)} type="button">
                    <ExternalLink className="h-3 w-3" />
                    {entry.repo}
                  </button>
                  <span className="community-ext-stars"><Star className="h-3 w-3" />{entry.stars}</span>
                  {entry.version ? <span>{tf("默认分支 {0}", [entry.version])}</span> : null}
                  {entry.license ? <span>{entry.license}</span> : null}
                  {entry.pushedAt ? <span>{tf("更新于 {0}", [entry.pushedAt.slice(0, 10)])}</span> : null}
                  {!entry.compatible ? <span className="community-ext-warning">{tf("需要拓展 API v{0}，请先更新 Codex++", [entry.engine])}</span> : null}
                </div>
              </article>
            ))}
          </div>
        ) : (
          <div className="empty">{loading ? t("正在读取社区拓展…") : listing?.entries.length ? t("没有匹配的拓展。") : t("还没有社区拓展。给仓库加上 codexpp-extension 标签即可出现在这里。")}</div>
        )
      ) : (
        <div className="empty">{t("社区拓展已关闭，不会访问 GitHub。")}</div>
      )}
      {prepared ? <InstallDialog onCancel={cancel} onInstall={(enable) => void install(enable)} prepared={prepared} /> : null}
    </div>
  );
}

export function InstallDialog({ prepared, onCancel, onInstall }: { prepared: Prepared; onCancel: () => void; onInstall: (enable: boolean) => void }) {
  const [enable, setEnable] = useState(false);
  const lines = permissionLines(prepared.permissions);
  // 精选 / 官方拓展审核过，不用危险样式吓人；社区拓展保持红色确认。
  const reviewed = prepared.channel === "official" || prepared.channel === "curated";
  return (
    <div aria-modal="true" className="modal-backdrop app-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }} role="dialog">
      <div className="modal-card confirm-modal app-dialog community-ext-dialog" data-danger={reviewed ? undefined : true}>
        <div className="modal-head">
          <div>
            <h2>{tf("安装 {0}", [prepared.name])}</h2>
          </div>
          <button aria-label={t("关闭")} className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          {prepared.channel === "official" ? (
            <p className="community-ext-dialog-trusted">{t("Codex++ 官方拓展，安装的是审核过的版本。")}</p>
          ) : prepared.channel === "curated" ? (
            <p className="community-ext-dialog-trusted">{t("经 Codex++ 审核的精选拓展，安装的是审核过的那一版；作者之后的改动要再审核才会更新。")}</p>
          ) : (
            <p className="community-ext-dialog-warning">{t("未经 Codex++ 审核，作者可以在新版本里加入任意代码。只安装你信任的作者的拓展。")}</p>
          )}
          <div className="community-ext-dialog-version">
            <code>{prepared.repo}</code>
            <span>
              {prepared.fromDefaultBranch
                ? tf("作者未发布版本，将安装默认分支的提交 {0}", [prepared.sha.slice(0, 7)])
                : tf("版本 {0}，锁定提交 {1}", [prepared.tag ?? prepared.version, prepared.sha.slice(0, 7)])}
            </span>
          </div>
          <div className="community-ext-dialog-section">{t("它会获得这些能力：")}</div>
          {lines.length ? (
            <ul className="community-ext-permissions">
              {lines.map((line, index) => <li data-risk={line.risk} key={index}>{line.text}</li>)}
            </ul>
          ) : (
            <p className="community-ext-dialog-section">{t("没有声明任何能力。")}</p>
          )}
          {prepared.conflict ? (
            <p className="community-ext-dialog-warning">
              {prepared.conflict.source === "github" && prepared.conflict.channel && prepared.conflict.channel !== "community" && prepared.channel === "community"
                ? tf("本机已装 {0} 的{1}版本 {2}，社区里的同 id 拓展不能替换它。", [prepared.conflict.repo ?? "", prepared.conflict.channel === "official" ? t("官方") : t("精选"), prepared.id])
                : prepared.conflict.source === "github"
                ? tf("本机已有来自 {0} 的同名拓展 {1}，安装会替换它（旧版本移到备份目录）。", [prepared.conflict.repo ?? "", prepared.id])
                : tf("本机已有手动或对话生成的同名拓展 {0}，安装会替换它（旧版本移到备份目录）。", [prepared.id])}
            </p>
          ) : null}
          <label className="community-ext-dialog-enable">
            <input checked={enable} onChange={(event) => setEnable(event.currentTarget.checked)} type="checkbox" />
            {t("安装后立即启用")}
          </label>
        </div>
        <div className="app-dialog-actions">
          <Button onClick={onCancel} variant="secondary">{t("取消")}</Button>
          <Button className={reviewed ? undefined : "app-dialog-danger"} onClick={() => onInstall(enable)}>
            {prepared.conflict ? t("替换并安装") : t("安装")}
          </Button>
        </div>
      </div>
    </div>
  );
}
