// Codex 多开页。
import { Info, Layers3, PowerOff, RefreshCw, RotateCcw, Rocket } from "lucide-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AgentIcon } from "./gateway-ui";
import { useGatewayPayload } from "./hub-shared";
import { Menu, MenuItem, PageHeader } from "./ManagementPages";
import { t, tf } from "@/i18n";
import { isAggregateRelayProfile } from "./relay-profile-editing";
import {
  multiLaunchInstanceCanRestart,
  multiLaunchInstanceCanStop,
  multiLaunchInstanceFailure,
  multiLaunchInstanceIsMain,
  multiLaunchInstanceLabel as formatMultiLaunchInstanceLabel,
  multiLaunchInstanceRunning,
  multiLaunchInstanceStatus,
  runningInstancesForProfile,
} from "./multi-launch-model";
import * as AgentCx from "./cx-layout"; // cx-pages:agents：Agent 页面在函数内解构，避免和别的分组的具名导入重名
import type { Actions, BackendSettings, RelayFilesResult, RelayProfile } from "./app-types";
import {
  CODEX_GATEWAY_RELAY_ID,
  confirmRemoveRelayProfile,
  enhancementPlatform,
  multiLaunchSupported,
  normalizeSettings,
  providerInitial,
  relayProfileConfigBrief,
  relayProfileModeLabel,
  relayProtocolLabel,
  statusClass,
  statusLabel,
} from "./app-shared";
import { RelayProfileDetail } from "./RelayEditor";

export function CodexMultiLaunchScreen({
  form,
  relayFiles,
  onOpenRelay,
  actions,
}: {
  form: BackendSettings;
  relayFiles: RelayFilesResult | null;
  onOpenRelay: () => void;
  actions: Actions;
}) {
  // normalizeSettings 要解析共享 TOML、规整每个供应商，不随每次渲染重算。
  const normalized = useMemo(() => normalizeSettings(form), [form]);
  // #2801：Codex 被网关接管、或「配置切换」关闭时，旧版供应商页的列表整块隐藏，
  // 多开里这些供应商（多是早年从 cc-switch 导入的）就没地方编辑、删除了，这里直接给入口。
  const [detailProfileId, setDetailProfileId] = useState<string | null>(null);
  const gatewayPayload = useGatewayPayload(15_000);
  const detailProfile = detailProfileId
    ? normalized.relayProfiles.find((profile) => profile.id === detailProfileId) || null
    : null;
  const detailPoolEntry = detailProfile
    ? (gatewayPayload?.pool ?? []).find((entry) => entry.linkedCodexRelay === detailProfile.id || entry.source === `codex:${detailProfile.id}`) ?? null
    : null;
  useEffect(() => {
    if (detailProfileId && !normalized.relayProfiles.some((profile) => profile.id === detailProfileId)) {
      setDetailProfileId(null);
    }
  }, [detailProfileId, normalized.relayProfiles]);
  const deleteProfile = async (profile: RelayProfile) => {
    const next = await confirmRemoveRelayProfile(normalized, profile, actions, "multiLaunch");
    if (next) await actions.saveSettingsValue(next, true);
  };
  // 每个实例的「关闭 Windows 沙箱」（#3259）。按供应商启动的实例 id 就是供应商 id。
  const windows = enhancementPlatform === "windows";
  const [sandboxOff, setSandboxOff] = useState<Set<string>>(() => new Set());
  const loadInstanceOptions = useCallback(async () => {
    try {
      const result = await invoke<{ status: string; options?: Array<{ instanceId: string; disableWindowsSandbox?: boolean }> }>("list_codex_multi_instance_options");
      if (result.status !== "ok") return;
      setSandboxOff(new Set((result.options ?? []).filter((item) => item.disableWindowsSandbox).map((item) => item.instanceId)));
    } catch {
      // 读不到就当都没关，不挡页面。
    }
  }, []);
  useEffect(() => {
    if (windows) void loadInstanceOptions();
  }, [windows, loadInstanceOptions]);
  const toggleWindowsSandbox = async (instanceId: string) => {
    const disabled = !sandboxOff.has(instanceId);
    try {
      const result = await invoke<{ status: string; message: string }>("set_codex_multi_instance_windows_sandbox", { request: { instanceId, disabled } });
      await actions.showMessage(t("Windows 沙箱"), t(result.message), result.status === "ok" ? "ok" : "failed");
    } catch (error) {
      await actions.showMessage(t("Windows 沙箱"), String(error), "failed");
    }
    await loadInstanceOptions();
  };
  const { CxAddRow, CxBadge, CxCard, CxEmpty, CxRow, CxSection, CxStatusIcon } = AgentCx;
  const instances = actions.multiLaunchInstances;
  return (
    <div className="mg-page relay-management-page cx-page multi-launch-page">
      {detailProfile ? (
        <RelayProfileDetail
          profile={detailProfile}
          poolEntry={detailPoolEntry}
          relayFiles={detailProfile.id === normalized.activeRelayId ? relayFiles : null}
          form={normalized}
          onBack={() => setDetailProfileId(null)}
          onFormChange={(next) => actions.saveSettingsValue(next, true)}
          onSaved={() => setDetailProfileId(null)}
          actions={actions}
        />
      ) : null}
      <PageHeader
        icon={<Layers3 className="h-5 w-5" />}
        title={t("Codex多开")}
        actions={(
          // 一个刷新同时读供应商和运行实例。
          <button
            className="cx-secondary"
            disabled={actions.multiLaunchInstancesLoading}
            onClick={() => {
              void actions.refreshSettings(true);
              void actions.refreshMultiLaunchInstances(false);
            }}
            type="button"
          >
            <RefreshCw className={actions.multiLaunchInstancesLoading ? "animate-spin" : ""} />
            {t("刷新")}
          </button>
        )}
      />
      <CxSection count={instances.length} title={t("运行中的实例")}>
        <CxCard className="multi-launch-running-panel">
          {instances.length ? instances.map((instance) => {
            const status = multiLaunchInstanceStatus(instance);
            const running = multiLaunchInstanceRunning(instance);
            const mainInstance = multiLaunchInstanceIsMain(instance);
            const canStop = multiLaunchInstanceCanStop(instance);
            const canRestart = multiLaunchInstanceCanRestart(instance);
            const pending = actions.multiLaunchActionPending === instance.instanceId;
            const label = formatMultiLaunchInstanceLabel(instance, { main: t("主 Codex"), fallback: t("Codex 实例") });
            // 启动失败的原因必须让用户看得见：Windows 上多开曾经完全不吭声。
            const failure = multiLaunchInstanceFailure(instance);
            const detail = [
              instance.isMain ? t("主进程") : instance.instanceId,
              typeof instance.pid === "number" ? `PID ${instance.pid}` : "",
              typeof instance.debugPort === "number" ? `CDP ${instance.debugPort}` : "",
              typeof instance.helperPort === "number" ? `API ${instance.helperPort}` : "",
              failure ? `${instance.launchResult?.status === "cleanup_failed" ? t("失败") : t("启动失败")}：${failure}` : "",
            ].filter(Boolean).join(" · ");
            const tone = statusClass(status);
            // Disabled buttons do not receive hover, so the hint sits on the wrapper
            // and the button itself ignores pointer events.
            const restartTitle = mainInstance
              ? t("主 Codex 请使用顶部的「重启 Codex」。")
              : t("重启此 Codex 实例；修改供应商或 Key 后重启才会生效");
            const stopTitle = mainInstance
              ? t("主 Codex 请使用侧栏的「停止 Codex」。")
              : canStop
                ? t("结束此 Codex 实例")
                : t("实例当前未运行");
            return (
              <div className="multi-launch-instance" data-instance-id={instance.instanceId} key={instance.instanceId}>
                <CxRow
                  badges={<CxBadge tone={tone === "good" ? "ok" : tone === "bad" ? "danger" : tone === "warn" ? "warn" : "neutral"}>{statusLabel(status)}</CxBadge>}
                  detail={<span data-failed={failure ? "true" : undefined} title={detail}>{detail || t("未提供进程详情")}</span>}
                  icon={<CxStatusIcon tone={running ? "ok" : failure ? "danger" : "off"}>{instance.isMain ? <AgentIcon agent="codex" className="multi-launch-main-icon" /> : <span className="multi-launch-initial">{providerInitial(label)}</span>}</CxStatusIcon>}
                  title={label}
                >
                  <span title={mainInstance ? restartTitle : undefined}>
                    <button
                      className="cx-secondary"
                      disabled={!canRestart || pending}
                      onClick={() => void actions.restartMultiLaunchInstance(instance)}
                      style={mainInstance ? { pointerEvents: "none" } : undefined}
                      title={restartTitle}
                      type="button"
                    >
                      <RotateCcw />
                      {pending ? t("处理中") : t("重启")}
                    </button>
                  </span>
                  {/* 停止带文字，和「重启」并排；只有图标时看不出是什么。 */}
                  <span title={mainInstance ? stopTitle : undefined}>
                    <button
                      aria-label={stopTitle}
                      className="cx-secondary"
                      disabled={!canStop || pending}
                      onClick={() => void actions.stopMultiLaunchInstance(instance)}
                      style={mainInstance ? { pointerEvents: "none" } : undefined}
                      title={stopTitle}
                      type="button"
                    >
                      <PowerOff />
                      {t("停止")}
                    </button>
                  </span>
                </CxRow>
              </div>
            );
          }) : (
            <CxEmpty title={actions.multiLaunchInstancesLoading ? t("正在读取运行实例…") : t("当前没有已发现的 Codex 实例。")} />
          )}
        </CxCard>
      </CxSection>
      <CxSection
        aside={<span title={t("每个身份使用独立的配置、会话和用户数据目录，可与其他单一 Responses 或 Chat Completions 供应商同时运行。")}><Info className="multi-launch-hint-icon" /></span>}
        count={normalized.relayProfiles.length}
        title={t("按供应商启动")}
      >
        <CxCard className="relay-profile-list">
          {normalized.relayProfiles.map((profile) => {
            const aggregate = isAggregateRelayProfile(profile);
            const unsupported = !multiLaunchSupported(profile);
            const pending = actions.multiLaunchPending === profile.id;
            // 网关托管的那一项由「Agent 接入」管理，和旧版列表一样不给编辑、删除。
            const gatewayManaged = profile.id === CODEX_GATEWAY_RELAY_ID;
            const active = profile.id === normalized.activeRelayId;
            const runningCount = runningInstancesForProfile(actions.multiLaunchInstances, profile.id).length;
            const name = profile.name || t("未命名供应商");
            return (
              <div className="multi-launch-card" data-relay-profile-id={profile.id} key={profile.id}>
                <CxRow
                  badges={(
                    <>
                      {active ? <CxBadge>{t("使用中")}</CxBadge> : null}
                      {runningCount ? <CxBadge tone="ok">{tf("{0} 个实例运行中", [runningCount])}</CxBadge> : null}
                      {windows && sandboxOff.has(profile.id) ? <CxBadge tone="warn">{t("沙箱已关")}</CxBadge> : null}
                    </>
                  )}
                  detail={`${relayProfileModeLabel(profile)} · ${relayProtocolLabel(profile.protocol)} · ${relayProfileConfigBrief(profile)}`}
                  icon={<span className="multi-launch-initial">{providerInitial(profile.name)}</span>}
                  muted={unsupported}
                  onClick={gatewayManaged ? undefined : () => setDetailProfileId(profile.id)}
                  title={name}
                >
                  <button
                    className="cx-secondary"
                    disabled={unsupported || pending}
                    onClick={() => void actions.launchMultiInstance(profile)}
                    title={aggregate
                      ? t("聚合供应商暂不支持独立多开")
                      : unsupported
                        ? t("仅支持单一 Responses 或 Chat Completions 供应商，且不能配置模型路由")
                        : t("启动此供应商的独立 Codex 实例")}
                    type="button"
                  >
                    <Rocket />
                    {pending ? t("启动中") : t("启动")}
                  </button>
                  {gatewayManaged ? null : (
                    <Menu label={tf("供应商「{0}」的操作", [name])}>
                      {windows ? (
                        <MenuItem
                          checked={sandboxOff.has(profile.id)}
                          disabled={unsupported}
                          onClick={() => void toggleWindowsSandbox(profile.id)}
                          title={t("在这个实例的 config.toml 里关掉 Codex 的 Windows 沙箱（experimental_windows_sandbox、windows_sandbox_service 设为 false）。选「完全访问权限」后仍被限制在工作区里时打开；重启该实例后生效。")}
                        >
                          {t("关闭 Windows 沙箱")}
                        </MenuItem>
                      ) : null}
                      <MenuItem onClick={() => setDetailProfileId(profile.id)}>{t("编辑")}</MenuItem>
                      <MenuItem danger disabled={normalized.relayProfiles.length <= 1} onClick={() => void deleteProfile(profile)}>{t("删除供应商")}</MenuItem>
                    </Menu>
                  )}
                </CxRow>
              </div>
            );
          })}
          {!normalized.relayProfiles.length ? <CxEmpty title={t("暂无供应商，请先添加供应商配置。")} /> : null}
        </CxCard>
        {/* #2754：新版的 API 多加在「共享供应商」，多开却只读旧版供应商配置；添加入口直接说明这一点。 */}
        <CxAddRow
          detail={t("多开用「连接与供应商」里的直连供应商；共享供应商里的不在这里")}
          onClick={onOpenRelay}
          title={t("导入或添加供应商")}
        />
      </CxSection>
    </div>
  );
}
