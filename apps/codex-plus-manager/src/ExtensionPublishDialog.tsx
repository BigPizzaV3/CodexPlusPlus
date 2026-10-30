import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { CheckCircle2, ExternalLink, Loader2, XCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { t, tf } from "./i18n";

// 与 crates/codex-plus-core/src/extension_publish.rs 的 CheckReport / publish 结果对应。
type Report = {
  id: string;
  name: string;
  version: string;
  description: string;
  errors: string[];
  warnings: string[];
  secrets: { file: string; line: number; kind: string }[];
  files: number;
  bytes: number;
  hasReadme: boolean;
  hasLicense: boolean;
  publishedRepo: string | null;
  suggestedRepoName: string;
  gh: { ready: boolean; account?: string; message?: string };
};
type Step = { step: string; ok: boolean; output: string };
// 后端每步开始发 running、结束发 ok / error（extension_publish_commands.rs 的 PUBLISH_PROGRESS_EVENT）。
type StepEvent = { index: number; step: string; state: "running" | "ok" | "error"; output: string };
type LiveStep = { step: string; state: StepEvent["state"]; output: string };
type PublishResult = { ok: boolean; repo?: string; url?: string; version?: string; error?: string; steps: Step[] };
type License = "mit" | "apache-2.0" | "none";

function request<T>(action: string, payload: unknown): Promise<T> {
  return invoke<T>("extension_publish_request", { action, payload });
}

function errorText(error: unknown): string {
  return typeof error === "string" ? error : error instanceof Error ? error.message : String(error);
}

/** 本地拓展包「发布到 GitHub」：检查 → 选仓库名与许可证 → 发布，逐步显示 gh / git 的输出。 */
export function ExtensionPublishDialog({ id, onClose, onOpenUrl }: { id: string; onClose: () => void; onOpenUrl: (url: string) => void }) {
  const [report, setReport] = useState<Report | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const [repoName, setRepoName] = useState("");
  const [license, setLicense] = useState<License>("mit");
  const [result, setResult] = useState<PublishResult | null>(null);
  const [liveSteps, setLiveSteps] = useState<LiveStep[]>([]);

  const load = async (action: "check" | "bump", part?: "patch" | "minor") => {
    setBusy(true);
    setError("");
    try {
      const next = await request<Report>(action, { id, part });
      setReport(next);
      setRepoName((current) => current || next.suggestedRepoName);
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setBusy(false);
    }
  };

  useEffect(() => {
    void load("check");
    // 只在打开时检查一次。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [id]);

  const publish = async () => {
    setBusy(true);
    setError("");
    setResult(null);
    setLiveSteps([]);
    const unlisten = await listen<StepEvent>("extension-publish-progress", ({ payload }) => {
      setLiveSteps((current) => {
        const next = [...current];
        next[payload.index] = { step: payload.step, state: payload.state, output: payload.output };
        return next;
      });
    });
    try {
      setResult(await request<PublishResult>("publish", { id, repoName, license: report?.hasLicense ? "none" : license, generateReadme: true }));
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      unlisten();
      setBusy(false);
    }
  };

  // 发布中显示推送来的进度；结束后以返回结果为准（事件丢了也不影响最终显示）。
  const steps: LiveStep[] = result ? result.steps.map((item) => ({ step: item.step, state: item.ok ? "ok" : "error", output: item.output })) : liveSteps;
  const versionTaken = result?.error?.includes("已经有 v") ?? false;
  const canPublish = !!report && report.errors.length === 0 && report.gh.ready && !busy && !result?.ok;
  return (
    <div aria-modal="true" className="modal-backdrop app-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) onClose(); }} role="dialog">
      <div className="modal-card confirm-modal app-dialog community-ext-dialog extension-publish-dialog">
        <div className="modal-head">
          <div>
            <h2>{report?.publishedRepo ? t("发布新版本") : t("发布到 GitHub")}</h2>
          </div>
          <button aria-label={t("关闭")} className="toast-close" disabled={busy} onClick={onClose} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          {!report ? (
            <p className="community-ext-dialog-section">{busy ? t("正在检查…") : error}</p>
          ) : (
            <>
              <div className="community-ext-dialog-version">
                <code>{report.name} v{report.version}</code>
                <span>{tf("{0} 个文件，{1} KB", [report.files, Math.ceil(report.bytes / 1024)])}</span>
              </div>
              {report.gh.ready ? (
                <p className="community-ext-dialog-section">{tf("将以 GitHub 账号 {0} 发布为公开仓库，打上 codexpp-extension 标签，其他 Codex++ 用户能在「社区拓展」里看到并安装。", [report.gh.account ?? ""])}</p>
              ) : (
                <p className="community-ext-dialog-warning">{tf("还不能发布：{0}", [report.gh.message ?? ""])}</p>
              )}
              {report.errors.length ? (
                <ul className="community-ext-permissions">
                  {report.errors.map((item) => <li data-risk="high" key={item}>{item}</li>)}
                  {report.secrets.map((hit) => <li data-risk="high" key={`${hit.file}:${hit.line}`}>{hit.line ? tf("{0} 第 {1} 行：{2}", [hit.file, hit.line, hit.kind]) : tf("{0}：{1}", [hit.file, hit.kind])}</li>)}
                </ul>
              ) : null}
              {report.warnings.length ? (
                <ul className="community-ext-permissions">
                  {report.warnings.map((item) => <li data-risk="medium" key={item}>{item}</li>)}
                </ul>
              ) : null}
              <label className="extension-publish-field">
                <span>{t("仓库名")}</span>
                <Input aria-label={t("仓库名")} disabled={busy || !!report.publishedRepo} onChange={(event) => setRepoName(event.currentTarget.value)} value={repoName} />
              </label>
              {!report.hasLicense ? (
                <div className="extension-publish-field">
                  <span>{t("许可证")}</span>
                  <div className="community-ext-sort" role="group" aria-label={t("许可证")}>
                    {(["mit", "apache-2.0", "none"] as License[]).map((value) => (
                      <Button aria-pressed={license === value} disabled={busy} key={value} onClick={() => setLicense(value)} size="sm" variant={license === value ? "secondary" : "ghost"}>
                        {value === "mit" ? "MIT" : value === "apache-2.0" ? "Apache-2.0" : t("不加")}
                      </Button>
                    ))}
                  </div>
                </div>
              ) : null}
              <div className="extension-publish-field">
                <span>{tf("版本 {0}", [report.version])}</span>
                <div className="community-ext-sort">
                  <Button disabled={busy} onClick={() => void load("bump", "patch")} size="sm" variant={versionTaken ? "secondary" : "ghost"}>{t("补丁号 +1")}</Button>
                  <Button disabled={busy} onClick={() => void load("bump", "minor")} size="sm" variant="ghost">{t("次版本号 +1")}</Button>
                </div>
              </div>
            </>
          )}
          {steps.length ? (
            <ol className="extension-publish-steps">
              {steps.map((item, index) => (
                <li data-ok={item.state === "running" ? undefined : item.state === "ok"} data-state={item.state} key={index}>
                  {item.state === "running" ? <Loader2 className="h-4 w-4 animate-spin" /> : item.state === "ok" ? <CheckCircle2 className="h-4 w-4" /> : <XCircle className="h-4 w-4" />}
                  <div>
                    <strong>{item.step}</strong>
                    {item.output ? <pre>{item.output}</pre> : null}
                  </div>
                </li>
              ))}
            </ol>
          ) : null}
          {result?.ok && result.url ? (
            <p className="community-ext-dialog-trusted">
              {tf("已发布 {0} v{1}。社区栏最长 10 分钟后可见；想进入精选，可以向市场仓库提 PR。", [result.repo ?? "", result.version ?? ""])}
              {" "}
              <button className="community-ext-repo" onClick={() => onOpenUrl(result.url!)} type="button"><ExternalLink className="h-3 w-3" />{result.repo}</button>
            </p>
          ) : null}
          {result && !result.ok ? <p className="community-ext-dialog-warning">{result.error}</p> : null}
          {report && error ? <p className="community-ext-dialog-warning">{error}</p> : null}
        </div>
        <div className="app-dialog-actions">
          <Button disabled={busy} onClick={onClose} variant="secondary">{result?.ok ? t("完成") : t("取消")}</Button>
          {result?.ok ? null : (
            <Button disabled={!canPublish} onClick={() => void publish()}>
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : null}
              {busy ? t("正在发布…") : t("发布")}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
