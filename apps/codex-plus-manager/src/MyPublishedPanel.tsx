import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArchiveRestore, Check, Copy, EyeOff, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { t, tf } from "./i18n";
import { ExtensionCard, githubAvatar } from "./ExtensionCard";
import { CxBadge, CxCard, CxEmpty, CxSection } from "./cx-layout";
import { MenuItem } from "./ManagementPages";
import { useEscapeLayer } from "./use-escape-layer";
import { errorText } from "./CommunityExtensionsPanel";
import "./CommunityExtensionsPanel.css";

// 与 crates/codex-plus-core/src/extension_publish.rs 的 list_published 对应。
type PublishedEntry = {
  repo: string;
  url: string;
  description: string;
  stars: number;
  pushedAt: string;
  visibility: string;
  archived: boolean;
  /** 带 codexpp-extension topic：社区栏里能看到。 */
  listed: boolean;
  /** 本机发布记录里的拓展 id / 版本（从别的机器发布的没有）。 */
  id: string | null;
  version: string | null;
};
type Listing = { account: string; entries: PublishedEntry[] };

function request<T>(action: string, payload?: unknown): Promise<T> {
  return invoke<T>("extension_publish_request", { action, payload: payload ?? null });
}

/** 「我发布的」：当前 gh 账号名下的拓展仓库，可下架（去掉 topic）、重新上架、删除仓库。 */
export function MyPublishedPanel({ onOpenUrl, search = "" }: { onOpenUrl: (url: string) => void; search?: string }) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");

  const load = useCallback(async () => {
    setLoading(true);
    setError("");
    try {
      setListing(await request<Listing>("mine"));
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const run = async (entry: PublishedEntry, action: "unlist" | "relist" | "deleteRepo", done: string) => {
    setBusy(entry.repo);
    setError("");
    setNotice("");
    try {
      await request(action, { repo: entry.repo });
      setNotice(done);
      await load();
    } catch (failure) {
      setError(errorText(failure));
    } finally {
      setBusy("");
    }
  };

  // 下架可以随时重新上架，不再弹确认；说明放在按钮悬停里。删除仓库仍要输入仓库名确认。
  const unlist = (entry: PublishedEntry) => run(entry, "unlist", tf("已下架 {0}", [entry.repo]));

  const [deleting, setDeleting] = useState<PublishedEntry | null>(null);

  const query = search.trim().toLocaleLowerCase();
  const entries = (listing?.entries ?? []).filter((entry) => !query || [entry.repo, entry.id ?? "", entry.description].join(" ").toLocaleLowerCase().includes(query));
  const row = (entry: PublishedEntry) => {
    const pending = busy === entry.repo;
    const facts = [
      entry.repo,
      tf("★ {0}", [entry.stars]),
      entry.version ? tf("本机发布 v{0}", [entry.version]) : "",
      entry.pushedAt ? tf("更新于 {0}", [entry.pushedAt.slice(0, 10)]) : "",
    ].filter(Boolean).join(" · ");
    return (
      <ExtensionCard
        badges={
          <>
            <CxBadge tone={entry.listed ? "ok" : "neutral"}>{entry.listed ? t("已上架") : t("已下架")}</CxBadge>
            {entry.visibility && entry.visibility !== "PUBLIC" ? <CxBadge>{t("私有")}</CxBadge> : null}
            {entry.archived ? <CxBadge>{t("已归档")}</CxBadge> : null}
          </>
        }
        description={entry.description}
        dimmed={!entry.listed}
        icon={githubAvatar(entry.repo)}
        key={entry.repo}
        link={{ url: entry.url, text: entry.repo }}
        menu={
          <MenuItem danger disabled={busy !== ""} onClick={() => setDeleting(entry)}>
            <span className="ext-row-menu-item"><Trash2 size={14} />{t("删除仓库")}</span>
          </MenuItem>
        }
        meta={facts}
        name={entry.id || entry.repo.split("/")[1]}
        onOpenUrl={onOpenUrl}
        primary={
          entry.listed ? (
            <button className="cx-secondary" disabled={busy !== ""} onClick={() => void unlist(entry)} title={tf("下架后 {0} 不再出现在社区拓展里，已安装的用户不受影响；仓库和 release 都保留，之后可以重新上架。", [entry.repo])} type="button">
              {pending ? <Loader2 className="animate-spin" /> : <EyeOff />}
              {t("下架")}
            </button>
          ) : (
            <button className="cx-secondary" disabled={busy !== ""} onClick={() => void run(entry, "relist", tf("已重新上架 {0}，社区列表刷新后可见", [entry.repo]))} type="button">
              {pending ? <Loader2 className="animate-spin" /> : <ArchiveRestore />}
              {t("重新上架")}
            </button>
          )
        }
      />
    );
  };
  return (
    <>
      {error ? <div className="community-ext-notice is-error">{error}</div> : null}
      {notice ? <div className="community-ext-notice">{notice}</div> : null}
      <CxSection
        aside={
          <button aria-label={t("刷新")} className="mg-icon-button" disabled={loading || busy !== ""} onClick={() => void load()} title={t("刷新")} type="button">
            {loading ? <Loader2 className="animate-spin" size={14} /> : <RefreshCw size={14} />}
          </button>
        }
        count={listing ? entries.length : undefined}
        title={<span title={t("列出当前 gh 账号名下带 codexpp-extension 标签的仓库，以及从本机发布、已下架的仓库。下架只是去掉标签，仓库保留；删除仓库不可恢复。")}>{listing?.account ? tf("GitHub 账号 {0} 发布的拓展", [listing.account]) : t("我发布的拓展")}</span>}
      >
        <CxCard>
          {entries.length ? entries.map(row) : (
            <CxEmpty title={loading ? t("正在读取…") : listing?.entries?.length ? t("没有匹配的拓展。") : t("还没有发布过拓展。")} detail={!loading && !listing?.entries?.length ? t("在「本地」里点拓展包的「…」→「发布到 GitHub」。") : undefined} />
          )}
        </CxCard>
      </CxSection>
      {deleting ? (
        <DeleteRepoDialog
          onCancel={() => setDeleting(null)}
          onConfirm={() => {
            const entry = deleting;
            setDeleting(null);
            void run(entry, "deleteRepo", tf("已删除仓库 {0}", [entry.repo]));
          }}
          repo={deleting.repo}
        />
      ) : null}
    </>
  );
}

/**
 * 删除仓库的确认框：仓库名放在可选中的 code 里并带「复制」按钮，粘贴进输入框、完全一致后才能删。
 * 不用通用的 appPrompt：那里的说明文字不能选中，用户只能手打仓库名。
 */
function DeleteRepoDialog({ repo, onCancel, onConfirm }: { repo: string; onCancel: () => void; onConfirm: () => void }) {
  const [value, setValue] = useState("");
  const [copied, setCopied] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  const matches = value.trim() === repo;
  useEscapeLayer(true, onCancel);
  useEffect(() => inputRef.current?.focus(), []);
  const copy = () => {
    void navigator.clipboard?.writeText(repo).then(() => {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    }).catch(() => undefined);
  };
  return (
    <div aria-modal="true" className="modal-backdrop app-dialog-backdrop" onMouseDown={(event) => { if (event.target === event.currentTarget) onCancel(); }} role="dialog">
      <div className="modal-card confirm-modal app-dialog" data-danger>
        <div className="modal-head">
          <div>
            <h2>{t("删除 GitHub 仓库")}</h2>
          </div>
          <button aria-label={t("关闭")} className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          <p className="modal-message app-dialog-message">{t("将永久删除这个 GitHub 仓库（含所有提交、release 和 star），无法恢复；本机的拓展包不会被删除。确认请把仓库名粘贴到下面：")}</p>
          <div className="my-published-repo-copy">
            <code>{repo}</code>
            <Button onClick={copy} size="sm" type="button" variant="secondary">
              {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
              {copied ? t("已复制") : t("复制")}
            </Button>
          </div>
          <input
            className="app-dialog-input"
            onChange={(event) => setValue(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter" && matches) {
                event.preventDefault();
                onConfirm();
              }
            }}
            placeholder={repo}
            ref={inputRef}
            value={value}
          />
        </div>
        <div className="app-dialog-actions">
          <Button onClick={onCancel} variant="secondary">{t("取消")}</Button>
          <Button className="app-dialog-danger" disabled={!matches} onClick={onConfirm}>{t("永久删除")}</Button>
        </div>
      </div>
    </div>
  );
}
