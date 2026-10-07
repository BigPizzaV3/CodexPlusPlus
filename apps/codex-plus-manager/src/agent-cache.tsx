import { invoke } from "@tauri-apps/api/core";
import { confirm } from "@tauri-apps/plugin-dialog";
import { useEffect, useRef, useState } from "react";
import { RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { t, tf } from "./i18n";

type CacheGroup = {
  id: string;
  app: string;
  kind: string;
  path: string;
  totalBytes: number;
  eligibleBytes: number;
  eligibleFiles: number;
  skippedFiles: number;
  cleanable: boolean;
};
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

  const chosen = report?.groups.filter((group) => selected.includes(group.id)) ?? [];
  const chosenBytes = chosen.reduce((sum, group) => sum + group.eligibleBytes, 0);
  const chosenFiles = chosen.reduce((sum, group) => sum + group.eligibleFiles, 0);
  const totalBytes = report?.groups.reduce((sum, group) => sum + group.totalBytes, 0) ?? 0;
  const eligibleBytes = report?.groups.reduce((sum, group) => sum + group.eligibleBytes, 0) ?? 0;

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
        "将永久删除以下目录中扫描到的 {0} 个缓存文件（逻辑大小 {1}）：\n\n{2}\n\n请先退出相关 AI 应用及 CLI。只处理超过 24 小时未修改的文件。日志也会被删除（若已选择）。此操作不可撤销。是否继续？",
        [chosenFiles, bytes(chosenBytes), chosen.map((group) => `${group.app} · ${kindLabel(group.kind)}\n${group.path}`).join("\n\n")],
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
    <Card className="panel">
      <CardHeader className="panel-head">
        <CardTitle>{t("AI Agent 缓存清理")}</CardTitle>
        <CardDescription>{t("扫描 Codex、Claude 与 Codex++ 的已知缓存目录，由你选择清理项目。")}</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <p className="text-sm text-muted-foreground">
          {t("会话、凭据、配置、插件和虚拟机资源不在清理范围。临时文件仅统计；最近 24 小时修改的文件、链接及变化的文件会跳过。")}
        </p>
        <div className="flex flex-wrap items-center gap-2">
          <Button disabled={busy !== null} onClick={() => void scan()} variant="secondary">
            <RefreshCw className={`h-4 w-4 ${busy === "scan" ? "animate-spin" : ""}`} />
            {busy === "scan" ? t("正在扫描缓存…") : t("扫描缓存")}
          </Button>
          {report ? <>
            <Button disabled={busy !== null} variant="outline" onClick={() => setSelected(
              report.groups.filter((group) => group.cleanable && group.eligibleFiles > 0 && group.kind !== "logs").map((group) => group.id),
            )}>{t("选择缓存（不含日志）")}</Button>
            <Button disabled={busy !== null || selected.length === 0} variant="outline" onClick={() => setSelected([])}>{t("取消选择")}</Button>
            <Button disabled={busy !== null || chosenFiles === 0} onClick={() => void clean()}>
              <Trash2 className="h-4 w-4" />
              {t("清理所选缓存")}
            </Button>
          </> : null}
        </div>
        <div role="status" aria-live="polite" aria-busy={busy !== null} className="text-sm">
          {busy === "clean" ? t("正在确认或清理缓存…") : null}
          {!busy && report ? tf("已扫描 {0}，可清理 {1}；已选择 {2} 个文件 / {3}。", [bytes(totalBytes), bytes(eligibleBytes), chosenFiles, bytes(chosenBytes)]) : null}
          {!busy && !report && !result && !error ? t("点击扫描查看占用。扫描结果有效期为 10 分钟。") : null}
          {result ? <p>{tf("已删除 {0} 个文件（逻辑大小 {1}），跳过 {2} 个，失败 {3} 个。请重新扫描查看剩余占用。", [result.removedFiles, bytes(result.removedBytes), result.skippedFiles, result.failures.length])}</p> : null}
        </div>
        {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
        {report?.groups.length === 0 ? <p className="text-sm text-muted-foreground">{t("未发现可识别的缓存目录。")}</p> : null}
        {report?.groups.map((group) => (
          <label key={group.id} className="flex items-start gap-3 rounded-lg border p-3">
            <input className="mt-1" type="checkbox" checked={selected.includes(group.id)}
              disabled={busy !== null || !group.cleanable || group.eligibleFiles === 0}
              onChange={(event) => setSelected(event.currentTarget.checked ? [...selected, group.id] : selected.filter((id) => id !== group.id))} />
            <span className="min-w-0 flex-1 space-y-1 text-sm">
              <span className="block font-medium">{group.app} · {kindLabel(group.kind)}</span>
              <code className="block break-all text-xs text-muted-foreground">{group.path}</code>
              <span className="block">{tf("占用 {0} · 可清理 {1} / {2} 个文件 · 跳过 {3} 个", [bytes(group.totalBytes), bytes(group.eligibleBytes), group.eligibleFiles, group.skippedFiles])}</span>
            </span>
          </label>
        ))}
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
