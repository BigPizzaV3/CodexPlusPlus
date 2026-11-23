// 会话页的「会话修复」面板：供应商同步与会话索引修复。
import { ChevronDown, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { Button } from "@/components/ui/button";
import { AppSelect } from "./AppSelect";
import { enhancementItem } from "./enhancement-catalog";
import { historyRecoveryStatusLabel } from "./history-recovery-report";
import { isProviderSyncTargetSelectable } from "./provider-sync-target";
import { t, tf } from "@/i18n";
import type {
  Actions,
  BackendSettings,
  ProviderSyncProgress,
  ProviderSyncTargetOption,
  ProviderSyncTargetSource,
  ProviderSyncTargetsResult,
  SessionIndexRepairReport,
} from "./app-types";
import { formatProgressPercent, formatTime, SettingRow, SettingsAutoSave, ToggleVisual } from "./app-shared";

/// 会话维护页的两条开关与 Codex 设置页共用清单里的文案（#2846 拆开后分别显示）。
const providerSyncItem = enhancementItem("providerSyncEnabled");
const sessionIndexRepairItem = enhancementItem("sessionIndexRepairEnabled");

const providerSyncSourceLabels: Record<ProviderSyncTargetSource, string> = {
  get config() { return t("配置"); },
  get rollout() { return t("会话"); },
  get sqlite() { return t("索引"); },
  get manual() { return t("手动"); },
};

function providerSyncTargetLabel(target: ProviderSyncTargetOption): string {
  const labels = target.sources.map((source) => providerSyncSourceLabels[source]).filter(Boolean);
  const current = target.isCurrentProvider ? [t("当前")] : [];
  const unavailable = isProviderSyncTargetSelectable(target) ? [] : [t("供应商切换不可用")];
  return [...labels, ...current, ...unavailable].join(" / ") || t("发现");
}

export function SessionRepairPanel({ dirty, form, providerSyncProgress, sessionIndexRepairActive, sessionIndexRepairReport, sessionIndexRepairReportError, providerSyncTargets, selectedProviderSyncTarget, onFormChange, actions }: {
  dirty: boolean;
  form: BackendSettings;
  providerSyncProgress: ProviderSyncProgress;
  sessionIndexRepairActive: boolean;
  sessionIndexRepairReport: SessionIndexRepairReport | null;
  sessionIndexRepairReportError: string | null;
  providerSyncTargets: ProviderSyncTargetsResult | null;
  selectedProviderSyncTarget: string;
  onFormChange: (value: BackendSettings) => void;
  actions: Actions;
}) {
  const providerTargets = providerSyncTargets?.targets ?? [];
  const selectedProviderTarget = providerTargets.find((target) => target.id === selectedProviderSyncTarget);
  const canRepairProviderSessions = Boolean(selectedProviderTarget && isProviderSyncTargetSelectable(selectedProviderTarget));
  const maintenanceBusy = providerSyncProgress.active || sessionIndexRepairActive;
  const maintenanceHasError = Boolean(
    sessionIndexRepairReportError
    || providerSyncProgress.result?.status === "failed"
    || providerSyncProgress.result?.syncStatus === "partial_failure"
    || providerSyncProgress.result?.recoveryErrors?.length
    || sessionIndexRepairReport?.abortedReason
    || sessionIndexRepairReport?.warnings?.length,
  );
  const maintenanceNeedsAttention = maintenanceBusy || maintenanceHasError;
  const [maintenanceExpanded, setMaintenanceExpanded] = useState(maintenanceNeedsAttention);

  useEffect(() => {
    if (maintenanceNeedsAttention) setMaintenanceExpanded(true);
  }, [maintenanceNeedsAttention, providerSyncProgress.active, sessionIndexRepairActive, sessionIndexRepairReportError]);

  return <details
    className="session-maintenance"
    open={maintenanceExpanded}
    onToggle={(event) => setMaintenanceExpanded(event.currentTarget.open)}
  >
    <summary className="session-maintenance-summary">
      <Wrench aria-hidden="true" className="h-4 w-4" />
      <strong>{t("修复与维护")}</strong>
      <span className="session-maintenance-auto-state" data-enabled={form.sessionIndexRepairEnabled ?? true}>
        {(form.sessionIndexRepairEnabled ?? true) ? t("自动修复已开启") : t("自动修复已关闭")}
      </span>
      <span className="session-maintenance-status" data-error={maintenanceHasError} role="status" aria-live="polite">
        {providerSyncProgress.active
          ? <>{t("正在修复历史会话")} {formatProgressPercent(providerSyncProgress.percent)}%</>
          : sessionIndexRepairActive
            ? t("正在检查索引…")
            : maintenanceHasError ? t("需要处理") : null}
      </span>
      <ChevronDown aria-hidden="true" className="session-maintenance-chevron h-4 w-4" />
    </summary>
    <div className="session-maintenance-content">
          {/* 和设置页同一种行：左边标题 + 一行说明，右边控件；开关即改即存（SettingsAutoSave）。 */}
          <div className="session-repair-rows">
            <SettingRow title={t(sessionIndexRepairItem.title)} description={t(sessionIndexRepairItem.description)}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t(sessionIndexRepairItem.title)}
                  checked={form.sessionIndexRepairEnabled ?? true}
                  onChange={(event) => onFormChange({ ...form, sessionIndexRepairEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t(providerSyncItem.title)} description={t(providerSyncItem.description)}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t(providerSyncItem.title)}
                  checked={form.providerSyncEnabled}
                  onChange={(event) => onFormChange({ ...form, providerSyncEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("切换模型后旧会话跟随新默认")} description={t("切换供应商或默认模型后，下次启动前把仍停在旧默认模型的会话改为新默认；手动选过模型的会话不变。保存设置后生效。")}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t("切换模型后旧会话跟随新默认")}
                  checked={Boolean(form.threadModelFollowSwitchEnabled)}
                  onChange={(event) => onFormChange({ ...form, threadModelFollowSwitchEnabled: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("非官方登录时保留本地项目")} description={t("登录过 ChatGPT 后改用 API Key、本地服务或聚合供应商，Codex 每次启动都会清空项目列表。开启后由 Codex++ 启动前把最近使用账号的项目带到当前模式，切回官方登录时归还该账号；写入前自动备份。重新启动 Codex 后生效。")}>
              <label className="relay-bare-switch">
                <input
                  aria-label={t("非官方登录时保留本地项目")}
                  checked={Boolean(form.keepProjectsWithoutAccount)}
                  onChange={(event) => onFormChange({ ...form, keepProjectsWithoutAccount: event.currentTarget.checked })}
                  type="checkbox"
                />
                <ToggleVisual />
              </label>
            </SettingRow>
            <SettingRow title={t("同步目标")}>
              <AppSelect
                disabled={providerSyncProgress.active || !providerTargets.length}
                value={selectedProviderSyncTarget}
                onChange={(value) => actions.setProviderSyncTarget(value)}
                options={
                  providerTargets.length
                    ? [
                        ...(!selectedProviderSyncTarget
                          ? [{ value: "", label: t("当前配置 provider"), disabled: true }]
                          : []),
                        ...providerTargets.map((target) => ({
                          value: target.id,
                          label: `${target.id}${t("（")}${providerSyncTargetLabel(target)}${t("）")}`,
                          disabled: !isProviderSyncTargetSelectable(target),
                          title: target.unavailableReason ?? undefined,
                        })),
                      ]
                    : [{ value: "", label: t("当前配置 provider"), disabled: true }]
                }
              />
              <Button
                disabled={providerSyncProgress.active || sessionIndexRepairActive || !canRepairProviderSessions}
                onClick={() => void actions.syncProvidersNow()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {providerSyncProgress.active ? t("正在修复…") : t("修复历史会话")}
              </Button>
            </SettingRow>
            <SettingRow title={t("会话索引")} description={t("仅恢复有本地原文且可确认位置的消息；已打开的会话可能需要重新打开才能显示。")}>
              <Button
                disabled={sessionIndexRepairActive || providerSyncProgress.active}
                onClick={() => void actions.repairSessionIndex()}
                variant="outline"
              >
                <Wrench className="h-4 w-4" />
                {sessionIndexRepairActive ? t("正在检查索引…") : t("修复会话索引")}
              </Button>
            </SettingRow>
            <SettingsAutoSave dirty={dirty} form={form} actions={actions} />
          </div>

          {providerSyncProgress.active || providerSyncProgress.percent > 0 ? (
            <div className="provider-sync-progress session-repair-progress" data-active={providerSyncProgress.active}>
              <div className="provider-sync-progress-head">
                <strong>{providerSyncProgress.active ? t("正在修复历史会话") : t("历史会话修复进度")}</strong>
                <span>{formatProgressPercent(providerSyncProgress.percent)}%</span>
              </div>
              <div
                aria-valuemax={100}
                aria-valuemin={0}
                aria-valuenow={providerSyncProgress.percent}
                className="provider-sync-progress-bar"
                role="progressbar"
              >
                <div className="provider-sync-progress-fill" style={{ width: `${providerSyncProgress.percent}%` }} />
              </div>
              <small>{providerSyncProgress.message}</small>
            </div>
          ) : null}

          {sessionIndexRepairActive ? (
            <p role="status">{t("正在检查全部会话并恢复高可信缺失消息，首次检查可能需要较长时间…")}</p>
          ) : null}
          {sessionIndexRepairReportError ? (
            <p role="alert" className="break-all">{sessionIndexRepairReportError}</p>
          ) : null}
          {sessionIndexRepairReport ? (
            <div className="provider-sync-progress session-repair-progress" aria-live="polite">
              <strong title={`${t("自动检查需要从此界面启动 Codex，并保持 Codex++ 运行，且自动修复开关已开启并保存；每次检查完成后间隔 30 分钟复查。此页面每 15 秒刷新报告，不会单独启动修复；再次检查不保证恢复。")}\n${t("短暂等待最长 30 分钟；原文和记录文件都已超过 24 小时未更新的项目直接转入需核查。缺少对应轮次或结束状态，当前证据不足以安全补回；后续检查仍会核验。")}`}>{t("最近一次会话索引修复报告")}</strong>
              <p>{t("最后检查：")}{sessionIndexRepairReport.checkedAtMs ? formatTime(sessionIndexRepairReport.checkedAtMs) : t("旧版报告未记录时间")}</p>
              <p>
                {t("读取文件")} {sessionIndexRepairReport.scannedFiles} · {t("复用缓存")} {sessionIndexRepairReport.cachedFiles} · {t("耗时")} {(sessionIndexRepairReport.elapsedMs / 1000).toFixed(1)} s
              </p>
              <p>
                {t("恢复消息")} {sessionIndexRepairReport.repairedItems} · {t("已存在")} {sessionIndexRepairReport.alreadyPresent} · {t("短暂等待")} {sessionIndexRepairReport.deferredItems ?? 0} · {t("需核查")} {sessionIndexRepairReport.skippedItems}
              </p>
              {sessionIndexRepairReport.frontendRecovery ? (
                <p role="status" className="break-all">
                  <strong>{t("历史轮次补载：")}</strong>
                  {t(historyRecoveryStatusLabel(sessionIndexRepairReport.frontendRecovery))}
                  {" · "}{sessionIndexRepairReport.frontendRecovery.status}
                  {" · "}{t("消息页")} {sessionIndexRepairReport.frontendRecovery.loadedPages}
                  {" · "}{t("元数据页")} {sessionIndexRepairReport.frontendRecovery.metadataPages}
                  {sessionIndexRepairReport.frontendRecovery.reason ? (
                    <small> · {t("原因：")}{sessionIndexRepairReport.frontendRecovery.reason}</small>
                  ) : null}
                </p>
              ) : null}
              {sessionIndexRepairReport.backupPath ? <p className="break-all">{t("修复前备份：")}{sessionIndexRepairReport.backupPath}</p> : null}
              {sessionIndexRepairReport.abortedReason ? (
                <p role="alert" className="break-all"><strong>{t("修复已中止：")}</strong>{sessionIndexRepairReport.abortedReason}</p>
              ) : null}
              {sessionIndexRepairReport.warnings?.map((warning, index) => (
                <p key={index} role="alert" className="break-all"><strong>{t("修复警告：")}</strong>{warning}</p>
              ))}
              {sessionIndexRepairReport.pendingDetails?.length ? (
                <details>
                  <summary>{t("等待与持续无法恢复详情")} ({sessionIndexRepairReport.pendingDetails.length})</summary>
                  <ul>
                    {sessionIndexRepairReport.pendingDetails.map((item, index) => (
                      <li key={`${item.threadId}-${item.turnId}-${index}`} className="break-all my-3">
                        <strong>{item.state === "waiting" ? t("短暂等待") : t("持续无法恢复")}</strong>
                        <p>{t("任务 ID：")}{item.threadId ?? "—"} · {t("轮次 ID：")}{item.turnId ?? "—"}</p>
                        <p>{t("原因：")}{item.reason}</p>
                        <small>{t("首次发现：")}{formatTime(item.firstSeenAtMs)} · {t("最后检查：")}{formatTime(item.lastCheckedAtMs)} · {t("检查次数：")}{item.checks}</small>
                      </li>
                    ))}
                  </ul>
                </details>
              ) : null}
              {sessionIndexRepairReport.issues.length ? (
                <details>
                  <summary>{t("查看检查详情")} ({sessionIndexRepairReport.issues.length})</summary>
                  <ul>{sessionIndexRepairReport.issues.map((issue, index) => <li key={index} className="break-all">{issue}</li>)}</ul>
                </details>
              ) : null}
              {sessionIndexRepairReport.issuesTruncated ? (
                <p><small>{tf("另有 {0} 条检查详情因报告上限未显示。", [sessionIndexRepairReport.issuesTruncated])}</small></p>
              ) : null}
            </div>
          ) : null}

    {providerSyncProgress.result?.recoveryJournalPath ? <div>
      {!providerSyncProgress.active && providerSyncProgress.percent === 0 ? <p role="alert">{providerSyncProgress.message}</p> : null}
      <p role="alert" className="break-all">{t("恢复记录：")}{providerSyncProgress.result.recoveryJournalPath}</p>
      {providerSyncProgress.result.syncStatus === "partial_failure" && providerSyncProgress.result.backupDir && providerSyncProgress.result.recoveryAvailable !== false ? <Button
        disabled={providerSyncProgress.active || sessionIndexRepairActive}
        onClick={() => void actions.recoverProviderSync()}>{t("恢复未完成修复")}</Button> : null}
    </div> : null}
    {providerSyncProgress.result?.recoveryErrors?.map((error, index) => <p role="alert" key={index}>{error}</p>)}
    {providerSyncProgress.result?.syncStatus === "partial_failure" && Boolean(providerSyncProgress.result.databaseOutcomes?.length || (providerSyncProgress.result.changedSessionFiles ?? 0) > 0 || (providerSyncProgress.result.sqliteRowsUpdated ?? 0) > 0) ? <p role="status">
      {t("仍有改动：")}{providerSyncProgress.result.changedSessionFiles ?? 0} {t("个会话文件，")}
      {providerSyncProgress.result.sqliteRowsUpdated ?? 0} {t("行索引。请查看恢复记录处理剩余冲突。")}
    </p> : null}
    </div>
  </details>;
}
