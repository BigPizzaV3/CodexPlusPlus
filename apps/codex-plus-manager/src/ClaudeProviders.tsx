import { useState } from "react";
import { Download, Plus } from "lucide-react";
import { CxBadge, CxCard, CxEmpty, CxRow, CxSection } from "./cx-layout";
import { AgentModelPicker } from "./GatewayAgents";
import { ProviderAdder } from "./GatewayProviders";
import { publishGatewayPayload, useGatewayPayload } from "./gateway-store";
import { Avatar, Switch, makeRun, type Notify, type Run } from "./gateway-ui";
import {
  bindingFromDraft,
  draftFromBinding,
  importCandidates,
  poolExcludeToggle,
  presetHost,
  protocolLabel,
  providerEnabled,
  type GatewayAgentId,
} from "./gateway-model";
import { t, tf } from "./i18n";

/** Claude Desktop 在网关里的绑定：3p 模式直接用它，完整功能模式也按它的模型和「挑选」列模型。 */
const AGENT: GatewayAgentId = "claude-desktop";
/** 从 CC Switch、ClaudeMom 导进来的也算 Claude 的来源，导入按钮上提示还有几个没导。 */
const CLAUDE_IMPORT_SOURCES = new Set(["claude", "ccswitch", "claudemom"]);

/**
 * Desktop 增强页的「供应商」：Claude Desktop 用哪些供应商、默认哪个模型，加供应商、从 CC Switch / ClaudeMom 导入。
 * 供应商存在网关的共享供应商里（和 Codex 等共用一份，网关负责把 Anthropic 请求转成各家的协议），这里只是 Claude 视角的入口：
 * 开关就是 Agent 接入页里 Claude Desktop 的「挑选」，默认模型就是那一行的模型下拉。
 */
export function ClaudeProviders({ notify, onManage }: { notify: Notify; onManage?: () => void }) {
  const payload = useGatewayPayload(10_000);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const [adding, setAdding] = useState<"catalog" | "import" | null>(null);
  const binding = payload?.gateway.bindings[AGENT];
  const exclude = binding?.poolExclude ?? [];
  const pool = payload?.pool ?? [];
  const used = pool.filter((entry) => providerEnabled(entry) && !exclude.includes(entry.id)).length;
  const pendingImports = importCandidates(payload).filter((item) => !item.imported && CLAUDE_IMPORT_SOURCES.has(item.source)).length;

  /** 开关一家供应商给不给 Claude 用：改的是 Claude Desktop 绑定的排除列表，保存同 Agent 接入页。 */
  const toggle = (providerId: string, on: boolean) => {
    const draft = draftFromBinding(binding);
    const next = { ...draft, poolExclude: poolExcludeToggle(draft.poolExclude, providerId, on) };
    void run(`exclude:${providerId}`, "save_gateway_binding", { agent: AGENT, binding: bindingFromDraft(next, binding) }, t("保存本地服务绑定"), true);
  };

  return (
    <CxSection
      aside={(
        <span className="claude-providers-actions">
          <button className="cx-secondary" disabled={!payload || busy !== null} onClick={() => setAdding("import")} title={t("从 CC Switch、ClaudeMom 或直连模式导入")} type="button">
            <Download />{pendingImports ? tf("导入 · {0}", [pendingImports]) : t("导入")}
          </button>
          <button className="cx-secondary" disabled={!payload || busy !== null} onClick={() => setAdding("catalog")} type="button">
            <Plus />{t("添加供应商")}
          </button>
          {/* Key、额度、路由组在设置的「本地服务 → 共享供应商」里管，「返回」回到这一页。 */}
          {onManage ? <button className="cx-secondary" onClick={onManage} type="button">{t("管理共享供应商")}</button> : null}
        </span>
      )}
      className="claude-providers"
      count={pool.length ? used : undefined}
      title={t("供应商")}
    >
      <CxCard>
        {!payload ? (
          <CxEmpty title={t("正在读取供应商…")} />
        ) : !pool.length ? (
          <CxEmpty detail={t("DeepSeek、Kimi、智谱这些 OpenAI 协议的也能用，本地服务会自动转换。")} title={t("还没有供应商")}>
            <button className="cx-secondary" onClick={() => setAdding("catalog")} type="button"><Plus />{t("添加供应商")}</button>
            <button className="cx-secondary" onClick={() => setAdding("import")} type="button"><Download />{t("从 CC Switch / ClaudeMom 导入")}</button>
          </CxEmpty>
        ) : (
          <>
            <CxRow
              detail={t("Claude 启动后默认用这个模型；下面打开的供应商，它们的聊天模型都会列进 Claude 的模型选择器。")}
              title={t("默认模型")}
            >
              <AgentModelPicker agentId={AGENT} busy={busy} notify={notify} payload={payload} run={run} />
            </CxRow>
            {pool.map((entry) => {
              const enabled = providerEnabled(entry);
              const on = enabled && !exclude.includes(entry.id);
              return (
                <CxRow
                  badges={enabled ? null : <CxBadge>{t("已停用")}</CxBadge>}
                  detail={[protocolLabel(entry.protocol), presetHost(entry.baseUrl), tf("{0} 个模型", [entry.models.length])].filter(Boolean).join(" · ")}
                  icon={<Avatar baseUrl={entry.baseUrl} name={entry.name} size="sm" />}
                  key={entry.id}
                  muted={!on}
                  title={entry.name}
                >
                  <Switch
                    disabled={!enabled}
                    label={tf("给 Claude 用 {0}", [entry.name])}
                    on={on}
                    onToggle={() => toggle(entry.id, !on)}
                    pending={busy === `exclude:${entry.id}`}
                  />
                </CxRow>
              );
            })}
          </>
        )}
      </CxCard>
      {adding ? (
        <ProviderAdder
          busy={busy}
          includeAgents={[AGENT]}
          notify={notify}
          onClose={() => setAdding(null)}
          payload={payload}
          run={run}
          startWith={adding}
        />
      ) : null}
    </CxSection>
  );
}

/**
 * Claude 的「供应商」页里的 Claude Code（命令行）一行：和 Agent 接入页那一行同一个模型下拉，
 * 选一个共享供应商的模型就是让 Claude Code 经本地服务，下拉顶部「断开」就还原成它自己的 settings.json。
 * 入口跟着 Agent 走：不用再去「本地服务 → Agent 接入」里找。
 */
export function ClaudeCodeConnection({ notify }: { notify: Notify }) {
  const payload = useGatewayPayload(10_000);
  const [busy, setBusy] = useState<string | null>(null);
  const run: Run = makeRun(setBusy, publishGatewayPayload, notify);
  const agent = payload?.agents.find((item) => item.agent === "claude");
  if (payload && !agent) return null;
  const detail = !agent
    ? t("正在读取…")
    : agent.drift
      ? agent.drift.by ? tf("配置被 {0} 改掉了，没有经过本地服务", [agent.drift.by]) : t("配置被别的工具改掉了，没有经过本地服务")
      : agent.enabled
        ? t("经本地服务：用上面这些共享供应商，OpenAI 协议的自动转换")
        : t("直连：用它自己的 ~/.claude/settings.json；选一个模型就改为经本地服务");
  return (
    <CxSection title={t("Claude Code（命令行）")}>
      <CxCard>
        <CxRow
          badges={agent?.version ? <CxBadge>{agent.version}</CxBadge> : null}
          detail={detail}
          title={t("默认模型")}
        >
          <AgentModelPicker agentId="claude" busy={busy} notify={notify} payload={payload} run={run} />
        </CxRow>
      </CxCard>
    </CxSection>
  );
}
