import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft, ArrowRight, CheckCircle2, Download, KeyRound, LoaderCircle, PackageOpen, RefreshCw, Search, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";
import { applyPluginInstallation, filterMarketPlugins, isPluginMarketSuccess, mergePluginMarketCatalog, pluginMarketPage, PluginMarketRequests, type MarketCatalogs, type MarketFilter, type MarketPlugin, type MarketResult, type MarketSource } from "./plugin-market-model";
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
  const [notice, setNotice] = useState<{ message: string; failed: boolean; restart: boolean } | null>(null);
  const [detailPlugin, setDetailPlugin] = useState<MarketPlugin | null>(null);
  const detailDialog = useRef<HTMLDialogElement | null>(null);
  const detailTrigger = useRef<HTMLButtonElement | null>(null);
  const searchInput = useRef<HTMLInputElement | null>(null);
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
    else searchInput.current?.focus({ preventScroll: true });
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

  return (
    <div className="plugin-market-screen">
      <Card className="panel plugin-market-toolbar">
        <CardContent>
          <div className="plugin-market-controls">
            <div aria-label={t("插件市场来源")} className="plugin-market-source" role="group">
              <Button aria-pressed={source === "public"} disabled={!!installing} onClick={() => selectSource("public")} title={t("收录已确认可公开分发的插件。")} variant={source === "public" ? "secondary" : "ghost"}>
                {t("公开精选")}
              </Button>
              <Button aria-pressed={source === "full"} disabled={!!installing} onClick={() => selectSource("full")} title={t("完整缓存为私有仓库，需要 GitHub 访问权限。")} variant={source === "full" ? "secondary" : "ghost"}>
                {t("完整缓存")}
              </Button>
            </div>
            <label className="plugin-market-search">
              <Search aria-hidden="true" />
              <Input aria-label={t("搜索插件")} onChange={(event) => setQuery(event.currentTarget.value)} placeholder={t("搜索名称、作者、描述或标签")} ref={searchInput} type="search" value={query} />
            </label>
            <label className="plugin-market-filter">
              <span className="sr-only">{t("筛选插件")}</span>
              <select onChange={(event) => setFilter(event.currentTarget.value as MarketFilter)} value={filter}>
                <option value="all">{t("全部插件")}</option>
                <option value="available">{t("可安装")}</option>
                <option value="installed">{t("已安装")}</option>
                <option value="updates">{t("有更新")}</option>
                <option value="auth">{t("需服务授权")}</option>
              </select>
            </label>
            <Button disabled={loading || !!installing} onClick={() => void loadCatalog(source, true)} variant="outline">
              <RefreshCw aria-hidden="true" className={loading ? "plugin-market-spinner" : ""} />
              {loading ? t("加载中…") : t("刷新索引")}
            </Button>
          </div>
          <p aria-live="polite" className="plugin-market-summary">
            {market ? query.trim() || filter !== "all" ? tf("{0} / {1} 个插件", [filtered.length, market.total]) : tf("{0} 个插件", [market.total]) : loading ? t("正在读取插件清单…") : t("尚未加载插件清单。")}
            {market?.cached ? <span title={t("使用本地索引缓存")}>{t("缓存")}</span> : null}
          </p>
        </CardContent>
      </Card>

      {error ? (
        <div className="plugin-market-feedback plugin-market-error" role="alert">
          <span>{error}</span>
          <Button disabled={loading || !!installing} onClick={() => void loadCatalog(source, true)} variant="outline">{t("重试")}</Button>
        </div>
      ) : null}
      {notice ? (
        <div aria-live="polite" className={`plugin-market-feedback ${notice.failed ? "plugin-market-error" : ""}`} role={notice.failed ? "alert" : "status"}>
          <span>{notice.message}</span>
          {notice.restart ? <span>{t("新开聊天或重启 Codex++ 后加载已安装插件。")}</span> : null}
        </div>
      ) : null}
      {installing ? (
        <div aria-live="polite" className="plugin-market-feedback" role="status">
          <LoaderCircle aria-hidden="true" className="plugin-market-spinner" />
          <span>{tf("正在下载并安装 {0}…", [installing.displayName || installing.name])}</span>
        </div>
      ) : null}

      {!market && loading ? (
        <div className="plugin-market-empty"><LoaderCircle aria-hidden="true" className="plugin-market-spinner" />{t("正在读取插件清单…")}</div>
      ) : visible.length ? (
        <div aria-busy={loading} className="plugin-market-grid">
          {visible.map((plugin) => {
            const isInstalling = installing?.id === plugin.id;
            const title = plugin.displayName || plugin.name;
            return (
              <Card className="panel plugin-market-card" key={plugin.id}>
                <CardHeader>
                  <div className="plugin-market-card-heading">
                    <PackageOpen aria-hidden="true" className="plugin-market-package" />
                    <div>
                      <CardTitle>{title}</CardTitle>
                    </div>
                  </div>
                  <CardDescription className="plugin-market-description">{plugin.description || t("暂无插件描述。")}</CardDescription>
                </CardHeader>
                <CardContent>
                  <div className="plugin-market-status">
                    {plugin.installed ? <Badge variant="secondary"><CheckCircle2 aria-hidden="true" />{t("已安装")}</Badge> : null}
                    {plugin.requiresAuth ? <Badge variant="outline"><KeyRound aria-hidden="true" />{t("需服务授权")}</Badge> : null}
                  </div>
                  <div className="plugin-market-metadata">
                    <span>{plugin.version ? `v${plugin.version}` : t("版本未声明")}</span>
                    {plugin.bytes > 0 ? <span>{formatBytes(plugin.bytes)}</span> : null}
                  </div>
                  <div className="plugin-market-card-actions">
                    <Button aria-label={tf("查看插件 {0} 的详情", [title])} onClick={(event) => { detailTrigger.current = event.currentTarget; setDetailPlugin(plugin); }} variant="ghost">{t("详情")}</Button>
                    <Button aria-label={tf("安装插件 {0}", [title])} disabled={loading || !!installing || (plugin.installed && !plugin.updateAvailable)} onClick={() => void install(plugin)}>
                      {isInstalling ? <LoaderCircle aria-hidden="true" className="plugin-market-spinner" /> : plugin.installed && !plugin.updateAvailable ? <CheckCircle2 aria-hidden="true" /> : <Download aria-hidden="true" />}
                      {isInstalling ? t("安装中…") : plugin.updateAvailable ? t("更新插件") : plugin.installed ? t("已安装") : t("下载并安装")}
                    </Button>
                  </div>
                </CardContent>
              </Card>
            );
          })}
        </div>
      ) : !error ? (
        <div className="plugin-market-empty"><PackageOpen aria-hidden="true" />{market ? t("没有匹配的插件。") : t("尚未加载插件清单。")}</div>
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
