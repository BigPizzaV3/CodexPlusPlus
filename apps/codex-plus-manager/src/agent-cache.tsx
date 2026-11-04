import { invoke } from "@tauri-apps/api/core";
import { appConfirm } from "./app-dialog";
import { useEffect, useRef, useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";
import { t, tf } from "./i18n";
import { cacheApps, cacheCleanupSummary, cacheKindTotals, cacheSelection, selectableCacheGroups, type CacheGroup } from "./agent-cache-model";
import { APP_LOGO_URL } from "./gateway-ui";
import { AgentGlyph } from "./HubOverview";
import { TopbarActions } from "./TopbarActions";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxRow, CxSection, CxStats } from "./cx-layout";
import { Menu, MenuItem } from "./ManagementPages";
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

// 行标题用短名；「单独选择」「仅统计」做成标题旁的小标签。
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

  /** `keepResult`：清理后自动重扫时保留上一次的清理结果或失败原因。 */
  async function scan(keepResult = false) {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy("scan");
    setReport(null);
    setSelected([]);
    if (!keepResult) { setResult(null); setError(""); }
    try {
      const response = await invoke<Response<{ report?: CacheReport }>>("scan_agent_cache");
      if (response.status !== "ok" || !response.report) throw new Error(t(response.message));
      // 扫描完直接勾上推荐项（不含日志），大多数人只需要点一次「清理」。
      if (mounted.current) {
        setReport(response.report);
        setSelected(cacheSelection(response.report.groups, "recommended"));
      }
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

  const ratioOf = (value: number, total: number) => `${total ? Math.min(100, (value / total) * 100) : 0}%`;
  // 参照 Cursor 设置页：顶部一排数字 + 一个主按钮；每个应用一节，节里一行一个目录（类别、大小条、勾选）。
  return (
    <div className="tp-page cache-page cx-cache-page" aria-busy={busy !== null}>
      <TopbarActions>
        <button className="cx-secondary" disabled={busy !== null} onClick={() => void scan()} type="button">
          <RefreshCw className={busy === "scan" ? "animate-spin" : ""} />
          {busy === "scan" ? t("正在扫描缓存…") : report ? t("重新扫描") : t("扫描缓存")}
        </button>
        <Menu>
          <MenuItem disabled={busy !== null || selectable.length === 0} onClick={() => setSelected(cacheSelection(report?.groups ?? [], "recommended"))}>{t("推荐选择")}</MenuItem>
          <MenuItem disabled={busy !== null || selectable.length === 0} onClick={() => setSelected(selectable)}>{t("全部选择（含日志）")}</MenuItem>
          <MenuItem disabled={busy !== null || selected.length === 0} onClick={() => setSelected([])}>{t("取消选择")}</MenuItem>
        </Menu>
      </TopbarActions>

      {/* 总账：扫描到多少、能清多少、选了多少，清理按钮跟着总账走。 */}
      <div className="cx-cache-summary">
        <CxStats items={[
          { label: t("已扫描"), value: report ? bytes(totalBytes) : "—" },
          { label: t("可清理"), value: report ? bytes(eligibleBytes) : "—" },
          { label: t("已选择"), value: bytes(chosenBytes), hint: tf("{0} 个文件", [chosenFiles]) },
        ]} />
        <button className="cx-primary" disabled={busy !== null || chosenFiles === 0} onClick={() => void clean()}
          title={t("大小按文件逻辑长度统计，实际释放空间可能不同。缓存可能在应用重启后重新生成。")} type="button">
          <Trash2 />{busy === "clean" ? t("正在确认或清理缓存…") : t("清理所选缓存")}
        </button>
      </div>

      <div role="status" aria-live="polite">
        {result ? <CxBanner title={tf("已删除 {0} 个文件（逻辑大小 {1}），跳过 {2} 个，失败 {3} 个。请重新扫描查看剩余占用。", [result.removedFiles, bytes(result.removedBytes), result.skippedFiles, result.failures.length])} tone="ok" /> : null}
      </div>
      {error ? <div role="alert"><CxBanner title={error} tone="warn" /></div> : null}

      {apps.map((app) => {
        const appSelected = chosen.filter((group) => group.app === app.name);
        const selectedBytes = appSelected.reduce((sum, group) => sum + group.eligibleBytes, 0);
        return <CxSection key={app.name}
          aside={<>
            {report ? <span>{selectedBytes ? tf("已选择 {0}", [bytes(selectedBytes)]) : tf("可清理 {0}", [bytes(app.eligibleBytes)])}</span> : null}
            <AppSelection app={app.name} groups={app.groups} selected={selected} disabled={busy !== null} onChange={setSelected} />
          </>}
          title={<span className="cx-cache-app">
            <span className="cache-app-icon">{app.name === "Codex++" ? <img alt="" src={APP_LOGO_URL} /> : <AgentGlyph agent={app.name === "Claude" ? "claude" : "codex"} />}</span>
            {app.name}
            {report ? <span className="cx-section-count">{bytes(app.totalBytes)}</span> : null}
          </span>}>
          <CxCard>
            {!report ? <CxEmpty title={busy === "scan" ? t("正在扫描…") : t("扫描后查看占用")} /> : null}
            {report && app.groups.length === 0 ? <CxEmpty title={t("未发现可识别的缓存目录。")} /> : null}
            {report ? app.groups.map((group) => {
              const usable = group.cleanable && group.eligibleFiles > 0;
              return <CxRow key={group.id}
                badges={group.kind === "logs" ? <CxBadge>{t("单独选择")}</CxBadge> : group.kind === "temporary" ? <CxBadge>{t("仅统计")}</CxBadge> : null}
                detail={<code className="cx-set-path">{group.path}</code>}
                title={kindShortLabel(group.kind)}
                tooltip={tf("占用 {0} · 可清理 {1} / {2} 个文件 · 跳过 {3} 个", [bytes(group.totalBytes), bytes(group.eligibleBytes), group.eligibleFiles, group.skippedFiles])}>
                <span className="cx-cache-bar" aria-hidden="true" data-kind={group.kind}><i style={{ width: ratioOf(group.totalBytes, app.totalBytes) }} /></span>
                <span className="cx-cache-size">{bytes(group.totalBytes)}</span>
                <input className="tp-check" type="checkbox" aria-label={tf("选择 {0}", [`${app.name} ${kindShortLabel(group.kind)}`])}
                  checked={selected.includes(group.id)} disabled={busy !== null || !usable}
                  onChange={(event) => setSelected(event.currentTarget.checked
                    ? [...new Set([...selected, group.id])] : selected.filter((id) => id !== group.id))} />
              </CxRow>;
            }) : null}
          </CxCard>
        </CxSection>;
      })}

      {(report?.warnings.length ?? 0) > 0 ? <details className="tp-details">
        <summary>{t("扫描提示（部分目录可能未统计）")}</summary>
        <ul>{report?.warnings.map((warning, index) => <li key={index}>{t(warning)}</li>)}</ul>
      </details> : null}
      {(result?.failures.length ?? 0) > 0 ? <details className="tp-details">
        <summary>{t("查看清理失败的文件")}</summary>
        <ul>{result?.failures.map((failure, index) => <li key={index}>{failure}</li>)}</ul>
      </details> : null}
      <p className="tp-foot">{t("会话、凭据、配置、插件和虚拟机资源不在清理范围。临时文件仅统计；最近 24 小时修改的文件、链接及变化的文件会跳过。")}</p>
    </div>
  );
}
