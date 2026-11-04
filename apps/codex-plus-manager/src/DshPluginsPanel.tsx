import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { ChevronDown, Copy, FolderOpen, Info, Loader2, Plug, RefreshCw, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { appConfirm } from "./app-dialog";
import { ExtensionConfigForm, schemasteryFields, type SchemasteryJson } from "./extension-config-form";
import { Switch } from "./gateway-ui";
import { t, tf } from "./i18n";
import { CxBadge, CxBanner, CxCard, CxEmpty, CxRow, CxSection, CxStatusIcon } from "./cx-layout";
import { Menu, MenuItem } from "./ManagementPages";
import "./DshPluginsPanel.css";

// 与 crates/codex-plus-core/src/dsh_plugins.rs 的 PluginRecord / 宿主报告对应。
type RowStatus = "compatible" | "partial" | "incompatible" | "error" | "disabled" | "skills";
type ReportRow = {
  id: string;
  module: string;
  status: RowStatus;
  kind: string;
  reasons: string[];
  notes: string[];
  tools: { name: string; description: string }[];
  /** 映射到 Codex 的去处：Codex 钩子事件名，或 "instructions"（经网关追加的系统提示词）。 */
  mappings?: string[];
  /** 插件 Config 的 Schemastery JSON：管理器据此生成设置表单。 */
  configSchema?: SchemasteryJson;
};
type Report = {
  kind?: string;
  title?: string;
  description?: string;
  client?: boolean;
  rows?: ReportRow[];
  skills?: { name: string; description: string }[];
  diagnostics?: string[];
};
type PluginSource = { type: "local"; path: string } | { type: "npm"; spec: string; registry?: string } | { type: "git"; url: string; reference?: string; path?: string };
type DshPlugin = {
  id: string;
  name: string;
  version: string;
  source: PluginSource;
  root: string;
  managed: boolean;
  enabled: boolean;
  disabledRows: string[];
  configs: Record<string, unknown>;
  report: Report | null;
  importedSkills: string[];
  toolCount: number;
  mcpJson: string | null;
  /** 配套拓展包（系统提示词 → 网关 instructions，dsh 事件 → Codex 钩子）；没有可映射的内容时为 null。 */
  companion: {
    id: string;
    hooks: string[];
    instructions: boolean;
    staticPrompt: boolean;
    /** 每一步的模型请求交给插件（agent/request、按步的 agent/pre-step、改写工具结果）。 */
    gatewayStep?: boolean;
    /** llm/stream / agent/assistant-stream 经网关响应流中间件。 */
    gatewayResponse?: boolean;
    /** agent/request-error 经网关失败重试。 */
    gatewayError?: boolean;
    /** 插件注册的斜杠命令，在 Codex 输入框里同名可用。 */
    commands?: { name: string; description: string; hint: string }[];
    installed: boolean;
    enabled: boolean;
    scriptsEnabled: boolean;
  } | null;
};
type Snapshot = {
  message: string;
  plugins: DshPlugin[];
  integration: { status: string; registered: boolean; message: string; configPath?: string };
  toolchain: { node?: string | null; npm?: string | null; git?: string | null };
  pluginDir: string;
};

const STATUS_LABEL: Record<RowStatus, () => string> = {
  compatible: () => t("完全兼容"),
  partial: () => t("部分兼容"),
  incompatible: () => t("不兼容"),
  error: () => t("加载失败"),
  disabled: () => t("已停用"),
  skills: () => t("按 skill 导入"),
};

function sourceText(source: PluginSource): string {
  switch (source.type) {
    case "local":
      return source.path;
    case "npm":
      return `npm: ${source.spec}${source.registry ? `（${source.registry}）` : ""}`;
    case "git":
      return `git: ${source.url}${source.reference ? `#${source.reference}` : ""}${source.path ? ` (${source.path})` : ""}`;
  }
}

async function request(action: string, payload: Record<string, unknown> = {}): Promise<Snapshot> {
  return invoke<Snapshot>("dsh_plugins_request", { action, payload });
}

/** 插件市场索引（格式 1）里的一项，source 可直接交给「导入」。与 dsh_plugins.rs 的 MarketEntry 对应。 */
type MarketEntry = { name: string; title: string; description: string; version: string; source: string; note: string | null };

export function DshPluginsPanel({ search = "" }: { search?: string } = {}) {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: "ok" | "error"; text: string } | null>(null);
  const [indexUrl, setIndexUrl] = useState("");
  const [market, setMarket] = useState<MarketEntry[] | null>(null);

  const browseIndex = async () => {
    setBusy("browse");
    try {
      const result = await invoke<{ entries: MarketEntry[] }>("dsh_plugins_request", { action: "browse_index", payload: { url: indexUrl.trim() } });
      setMarket(result.entries);
      if (!result.entries.length) setNotice({ tone: "ok", text: t("索引里没有能导入的插件。") });
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
    } finally {
      setBusy(null);
    }
  };

  const run = useCallback(async (label: string, action: string, payload: Record<string, unknown> = {}) => {
    setBusy(label);
    try {
      const next = await request(action, payload);
      setSnapshot(next);
      if (next.message) setNotice({ tone: "ok", text: next.message });
      return true;
    } catch (error) {
      setNotice({ tone: "error", text: String(error) });
      return false;
    } finally {
      setBusy(null);
    }
  }, []);

  useEffect(() => {
    void request("snapshot").then(setSnapshot, (error) => setNotice({ tone: "error", text: String(error) }));
  }, []);

  const importSource = async (value: string) => {
    const trimmed = value.trim();
    if (!trimmed) return;
    const confirmed = await appConfirm(
      t("导入时会在本机运行插件代码来列出它的工具，插件以你的用户权限运行、没有沙箱。只导入你信任的插件。继续吗？"),
      { title: t("导入 dsh 插件"), confirmText: t("导入") },
    );
    if (!confirmed) return;
    if (await run("import", "import", { source: trimmed })) setSource("");
  };

  const pickDirectory = async () => {
    const picked = await open({ directory: true, multiple: false, title: t("选择 dsh 插件包目录（含 package.json）") });
    if (typeof picked === "string") await importSource(picked);
  };

  const plugins = snapshot?.plugins ?? [];
  const query = search.trim().toLocaleLowerCase();
  const shown = plugins.filter((plugin) => !query || [plugin.name, plugin.id, plugin.report?.title ?? "", plugin.report?.description ?? ""].join(" ").toLocaleLowerCase().includes(query));
  const toolTotal = plugins.filter((plugin) => plugin.enabled).reduce((sum, plugin) => sum + plugin.toolCount, 0);
  const integration = snapshot?.integration;
  const missingNode = snapshot && !snapshot.toolchain.node;
  const intro = t("导入 dsh（deepseek-harness）插件：工具插件经 Codex++ 托管的 MCP 在 Codex 里调用，skill 导入 Codex Skills，MCP 配置可一键复制导入；系统提示词与工具前后、回合前后的事件经自动生成的配套拓展转成网关 instructions 与 Codex 钩子。依赖 dsh agent 循环、会话或模型服务的插件无法在 Codex 里运行，会逐项说明原因。");

  return (
    <>
      {/* 顶部只留一条：缺 Node 时说缺 Node，否则没接入时给「接入 Codex」。 */}
      {missingNode ? (
        <CxBanner icon={<Plug />} title={t("没有找到 Node 运行时")} tone="warn" detail={t("没有找到 Node 运行时：请安装 Codex App（自带 Node）或 Node.js 后再导入。")} />
      ) : integration && !integration.registered ? (
        <CxBanner
          detail={integration.status === "conflict" ? integration.message : t("工具插件需要先把 dsh 插件 MCP 接入 Codex（只写入 Codex++ 托管的一条 MCP 配置）。")}
          icon={<Plug />}
          title={t("dsh 插件 MCP 未接入 Codex")}
          tone="warn"
        >
          <button className="cx-secondary" disabled={busy !== null || integration.status === "conflict"} onClick={() => void run("connect", "connect_codex")} type="button">
            {busy === "connect" ? <Loader2 className="animate-spin" /> : <Plug />}
            {t("接入 Codex")}
          </button>
        </CxBanner>
      ) : null}

      {notice ? <div className={`dsh-plugins-notice is-${notice.tone}`}>{notice.text}</div> : null}

      <CxSection
        aside={integration?.registered ? <span title={t("改动在新开 Codex 对话后生效")}>{tf("已接入 Codex · {0} 个工具", [toolTotal])}</span> : null}
        count={plugins.length}
        title={t("已导入")}
      >
        <CxCard>
          {shown.length ? shown.map((plugin) => <DshPluginCard busy={busy} key={plugin.id} onRun={run} plugin={plugin} />) : (
            <CxEmpty title={plugins.length ? t("没有匹配的插件。") : t("还没有导入 dsh 插件。")} detail={plugins.length ? undefined : t("在下面填 npm 包名、git 地址或选本地目录。")} />
          )}
        </CxCard>
      </CxSection>

      <CxSection aside={<span className="dsh-intro-hint" title={intro}><Info size={14} /></span>} title={t("导入插件")}>
        <CxCard className="dsh-import-card">
          <div className="dsh-plugins-import">
            <Input
              aria-label={t("插件来源")}
              disabled={busy !== null}
              onChange={(event) => setSource(event.currentTarget.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") void importSource(source);
              }}
              placeholder={t("npm 包名（如 @scope/dsh-tool-x，可加 #registry:地址）、git 地址（可加 #分支&path:子目录）或本地目录绝对路径")}
              value={source}
            />
            <button className="cx-secondary" disabled={busy !== null} onClick={() => void pickDirectory()} type="button">
              <FolderOpen />
              {t("本地目录")}
            </button>
            <button className="cx-primary" disabled={busy !== null || !source.trim()} onClick={() => void importSource(source)} type="button">
              {busy === "import" ? <Loader2 className="animate-spin" /> : <Upload />}
              {t("导入")}
            </button>
          </div>
          {snapshot && !snapshot.toolchain.npm ? <div className="dsh-plugins-hint">{t("未找到 npm：只能导入本地目录，或不需要额外依赖的 git 仓库。")}</div> : null}
          {/* 从插件市场索引挑：不常用，收进折叠区。 */}
          <details className="dsh-index" open={Boolean(market?.length) || undefined}>
            <summary>{t("从插件市场索引导入")}</summary>
            <div className="dsh-plugins-import">
              <Input
                aria-label={t("插件市场索引地址")}
                disabled={busy !== null}
                onChange={(event) => setIndexUrl(event.currentTarget.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && indexUrl.trim()) void browseIndex();
                }}
                placeholder={t("dsh 插件市场索引地址（格式 1 的 JSON），列出后可逐个导入")}
                value={indexUrl}
              />
              <button className="cx-secondary" disabled={busy !== null || !indexUrl.trim()} onClick={() => void browseIndex()} type="button">
                {busy === "browse" ? <Loader2 className="animate-spin" /> : <RefreshCw />}
                {t("浏览索引")}
              </button>
            </div>
            {market?.length ? (
              <ul className="dsh-plugin-tools dsh-plugins-market">
                {market.map((entry) => (
                  <li key={`${entry.name}@${entry.version}`}>
                    <code>{entry.title}{entry.version ? ` ${entry.version}` : ""}</code>
                    <span>{entry.description}{entry.note ? `（${entry.note}）` : ""}</span>
                    <Button disabled={busy !== null} onClick={() => void importSource(entry.source)} size="sm" variant="ghost">{t("导入")}</Button>
                  </li>
                ))}
              </ul>
            ) : null}
          </details>
        </CxCard>
      </CxSection>
    </>
  );
}

/** 一个插件一行：开关 + 三点菜单；点行展开它的各行明细、配套拓展和 skill / MCP。 */
function DshPluginCard({ plugin, busy, onRun }: { plugin: DshPlugin; busy: string | null; onRun: (label: string, action: string, payload?: Record<string, unknown>) => Promise<boolean> }) {
  const report = plugin.report ?? {};
  const rows = report.rows ?? [];
  const [open, setOpen] = useState(false);
  const [editing, setEditing] = useState<string | null>(null);
  /** 正在编辑的行用表单还是 JSON：有可表达的 Config 字段时默认表单。 */
  const [rawJson, setRawJson] = useState(false);
  const [draft, setDraft] = useState("");
  const [draftError, setDraftError] = useState<string | null>(null);
  const key = (suffix: string) => `${plugin.id}:${suffix}`;

  const startEdit = (row: ReportRow) => {
    setEditing(row.id);
    setRawJson(schemasteryFields(row.configSchema).length === 0);
    setDraftError(null);
    const current = plugin.configs[row.id];
    setDraft(current === undefined ? "{\n  \n}" : JSON.stringify(current, null, 2));
  };

  const saveConfig = async (rowId: string) => {
    let config: unknown = null;
    if (draft.trim()) {
      try {
        config = JSON.parse(draft);
      } catch (error) {
        setDraftError(tf("不是有效 JSON：{0}", [String(error)]));
        return;
      }
    }
    if (config !== null && (typeof config !== "object" || Array.isArray(config))) {
      setDraftError(t("配置必须是 JSON 对象"));
      return;
    }
    if (await onRun(key("config"), "set_config", { id: plugin.id, rowId, config })) setEditing(null);
  };

  const remove = async () => {
    const confirmed = await appConfirm(
      plugin.managed
        ? tf("移除 {0}？下载的包会移入 Codex++ 的备份目录。", [plugin.name])
        : tf("移除 {0}？只解除登记，不会删除你的本地目录。", [plugin.name]),
      { title: t("移除 dsh 插件"), confirmText: t("移除"), danger: true },
    );
    if (confirmed) await onRun(key("remove"), "remove", { id: plugin.id });
  };

  const copyMcp = async () => {
    if (!plugin.mcpJson) return;
    await navigator.clipboard.writeText(plugin.mcpJson);
  };

  const skillCount = report.skills?.length ?? 0;
  const problems = rows.filter((row) => row.status === "incompatible" || row.status === "error").length + (report.diagnostics?.length ?? 0);
  return (
    <>
      <CxRow
        badges={
          <>
            {plugin.version ? <CxBadge>v{plugin.version}</CxBadge> : null}
            {report.kind === "bundle" ? <CxBadge>{t("组合包")}</CxBadge> : null}
            <CxBadge>{tf("{0} 个工具", [plugin.toolCount])}</CxBadge>
            {skillCount > 0 ? <CxBadge>{tf("{0} 个 skill", [skillCount])}</CxBadge> : null}
            {problems ? <CxBadge tone="warn">{tf("{0} 项不兼容", [problems])}</CxBadge> : null}
          </>
        }
        detail={<span title={sourceText(plugin.source)}>{report.description || sourceText(plugin.source)}</span>}
        icon={<CxStatusIcon tone={plugin.enabled ? "ok" : "off"}><Plug /></CxStatusIcon>}
        muted={!plugin.enabled}
        onClick={() => setOpen((value) => !value)}
        selected={open}
        title={report.title || plugin.name}
      >
        <span className="dsh-plugin-switch" title={t("停用后 Codex 里不再提供这个插件的工具")}>
          <Switch
            disabled={busy !== null}
            label={t("启用插件")}
            on={plugin.enabled}
            onToggle={() => void onRun(key("toggle"), "set_enabled", { id: plugin.id, enabled: !plugin.enabled })}
          />
        </span>
        <Menu label={tf("{0} 的更多操作", [plugin.name])}>
          <MenuItem disabled={busy !== null} onClick={() => void onRun(key("reinspect"), "reinspect", { id: plugin.id })}>{t("重新分析（改了本地源码或配置后）")}</MenuItem>
          {skillCount > 0 ? (
            <MenuItem disabled={busy !== null} onClick={() => void onRun(key("skills"), "import_skills", { id: plugin.id })}>
              {plugin.importedSkills.length ? t("重新导入 skill") : t("导入 skill")}
            </MenuItem>
          ) : null}
          {plugin.mcpJson ? <MenuItem onClick={() => void copyMcp()}>{t("复制 MCP JSON")}</MenuItem> : null}
          <MenuItem danger disabled={busy !== null} onClick={() => void remove()}>{t("移除")}</MenuItem>
        </Menu>
        <ChevronDown aria-hidden className="dsh-plugin-chevron" data-open={open || undefined} size={16} />
      </CxRow>
      {open ? (
        <div className="cx-row-expand dsh-plugin-card" data-enabled={plugin.enabled}>
          <div className="dsh-plugin-source">{sourceText(plugin.source)}</div>
          {(report.diagnostics ?? []).map((text) => (
            <div className="dsh-plugins-warning" key={text}>{text}</div>
          ))}
          {rows.length ? (
            <div className="dsh-plugin-rows">
              {rows.map((row) => (
                <div className="dsh-plugin-row" key={row.id}>
                  <div className="dsh-plugin-row-head">
                    <span className={`dsh-status is-${row.status}`}>{STATUS_LABEL[row.status]?.() ?? row.status}</span>
                    <strong>{row.id}</strong>
                    {row.module !== row.id ? <code>{row.module}</code> : null}
                    <span className="dsh-plugin-row-actions">
                      {(row.kind === "plugin" && row.status !== "incompatible") || row.status === "disabled" ? (
                        <>
                          <button disabled={busy !== null} onClick={() => startEdit(row)} type="button">{t("配置")}</button>
                          <button
                            disabled={busy !== null}
                            onClick={() => void onRun(key(`row:${row.id}`), "set_row_enabled", { id: plugin.id, rowId: row.id, enabled: plugin.disabledRows.includes(row.id) })}
                            type="button"
                          >
                            {plugin.disabledRows.includes(row.id) ? t("启用此行") : t("停用此行")}
                          </button>
                        </>
                      ) : null}
                    </span>
                  </div>
                  {row.mappings?.length ? (
                    <div className="dsh-plugin-mappings">
                      {row.mappings.map((target) => (
                        <span className="dsh-mapping" key={target}>
                          {mappingLabel(target)}
                        </span>
                      ))}
                    </div>
                  ) : null}
                  {row.tools.length ? (
                    <ul className="dsh-plugin-tools">
                      {row.tools.map((tool) => (
                        <li key={tool.name}><code>{tool.name}</code><span>{tool.description}</span></li>
                      ))}
                    </ul>
                  ) : null}
                  {[...row.reasons, ...row.notes].map((text) => (
                    <div className="dsh-plugin-reason" key={text}>{text}</div>
                  ))}
                  {editing === row.id && !rawJson ? (
                    <div className="dsh-plugin-config">
                      <ExtensionConfigForm
                        fields={schemasteryFields(row.configSchema)}
                        onCancel={() => setEditing(null)}
                        onSave={async (values) => {
                          if (await onRun(key("config"), "set_config", { id: plugin.id, rowId: row.id, config: values })) setEditing(null);
                        }}
                        saveLabel={t("保存并重新分析")}
                        values={(plugin.configs[row.id] as Record<string, unknown> | undefined) ?? {}}
                      />
                      <Button onClick={() => setRawJson(true)} size="sm" variant="ghost">{t("改用 JSON 编辑")}</Button>
                    </div>
                  ) : null}
                  {editing === row.id && rawJson ? (
                    <div className="dsh-plugin-config">
                      <textarea
                        aria-label={t("插件配置（JSON）")}
                        onChange={(event) => setDraft(event.currentTarget.value)}
                        rows={6}
                        spellCheck={false}
                        value={draft}
                      />
                      <small>{t("浅合并到组合包里的配置之上；留空表示清除。")}</small>
                      {draftError ? <div className="dsh-plugins-warning">{draftError}</div> : null}
                      <div className="dsh-plugin-config-actions">
                        <Button disabled={busy !== null} onClick={() => void saveConfig(row.id)} size="sm">{t("保存并重新分析")}</Button>
                        <Button onClick={() => setEditing(null)} size="sm" variant="ghost">{t("取消")}</Button>
                      </div>
                    </div>
                  ) : null}
                </div>
              ))}
            </div>
          ) : null}
          {plugin.companion ? <CompanionLine companion={plugin.companion} /> : null}
          {skillCount > 0 ? (
            <div className="dsh-plugin-skills">
              <span>
                {plugin.importedSkills.length
                  ? tf("已导入 Skills：{0}", [plugin.importedSkills.join("、")])
                  : tf("包含 skill：{0}", [(report.skills ?? []).map((skill) => skill.name).join("、")])}
              </span>
            </div>
          ) : null}
          {plugin.mcpJson ? (
            <div className="dsh-plugin-skills">
              <span>{t("包里有 MCP 配置，复制后可在「MCP」页用 JSON 导入。")}</span>
              <button className="cx-secondary" onClick={() => void copyMcp()} type="button">
                <Copy />
                {t("复制 MCP JSON")}
              </button>
            </div>
          ) : null}
        </div>
      ) : null}
    </>
  );
}

/** 一行插件映射到 Codex 的去处，给人看的说法。 */
function mappingLabel(target: string): string {
  switch (target) {
    case "instructions":
      return t("提示词 → instructions");
    case "mirror":
      return t("会话镜像（钩子 → dsh 会话日志）");
    case "commands":
      return t("斜杠命令");
    case "connection":
      return t("前端请求路由");
    case "gateway:request":
    case "gateway:step":
      return t("每一步的请求 → 网关");
    case "gateway:response":
      return t("回复流 → 网关");
    case "gateway:instructions":
      return t("提示词组装 → 网关");
    case "gateway:error":
      return t("失败重试 → 网关");
    default:
      return tf("钩子 {0}", [target]);
  }
}

/** 配套拓展包的现状：映射到了哪里、在不在、有没有被停用。 */
function CompanionLine({ companion }: { companion: NonNullable<DshPlugin["companion"]> }) {
  const parts: string[] = [];
  if (companion.instructions) {
    parts.push(companion.staticPrompt ? t("系统提示词追加到每次请求的 instructions") : t("系统提示词（含动态片段）每次请求求值后追加到 instructions"));
  }
  if (companion.hooks.length) parts.push(tf("事件转成 Codex 钩子 {0}", [companion.hooks.join("、")]));
  if (companion.gatewayStep) parts.push(t("每一步的模型请求经网关交给插件（换模型、按步准入、改写工具结果）"));
  if (companion.gatewayResponse) parts.push(t("回复流经网关交给插件（llm/stream、agent/assistant-stream）"));
  if (companion.gatewayError) parts.push(t("agent/request-error 经网关决定失败重试"));
  if (companion.commands?.length) parts.push(tf("斜杠命令 {0}", [companion.commands.map((command) => `/${command.name}`).join(" ")]));
  let state = "";
  if (!companion.installed) state = t("配套拓展包未生成，点「重新分析」重试。");
  else if (!companion.scriptsEnabled) state = t("拓展总开关已关闭，配套拓展包不会运行。");
  else if (!companion.enabled) state = t("配套拓展包已停用。");
  return (
    <div className="dsh-plugin-companion">
      <span>
        {tf("配套拓展「{0}」：{1}。", [companion.id, parts.join("；")])}
        {companion.instructions ? t("提示词只对经 Codex++ 本地代理 / 网关发出的请求生效。") : null}
        {companion.hooks.length ? t("钩子需在 Codex 的 Hooks 设置里信任「Codex++ 拓展钩子」；插件在常驻进程里运行，内存状态跨事件保留。") : null}
      </span>
      {state ? <div className="dsh-plugins-warning">{state}</div> : null}
    </div>
  );
}
