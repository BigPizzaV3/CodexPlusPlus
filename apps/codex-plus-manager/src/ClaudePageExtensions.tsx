import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { BadgeCheck, Download, FolderOpen, Loader2, PackagePlus, RefreshCw, Settings, Star, Trash2 } from "lucide-react";
import { AppSelect } from "./AppSelect";
import { appConfirm } from "./app-dialog";
import { CxBadge, CxCard, CxEmpty, CxFilters, CxLoading, CxSection } from "./cx-layout";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { Switch } from "./gateway-ui";
import { MenuItem } from "./ManagementPages";
import { t, tf } from "./i18n";
import { countUpdates, marketAction, matchesQuery, pageRuntimeSummary, sortMarket, type PageExtension, type PageMarketEntry, type PageMarketSort } from "./claude-page-extensions-model";
import type { ClaudeDelivery } from "./claude-enhance";
import { ExtensionConfigForm } from "./extension-config-form";
import type { ClaudePageStatus } from "./extension-host-model";

type Result<T> = T & { status: string; message: string };

const MARKET_REPO_URL = "https://github.com/BigPizzaV3/claude-mom-extensions";

/** 市场目录跨挂载留着：切走再回来不用重新拉。 */
const marketMemo: { entries: PageMarketEntry[] | null } = { entries: null };

/**
 * Claude 页面扩展：注入 Claude 页面的包，格式和扩展市场（BigPizzaV3/claude-mom-extensions）与 ClaudeMom 相同，
 * 装好的放在 Codex++ 自己的状态目录里。两种格式在同一个列表里：页面扩展（manifest.json，js / css）和
 * Codex++ 拓展包（codexpp.json，带页面接口、配置和后台）。完整功能模式由本地服务注入，刷新页面即生效；
 * 补丁副本里拓展包也是刷新页面生效，页面扩展要重启补丁版。
 */
export type ClaudeExtensionPane = "installed" | "market";

/**
 * 胶囊和 Codex 那边一样一排：「已安装」「扩展市场」。「已安装」里除了这里的列表，拓展页还会在下面接着列出
 * Codex++ 拓展目录里适配 Claude 的拓展包（`packageCount` 是它们的数量，算进「已安装」的计数里）。
 */
export function ClaudePageExtensions({ search, delivery, pane, onPaneChange, packageCount, onPageStatus, onOpenUrl, notify }: {
  search: string;
  delivery: ClaudeDelivery | null;
  pane: ClaudeExtensionPane;
  onPaneChange: (pane: ClaudeExtensionPane) => void;
  packageCount: number;
  /** Claude 页面回报的各拓展加载结果（按脚本 key），拓展页拿去标自己那几行。 */
  onPageStatus: (status: Record<string, ClaudePageStatus>) => void;
  onOpenUrl: (url: string) => void;
  notify: (title: string, message: string, status?: string) => void | Promise<void>;
}) {
  const [installed, setInstalled] = useState<PageExtension[] | null>(null);
  const [market, setMarket] = useState<PageMarketEntry[] | null>(marketMemo.entries);
  const [marketError, setMarketError] = useState<string | null>(null);
  const [marketLoading, setMarketLoading] = useState(false);
  const [busy, setBusy] = useState<string | null>(null);
  const [marketSort, setMarketSort] = useState<PageMarketSort>("default");
  const [configOpen, setConfigOpen] = useState<string | null>(null);

  const applyList = (result: Result<{ extensions?: PageExtension[]; pageStatus?: Record<string, ClaudePageStatus> }>) => {
    if (Array.isArray(result.extensions)) setInstalled(result.extensions);
    if (result.pageStatus && typeof result.pageStatus === "object") onPageStatus(result.pageStatus);
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
  const toggle = async (item: PageExtension) => {
    // 带后台的拓展包：启用前说清楚它会在本机运行什么。
    if (!item.enabled && item.capabilities?.includes("backend") && !(await appConfirm(
      tf("「{0}」带后台进程：启用后，Claude 页面用到它时会以你的身份在本机运行包里的 Node 脚本，它能读写文件、联网。只启用自己写的或信得过的拓展。", [item.name]),
      { title: t("启用带后台的拓展"), confirmText: t("启用") },
    ))) return;
    await run(`toggle:${item.id}`, () => invoke("claude_page_extension_enable", { id: item.id, enabled: !item.enabled }), true);
  };
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
  const applyHint = delivery === "copy"
    ? t("拓展包刷新 Claude 页面后生效，页面扩展重启补丁版后生效")
    : t("刷新 Claude 页面（⌘R / Ctrl+R）后生效");
  /** 拓展包在 Claude 页面里的运行情况：没加载上、运行出错用警示色，其余是一句淡色说明。 */
  const runtimeNote = (item: PageExtension) => {
    const summary = pageRuntimeSummary(item);
    if (!summary) return null;
    const text = summary.kind === "pending" ? t("还没在 Claude 页面里运行过")
      : summary.kind === "failed" ? tf("加载失败：{0}", [summary.detail || t("脚本没有执行")])
        : summary.kind === "error" ? tf("已加载，运行中出错：{0}", [summary.detail])
          : summary.kind === "partial" ? tf("已加载；这些接口在 Claude 里还不可用：{0}", [summary.detail])
            : t("已在 Claude 页面里运行");
    return { text, danger: summary.kind === "failed" || summary.kind === "error" };
  };

  const installedRow = (item: PageExtension) => {
    const running = runtimeNote(item);
    const schema = item.configSchema ?? [];
    // 一行只有一句说明：清单有误说原因；拓展包先说它在页面里的运行情况和要注意的事，再是描述。
    const details = [item.error, running?.text, ...(item.notes ?? []), item.error ? "" : item.description].filter(Boolean);
    return (
    <div key={item.id}>
    <ExtensionCard
      badges={<>
        {item.builtin ? <CxBadge>{t("内置")}</CxBadge> : null}
        {item.format === "codexpp" ? <CxBadge title={t("Codex++ 拓展包：脚本里能用 codexPlus 接口、读配置、调自己的后台")}>{t("拓展包")}</CxBadge> : null}
        {item.capabilities?.includes("backend") ? <CxBadge title={t("启用后会在本机运行包里的 Node 脚本")}>{t("带后台")}</CxBadge> : null}
        {item.error ? <CxBadge title={item.error} tone="danger">{t("清单有误")}</CxBadge> : null}
        {running?.danger ? <CxBadge title={running.text} tone="danger">{t("运行出错")}</CxBadge> : null}
      </>}
      description={details.join(" · ")}
      menu={item.builtin && !schema.length ? undefined : (
        <>
          {schema.length ? (
            <MenuItem onClick={() => setConfigOpen(configOpen === item.id ? null : item.id)}>
              <span className="ext-row-menu-item"><Settings size={14} />{configOpen === item.id ? t("收起设置") : t("设置")}</span>
            </MenuItem>
          ) : null}
          {item.builtin ? null : (
            <MenuItem disabled={busy !== null} onClick={() => void remove(item)}>
              <span className="ext-row-menu-item"><Trash2 size={14} />{t("删除")}</span>
            </MenuItem>
          )}
        </>
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
    {configOpen === item.id && schema.length ? (
      <div className="cx-row-expand"><ExtensionConfigForm
        fields={schema}
        onCancel={() => setConfigOpen(null)}
        onSave={async (values) => {
          await run(`config:${item.id}`, () => invoke("claude_page_extension_config", { id: item.id, values }));
          setConfigOpen(null);
        }}
        saveLabel={t("保存")}
        values={item.config ?? {}}
      /></div>
    ) : null}
    </div>
    );
  };

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
          { id: "installed" as const, label: t("已安装"), count: list.length + packageCount },
          { id: "market" as const, label: updates ? tf("扩展市场 · {0} 个更新", [updates]) : t("扩展市场"), count: updates ? undefined : market?.length },
        ]}
        label={t("Claude 拓展")}
        onChange={onPaneChange}
        trailing={pane === "installed" ? (
          <>
            <button className="cx-secondary" disabled={busy !== null} onClick={() => void importPackage()} title={t("选扩展包的 .zip，或扩展文件夹里的 manifest.json / codexpp.json")} type="button">
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
      {pane === "installed" ? (
        installed === null ? (
          <CxCard><CxLoading label={t("正在读取页面扩展…")} /></CxCard>
        ) : shownInstalled.length ? (
          <CxSection aside={<span className="claude-l10n-section-note">{applyHint}</span>} count={shownInstalled.length} title={t("页面扩展")}>
            <CxCard>{shownInstalled.map(installedRow)}</CxCard>
          </CxSection>
        ) : packageCount ? null : (
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
