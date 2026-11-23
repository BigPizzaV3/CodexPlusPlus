// 主界面的全局对话框：更新、通知、配置冲突、会话索引清理、待导入供应商、梦幻皮肤社区链接。
import { CheckCircle2, Copy, Download, Info, Loader2, Trash2, TriangleAlert, CircleX, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Button } from "@/components/ui/button";
import { noticeDuration, noticeTone } from "./notice-model";
import { ReleaseNotes } from "./ReleaseNotes";
import { t, tf } from "@/i18n";
import type { LiveConflictResolution, NoticeState, ProviderImportRequest, SessionIndexCleanupCandidate, TaskProgress, UpdateResult } from "./app-types";
import { Metric, TaskProgressBox, Toolbar } from "./app-shared";

export function UpdateDialog({ update, progress, downloaded = false, onClose, onSkipVersion, onMuteToday, onUpdate, onOpenUrl }: {
  update: UpdateResult;
  progress: TaskProgress;
  /** 安装包已在后台下好：主按钮写「立即安装」。 */
  downloaded?: boolean;
  onClose: () => void;
  onSkipVersion: () => void;
  onMuteToday: () => void;
  onUpdate: () => void;
  onOpenUrl: (url: string) => void;
}) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    if (dialog && !dialog.open) dialog.showModal();
    return () => { if (dialog?.open) dialog.close(); };
  }, []);

  return (
    <dialog className="modal-card update-dialog" ref={dialogRef} aria-labelledby="update-dialog-title" aria-describedby="update-dialog-description" onCancel={(event) => { event.preventDefault(); onClose(); }}>
      <div className="modal-head">
        <div>
          <h2 id="update-dialog-title">{tf("发现新版本 {0}", [update.latestVersion ?? ""])}</h2>
          <p id="update-dialog-description">{tf("当前版本 {0}", [update.currentVersion])}</p>
        </div>
        <button className="toast-close" onClick={onClose} type="button" aria-label={t("关闭更新提示")}>×</button>
      </div>
      <div className="update-dialog-body">
        <ReleaseNotes markdown={update.releaseSummary || t("发现可用更新。")} onOpenUrl={onOpenUrl} />
      </div>
      <TaskProgressBox completedTitle={t("上次更新结果")} progress={progress} title={t("安装包更新进度")} />
      <p className="update-dialog-hint">{t("也可随时在「设置 → 版本与更新」里检查和安装。")}</p>
      {!update.assetName || !update.assetUrl ? <p className="update-dialog-hint">{t("此版本暂未提供适用于当前系统的安装包。")}</p> : null}
      <Toolbar className="update-dialog-actions">
        <Button autoFocus disabled={progress.active || !update.assetName || !update.assetUrl} onClick={onUpdate}>
          <Download className="h-4 w-4" />
          {progress.active ? t("正在安装…") : downloaded ? t("立即安装") : t("立即更新")}
        </Button>
        <Button variant="secondary" onClick={onClose}>{t("关闭提示")}</Button>
        <Button variant="ghost" onClick={onMuteToday}>{t("今日不再提示")}</Button>
        <Button variant="ghost" onClick={onSkipVersion}>{t("跳过此版本")}</Button>
      </Toolbar>
    </dialog>
  );
}

export function NoticeDialog({
  notice,
  onClose,
}: {
  notice: NoticeState;
  onClose: () => void;
}) {
  const tone = noticeTone(notice.status);
  const duration = noticeDuration(tone, notice.message);
  const [paused, setPaused] = useState(false);
  const [copied, setCopied] = useState(false);
  const remaining = useRef(duration);

  useEffect(() => {
    if (paused) return;
    const startedAt = Date.now();
    const timer = window.setTimeout(onClose, Math.max(0, remaining.current));
    return () => {
      window.clearTimeout(timer);
      remaining.current -= Date.now() - startedAt;
    };
    // onClose 每次渲染都是新函数，倒计时只随暂停状态变化。
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [paused]);

  const Icon = tone === "error" ? CircleX : tone === "warning" ? TriangleAlert : tone === "info" ? Info : CheckCircle2;
  const copyDetails = () => {
    void navigator.clipboard?.writeText(`${notice.title}\n${notice.message}`).then(() => setCopied(true)).catch(() => undefined);
  };

  return (
    <div className="toast-wrap" role={tone === "error" ? "alert" : "status"} aria-live={tone === "error" ? "assertive" : "polite"}>
      <div
        className="toast-card"
        data-tone={tone}
        onBlur={() => setPaused(false)}
        onFocus={() => setPaused(true)}
        onMouseEnter={() => setPaused(true)}
        onMouseLeave={() => setPaused(false)}
      >
        <span className="toast-icon"><Icon aria-hidden="true" /></span>
        <div className="toast-body">
          <h2>{notice.title}</h2>
          {notice.message ? <p>{notice.message}</p> : null}
          {notice.action ? (
            <button className="toast-action" onClick={() => { notice.action?.run(); onClose(); }} type="button">
              {notice.action.label}
            </button>
          ) : null}
          {tone === "error" && notice.message ? (
            <button className="toast-action" onClick={copyDetails} type="button">
              {copied ? <CheckCircle2 aria-hidden="true" /> : <Copy aria-hidden="true" />}
              {copied ? t("已复制") : t("复制详情")}
            </button>
          ) : null}
        </div>
        <button aria-label={t("关闭提示")} className="toast-dismiss" onClick={onClose} type="button">
          <X aria-hidden="true" />
        </button>
      </div>
    </div>
  );
}

/** 切换时实时配置 endpoint 被外部改过（#2666）：让用户选导入还是放弃，不再只给报错。 */
export function LiveConflictDialog({
  profileName,
  onChoose,
}: {
  profileName: string;
  onChoose: (choice: LiveConflictResolution | null) => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card confirm-modal">
        <div className="modal-head">
          <div>
            <h2>{t("实时配置已被修改")}</h2>
          </div>
          <button className="toast-close" onClick={() => onChoose(null)} type="button">×</button>
        </div>
        <div className="confirm-modal-body">
          <p className="modal-message">
            {tf("当前生效的 config.toml / auth.json 里的供应商地址，和「{0}」保存的不一样。可能是其它工具改过，也可能是你手动改的，Codex++ 无法自动判断该保留哪一份。", [profileName])}
          </p>
          <p className="modal-message">{tf("导入为新供应商：把当前生效的地址和 Key 保存成一个新供应商，「{0}」保持原样，然后继续切换。", [profileName])}</p>
          <p className="modal-message">{tf("放弃实时改动：先把当前生效的文件备份一份，「{0}」保持原样，然后继续切换。", [profileName])}</p>
        </div>
        <Toolbar className="confirm-modal-actions">
          <Button onClick={() => onChoose("importAsNewProfile")}>
            <Download className="h-4 w-4" />
            {t("导入为新供应商")}
          </Button>
          <Button onClick={() => onChoose("discardLiveChanges")} variant="secondary">
            {t("放弃实时改动、继续切换")}
          </Button>
          <Button onClick={() => onChoose(null)} variant="ghost">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

export function SessionIndexCleanupDialog({
  request,
  onConfirm,
  onCancel,
}: {
  request: { candidates: SessionIndexCleanupCandidate[] };
  onConfirm: (selectedIds: string[]) => void;
  onCancel: () => void;
}) {
  const [selectedIds, setSelectedIds] = useState<Set<string>>(() => new Set());
  const allSelected = request.candidates.length > 0 && selectedIds.size === request.candidates.length;
  const toggleCandidate = (id: string, selected: boolean) => {
    setSelectedIds((current) => {
      const next = new Set(current);
      if (selected) next.add(id);
      else next.delete(id);
      return next;
    });
  };

  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card session-index-cleanup-modal">
        <div className="modal-head">
          <div>
            <h2>{t("清理幽灵任务索引")}</h2>
            <p className="modal-message">
              {tf("发现 {0} 条仅存在于 session_index.jsonl、未在本地数据库或 rollout 中找到来源的候选记录。它们也可能是云端或尚未落盘的任务，请逐项核对。任务标题仅用于预览，实际按 thread ID 与数据来源判断。清理前请先完全退出 Codex App / ChatGPT。", [request.candidates.length])}
            </p>
          </div>
          <button className="toast-close" onClick={onCancel} type="button">×</button>
        </div>
        <label className="session-index-cleanup-select-all">
          <input
            checked={allSelected}
            onChange={(event) => {
              setSelectedIds(event.target.checked ? new Set(request.candidates.map((candidate) => candidate.id)) : new Set());
            }}
            type="checkbox"
          />
          <span>{t("选择全部候选记录")}</span>
        </label>
        <div className="session-index-cleanup-list">
          {request.candidates.map((candidate) => (
            <label className="session-index-cleanup-item" key={candidate.id}>
              <input
                checked={selectedIds.has(candidate.id)}
                onChange={(event) => toggleCandidate(candidate.id, event.target.checked)}
                type="checkbox"
              />
              <span>
                <strong>{candidate.threadName || t("未命名任务")}</strong>
                <code>{candidate.id}</code>
                <small>{candidate.updatedAt}</small>
              </span>
            </label>
          ))}
        </div>
        <Toolbar>
          <Button disabled={selectedIds.size === 0} onClick={() => onConfirm(Array.from(selectedIds))}>
            <Trash2 className="h-4 w-4" />
            {tf("确认清理 {0} 条", [selectedIds.size])}
          </Button>
          <Button onClick={onCancel} variant="secondary">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

export function PendingProviderImportDialog({
  request,
  onConfirm,
  onDismiss,
}: {
  request: ProviderImportRequest;
  onConfirm: () => void;
  onDismiss: () => void;
}) {
  const toPool = request.target === "pool";
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-import-modal">
        <div className="modal-head">
          <div>
            <h2>{t("导入 Codex++ 供应商")}</h2>
            <p>{toPool ? t("检测到来自网页的供应商导入请求，确认后会加进网关共享供应商，并自动从上游获取模型。") : t("检测到来自网页的供应商配置导入请求，确认后会写入本机 Codex++。")}</p>
          </div>
          <button className="toast-close" onClick={onDismiss} type="button">×</button>
        </div>
        <div className="metric-list">
          <Metric label={t("名称")} value={request.name || t("未命名供应商")} />
          <Metric label={t("导入到")} value={toPool ? t("网关共享供应商") : t("直连模式供应商")} />
          <Metric label="Base URL" value={request.baseUrl || t("未填写")} />
          <Metric label={t("协议")} value={providerImportWireApiLabel(request.wireApi)} />
          {toPool ? null : <Metric label={t("模式")} value={providerImportRelayModeLabel(request.relayMode)} />}
          <Metric label="API Key" value={maskSecret(request.apiKey)} />
        </div>
        {toPool ? (
          <div className="hint-line" role="note">
            {t("导入后供应商默认启用，会按「Agent 接入」里各 Agent 的「添加供应商时自动加入」设置立即承接请求。")}
          </div>
        ) : null}
        <div className="hint-line" role="note">
          {t("安全提示：网页链接中的自定义 config.toml 和 auth.json 不会执行；管理工具只会使用上方字段生成受管配置。")}
        </div>
        <Toolbar>
          <Button onClick={onConfirm}>
            <Download className="h-4 w-4" />
            {t("确认导入")}
          </Button>
          <Button onClick={onDismiss} variant="secondary">{t("取消")}</Button>
        </Toolbar>
      </div>
    </div>
  );
}

export function DreamSkinCommunityLinkDialog({
  versionId,
  installPending,
  onConfirm,
  onDismiss,
}: {
  versionId: string;
  installPending: boolean;
  onConfirm: () => void;
  onDismiss: () => void;
}) {
  return (
    <div className="modal-backdrop" role="dialog" aria-modal="true">
      <div className="modal-card provider-import-modal">
        <div className="modal-head">
          <div>
            <h2>{t("从 DreamSkin.cc 安装主题")}</h2>
            <p>{t("检测到网页一键换肤请求。确认后会从固定社区 API 下载，并在本机重新校验大小、SHA-256、ZIP 清单与 Safe CSS。")}</p>
          </div>
          <button aria-label={t("关闭")} className="toast-close" disabled={installPending} onClick={onDismiss} type="button">×</button>
        </div>
        <div className="metric-list">
          <Metric label={t("主题版本 ID")} value={versionId} />
          <Metric label={t("来源")} value="api.dreamskin.cc" />
        </div>
        <div className="hint-line" role="note">
          {t("链接不能携带任意下载地址、文件路径或命令；安装后主题会进入“我的主题”，不会自动重启 Codex。")}
        </div>
        <Toolbar>
          <Button aria-busy={installPending} disabled={installPending} onClick={onConfirm}>
            {installPending
              ? <Loader2 aria-hidden="true" className="h-4 w-4 animate-spin" />
              : <Download className="h-4 w-4" />}
            {installPending ? t("安装中…") : t("下载并安装")}
          </Button>
          <Button disabled={installPending} onClick={onDismiss} variant="secondary">{t("取消")}</Button>
        </Toolbar>
        {installPending ? <DreamSkinInstallProgress /> : null}
      </div>
    </div>
  );
}

function DreamSkinInstallProgress({ label = t("正在下载、校验并安装主题包…") }: { label?: string }) {
  return (
    <div aria-live="polite" className="dream-skin-install-progress" role="status">
      <span>{label}</span>
      <div aria-label={label} className="dream-skin-install-progress-bar" role="progressbar">
        <div />
      </div>
    </div>
  );
}

function providerImportWireApiLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "chat" || normalized === "chat_completions" || normalized === "chat-completions") {
    return "Chat Completions";
  }
  if (normalized === "anthropic" || normalized === "anthropic_messages" || normalized === "anthropic-messages" || normalized === "messages") {
    return "Anthropic Messages";
  }
  return "Responses";
}

function providerImportRelayModeLabel(value: string): string {
  const normalized = value.trim().toLowerCase();
  if (normalized === "official") return t("官方登录");
  if (normalized === "mixedapi" || normalized === "mixed-api" || normalized === "mixed_api") return t("官方登录 + API");
  if (normalized === "aggregate") return t("聚合供应商");
  return t("纯 API");
}

function maskSecret(value: string): string {
  const trimmed = value.trim();
  if (!trimmed) return t("未填写");
  if (trimmed.length <= 10) return `${trimmed.slice(0, 2)}…${trimmed.slice(-2)}`;
  return `${trimmed.slice(0, 6)}…${trimmed.slice(-4)}`;
}
