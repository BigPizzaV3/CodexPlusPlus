import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, CheckCircle2, Download, KeyRound, LoaderCircle, PackageOpen, RefreshCw, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { t, tf } from "./i18n";
import { AppSelect } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { applyPluginInstallation, filterMarketPlugins, isPluginMarketSuccess, mergePluginMarketCatalog, pluginMarketPage, PluginMarketRequests, type MarketCatalogs, type MarketFilter, type MarketPlugin, type MarketResult, type MarketSource } from "./plugin-market-model";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxFilters, CxToolbar } from "./cx-layout";
import "./PluginMarketScreen.css";

type InstallResult = {
  status: string;
  message: string;
  plugin?: MarketPlugin;
  restartRequired?: boolean;
};

const PAGE_SIZE = 50;

function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "";
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`;
}

export function PluginMarketScreen() {
  const [source, setSource] = useState<MarketSource>("public");
  const [catalogs, setCatalogs] = useState<MarketCatalogs>({});
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<MarketFilter>("all");
  const [page, setPage] = useState(0);
  const [installing, setInstalling] = useState<MarketPlugin | null>(null);
  const [uninstalling, setUninstalling] = useState<MarketPlugin | null>(null);
  const [notice, setNotice] = useState<{ message: string; failed: boolean; restart: boolean } | null>(null);
  const [detailPlugin, setDetailPlugin] = useState<MarketPlugin | null>(null);
  const detailDialog = useRef<HTMLDialogElement | null>(null);
  const detailTrigger = useRef<HTMLButtonElement | null>(null);
  const catalogRef = useRef(catalogs);
  const requestRef = useRef<PluginMarketRequests | null>(null);
  if (!requestRef.current) requestRef.current = new PluginMarketRequests();
  const requests = requestRef.current;

  // 只在市场页面挂载、切换来源或点击刷新时读取索引，应用启动不下载插件包。
  const loadCatalog = useCallback(async (nextSource: MarketSource, refresh: boolean) => {
    const sequence = requests.beginCatalog();
    if (sequence === null) return;
    const cached = catalogRef.current[nextSource];
    setError("");
    if (cached && !refresh) {
      requests.finishCatalog(sequence);
      setLoading(false);
      return;
    }
    setLoading(true);
    try {
      const result = await invoke<MarketResult>("refresh_plugin_market", { source: nextSource, refresh });
      if (!requests.isCurrentCatalog(sequence)) return;
      const next = mergePluginMarketCatalog(catalogRef.current, nextSource, result);
      if (next === catalogRef.current) {
        setError(result.message || t("无法加载插件市场。"));
        return;
      }
      catalogRef.current = next;
      setCatalogs(next);
    } catch (cause) {
      if (requests.isCurrentCatalog(sequence)) {
        setError(String(cause));
      }
    } finally {
      if (requests.finishCatalog(sequence)) setLoading(false);
    }
  }, [requests]);

  useEffect(() => {
    requests.activate();
    return () => requests.dispose();
  }, [requests]);

  useEffect(() => {
    setPage(0);
    setNotice(null);
    void loadCatalog(source, false);
  }, [loadCatalog, source]);

  useEffect(() => { setPage(0); }, [query, filter]);

  useEffect(() => {
    if (detailPlugin && detailDialog.current && !detailDialog.current.open) detailDialog.current.showModal();
  }, [detailPlugin]);

  const closeDetails = () => {
    setDetailPlugin(null);
    if (detailTrigger.current?.isConnected) detailTrigger.current.focus({ preventScroll: true });
    else document.querySelector<HTMLInputElement>(".plugin-market-screen .cx-search input")?.focus({ preventScroll: true });
    detailTrigger.current = null;
  };

  const market = catalogs[source];
  const plugins = market?.plugins ?? [];
  const filtered = useMemo(() => filterMarketPlugins(plugins, query, filter), [plugins, query, filter]);
  const { totalPages, currentPage, visible } = pluginMarketPage(filtered, page, PAGE_SIZE);

  const selectSource = (nextSource: MarketSource) => {
    if (!requests.canSelectSource() || nextSource === source) return;
    requests.invalidateCatalog();
    setSource(nextSource);
  };

  const install = async (plugin: MarketPlugin) => {
    const sequence = requests.beginInstall();
    if (sequence === null) return;
    setInstalling(plugin);
    setNotice(null);
    const installSource = source;
    try {
      const result = await invoke<InstallResult>("install_plugin_market_item", { source: installSource, id: plugin.id });
      if (!requests.isCurrentInstall(sequence)) return;
      const succeeded = isPluginMarketSuccess(result.status);
      const installedPlugin = result.plugin;
      if (succeeded && installedPlugin) {
        const next = applyPluginInstallation(catalogRef.current, installSource, installedPlugin);
        catalogRef.current = next;
        setCatalogs(next);
      }
      setNotice({
        message: result.message || (succeeded ? t("插件安装完成。") : t("插件安装失败。")),
        failed: !succeeded,
        restart: succeeded && result.restartRequired === true,
      });
    } catch (cause) {
      if (requests.isCurrentInstall(sequence)) setNotice({ message: String(cause), failed: true, restart: false });
    } finally {
      if (requests.finishInstall(sequence)) setInstalling(null);
    }
  };

  // 卸载与安装共用同一把请求锁：进行中禁止刷新、切源和其他安装/卸载（#2701）。
  const uninstall = async (plugin: MarketPlugin) => {
    if (installing || uninstalling) return;
    const title = plugin.displayName || plugin.name;
    const confirmed = await appConfirm(
      tf("卸载插件“{0}”？将删除它下载到本机的插件文件，并从 Codex 配置中移除该插件；不影响其他插件。", [title]),
      { title: t("卸载插件"), confirmText: t("卸载"), danger: true },
    );
    if (!confirmed) return;
    const sequence = requests.beginInstall();
    if (sequence === null) return;
    setUninstalling(plugin);
    setNotice(null);
    const uninstallSource = source;
    try {
      const result = await invoke<InstallResult>("uninstall_plugin_market_item", { source: uninstallSource, id: plugin.id });
      if (!requests.isCurrentInstall(sequence)) return;
      const succeeded = isPluginMarketSuccess(result.status);
      if (succeeded) {
        // 后端返回最新安装状态；索引缓存缺失时按已卸载在本地更新这一张卡片。
        const next = applyPluginInstallation(catalogRef.current, uninstallSource, result.plugin ?? { ...plugin, installed: false, installedVersion: "", updateAvailable: false });
        catalogRef.current = next;
        setCatalogs(next);
      }
      setNotice({
        message: result.message || (succeeded ? t("插件已卸载。") : t("插件卸载失败。")),
        failed: !succeeded,
        restart: false,
      });
    } catch (cause) {
      if (requests.isCurrentInstall(sequence)) setNotice({ message: String(cause), failed: true, restart: false });
    } finally {
      if (requests.finishInstall(sequence)) setUninstalling(null);
    }
  };
  const busy = !!installing || !!uninstalling;
  const filterItems: Array<{ id: MarketFilter; label: string }> = [
    { id: "all", label: t("全部插件") },
    { id: "available", label: t("可安装") },
    { id: "installed", label: t("已安装") },
    { id: "updates", label: t("有更新") },
    { id: "auth", label: t("需服务授权") },
  ];
  // 顶部最多一条提示：进行中的安装 / 卸载 > 加载错误 > 上一次操作的结果。
  const progress = installing ? tf("正在下载并安装 {0}…", [installing.displayName || installing.name])
    : uninstalling ? tf("正在卸载 {0}…", [uninstalling.displayName || uninstalling.name]) : "";

  return (
    <div className="cx-page plugin-market-screen">
      {/* 一条工具条：搜索 + 来源 + 刷新；下面一排筛选胶囊带数量。 */}
      <div className="plugin-market-toolbar">
        <CxToolbar onQuery={setQuery} placeholder={t("搜索名称、作者、描述或标签")} query={query}>
          <AppSelect<MarketSource>
            ariaLabel={t("插件市场来源")}
            className="plugin-market-source"
            disabled={busy}
            onChange={selectSource}
            options={[
              { value: "public", label: t("公开精选") },
              { value: "full", label: t("完整缓存") },
            ]}
            size="sm"
            title={source === "public" ? t("收录已确认可公开分发的插件。") : t("完整市场收录全部插件，索引较大，首次加载可能需要几秒。")}
            value={source}
          />
          <button aria-label={t("刷新索引")} className="cx-icon-button" disabled={loading || busy} onClick={() => void loadCatalog(source, true)} title={t("刷新索引")} type="button">
            <RefreshCw aria-hidden="true" className={loading ? "plugin-market-spinner" : ""} />
          </button>
        </CxToolbar>
        <CxFilters
          items={filterItems.map((item) => ({ ...item, count: market ? filterMarketPlugins(plugins, query, item.id).length : undefined }))}
          label={t("筛选插件")}
          onChange={setFilter}
          trailing={(
            <p aria-live="polite" className="plugin-market-summary">
              {market ? null : loading ? t("正在读取插件清单…") : t("尚未加载插件清单。")}
              {market?.cached ? <span title={t("使用本地索引缓存")}>{t("缓存")}</span> : null}
            </p>
          )}
          value={filter}
        />
      </div>

      {progress ? (
        <div aria-live="polite" className="plugin-market-feedback" role="status">
          <CxBanner icon={<LoaderCircle aria-hidden="true" className="plugin-market-spinner" />} title={progress} />
        </div>
      ) : error ? (
        <div className="plugin-market-feedback plugin-market-error" role="alert">
          <CxBanner icon={<X aria-hidden="true" />} title={error} tone="warn">
            <button className="cx-secondary" disabled={loading || busy} onClick={() => void loadCatalog(source, true)} type="button">{t("重试")}</button>
          </CxBanner>
        </div>
      ) : notice ? (
        <div aria-live="polite" className={`plugin-market-feedback ${notice.failed ? "plugin-market-error" : ""}`} role={notice.failed ? "alert" : "status"}>
          <CxBanner
            detail={notice.restart ? t("新开聊天或重启 Codex++ 后加载已安装插件。") : undefined}
            icon={notice.failed ? <X aria-hidden="true" /> : <CheckCircle2 aria-hidden="true" />}
            title={notice.message}
            tone={notice.failed ? "warn" : "ok"}
          >
            <button aria-label={t("关闭")} className="cx-icon-button" onClick={() => setNotice(null)} type="button"><X aria-hidden="true" /></button>
          </CxBanner>
        </div>
      ) : null}

      {!market && loading ? (
        <CxCard><CxEmpty title={<span className="plugin-market-loading"><LoaderCircle aria-hidden="true" className="plugin-market-spinner" />{t("正在读取插件清单…")}</span>} /></CxCard>
      ) : visible.length ? (
        <div aria-busy={loading} className="plugin-market-grid">
          {visible.map((plugin) => {
            const isInstalling = installing?.id === plugin.id;
            const isUninstalling = uninstalling?.id === plugin.id;
            const title = plugin.displayName || plugin.name;
            return (
              // 轻卡片：标题（点开详情）+ 状态标签、两行说明，底部版本大小与一个操作。
              <article className="plugin-market-card" data-installed={plugin.installed || undefined} key={plugin.id}>
                <button
                  aria-label={tf("查看插件 {0} 的详情", [title])}
                  className="plugin-market-card-main"
                  onClick={(event) => { detailTrigger.current = event.currentTarget; setDetailPlugin(plugin); }}
                  type="button"
                >
                  <span className="plugin-market-card-heading">
                    <PackageOpen aria-hidden="true" className="plugin-market-package" />
                    <strong title={title}>{title}</strong>
                    {plugin.installed ? <CxBadge tone="ok">{t("已安装")}</CxBadge> : null}
                    {plugin.requiresAuth ? <CxBadge title={t("需服务授权")}><KeyRound aria-hidden="true" /></CxBadge> : null}
                  </span>
                  <span className="plugin-market-description">{plugin.description || t("暂无插件描述。")}</span>
                </button>
                <div className="plugin-market-card-foot">
                  <span className="plugin-market-metadata">
                    <span>{plugin.version ? `v${plugin.version}` : t("版本未声明")}</span>
                    {plugin.bytes > 0 ? <span>{formatBytes(plugin.bytes)}</span> : null}
                  </span>
                  <span className="plugin-market-card-actions">
                    {plugin.installed ? (
                      <Button aria-label={tf("卸载插件 {0}", [title])} className="plugin-market-uninstall" disabled={loading || busy} onClick={() => void uninstall(plugin)} size="sm" title={t("卸载")} variant="ghost">
                        {isUninstalling ? <LoaderCircle aria-hidden="true" className="plugin-market-spinner" /> : <Trash2 aria-hidden="true" />}
                        {isUninstalling ? t("卸载中…") : null}
                      </Button>
                    ) : null}
                    {plugin.installed && !plugin.updateAvailable ? null : (
                      <button aria-label={tf("安装插件 {0}", [title])} className="cx-secondary" disabled={loading || busy} onClick={() => void install(plugin)} type="button">
                        {isInstalling ? <LoaderCircle aria-hidden="true" className="plugin-market-spinner" /> : <Download aria-hidden="true" />}
                        {isInstalling ? t("安装中…") : plugin.updateAvailable ? t("更新插件") : t("安装")}
                      </button>
                    )}
                  </span>
                </div>
              </article>
            );
          })}
        </div>
      ) : !error ? (
        <CxCard>
          <CxEmpty title={market ? t("没有匹配的插件。") : t("尚未加载插件清单。")}>
            {market && (query || filter !== "all") ? (
              <button className="cx-secondary" onClick={() => { setQuery(""); setFilter("all"); }} type="button">{t("清除筛选")}</button>
            ) : !market ? (
              <button className="cx-secondary" disabled={loading || busy} onClick={() => void loadCatalog(source, true)} type="button">{t("刷新索引")}</button>
            ) : null}
          </CxEmpty>
        </CxCard>
      ) : null}

      {filtered.length > PAGE_SIZE ? (
        <nav aria-label={t("插件列表分页")} className="plugin-market-pagination">
          <Button disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)} variant="outline"><ArrowLeft aria-hidden="true" />{t("上一页")}</Button>
          <span aria-live="polite">{tf("第 {0} / {1} 页", [currentPage + 1, totalPages])}</span>
          <Button disabled={currentPage + 1 >= totalPages} onClick={() => setPage(currentPage + 1)} variant="outline">{t("下一页")}<ArrowRight aria-hidden="true" /></Button>
        </nav>
      ) : null}

      {detailPlugin ? (
        <dialog aria-labelledby="plugin-market-detail-title" className="plugin-market-detail-dialog" onClose={closeDetails} ref={detailDialog}>
          <div className="plugin-market-detail-heading">
            <h2 id="plugin-market-detail-title">{detailPlugin.displayName || detailPlugin.name}</h2>
            <Button aria-label={t("关闭详情")} autoFocus onClick={() => detailDialog.current?.close()} size="icon" variant="ghost"><X aria-hidden="true" /></Button>
          </div>
          <p className="plugin-market-detail-description">{detailPlugin.description || t("暂无插件描述。")}</p>
          <dl className="plugin-market-detail-metadata">
            <div><dt>{t("版本")}</dt><dd>{detailPlugin.version || t("版本未声明")}</dd></div>
            <div><dt>{t("体积")}</dt><dd>{formatBytes(detailPlugin.bytes) || t("未提供")}</dd></div>
            <div><dt>{t("作者")}</dt><dd>{detailPlugin.author || t("未提供")}</dd></div>
            <div><dt>{t("许可")}</dt><dd>{detailPlugin.license || t("未声明许可")}</dd></div>
            <div><dt>{t("技能数")}</dt><dd>{detailPlugin.skills}</dd></div>
            <div><dt>{t("内部名称")}</dt><dd>{detailPlugin.name}</dd></div>
            {detailPlugin.installedVersion ? <div><dt>{t("已安装")}</dt><dd>{detailPlugin.installedVersion}</dd></div> : null}
            {detailPlugin.tags.length ? <div><dt>{t("标签")}</dt><dd className="plugin-market-tags">{detailPlugin.tags.map((tag, index) => <Badge key={`${tag}:${index}`} variant="outline">{tag}</Badge>)}</dd></div> : null}
          </dl>
        </dialog>
      ) : null}
    </div>
  );
}
