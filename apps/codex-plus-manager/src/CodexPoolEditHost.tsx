/**
 * 共享供应商的就地编辑 / 同步 / 价格 / 代理 / 限速 / 测试弹窗宿主（按需加载，不打进主包）。
 * 供 Codex「连接与供应商」和 Claude「供应商」共用，无需在「本地服务」里单辟一个供应商页。
 * 配置类视图通过顶部标签栏统一在一个弹窗内切换，三点菜单只需一个「编辑」或「网关设置」入口。
 */
import { useEffect, useState } from "react";
import { t } from "./i18n";
import { providerInput, type GatewayPayload, type ProviderDraft, type ProviderPrice } from "./gateway-model";
import type { Run } from "./gateway-ui";
import { defaultPoolDialogKind, poolProviderTabs, type PoolProviderTabKind, type PoolProviderTabScope } from "./pool-provider-tabs";
import { ProviderFormDialog } from "./ProviderFormDialog";
import { ProviderKeyList } from "./ProviderKeyList";
import { ProviderTestDialog } from "./ProviderTestDialog";
import { ProviderWindowsDialog, withUnlistedWindows, type WindowSyncPick } from "./ProviderWindowsDialog";
import { ProviderModelsDialog, type ModelSyncPicks } from "./ProviderModelsDialog";
import { ProviderReasoningDialog, type ReasoningSyncPick } from "./ProviderReasoningDialog";
import { ProviderExtrasDialog, ProviderLimitsDialog } from "./ProviderExtrasDialog";
import { withUnlistedAutoCompact } from "./gateway-auto-compact-model";

export { defaultPoolDialogKind, poolProviderTabs, type PoolProviderTabKind, type PoolProviderTabScope };

export type PoolProviderDialogState =
  | { kind: "edit"; id: string; scope?: PoolProviderTabScope }
  | { kind: "models"; id: string; syncOnOpen?: boolean; scope?: PoolProviderTabScope }
  | { kind: "windows"; id: string; syncOnOpen?: boolean; scope?: PoolProviderTabScope }
  | { kind: "reasoning"; id: string; syncOnOpen?: boolean; scope?: PoolProviderTabScope }
  | { kind: "price"; id: string; scope?: PoolProviderTabScope }
  | { kind: "proxy"; id: string; scope?: PoolProviderTabScope }
  | { kind: "limits"; id: string; scope?: PoolProviderTabScope }
  | { kind: "test"; id: string; keyId?: string | null };

export function PoolProviderEditHost({
  payload,
  busy,
  run,
  dialog,
  onClose,
  onOpen,
}: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  dialog: PoolProviderDialogState | null;
  onClose: () => void;
  onOpen?: (next: PoolProviderDialogState) => void;
}) {
  // 在编辑弹窗里点「编辑上下文窗口」或单个 Key 的「测试」时，叠加在编辑弹窗之上，关掉后回到编辑弹窗。
  const [formWindows, setFormWindows] = useState<string[] | null>(null);
  const [inlineTestKey, setInlineTestKey] = useState<string | null>(null);
  // 同一个供应商在标签页间切换时不再重播入场弹起动画。
  const [tabSwitched, setTabSwitched] = useState(false);
  const dialogId = dialog?.id ?? null;
  useEffect(() => {
    setTabSwitched(false);
  }, [dialogId]);
  const entry = dialog ? payload?.pool.find((item) => item.id === dialog.id) ?? null : null;

  const closeAll = () => {
    setFormWindows(null);
    setInlineTestKey(null);
    setTabSwitched(false);
    onClose();
  };

  if (!dialog || !entry) return null;

  const metaSyncable = !entry.account && entry.models.length > 0;
  const tabScope: PoolProviderTabScope = dialog.kind !== "test" && dialog.scope
    ? dialog.scope
    : entry.linkedCodexRelay ? "gateway" : "full";
  const tabs = dialog.kind === "test" ? [] : poolProviderTabs(entry, tabScope);
  const renderTabs = onOpen && tabs.length > 1
    ? (runIfClean: (action: () => void) => void) => (
      <nav aria-label={t("供应商设置分区")} className="pf-tabs" role="tablist">
        {tabs.map((tab) => {
          const active = dialog.kind === tab.kind;
          return (
            <button
              aria-selected={active}
              data-active={active}
              key={tab.kind}
              onClick={() => {
                if (active) return;
                runIfClean(() => {
                  setFormWindows(null);
                  setInlineTestKey(null);
                  setTabSwitched(true);
                  onOpen({ kind: tab.kind, id: entry.id, scope: tabScope });
                });
              }}
              role="tab"
              type="button"
            >
              {t(tab.label)}
            </button>
          );
        })}
      </nav>
    )
    : undefined;

  const saveEdit = async (draft: ProviderDraft) => {
    if (await run(`edit:${entry.id}`, "update_gateway_provider", { id: entry.id, input: providerInput(draft) }, t("保存供应商"))) {
      closeAll();
    }
  };

  const saveFormWindows = async (
    modelWindows: Record<string, number>,
    fallback: number | null,
    autoCompact: { models: Record<string, number>; fallback: number | null },
  ) => {
    if (!formWindows) return;
    const windows = withUnlistedWindows(entry, formWindows, modelWindows);
    if (
      await run(
        `windows:${entry.id}`,
        "save_gateway_provider_windows",
        { id: entry.id, windows, fallback, autoCompact: withUnlistedAutoCompact(entry, formWindows, autoCompact) },
        t("保存上下文窗口"),
      )
    ) {
      setFormWindows(null);
    }
  };

  const saveModels = async (models: string[], defaultModel: string, sync: ModelSyncPicks | null) => {
    if (sync && !(await run(`sync-models:${entry.id}`, "apply_gateway_provider_models_sync", { id: entry.id, remove: sync.remove, add: sync.add }, t("从上游同步"), true))) return;
    if (await run(`models:${entry.id}`, "save_gateway_provider_models", { id: entry.id, models, defaultModel }, t("保存模型列表"))) {
      closeAll();
    }
  };

  const saveWindows = async (
    modelWindows: Record<string, number>,
    fallback: number | null,
    autoCompact: { models: Record<string, number>; fallback: number | null },
    synced: WindowSyncPick[],
  ) => {
    if (!(await run(`windows:${entry.id}`, "save_gateway_provider_windows", { id: entry.id, windows: modelWindows, fallback, autoCompact }, t("保存上下文窗口")))) return;
    if (synced.length) await run(`windows-sync:${entry.id}`, "apply_gateway_provider_windows_sync", { id: entry.id, picks: synced }, t("从上游同步上下文窗口"), true);
    closeAll();
  };

  const saveReasoning = async (modelMetadata: string, synced: ReasoningSyncPick[]) => {
    if (!(await run(`reasoning:${entry.id}`, "save_gateway_provider_metadata", { id: entry.id, modelMetadata }, t("保存思考等级")))) return;
    if (synced.length) await run(`reasoning-sync:${entry.id}`, "apply_gateway_provider_reasoning_sync", { id: entry.id, picks: synced }, t("从上游同步思考等级"), true);
    closeAll();
  };

  const saveExtras = async (nextPrice: ProviderPrice | null, modelPrices: Record<string, ProviderPrice> | null, proxy: string) => {
    const title = dialog.kind === "proxy" ? t("保存代理") : t("保存价格");
    if (await run(`extras:${entry.id}`, "save_gateway_provider_extras", { id: entry.id, price: nextPrice, modelPrices, proxy }, title)) {
      closeAll();
    }
  };

  const saveLimits = async (rpm: number | null, concurrency: number | null) => {
    if (await run(`limits:${entry.id}`, "save_gateway_provider_limits", { id: entry.id, rpm, concurrency }, t("保存限速"))) {
      closeAll();
    }
  };

  return (
    <>
      {dialog.kind === "edit" ? (
        <ProviderFormDialog
          busy={busy}
          form={{ mode: "edit", entry }}
          keyList={entry.keys?.length ? (
            <ProviderKeyList busy={busy} entry={entry} onTest={(keyId) => setInlineTestKey(keyId)} run={run} />
          ) : undefined}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onEditWindows={(models) => setFormWindows(models)}
          onSubmit={(draft) => void saveEdit(draft)}
          renderTabs={renderTabs}
          windowsEntry={entry}
        />
      ) : null}
      {dialog.kind === "edit" && formWindows ? (
        <ProviderWindowsDialog
          busy={busy !== null}
          entry={{ ...entry, models: formWindows }}
          onCancel={() => setFormWindows(null)}
          onSave={(windows, fallback, autoCompact) => void saveFormWindows(windows, fallback, autoCompact)}
        />
      ) : null}
      {dialog.kind === "edit" && inlineTestKey ? (
        <ProviderTestDialog
          entry={entry}
          keyId={inlineTestKey}
          onClose={() => setInlineTestKey(null)}
        />
      ) : null}
      {dialog.kind === "models" ? (
        <ProviderModelsDialog
          busy={busy !== null}
          entry={entry}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onSave={(models, defaultModel, sync) => void saveModels(models, defaultModel, sync)}
          renderTabs={renderTabs}
          syncOnOpen={dialog.syncOnOpen}
        />
      ) : null}
      {dialog.kind === "windows" ? (
        <ProviderWindowsDialog
          busy={busy !== null}
          entry={entry}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onSave={(windows, fallback, autoCompact, synced) => void saveWindows(windows, fallback, autoCompact, synced)}
          renderTabs={renderTabs}
          syncOnOpen={dialog.syncOnOpen}
          upstreamSync={metaSyncable}
        />
      ) : null}
      {dialog.kind === "reasoning" ? (
        <ProviderReasoningDialog
          busy={busy !== null}
          entry={entry}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onSave={(modelMetadata, synced) => void saveReasoning(modelMetadata, synced)}
          renderTabs={renderTabs}
          syncOnOpen={dialog.syncOnOpen}
          upstreamSync={metaSyncable}
        />
      ) : null}
      {dialog.kind === "price" || dialog.kind === "proxy" ? (
        <ProviderExtrasDialog
          busy={busy !== null}
          entry={entry}
          key={`${entry.id}:${dialog.kind}`}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onSave={(nextPrice, modelPrices, proxy) => void saveExtras(nextPrice, modelPrices, proxy)}
          renderTabs={renderTabs}
          section={dialog.kind}
        />
      ) : null}
      {dialog.kind === "limits" ? (
        <ProviderLimitsDialog
          busy={busy !== null}
          entry={entry}
          noEnterAnimation={tabSwitched}
          onCancel={closeAll}
          onSave={(rpm, concurrency) => void saveLimits(rpm, concurrency)}
          renderTabs={renderTabs}
        />
      ) : null}
      {dialog.kind === "test" ? (
        <ProviderTestDialog
          entry={entry}
          keyId={dialog.keyId ?? null}
          onClose={closeAll}
          onEditSubscription={entry.account === "chatgpt" ? () => { if (onOpen) onOpen({ kind: "edit", id: entry.id }); else closeAll(); } : undefined}
        />
      ) : null}
    </>
  );
}

/** 保持已有导入名兼容。 */
export function CodexPoolEditHost(props: {
  payload: GatewayPayload | null;
  busy: string | null;
  run: Run;
  dialog: PoolProviderDialogState | null;
  onClose: () => void;
  onOpen?: (next: PoolProviderDialogState) => void;
}) {
  return <PoolProviderEditHost {...props} />;
}
