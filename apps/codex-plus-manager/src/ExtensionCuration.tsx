import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { CheckCircle2, ExternalLink, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { errorText, permissionLines, type Prepared } from "./CommunityExtensionsPanel";
import { t, tf } from "./i18n";

// 与 src-tauri/src/extension_curation_commands.rs 对应。
export type CurationStatus = { maintainer: boolean; account?: string; marketRepo?: string; message?: string };
export type CuratedUpdate = { id: string; repo: string; currentTag: string | null; currentSha: string; latestTag: string | null; latestSha: string };
type DiffFile = { filename: string; status: string; additions: number; deletions: number; patch: string; truncated: boolean };
type Review = Prepared & {
  inspection: { files: { path: string; bytes: number }[]; bytes: number; secrets: { file: string; line: number; kind: string }[]; problems: string[] };
  entry: { name: string; description: string; author: string; official: boolean };
  current: { tag: string | null; sha: string; version: string; official: boolean } | null;
  /** 仓库属于官方账号，可以勾「标为官方」。 */
  canBeOfficial: boolean;
  diff?: { commits: number; url: string | null; files: DiffFile[] };
};

export function curationRequest<T>(action: string, payload?: unknown): Promise<T> {
  return invoke<T>("extension_curation_request", { action, payload: payload ?? null });
}

// 维护者身份整个会话只查一次（要跑 gh），各面板共用。
let statusPromise: Promise<CurationStatus> | null = null;

export function useCurationStatus(): CurationStatus | null {
  const [status, setStatus] = useState<CurationStatus | null>(null);
  useEffect(() => {
    let alive = true;
    statusPromise ??= curationRequest<CurationStatus>("status").catch(() => ({ maintainer: false }));
    void statusPromise.then((value) => { if (alive) setStatus(value); });
    return () => { alive = false; };
  }, []);
  return status;
}

/**
 * 维护者审核：下载并暂存仓库当前的最新版本，列出文件、能力、密钥扫描；精选里已有旧版本时附上差异。
 * 通过后由后端按暂存的 repo / tag / sha 写进精选清单。
 */
export function CurationReviewDialog({ repo, onClose, onApproved, onOpenUrl }: { repo: string; onClose: () => void; onApproved: (message: string) => void; onOpenUrl: (url: string) => void }) {
  const [review, setReview] = useState<Review | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(true);
  // 审核通过只算精选；官方要明确勾选。更新时沿用当前的标记。
  const [official, setOfficial] = useState(false);

  useEffect(() => {
    let alive = true;
    let token = "";
    curationRequest<Review>("review", { repo })
      .then((value) => {
        token = value.token;
        if (alive) {
          setReview(value);
          setOfficial(value.canBeOfficial && !!value.current?.official);
        }
        else void curationRequest("discard", { token }).catch(() => undefined);
      })
      .catch((failure) => { if (alive) setError(errorText(failure)); })
      .finally(() => { if (alive) setBusy(false); });
    return () => {
      alive = false;
    };
  }, [repo]);

  const close = () => {
    if (review) void curationRequest("discard", { token: review.token }).catch(() => undefined);
    onClose();
  };

  const approve = async () => {
    if (!review) return;
    setBusy(true);
    setError("");
    try {
      const result = await curationRequest<{ commitUrl: string; replaced: boolean }>("approve", { token: review.token, official: official && review.canBeOfficial });
      const version = review.tag ?? review.version;
      onApproved(
        unchanged
          ? official ? tf("{0} 已标为官方。", [review.name]) : tf("{0} 已取消官方标记，按精选显示。", [review.name])
          : result.replaced
          ? tf("{0} 已更新到 {1}，用户那边会看到「更新」。", [review.name, version])
          : tf("{0} {1} 已收录进精选。", [review.name, version]),
      );
    } catch (failure) {
      setError(errorText(failure));
      setBusy(false);
    }
  };

  const lines = review ? permissionLines(review.permissions) : [];
  const isUpdate = !!review?.current;
  const unchanged = !!review?.current && review.current.sha.toLowerCase() === review.sha.toLowerCase();
  // 版本没变时，只有改了官方标记才有东西可提交。
  const flagChanged = !!review?.current && review.canBeOfficial && official !== review.current.official;
  const blocked = !!review && (review.inspection.secrets.length > 0 || review.inspection.problems.length > 0);
  return (
    <div aria-modal="true" className="modal-backdrop app-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget && !busy) close(); }} role="dialog">
      <div className="modal-card confirm-modal app-dialog community-ext-dialog curation-dialog">
        <div className="modal-head">
          <div>
            <h2>{review ? (isUpdate ? tf("审核更新：{0}", [review.name]) : tf("审核：{0}", [review.name])) : t("审核拓展")}</h2>
          </div>
          <button aria-label={t("关闭")} className="toast-close" disabled={busy && !!review} onClick={close} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          {!review ? (
            <p className="community-ext-dialog-section">{busy ? tf("正在下载 {0} 的最新版本…", [repo]) : error}</p>
          ) : (
            <>
              <div className="community-ext-dialog-version">
                <code>{review.repo}</code>
                <span>
                  {review.fromDefaultBranch
                    ? tf("作者未发布版本，将锁定默认分支的提交 {0}", [review.sha.slice(0, 7)])
                    : tf("版本 {0}，锁定提交 {1}", [review.tag ?? review.version, review.sha.slice(0, 7)])}
                  {" · "}
                  {tf("{0} 个文件，{1} KB", [review.inspection.files.length, Math.ceil(review.inspection.bytes / 1024)])}
                </span>
                {review.current ? <span>{tf("精选里当前锁定 {0}（{1}）", [review.current.tag ?? review.current.version, review.current.sha.slice(0, 7)])}</span> : null}
              </div>
              {review.canBeOfficial ? (
                <label className="community-ext-dialog-enable" title={t("只给 Codex++ 自己维护的拓展用；审核通过的第三方拓展显示为「精选」")}>
                  <input checked={official} disabled={busy} onChange={(event) => setOfficial(event.currentTarget.checked)} type="checkbox" />
                  {t("标为官方拓展（Codex++ 自己维护的）")}
                </label>
              ) : null}
              <button className="community-ext-repo" onClick={() => onOpenUrl(`https://github.com/${review.repo}/tree/${review.sha}`)} type="button">
                <ExternalLink className="h-3 w-3" />{t("在 GitHub 上看这个提交的代码")}
              </button>
              <div className="community-ext-dialog-section">{t("它会获得这些能力：")}</div>
              {lines.length ? (
                <ul className="community-ext-permissions">{lines.map((line, index) => <li data-risk={line.risk} key={index}>{line.text}</li>)}</ul>
              ) : (
                <p className="community-ext-dialog-section">{t("没有声明任何能力。")}</p>
              )}
              {blocked ? (
                <ul className="community-ext-permissions">
                  {review.inspection.problems.map((item) => <li data-risk="high" key={item}>{item}</li>)}
                  {review.inspection.secrets.map((hit) => <li data-risk="high" key={`${hit.file}:${hit.line}`}>{hit.line ? tf("疑似密钥：{0} 第 {1} 行（{2}）", [hit.file, hit.line, hit.kind]) : tf("疑似密钥：{0}（{1}）", [hit.file, hit.kind])}</li>)}
                </ul>
              ) : (
                <p className="community-ext-dialog-section">{t("密钥扫描没有发现问题。")}</p>
              )}
              {unchanged ? <p className="community-ext-dialog-section">{t("仓库最新版本就是精选里锁定的那一版，不需要更新。")}</p> : null}
              {review.diff ? (
                <div className="curation-diff">
                  <div className="curation-diff-head">
                    {tf("与当前锁定版本相比：{0} 个提交，{1} 个文件改动", [review.diff.commits, review.diff.files.length])}
                    {review.diff.url ? <button className="community-ext-repo" onClick={() => onOpenUrl(review.diff!.url!)} type="button"><ExternalLink className="h-3 w-3" />{t("完整差异")}</button> : null}
                  </div>
                  {review.diff.files.map((file) => (
                    <details key={file.filename}>
                      <summary>
                        <code>{file.filename}</code>
                        <span className="curation-diff-stat"><em>+{file.additions}</em> <i>−{file.deletions}</i></span>
                      </summary>
                      {file.patch ? <pre>{file.patch.split("\n").map((line, index) => <span data-kind={line.startsWith("+") ? "add" : line.startsWith("-") ? "del" : line.startsWith("@@") ? "hunk" : undefined} key={index}>{line}{"\n"}</span>)}</pre> : null}
                      {file.truncated ? <p className="community-ext-dialog-section">{t("差异太长，已截断，完整内容去 GitHub 看。")}</p> : null}
                    </details>
                  ))}
                </div>
              ) : (
                <details className="curation-files">
                  <summary>{tf("文件列表（{0}）", [review.inspection.files.length])}</summary>
                  <ul>{review.inspection.files.map((file) => <li key={file.path}><code>{file.path}</code><span>{Math.max(1, Math.ceil(file.bytes / 1024))} KB</span></li>)}</ul>
                </details>
              )}
            </>
          )}
          {review && error ? <p className="community-ext-dialog-warning">{error}</p> : null}
        </div>
        <div className="app-dialog-actions">
          <Button disabled={busy && !!review} onClick={close} variant="secondary">{t("取消")}</Button>
          {review ? (
            <Button disabled={busy || blocked || (unchanged && !flagChanged)} onClick={() => void approve()} title={blocked ? t("先请作者处理疑似密钥或软链再收录") : undefined}>
              {busy ? <Loader2 className="h-4 w-4 animate-spin" /> : <CheckCircle2 className="h-4 w-4" />}
              {unchanged ? (official ? t("标为官方") : t("取消官方标记")) : isUpdate ? tf("更新到 {0}", [review.tag ?? review.version]) : t("通过，加入精选")}
            </Button>
          ) : null}
        </div>
      </div>
    </div>
  );
}
