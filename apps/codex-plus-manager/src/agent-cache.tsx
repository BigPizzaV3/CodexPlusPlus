import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { useEffect, useRef, useState } from "react";
import { CheckCheck, ChevronRight, Info, RefreshCw, Sparkles, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { cacheApps, cacheCleanupSummary, cacheKindTotals, cacheSelection, selectableCacheGroups, type CacheGroup } from "./agent-cache-model";
import { APP_LOGO_URL } from "./gateway-ui";
import { AgentGlyph } from "./HubOverview";
import { TopbarActions } from "./TopbarActions";
import "./ToolPages.css";

type CacheReport = { scanId: string; groups: CacheGroup[]; warnings: string[] };
type CleanResult = { removedFiles: number; removedBytes: number; skippedFiles: number; failures: string[] };
type Response<T> = { status: string; message: string } & T;

function bytes(value: number) {
  const units = ["B", "KiB", "MiB", "GiB", "TiB"];
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) { value /= 1024; unit += 1; }
  return `${value.toFixed(unit ? 2 : 0)} ${units[unit]}`;
}

function kindLabel(kind: string) {
  if (kind === "logs") return t("日志（单独选择）");
  if (kind === "update") return t("更新下载缓存");
  if (kind === "temporary") return t("临时文件（仅统计）");
  return t("应用缓存");
}

// 卡片内的类型小计用短名；完整说明（单独选择 / 仅统计）留在目录明细里。
function kindShortLabel(kind: string) {
  if (kind === "logs") return t("日志");
  if (kind === "update") return t("更新下载");
  if (kind === "temporary") return t("临时文件");
  return t("应用缓存");
}

function AppSelection({ app, groups, selected, disabled, onChange }: {
  app: string;
  groups: CacheGroup[];
  selected: string[];
  disabled: boolean;
  onChange: (ids: string[]) => void;
}) {
  const input = useRef<HTMLInputElement>(null);
  const ids = cacheSelection(groups, "all");
  const count = ids.filter((id) => selected.includes(id)).length;
  useEffect(() => {
    if (input.current) input.current.indeterminate = count > 0 && count < ids.length;
  }, [count, ids.length]);
  return <input ref={input} className="tp-check" type="checkbox" aria-label={tf("选择 {0} 的全部可清理项目", [app])}
    checked={ids.length > 0 && count === ids.length} disabled={disabled || ids.length === 0}
    onChange={(event) => onChange(event.currentTarget.checked
      ? [...new Set([...selected, ...ids])] : selected.filter((id) => !ids.includes(id)))} />;
}

function CacheDirectories({ app, groups, selected, disabled, onChange }: {
  app: string;
  groups: CacheGroup[];
  selected: string[];
  disabled: boolean;
  onChange: (ids: string[]) => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  return <>
    <button className="tp-link" type="button" disabled={disabled} onClick={() => dialog.current?.showModal()}>
      {tf("目录明细（{0}）", [groups.length])}<ChevronRight aria-hidden="true" />
    </button>
    <dialog ref={dialog} className="cache-dialog" aria-label={tf("{0} 的目录明细（{1}）", [app, groups.length])}
      onClick={(event) => { if (event.target === event.currentTarget) dialog.current?.close(); }}>
      <div className="cache-dialog-head">
        <h3>{tf("{0} 的目录明细（{1}）", [app, groups.length])}</h3>
        <Button variant="ghost" size="icon" aria-label={t("关闭窗口")} onClick={() => dialog.current?.close()}>
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="cache-dialog-list">
        {groups.map((group) => <label key={group.id} className="cache-dir" data-disabled={!group.cleanable || group.eligibleFiles === 0}>
          <input className="tp-check" type="checkbox" checked={selected.includes(group.id)}
            disabled={disabled || !group.cleanable || group.eligibleFiles === 0}
            onChange={(event) => onChange(event.currentTarget.checked
              ? [...new Set([...selected, group.id])] : selected.filter((id) => id !== group.id))} />
          <span>
            <strong>{kindLabel(group.kind)}</strong>
            <code>{group.path}</code>
            <small>{tf("占用 {0} · 可清理 {1} / {2} 个文件 · 跳过 {3} 个", [bytes(group.totalBytes), bytes(group.eligibleBytes), group.eligibleFiles, group.skippedFiles])}</small>
          </span>
        </label>)}
      </div>
    </dialog>
  </>;
}

export function AgentCachePanel() {
  const [report, setReport] = useState<CacheReport | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [busy, setBusy] = useState<"scan" | "clean" | null>(null);
  const [error, setError] = useState("");
  const [result, setResult] = useState<CleanResult | null>(null);
  const inFlight = useRef(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const chosen = selectableCacheGroups(report?.groups ?? []).filter((group) => selected.includes(group.id));
  const chosenSummary = cacheCleanupSummary(chosen);
  const chosenBytes = chosenSummary.bytes;
  const chosenFiles = chosenSummary.files;
  const totalBytes = report?.groups.reduce((sum, group) => sum + group.totalBytes, 0) ?? 0;
  const eligibleBytes = report?.groups.reduce((sum, group) => sum + group.eligibleBytes, 0) ?? 0;
  const apps = cacheApps(report?.groups ?? []);
  const selectable = cacheSelection(report?.groups ?? [], "all");

  /** `keepResult`：清理后自动重扫时保留上一次的清理结果提示。 */
  async function scan(keepResult = false) {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy("scan");
    setError("");
    setReport(null);
    setSelected([]);
    if (!keepResult) setResult(null);
    try {
      const response = await invoke<Response<{ report?: CacheReport }>>("scan_agent_cache");
      if (response.status !== "ok" || !response.report) throw new Error(t(response.message));
      if (mounted.current) setReport(response.report);
    } catch (cause) {
      if (mounted.current) setError(String(cause));
    } finally {
      inFlight.current = false;
      if (mounted.current) setBusy(null);
    }
  }

  async function clean() {
    if (inFlight.current || !report || !chosenFiles) return;
    inFlight.current = true;
    setBusy("clean");
    setError("");
    let submitted = false;
    try {
      const accepted = await appConfirm(tf(
        "将永久删除 {0} 个缓存文件（逻辑大小 {1}）。\n涉及应用：{2}\n\n请先退出相关 AI 应用及 CLI。只处理超过 24 小时未修改的文件，包含已选择的日志。\n\n此操作不可撤销。是否继续？",
        [chosenFiles, bytes(chosenBytes), chosenSummary.apps.join("、")],
      ), { title: t("确认清理 AI Agent 缓存"), confirmText: t("清理"), danger: true });
      if (!accepted || !mounted.current) return;
      submitted = true;
      const response = await invoke<Response<{ result?: CleanResult }>>("clean_agent_cache", {
        scanId: report.scanId, groupIds: selected, confirmed: true,
      });
      if (response.status !== "ok" || !response.result) throw new Error(t(response.message));
      if (mounted.current) setResult(response.result);
    } catch (cause) {
      if (mounted.current) setError(String(cause));
    } finally {
      inFlight.current = false;
      if (mounted.current) {
        setBusy(null);
        if (submitted) { setReport(null); setSelected([]); }
      }
    }
    // 清理后旧快照已失效：直接重扫给出剩余占用，不让用户对着空卡片再手动点一次。
    if (submitted && mounted.current) void scan(true);
  }

  // 打开即扫描：扫描只读统计、不删任何文件，空着一排「—」等用户点按钮没有意义。
  useEffect(() => {
    void scan();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const selectedRatio = eligibleBytes ? Math.min(1, chosenBytes / eligibleBytes) : 0;
  return (
    <div className="tp-page cache-page" aria-busy={busy !== null}>
      <TopbarActions>
        <Button disabled={busy !== null || selectable.length === 0} size="sm" title={t("选择可清理缓存，不包含日志。")} variant="ghost"
          onClick={() => setSelected(cacheSelection(report?.groups ?? [], "recommended"))}>
          <Sparkles />{t("推荐选择")}
        </Button>
        <Button disabled={busy !== null || selectable.length === 0} size="sm" title={t("选择全部可清理项目，包含日志。")} variant="ghost"
          onClick={() => setSelected(selectable)}>
          <CheckCheck />{t("全部选择")}
        </Button>
        {selected.length ? <Button disabled={busy !== null} size="sm" variant="ghost" onClick={() => setSelected([])}>{t("取消选择")}</Button> : null}
        <Button disabled={busy !== null} size="sm" variant="secondary" onClick={() => void scan()}>
          <RefreshCw className={busy === "scan" ? "animate-spin" : ""} />
          {busy === "scan" ? t("正在扫描缓存…") : report ? t("重新扫描") : t("扫描缓存")}
        </Button>
      </TopbarActions>

      {/* 总账：扫描到多少、能清多少、选了多少，清理按钮跟着总账走。 */}
      <section className="tp-card cache-summary">
        <dl className="tp-stats">
          <div><dt>{t("已扫描")}</dt><dd>{report ? bytes(totalBytes) : "—"}</dd></div>
          <div><dt>{t("可清理")}</dt><dd>{report ? bytes(eligibleBytes) : "—"}</dd></div>
          <div><dt>{t("已选择")}</dt><dd data-tone={chosenFiles ? "accent" : undefined}>{bytes(chosenBytes)}<small>{tf("{0} 个文件", [chosenFiles])}</small></dd></div>
        </dl>
        <div className="cache-summary-side">
          <div className="tp-meter" aria-hidden="true"><i style={{ width: `${selectedRatio * 100}%` }} /></div>
          <Button disabled={busy !== null || chosenFiles === 0} onClick={() => void clean()}>
            <Trash2 />{busy === "clean" ? t("正在确认或清理缓存…") : t("清理所选缓存")}
          </Button>
        </div>
      </section>

      <p className="tp-note"><Info aria-hidden="true" />{t("会话、凭据、配置、插件和虚拟机资源不在清理范围。临时文件仅统计；最近 24 小时修改的文件、链接及变化的文件会跳过。")}</p>

      <div className="cache-grid">
        {apps.map((app) => {
          const appSelected = chosen.filter((group) => group.app === app.name);
          const selectedBytes = appSelected.reduce((sum, group) => sum + group.eligibleBytes, 0);
          const kinds = cacheKindTotals(app.groups);
          return <section key={app.name} className="tp-card cache-card" data-selected={appSelected.length > 0}>
            <header className="cache-card-head">
              <span className="cache-app-icon">
                {app.name === "Codex++" ? <img alt="" src={APP_LOGO_URL} /> : <AgentGlyph agent={app.name === "Claude" ? "claude" : "codex"} />}
              </span>
              <div className="cache-card-title">
                <strong>{app.name}</strong>
                <small>{report ? tf("可清理 {0}", [bytes(app.eligibleBytes)]) : busy === "scan" ? t("正在扫描…") : t("扫描后查看占用")}</small>
              </div>
              <AppSelection app={app.name} groups={app.groups} selected={selected} disabled={busy !== null} onChange={setSelected} />
            </header>
            <div className="cache-size">{report ? bytes(app.totalBytes) : "—"}</div>
            {report && kinds.length > 0 ? <ul className="cache-kinds" aria-label={tf("{0} 的占用构成", [app.name])}>
              {kinds.map((entry) => <li key={entry.kind} data-kind={entry.kind}>
                <span>{kindShortLabel(entry.kind)}</span>
                <i aria-hidden="true"><b style={{ width: `${app.totalBytes ? (entry.totalBytes / app.totalBytes) * 100 : 0}%` }} /></i>
                <strong>{bytes(entry.totalBytes)}</strong>
              </li>)}
            </ul> : report ? <p className="tp-muted">{t("未发现可识别的缓存目录。")}</p> : null}
            <footer className="cache-card-foot">
              <span>{tf("已选择 {0}", [bytes(selectedBytes)])}</span>
              {app.groups.length > 0 ? <CacheDirectories app={app.name} groups={app.groups} selected={selected} disabled={busy !== null} onChange={setSelected} /> : null}
            </footer>
          </section>;
        })}
      </div>

      <div role="status" aria-live="polite">
        {!busy && !report && !result && !error ? <p className="tp-muted">{t("点击扫描查看占用。扫描结果有效期为 10 分钟。")}</p> : null}
        {result ? <p className="tp-banner" data-tone="ok">{tf("已删除 {0} 个文件（逻辑大小 {1}），跳过 {2} 个，失败 {3} 个。请重新扫描查看剩余占用。", [result.removedFiles, bytes(result.removedBytes), result.skippedFiles, result.failures.length])}</p> : null}
      </div>
      {error ? <p role="alert" className="tp-banner" data-tone="error">{error}</p> : null}
      {(report?.warnings.length ?? 0) > 0 ? <details className="tp-details">
        <summary>{t("扫描提示（部分目录可能未统计）")}</summary>
        <ul>{report?.warnings.map((warning, index) => <li key={index}>{t(warning)}</li>)}</ul>
      </details> : null}
      {(result?.failures.length ?? 0) > 0 ? <details className="tp-details">
        <summary>{t("查看清理失败的文件")}</summary>
        <ul>{result?.failures.map((failure, index) => <li key={index}>{failure}</li>)}</ul>
      </details> : null}
      <p className="tp-foot">{t("大小按文件逻辑长度统计，实际释放空间可能不同。缓存可能在应用重启后重新生成。")}</p>
    </div>
  );
}
