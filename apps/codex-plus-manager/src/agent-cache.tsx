import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { Blocks, Bot, CheckCheck, ChevronRight, RefreshCw, Sparkles, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { t, tf } from "./i18n";
import { cacheApps, cacheCleanupSummary, cacheSelection, selectableCacheGroups, type CacheGroup } from "./agent-cache-model";

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
  return <input ref={input} type="checkbox" aria-label={tf("选择 {0} 的全部可清理项目", [app])}
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
  return <div className="agent-cache-directories">
    <Button variant="ghost" disabled={disabled} onClick={() => dialog.current?.showModal()}>
      <ChevronRight className="h-4 w-4" />{tf("目录明细（{0}）", [groups.length])}
    </Button>
    <dialog ref={dialog} className="agent-cache-directory-dialog" aria-label={tf("{0} 的目录明细（{1}）", [app, groups.length])}
      onClick={(event) => { if (event.target === event.currentTarget) dialog.current?.close(); }}>
      <div className="agent-cache-directory-dialog-header">
        <h3>{tf("{0} 的目录明细（{1}）", [app, groups.length])}</h3>
        <Button variant="ghost" size="icon" aria-label={t("关闭窗口")} onClick={() => dialog.current?.close()}>
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="agent-cache-directory-list">
        {groups.map((group) => <label key={group.id} className="agent-cache-directory">
          <input type="checkbox" checked={selected.includes(group.id)}
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
  </div>;
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

  async function scan() {
    if (inFlight.current) return;
    inFlight.current = true;
    setBusy("scan");
    setError("");
    setReport(null);
    setSelected([]);
    setResult(null);
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
      const accepted = await confirm(tf(
        "将永久删除 {0} 个缓存文件（逻辑大小 {1}）。\n涉及应用：{2}\n\n请先退出相关 AI 应用及 CLI。只处理超过 24 小时未修改的文件，包含已选择的日志。\n\n此操作不可撤销。是否继续？",
        [chosenFiles, bytes(chosenBytes), chosenSummary.apps.join("、")],
      ), { title: t("确认清理 AI Agent 缓存"), kind: "warning" });
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
  }

  return (
    <Card className="panel agent-cache-panel">
      <CardHeader className="panel-head">
        <CardTitle>{t("AI Agent 缓存清理")}</CardTitle>
        <CardDescription>{t("扫描 Codex、Claude 与 Codex++ 的已知缓存目录，由你选择清理项目。")}</CardDescription>
      </CardHeader>
      <CardContent className="agent-cache-content">
        <div className="agent-cache-toolbar">
          <Button disabled={busy !== null} onClick={() => void scan()} variant="secondary">
            <RefreshCw className={`h-4 w-4 ${busy === "scan" ? "animate-spin" : ""}`} />
            {busy === "scan" ? t("正在扫描缓存…") : t("扫描缓存")}
          </Button>
          <div className="agent-cache-selection-actions" aria-label={t("批量选择缓存")}>
            <Button disabled={busy !== null || selectable.length === 0} variant="outline"
              title={t("选择可清理缓存，不包含日志。")}
              onClick={() => setSelected(cacheSelection(report?.groups ?? [], "recommended"))}>
              <Sparkles className="h-4 w-4" />{t("推荐选择")}
            </Button>
            <Button disabled={busy !== null || selectable.length === 0} variant="outline"
              title={t("选择全部可清理项目，包含日志。")}
              onClick={() => setSelected(selectable)}>
              <CheckCheck className="h-4 w-4" />{t("全部选择")}
            </Button>
            <Button disabled={busy !== null || selected.length === 0} variant="ghost" onClick={() => setSelected([])}>{t("取消选择")}</Button>
          </div>
        </div>
        <p className="agent-cache-note">
          {t("会话、凭据、配置、插件和虚拟机资源不在清理范围。临时文件仅统计；最近 24 小时修改的文件、链接及变化的文件会跳过。")}
        </p>
        <div className="agent-cache-grid" aria-busy={busy === "scan"}>
          {apps.map((app) => {
            const Icon = app.name === "Codex" ? Bot : app.name === "Claude" ? Sparkles : Blocks;
            const appSelected = chosen.filter((group) => group.app === app.name);
            const selectedBytes = appSelected.reduce((sum, group) => sum + group.eligibleBytes, 0);
            return <section key={app.name} className={`agent-cache-card ${appSelected.length ? "is-selected" : ""}`}>
              <div className="agent-cache-card-head">
                <h3>{app.name}</h3>
                <AppSelection app={app.name} groups={app.groups} selected={selected} disabled={busy !== null} onChange={setSelected} />
              </div>
              <div className="agent-cache-app-size">
                <span className={`agent-cache-app-icon ${app.name === "Claude" ? "is-claude" : app.name === "Codex++" ? "is-plus" : "is-codex"}`}>
                  <Icon aria-hidden="true" />
                </span>
                <strong>{report ? bytes(app.totalBytes) : "—"}</strong>
                <span>{report ? tf("可清理 {0}", [bytes(app.eligibleBytes)]) : t("扫描后查看占用")}</span>
              </div>
              <div className="agent-cache-card-selection">{tf("已选择 {0}", [bytes(selectedBytes)])}</div>
              {app.groups.length > 0 ? <CacheDirectories app={app.name} groups={app.groups}
                selected={selected} disabled={busy !== null} onChange={setSelected} />
                : report ? <p className="agent-cache-empty">{t("未发现可识别的缓存目录。")}</p> : null}
            </section>;
          })}
        </div>
        <div className="agent-cache-summary">
          <div role="status" aria-live="polite" aria-busy={busy !== null}>
            {busy === "clean" ? t("正在确认或清理缓存…") : null}
            {!busy && report ? tf("已扫描 {0}，可清理 {1}；已选择 {2} 个文件 / {3}。", [bytes(totalBytes), bytes(eligibleBytes), chosenFiles, bytes(chosenBytes)]) : null}
            {!busy && !report && !result && !error ? t("点击扫描查看占用。扫描结果有效期为 10 分钟。") : null}
            {result ? <p>{tf("已删除 {0} 个文件（逻辑大小 {1}），跳过 {2} 个，失败 {3} 个。请重新扫描查看剩余占用。", [result.removedFiles, bytes(result.removedBytes), result.skippedFiles, result.failures.length])}</p> : null}
          </div>
          <Button disabled={busy !== null || chosenFiles === 0} onClick={() => void clean()}>
            <Trash2 className="h-4 w-4" />{t("清理所选缓存")}
          </Button>
        </div>
        {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
        {(report?.warnings.length ?? 0) > 0 ? <details className="text-sm">
          <summary>{t("扫描提示（部分目录可能未统计）")}</summary>
          <ul className="mt-2 space-y-1 break-all">{report?.warnings.map((warning, index) => <li key={index}>{t(warning)}</li>)}</ul>
        </details> : null}
        {(result?.failures.length ?? 0) > 0 ? <details className="text-sm">
          <summary>{t("查看清理失败的文件")}</summary>
          <ul className="mt-2 space-y-1 break-all">{result?.failures.map((failure, index) => <li key={index}>{failure}</li>)}</ul>
        </details> : null}
        <p className="text-xs text-muted-foreground">{t("大小按文件逻辑长度统计，实际释放空间可能不同。缓存可能在应用重启后重新生成。")}</p>
      </CardContent>
    </Card>
  );
}
