/**
 * Codex「连接与供应商」的「从第三方导入」：把别的工具里的供应商复制成直连供应商。
 * 来源在顶部切换（CC Switch、Magpie），每家可以单独勾，已经导入过的标出来、不能再勾；
 * 两个来源勾的一起导入。只读对方的数据，不改它；导入后两边各改各的。
 * Magpie 有两种导入方式，二选一：复制它的供应商（逐条勾），或者把 Magpie 本身当作一个上游（请求经它转发，只有一条）。
 * 外观和共享供应商的「导入」（GatewayProviders 的 ImportDialog）是同一套弹窗和条目样式。
 */
import { useEffect, useId, useMemo, useState } from "react";
import { createPortal } from "react-dom";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Check, FolderOpen, RefreshCw, X } from "lucide-react";
import { Input } from "@/components/ui/input";
import { AppSelect } from "./AppSelect";
import { isSuccessStatus, stringifyError } from "./app-shared";
import { protocolLabel } from "./gateway-model";
import { Avatar } from "./gateway-ui";
import { t, tf } from "@/i18n";
import {
  THIRD_PARTY_SOURCES,
  THIRD_PARTY_SOURCE_NAME,
  defaultMagpieMode,
  defaultPicked,
  importPlan,
  initialSource,
  itemKey,
  thirdPartyItems,
  urlHost,
  type MagpieImportMode,
  type ThirdPartyItem,
  type ThirdPartySource,
} from "./third-party-import-model";
import { useModalShell } from "./use-modal-shell";
import type { Actions, BackendSettings, CcsProvidersResult, MagpieProvidersResult } from "./app-types";

export function ThirdPartyImportDialog({ ccsProviders, form, actions, onClose }: {
  ccsProviders: CcsProvidersResult | null;
  /** 当前设置：cc-switch 数据库路径、已有的直连供应商（「关联已有」用）。 */
  form: BackendSettings;
  actions: Actions;
  onClose: () => void;
}) {
  const titleId = useId();
  const { backdropProps, dialogProps } = useModalShell<HTMLDivElement>({ onClose, labelledBy: titleId });
  const [magpie, setMagpie] = useState<MagpieProvidersResult | null>(null);
  const [loading, setLoading] = useState(false);
  const [importing, setImporting] = useState(false);
  // null：还没手动切过，跟着数据走（哪个来源有东西可导就停在哪个）。
  const [chosenSource, setChosenSource] = useState<ThirdPartySource | null>(null);
  // null：还没手动勾过，默认勾上所有没导入的。
  const [pickedOverride, setPickedOverride] = useState<Set<string> | null>(null);
  // Magpie 的导入方式，二选一：把 Magpie 当作一个上游（默认），或复制它的供应商。一次只导选中的那一种。
  // null：还没手动选过，跟着数据走（当作上游那一条还没导入就默认它）。
  const [chosenMagpieMode, setMagpieMode] = useState<MagpieImportMode | null>(null);
  const [ccsBindings, setCcsBindings] = useState<Record<string, string>>({});
  const [ccsDbPathDraft, setCcsDbPathDraft] = useState(form.ccsDbPath);
  const [ccsPathSaving, setCcsPathSaving] = useState(false);
  // 路径草稿与已保存的不一致时，列表还是旧库的：先「应用」才能导入。
  const ccsPathDirty = ccsDbPathDraft.trim() !== form.ccsDbPath;
  useEffect(() => setCcsDbPathDraft(form.ccsDbPath), [form.ccsDbPath]);

  const loadMagpie = async () => {
    try {
      setMagpie(await invoke<MagpieProvidersResult>("load_magpie_providers"));
    } catch (error) {
      setMagpie({ status: "failed", message: stringifyError(error), path: "", found: false, providers: [], gateway: null });
    }
  };
  const reload = async () => {
    setLoading(true);
    try {
      await Promise.all([actions.refreshCcsProviders(true), loadMagpie()]);
    } finally {
      setLoading(false);
    }
  };
  // 打开时两个来源都重新读一次：别的工具里刚加的供应商、刚导入过的标记都是最新的。
  useEffect(() => {
    void reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const items = useMemo(() => thirdPartyItems(ccsProviders, magpie), [ccsProviders, magpie]);
  const source = chosenSource ?? initialSource(items);
  const picked = pickedOverride ?? defaultPicked(items);
  const magpieMode = chosenMagpieMode ?? defaultMagpieMode(items);
  const plan = importPlan(items, picked, magpieMode);
  // 路径改了还没应用时，cc-switch 那一栏的列表是旧的，不算进这次导入。
  const ccsChosen = ccsPathDirty ? [] : plan.ccswitch;
  const total = ccsChosen.length + plan.magpie.length;
  const shown = items.filter((item) => item.source === source);
  const upstreamItem = shown.find((item) => item.upstream) ?? null;
  // 这个来源现在是不是「当作上游」方式：只有 Magpie 有这一种，而且它得在（装了 Magpie）。
  const upstreamMode = source === "magpie" && magpieMode === "upstream" && upstreamItem !== null;
  const copyItems = shown.filter((item) => !item.upstream);
  // 「全选未导入的」只管复制的那些；「当作上游」只有一条，不用勾。
  const pending = upstreamMode ? [] : copyItems.filter((item) => !item.imported);
  const allPicked = pending.length > 0 && pending.every((item) => picked.has(itemKey(item)));
  const busy = loading || importing || ccsPathSaving;

  const toggle = (item: ThirdPartyItem) => {
    const next = new Set(picked);
    const key = itemKey(item);
    if (next.has(key)) next.delete(key);
    else next.add(key);
    setPickedOverride(next);
  };
  const toggleAll = () => {
    const next = new Set(picked);
    for (const item of pending) {
      if (allPicked) next.delete(itemKey(item));
      else next.add(itemKey(item));
    }
    setPickedOverride(next);
  };

  const saveCcsDbPath = async (path = ccsDbPathDraft) => {
    setCcsPathSaving(true);
    try {
      const saved = await actions.saveSettingsValue({ ...form, ccsDbPath: path.trim() }, true);
      if (saved) await actions.refreshCcsProviders();
    } finally {
      setCcsPathSaving(false);
    }
  };
  const selectCcsDatabase = async () => {
    try {
      const selected = await open({
        title: t("选择 cc-switch 数据库"),
        multiple: false,
        directory: false,
        filters: [{ name: "SQLite", extensions: ["db", "sqlite", "sqlite3"] }],
      });
      const path = Array.isArray(selected) ? selected[0] : selected;
      if (path) {
        setCcsDbPathDraft(path);
        await saveCcsDbPath(path);
      }
    } catch (error) {
      await actions.showMessage(t("cc-switch 导入"), stringifyError(error), "failed");
    }
  };

  const submit = async () => {
    if (!total || busy) return;
    setImporting(true);
    try {
      // 只带上勾着的那几条的「关联已有」选择。
      const bindings = Object.fromEntries(ccsChosen.filter((id) => ccsBindings[id]).map((id) => [id, ccsBindings[id]]));
      if (ccsChosen.length) await actions.importCcsProviders(bindings, ccsChosen);
      if (plan.magpie.length) await actions.importMagpieProviders(plan.magpie);
      onClose();
    } finally {
      setImporting(false);
    }
  };

  /** 这个来源现在读到了什么：没装、读失败、没有可导入的。有条目时不显示。 */
  const emptyNote = (): string => {
    if (source === "ccswitch") {
      if (!ccsProviders) return t("正在读取…");
      if (!isSuccessStatus(ccsProviders.status)) return ccsProviders.message || t("读取 cc-switch 供应商失败。");
      return t("未发现可导入供应商");
    }
    if (!magpie) return t("正在读取…");
    if (!isSuccessStatus(magpie.status)) return magpie.message || t("读取 Magpie 的供应商失败。");
    return magpie.found ? t("Magpie 里还没有可导入的供应商") : t("没有找到 Magpie 的供应商文件，可能还没安装 Magpie。");
  };

  const renderRow = (item: ThirdPartyItem) => {
    const key = itemKey(item);
    // 「当作上游」那一条不靠勾选：选了这种方式就是导它。
    const on = !item.imported && (item.upstream || picked.has(key));
    return (
      // 一行里除了勾选还有「导入为」下拉，所以勾选是行里的一个按钮，不是整行一个按钮。
      <div className="gw-import-item tpi-row" data-on={on} key={key}>
        <button
          aria-pressed={on}
          className="tpi-pick"
          disabled={busy || item.imported || item.upstream || (item.source === "ccswitch" && ccsPathDirty)}
          onClick={() => toggle(item)}
          type="button"
        >
          <span className="gw-import-check">{on ? <Check className="h-3.5 w-3.5" /> : null}</span>
          <Avatar baseUrl={item.baseUrl} name={item.name} size="sm" />
          <span className="tpi-copy">
            <strong title={item.name}>{item.name}</strong>
            <small title={item.baseUrl}>{item.upstream ? item.baseUrl : [protocolLabel(item.protocol), urlHost(item.baseUrl) || t("未填写地址")].join(" · ")}</small>
          </span>
        </button>
        {item.hasKey ? null : <em className="gw-import-badge" title={t("导入后需要在供应商里补上 Key")}>{t("没有 Key")}</em>}
        {item.disabled ? <em className="gw-import-badge" title={t("在 Magpie 里是关着的，导入后照常可用")}>{t("在 Magpie 里已关闭")}</em> : null}
        {item.imported ? <em className="gw-import-badge">{t("已导入")}</em> : null}
        {/* cc-switch 的旧版导入没有记来源：同名的旧供应商可以在这里手动关联，免得重复导入一份。 */}
        {item.source === "ccswitch" && on ? (
          <AppSelect
            ariaLabel={tf("{0} 导入为", [item.name])}
            menuWidth={240}
            onChange={(next) => setCcsBindings((current) => ({ ...current, [item.id]: next }))}
            options={[
              { value: "", label: t("新增（同名自动关联）") },
              ...form.relayProfiles.map((profile) => ({ value: profile.id, label: `${t("关联已有")}：${profile.name}` })),
            ]}
            size="sm"
            value={ccsBindings[item.id] || ""}
          />
        ) : null}
      </div>
    );
  };

  return createPortal(
    <div className="gw-modal-backdrop gw-portal" {...backdropProps}>
      <div {...dialogProps} className="gw-modal gw-import-modal tpi-modal">
        <header>
          <strong id={titleId}>{t("从第三方导入")}</strong>
          {pending.length ? (
            <button className="gw-text-button" disabled={busy} onClick={toggleAll} type="button">
              {allPicked ? t("全不选") : t("全选未导入的")}
            </button>
          ) : null}
          <button aria-label={t("关闭")} className="gw-icon-button" onClick={onClose} type="button"><X className="h-4 w-4" /></button>
        </header>
        <div className="gw-modal-body">
          {/* 来源：数字是这个来源里的供应商数。 */}
          <div aria-label={t("导入来源")} className="mg-segmented tpi-sources" role="tablist">
            {THIRD_PARTY_SOURCES.map((key) => (
              <button aria-selected={source === key} data-active={source === key} key={key} onClick={() => setChosenSource(key)} role="tab" type="button">
                {THIRD_PARTY_SOURCE_NAME[key]}
                <span className="tpi-count">{items.filter((item) => item.source === key && !item.upstream).length}</span>
              </button>
            ))}
          </div>
          <p className="gw-muted gw-import-note">{t("复制一份成直连供应商；只读取对方的数据，不会改动它，之后两边各改各的。")}</p>
          {source === "ccswitch" ? (
            <div className="tpi-origin">
              <div className="tpi-path">
                <Input
                  aria-label={t("cc-switch 数据库路径")}
                  onChange={(event) => setCcsDbPathDraft(event.currentTarget.value)}
                  placeholder={t("默认 ~/.cc-switch/cc-switch.db")}
                  value={ccsDbPathDraft}
                />
                <button className="gw-text-button" disabled={ccsPathSaving} onClick={() => void selectCcsDatabase()} title={t("选择数据库")} type="button">
                  <FolderOpen className="h-3.5 w-3.5" />
                  {t("选择")}
                </button>
                {ccsPathDirty ? (
                  <button className="gw-text-button strong" disabled={ccsPathSaving} onClick={() => void saveCcsDbPath()} type="button">
                    {ccsPathSaving ? t("应用中") : t("应用")}
                  </button>
                ) : null}
              </div>
              {ccsPathDirty ? <small className="tpi-warning">{t("路径已修改，点「应用」读取新数据库后再导入。")}</small> : null}
              {ccsProviders?.fallbackReason ? <small className="tpi-warning">{ccsProviders.fallbackReason}</small> : null}
              {ccsProviders && isSuccessStatus(ccsProviders.status) ? (
                <small className="tpi-meta" title={ccsProviders.dbPath}>{t("实际来源")}：{ccsProviders.dbPath}</small>
              ) : null}
            </div>
          ) : magpie?.path ? (
            <div className="tpi-origin">
              <small className="tpi-meta" title={magpie.path}>{t("实际来源")}：{magpie.path}</small>
            </div>
          ) : null}
          {/* Magpie 的两种导入方式二选一（默认当作上游）：先选方式，下面只列这种方式要导的。别的来源只有复制，不出这一排。 */}
          {upstreamItem ? (
            <div aria-label={t("导入方式")} className="tpi-modes" role="radiogroup">
              <button aria-checked={upstreamMode} className="tpi-mode" disabled={busy} onClick={() => setMagpieMode("upstream")} role="radio" type="button">
                <strong>{t("把 Magpie 当作一个上游")}</strong>
                <small>{t("只加一条指向 Magpie 的供应商：请求经 Magpie 转发，用它里面的账号、路由和额度；Magpie 要开着。")}</small>
              </button>
              <button aria-checked={!upstreamMode} className="tpi-mode" disabled={busy} onClick={() => setMagpieMode("copy")} role="radio" type="button">
                <strong>{t("复制它的供应商")}</strong>
                <small>{t("把地址和 Key 复制成直连供应商，之后不依赖 Magpie。")}</small>
              </button>
            </div>
          ) : null}
          <section className="tpi-list">
            {upstreamMode && upstreamItem ? renderRow(upstreamItem) : (
              <>
                {copyItems.length ? null : <p className="gw-muted gw-import-empty">{emptyNote()}</p>}
                {copyItems.map(renderRow)}
              </>
            )}
          </section>
        </div>
        <footer className="gw-import-foot">
          <button aria-label={t("刷新列表")} className="gw-icon-button" disabled={busy} onClick={() => void reload()} title={t("刷新列表")} type="button">
            <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          </button>
          <button className="gw-primary-button" disabled={busy || total === 0} onClick={() => void submit()} type="button">
            {importing ? t("导入中…") : tf("导入 {0} 个", [total])}
          </button>
        </footer>
      </div>
    </div>,
    document.body,
  );
}
