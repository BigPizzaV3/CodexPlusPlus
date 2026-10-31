import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { Copy, FolderOpen, Loader2, Plug, RefreshCw, Trash2, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { appConfirm } from "./app-dialog";
import { Switch } from "./gateway-ui";
import { t, tf } from "./i18n";
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
type PluginSource = { type: "local"; path: string } | { type: "npm"; spec: string } | { type: "git"; url: string; reference?: string; path?: string };
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
    /** agent/request（改模型 / 推理强度）经网关请求中间件。 */
    gatewayRequest?: boolean;
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
      return `npm: ${source.spec}`;
    case "git":
      return `git: ${source.url}${source.reference ? `#${source.reference}` : ""}${source.path ? ` (${source.path})` : ""}`;
  }
}

async function request(action: string, payload: Record<string, unknown> = {}): Promise<Snapshot> {
  return invoke<Snapshot>("dsh_plugins_request", { action, payload });
}

export function DshPluginsPanel() {
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [source, setSource] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ tone: "ok" | "error"; text: string } | null>(null);

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
  const toolTotal = plugins.filter((plugin) => plugin.enabled).reduce((sum, plugin) => sum + plugin.toolCount, 0);
  const integration = snapshot?.integration;
  const missingNode = snapshot && !snapshot.toolchain.node;

  return (
    <div className="dsh-plugins">
      <div className="dsh-plugins-intro">
        <p>
          {t("导入 dsh（deepseek-harness）插件：工具插件经 Codex++ 托管的 MCP 在 Codex 里调用，skill 导入 Codex Skills，MCP 配置可一键复制导入；系统提示词与工具前后、回合前后的事件经自动生成的配套拓展转成网关 instructions 与 Codex 钩子。依赖 dsh agent 循环、会话或模型服务的插件无法在 Codex 里运行，会逐项说明原因。")}
        </p>
        <div className="dsh-plugins-import">
          <Input
            aria-label={t("插件来源")}
            disabled={busy !== null}
            onChange={(event) => setSource(event.currentTarget.value)}
            onKeyDown={(event) => {
              if (event.key === "Enter") void importSource(source);
            }}
            placeholder={t("npm 包名（如 @scope/dsh-tool-x）、git 地址（可加 #分支&path:子目录）或本地目录绝对路径")}
            value={source}
          />
          <Button disabled={busy !== null || !source.trim()} onClick={() => void importSource(source)} size="sm">
            {busy === "import" ? <Loader2 className="animate-spin" /> : <Upload />}
            {t("导入")}
          </Button>
          <Button disabled={busy !== null} onClick={() => void pickDirectory()} size="sm" variant="secondary">
            <FolderOpen />
            {t("本地目录")}
          </Button>
        </div>
        {missingNode ? <div className="dsh-plugins-warning">{t("没有找到 Node 运行时：请安装 Codex App（自带 Node）或 Node.js 后再导入。")}</div> : null}
        {snapshot && !snapshot.toolchain.npm ? <div className="dsh-plugins-hint">{t("未找到 npm：只能导入本地目录，或不需要额外依赖的 git 仓库。")}</div> : null}
      </div>

      {notice ? <div className={`dsh-plugins-notice is-${notice.tone}`}>{notice.text}</div> : null}

      <div className="dsh-plugins-integration">
        <Plug className="h-4 w-4" />
        <span>
          {integration?.registered
            ? tf("dsh 插件 MCP 已接入 Codex，当前共 {0} 个工具；改动在新开 Codex 对话后生效。", [toolTotal])
            : integration?.status === "conflict"
              ? integration.message
              : t("工具插件需要先把 dsh 插件 MCP 接入 Codex（只写入 Codex++ 托管的一条 MCP 配置）。")}
        </span>
        {!integration?.registered ? (
          <Button disabled={busy !== null || integration?.status === "conflict"} onClick={() => void run("connect", "connect_codex")} size="sm">
            {busy === "connect" ? <Loader2 className="animate-spin" /> : <Plug />}
            {t("接入 Codex")}
          </Button>
        ) : null}
      </div>

      {plugins.length === 0 ? (
        <div className="empty">{t("还没有导入 dsh 插件。")}</div>
      ) : (
        <div className="dsh-plugins-list">
          {plugins.map((plugin) => (
            <DshPluginCard busy={busy} key={plugin.id} onRun={run} plugin={plugin} />
          ))}
        </div>
      )}
    </div>
  );
}

function DshPluginCard({ plugin, busy, onRun }: { plugin: DshPlugin; busy: string | null; onRun: (label: string, action: string, payload?: Record<string, unknown>) => Promise<boolean> }) {
  const report = plugin.report ?? {};
  const rows = report.rows ?? [];
  const [editing, setEditing] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [draftError, setDraftError] = useState<string | null>(null);
  const key = (suffix: string) => `${plugin.id}:${suffix}`;

  const startEdit = (row: ReportRow) => {
    setEditing(row.id);
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

  return (
    <section className="dsh-plugin-card" data-enabled={plugin.enabled}>
      <header>
        <div className="dsh-plugin-title">
          <strong>{report.title || plugin.name}</strong>
          {plugin.version ? <code>v{plugin.version}</code> : null}
          {report.kind === "bundle" ? <span className="mg-badge">{t("组合包")}</span> : null}
          <span className="mg-badge">{tf("{0} 个工具", [plugin.toolCount])}</span>
          {(report.skills?.length ?? 0) > 0 ? <span className="mg-badge">{tf("{0} 个 skill", [report.skills?.length ?? 0])}</span> : null}
        </div>
        <div className="dsh-plugin-actions">
          <span className="dsh-plugin-switch" title={t("停用后 Codex 里不再提供这个插件的工具")}>
            <Switch
              disabled={busy !== null}
              label={t("启用插件")}
              on={plugin.enabled}
              onToggle={() => void onRun(key("toggle"), "set_enabled", { id: plugin.id, enabled: !plugin.enabled })}
            />
          </span>
          <Button disabled={busy !== null} onClick={() => void onRun(key("reinspect"), "reinspect", { id: plugin.id })} size="sm" title={t("重新分析（改了本地源码或配置后）")} variant="ghost">
            <RefreshCw className={busy === key("reinspect") ? "animate-spin" : ""} />
          </Button>
          <Button disabled={busy !== null} onClick={() => void remove()} size="sm" title={t("移除")} variant="ghost">
            <Trash2 />
          </Button>
        </div>
      </header>
      {report.description ? <p className="dsh-plugin-description">{report.description}</p> : null}
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
              {editing === row.id ? (
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
      {(report.skills?.length ?? 0) > 0 ? (
        <div className="dsh-plugin-skills">
          <span>
            {plugin.importedSkills.length
              ? tf("已导入 Skills：{0}", [plugin.importedSkills.join("、")])
              : tf("包含 skill：{0}", [(report.skills ?? []).map((skill) => skill.name).join("、")])}
          </span>
          <Button disabled={busy !== null} onClick={() => void onRun(key("skills"), "import_skills", { id: plugin.id })} size="sm" variant="secondary">
            {plugin.importedSkills.length ? t("重新导入 skill") : t("导入 skill")}
          </Button>
        </div>
      ) : null}
      {plugin.mcpJson ? (
        <div className="dsh-plugin-skills">
          <span>{t("包里有 MCP 配置，复制后可在「MCP」页用 JSON 导入。")}</span>
          <Button onClick={() => void copyMcp()} size="sm" variant="secondary">
            <Copy />
            {t("复制 MCP JSON")}
          </Button>
        </div>
      ) : null}
    </section>
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
      return t("模型请求 → 网关");
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
  if (companion.gatewayRequest) parts.push(t("agent/request 经网关改模型与推理强度"));
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
