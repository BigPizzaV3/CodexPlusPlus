import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { BadgeCheck, Download, FolderOpen, Loader2, PackagePlus, RefreshCw, Star, Trash2 } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { CxBadge, CxCard, CxEmpty, CxFilters, CxLoading, CxSection } from "./cx-layout";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { Switch } from "./gateway-ui";
import { MenuItem } from "./ManagementPages";
import { t, tf } from "./i18n";
import { countUpdates, marketAction, matchesQuery, sortMarket, type PageExtension, type PageMarketEntry, type PageMarketSort } from "./claude-page-extensions-model";
import type { ClaudeDelivery } from "./claude-enhance";

type Result<T> = T & { status: string; message: string };

const MARKET_REPO_URL = "https://github.com/BigPizzaV3/claude-mom-extensions";

/** 市场目录跨挂载留着：切走再回来不用重新拉。 */
const marketMemo: { entries: PageMarketEntry[] | null } = { entries: null };

/**
 * Claude 页面扩展：注入 Claude 页面的 js / css 包。格式和扩展市场（BigPizzaV3/claude-mom-extensions）与 ClaudeMom 相同，
 * 装好的扩展放在 Codex++ 自己的状态目录里。完整功能模式由本地服务注入，刷新页面即生效；
 * 补丁副本在启动时注入，改了要重启补丁版。
 */
export type ClaudeExtensionPane = "installed" | "market" | "code";

/**
 * 胶囊和 Codex 那边一样一排：「已安装」「扩展市场」是页面扩展，「Claude Code 拓展」那一栏的列表由拓展页自己渲染，
 * 这里只出胶囊（组件一直挂着，切过去再切回来不用重新读）。
 */
export function ClaudePageExtensions({ search, delivery, pane, onPaneChange, codeCount, onOpenUrl, notify }: {
  search: string;
  delivery: ClaudeDelivery | null;
  pane: ClaudeExtensionPane;
  onPaneChange: (pane: ClaudeExtensionPane) => void;
  codeCount: number;
  onOpenUrl: (url: string) => void;
  notify: (title: string, message: string, status?: string) => void | Promise<void>;
}) {
  const [installed, setInstalled] = useState<PageExtension[] | null>(null);
  const [market, setMarket] = useState<PageMarketEntry[] | null>(marketMemo.entries);
  const [marketError, setMarketError] = useState<string | null>(null);
  const [marketLoading, setMarketLoading] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [marketSort, setMarketSort] = useState<PageMarketSort>("default");

  const applyList = (result: Result<{ extensions?: PageExtension[] }>) => {
    if (Array.isArray(result.extensions)) setInstalled(result.extensions);
  };
  const report = (result: Result<unknown>) => {
    void notify(t("页面扩展"), result.message, result.status === "ok" ? "ok" : "failed");
  };

  const loadInstalled = async () => {
    try { applyList(await invoke<Result<{ extensions?: PageExtension[] }>>("claude_page_extensions")); }
    catch (error) { void notify(t("页面扩展"), String(error), "failed"); }
  };
  const loadMarket = async () => {
    if (marketLoading) return;
    setMarketLoading(true); setMarketError(null);
    try {
      const result = await invoke<Result<{ entries?: PageMarketEntry[] }>>("claude_page_extension_market");
      if (result.status === "ok" && Array.isArray(result.entries)) {
        marketMemo.entries = result.entries;
        setMarket(result.entries);
      } else setMarketError(result.message);
    } catch (error) { setMarketError(String(error)); }
    finally { setMarketLoading(false); }
  };
  useEffect(() => {
    void loadInstalled();
    if (!marketMemo.entries) void loadMarket();
  }, []);

  /** 同一时间只做一件事：扩展目录的读改写在后端是串行的，前端也别并发点。 */
  const run = async (key: string, task: () => Promise<Result<{ extensions?: PageExtension[] }>>, quiet = false) => {
    if (busy) return;
    setBusy(key);
    try {
      const result = await task();
      applyList(result);
      if (!quiet || result.status !== "ok") report(result);
    } catch (error) { void notify(t("页面扩展"), String(error), "failed"); }
    finally { setBusy(null); }
  };
  const toggle = (item: PageExtension) => run(`toggle:${item.id}`, () => invoke("claude_page_extension_enable", { id: item.id, enabled: !item.enabled }), true);
  const remove = async (item: PageExtension) => {
    if (!(await appConfirm(tf("删除「{0}」？扩展文件夹会一起删掉。", [item.name]), { title: t("删除扩展"), confirmText: t("删除"), danger: true }))) return;
    await run(`remove:${item.id}`, () => invoke("claude_page_extension_remove", { id: item.id }));
  };
  const importPackage = async () => {
    const picked = await open({ multiple: false, directory: false, filters: [{ name: t("扩展包"), extensions: ["zip", "json"] }] });
    if (typeof picked !== "string") return;
    await run("import", () => invoke("claude_page_extension_import", { path: picked }));
  };
  const install = async (entry: PageMarketEntry) => {
    if (entry.source === "community" && !(await appConfirm(
      tf("「{0}」是社区扩展，来自 {1}，没有经过审核。扩展的代码会在 Claude 页面里运行，能读到页面上的内容。确定安装？", [entry.name, entry.repo]),
      { title: t("安装社区扩展"), confirmText: t("安装") },
    ))) return;
    await run(`install:${entry.id}`, () => invoke("claude_page_extension_install", { entry }));
  };

  const list = installed ?? [];
  const updates = market ? countUpdates(market, list) : 0;
  const shownInstalled = useMemo(
    () => list.filter((item) => matchesQuery(search, item.name, item.id, item.author, item.description)),
    [list, search],
  );
  const shownMarket = useMemo(
    () => sortMarket(
      (market ?? []).filter((entry) => matchesQuery(search, entry.name, entry.id, entry.author, entry.description, entry.repo)),
      marketSort,
    ),
    [market, search, marketSort],
  );
  const applyHint = delivery === "copy" ? t("重启补丁版后生效") : t("刷新 Claude 页面（⌘R / Ctrl+R）后生效");

  const installedRow = (item: PageExtension) => (
    <ExtensionCard
      badges={<>
        {item.builtin ? <CxBadge>{t("内置")}</CxBadge> : null}
        {item.error ? <CxBadge title={item.error} tone="danger">{t("清单有误")}</CxBadge> : null}
      </>}
      description={item.error ?? item.description}
      key={item.id}
      menu={item.builtin ? undefined : (
        <MenuItem disabled={busy !== null} onClick={() => void remove(item)}>
          <span className="ext-row-menu-item"><Trash2 size={14} />{t("删除")}</span>
        </MenuItem>
      )}
      meta={[item.author, item.version ? `v${item.version}` : ""].filter(Boolean).join(" · ")}
      name={item.name}
      onOpenUrl={onOpenUrl}
      primary={(
        <Switch
          disabled={Boolean(item.error) && !item.enabled}
          label={tf("启用 {0}", [item.name])}
          on={item.enabled}
          onToggle={() => void toggle(item)}
          pending={busy === `toggle:${item.id}`}
        />
      )}
    />
  );

  // 照 Codex「精选」的样子：作者头像、「官方」标签、状态标签（可更新 / 已安装 / 内置），只有能装或能更新时才给按钮。
  const marketRow = (entry: PageMarketEntry) => {
    const action = marketAction(entry, list);
    const pending = busy === `install:${entry.id}`;
    const status = action.kind === "update" ? { text: t("可更新"), highlight: true, title: tf("当前 {0}", [action.from]) }
      : action.kind === "installed" ? { text: tf("已安装 {0}", [entry.version]) }
        : action.kind === "builtin" ? { text: t("已内置"), title: t("随 Codex++ 打包，在「已安装」里开关") }
          : null;
    const facts = [
      entry.author || entry.repo.split("/")[0],
      `v${entry.version}`,
      tf("锁定 {0}", [entry.commit.slice(0, 7)]),
    ].join(" · ");
    return (
      <ExtensionCard
        badges={entry.source === "official"
          ? <CxBadge tone="ok"><BadgeCheck size={12} />{t("官方")}</CxBadge>
          : <>
            <CxBadge title={t("作者自己的仓库，没有经过审核")}>{t("社区")}</CxBadge>
            {entry.stars != null ? <CxBadge title={t("GitHub 星数")}><Star size={11} />{entry.stars}</CxBadge> : null}
          </>}
        description={entry.description}
        icon={githubAvatar(entry.repo)}
        key={entry.id}
        link={{ url: entry.url || `https://github.com/${entry.repo}`, text: entry.repo }}
        meta={facts}
        name={entry.name}
        onOpenUrl={onOpenUrl}
        primary={action.kind === "install" || action.kind === "update" ? (
          <button className={action.kind === "update" ? "cx-secondary" : "cx-secondary ext-install"} disabled={busy !== null} onClick={() => void install(entry)} type="button">
            {pending ? <Loader2 className="animate-spin" /> : <Download />}
            {action.kind === "update" ? t("更新") : t("安装")}
          </button>
        ) : null}
        status={status}
      />
    );
  };
  // 和精选一样分「已安装」「可安装」两节；已安装里可更新的排前面。
  const marketInstalled = shownMarket
    .filter((entry) => marketAction(entry, list).kind !== "install")
    .sort((a, b) => Number(marketAction(b, list).kind === "update") - Number(marketAction(a, list).kind === "update"));
  const marketAvailable = shownMarket.filter((entry) => marketAction(entry, list).kind === "install");

  return (
    <>
      <CxFilters
        items={[
          { id: "installed" as const, label: t("已安装"), count: list.length },
          { id: "market" as const, label: updates ? tf("扩展市场 · {0} 个更新", [updates]) : t("扩展市场"), count: updates ? undefined : market?.length },
          { id: "code" as const, label: t("Claude Code 拓展"), count: codeCount },
        ]}
        label={t("Claude 拓展")}
        onChange={onPaneChange}
        trailing={pane === "installed" ? (
          <>
            <button className="cx-secondary" disabled={busy !== null} onClick={() => void importPackage()} title={t("选扩展包的 .zip，或扩展文件夹里的 manifest.json")} type="button">
              <PackagePlus />{t("导入扩展包")}
            </button>
            <button
              aria-label={t("打开文件夹")}
              className="cx-secondary"
              onClick={() => void invoke<Result<unknown>>("claude_page_extension_open_folder").then((result) => { if (result.status !== "ok") report(result); })}
              title={t("打开扩展文件夹")}
              type="button"
            >
              <FolderOpen />
            </button>
          </>
        ) : pane === "market" ? (
          <>
            <button aria-label={t("刷新")} className="cx-secondary" disabled={marketLoading} onClick={() => void loadMarket()} title={t("刷新扩展市场")} type="button">
              <RefreshCw className={marketLoading ? "animate-spin" : ""} />
            </button>
            <AppSelect
              ariaLabel={t("扩展市场排序")}
              className="script-market-sort"
              onChange={(value) => setMarketSort(value)}
              options={[
                { value: "default" as PageMarketSort, label: t("默认顺序") },
                { value: "name" as PageMarketSort, label: t("名称") },
                { value: "stars" as PageMarketSort, label: t("星数") },
              ]}
              size="sm"
              title={t("扩展市场排序")}
              value={marketSort}
            />
          </>
        ) : null}
        value={pane}
      />
      {pane === "code" ? null : pane === "installed" ? (
        installed === null ? (
          <CxCard><CxLoading label={t("正在读取页面扩展…")} /></CxCard>
        ) : shownInstalled.length ? (
          <CxSection aside={<span className="claude-l10n-section-note">{applyHint}</span>} count={shownInstalled.length} title={t("已安装")}>
            <CxCard>{shownInstalled.map(installedRow)}</CxCard>
          </CxSection>
        ) : (
          <CxCard>
            <CxEmpty title={search ? t("没有匹配的扩展。") : t("还没有页面扩展。")}>
              {search ? null : <button className="cx-secondary" onClick={() => onPaneChange("market")} type="button">{t("浏览扩展市场")}</button>}
            </CxEmpty>
          </CxCard>
        )
      ) : marketError && !market ? (
        <CxCard>
          <CxEmpty detail={marketError} title={t("读不到扩展市场")}>
            <button className="cx-secondary" onClick={() => void loadMarket()} type="button">{t("重试")}</button>
          </CxEmpty>
        </CxCard>
      ) : !market ? (
        <CxCard><CxLoading label={t("正在读取扩展市场…")} /></CxCard>
      ) : shownMarket.length ? (
        <>
          {marketInstalled.length ? (
            <CxSection count={marketInstalled.length} title={t("已安装")}>
              <CxCard>{marketInstalled.map(marketRow)}</CxCard>
            </CxSection>
          ) : null}
          {marketAvailable.length ? (
            <CxSection count={marketAvailable.length} title={t("可安装")}>
              <CxCard>{marketAvailable.map(marketRow)}</CxCard>
            </CxSection>
          ) : null}
          <p className="claude-l10n-hint">
            {t("想上架自己的扩展：在 GitHub 仓库加上 claude-mom-extension 话题，市场每半小时收录一次。")}
            {" "}
            <button className="cx-link" onClick={() => onOpenUrl(MARKET_REPO_URL)} type="button">{t("怎么写扩展")}</button>
          </p>
        </>
      ) : (
        <CxCard><CxEmpty title={search ? t("没有匹配的扩展。") : t("市场里还没有扩展。")} /></CxCard>
      )}
    </>
  );
}
