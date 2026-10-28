/** 页面结构、尺寸参考 cc-switch v7 (MIT), Jason Young / farion1231。
 * https://github.com/farion1231/cc-switch — Codex 写入使用本仓库的原有后端。
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, Copy, Download, Edit3, ExternalLink, Folder, History, MoreHorizontal, Plus, RefreshCw, Search, Server, Trash2, X } from "lucide-react";
import { t } from "./i18n";
import { AppSelect } from "./AppSelect";
import { matchesManagementQuery, sessionGroups, skillRepoKey, type ManagedSession } from "./management-model";
import { TopbarActions } from "./TopbarActions";
import { AgentGlyph } from "./HubOverview";
import { MCP_AGENTS, mcpCount, mcpOn, mcpPresent, mergeMcpRows, type AgentMcpConfig, type McpAgentId, type McpRow } from "./agent-mcp-model";
import "./ManagementPages.css";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed" | "pending" | "notFound" | "unsupported" | "notApplicable") => Promise<void>;
type ContextEntry = { kind: "mcp" | "skill" | "plugin"; id: string; title: string; tomlBody: string; enabled: boolean };
const succeeded = (result: Result) => result.status === "ok";

export function Action({ children, onClick, disabled, primary = false, title }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean; title?: string }) {
  return <button type="button" className={`mg-button ${primary ? "mg-primary" : ""}`} onClick={onClick} disabled={disabled} title={title}>{children}</button>;
}
function IconAction({ children, onClick, label, disabled }: { children: ReactNode; onClick: () => void; label: string; disabled?: boolean }) {
  return <button type="button" className="mg-icon-button" onClick={onClick} aria-label={label} title={label} disabled={disabled}>{children}</button>;
}
/**
 * 页面标题已经由外壳顶栏的面包屑显示，这里不再画第二条标题栏；
 * `icon`、`title` 保留给调用方表明页面身份，实际只把 `actions` 送进顶栏。
 */
export function PageHeader({ actions }: { icon?: ReactNode; title?: string; actions?: ReactNode }) {
  return <TopbarActions>{actions}</TopbarActions>;
}
export function SearchField({ value, onChange, placeholder }: { value: string; onChange: (value: string) => void; placeholder: string }) {
  return <div className="mg-search"><Search size={15} /><input aria-label={placeholder} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />{value && <IconAction label={t("清除搜索")} onClick={() => onChange("")}><X size={13} /></IconAction>}</div>;
}
export function Menu({ label = t("更多操作"), children }: { label?: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (e: MouseEvent) => { if (!ref.current?.contains(e.target as Node)) setOpen(false); };
    const escape = (e: KeyboardEvent) => { if (e.key === "Escape") setOpen(false); };
    document.addEventListener("mousedown", close); document.addEventListener("keydown", escape);
    return () => { document.removeEventListener("mousedown", close); document.removeEventListener("keydown", escape); };
  }, [open]);
  return <div className="mg-menu-wrap" ref={ref}><button type="button" className="mg-icon-button" aria-label={label} aria-expanded={open} aria-haspopup="menu" onClick={() => setOpen(!open)}><MoreHorizontal size={17} /></button>{open && <div className="mg-menu" role="menu" onClick={() => setOpen(false)}>{children}</div>}</div>;
}
/** 菜单项；传了 `checked` 就是单选项（例如 Skill 的分组），选中的项右侧打勾。 */
export function MenuItem({ children, onClick, disabled, danger, checked }: { children: ReactNode; onClick: () => void; disabled?: boolean; danger?: boolean; checked?: boolean }) {
  return <button type="button" role={checked === undefined ? "menuitem" : "menuitemradio"} aria-checked={checked} className={danger ? "mg-danger" : ""} disabled={disabled} onClick={onClick}>{children}</button>;
}
export function ManagementDrawer({ title, onClose, children, footer, wide = false, dismissOnBackdrop = true }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode; wide?: boolean; dismissOnBackdrop?: boolean }) {
  const ref = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose); closeRef.current = onClose;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const root = ref.current;
    root?.querySelector<HTMLElement>("button, input, textarea, select")?.focus();
    const keyboard = (e: KeyboardEvent) => {
      // 多层抽屉只让最上层处理 Escape/Tab，保留下层页面的状态。
      const layers = document.querySelectorAll(".mg-drawer-backdrop");
      if (e.defaultPrevented || layers.item(layers.length - 1) !== root?.parentElement) return;
      if (e.key === "Escape") { e.preventDefault(); closeRef.current(); }
      if (e.key !== "Tab" || !root) return;
      const nodes = [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]')];
      const first = nodes[0]; const last = nodes.at(-1);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    };
    document.addEventListener("keydown", keyboard);
    return () => { document.removeEventListener("keydown", keyboard); previous?.focus({ preventScroll: true }); };
  }, []);
  return <div className="mg-drawer-backdrop" onMouseDown={(e) => { if (dismissOnBackdrop && e.target === e.currentTarget) onClose(); }}><section className={"mg-drawer" + (wide ? " mg-drawer-wide" : "")} role="dialog" aria-modal="true" aria-label={title} ref={ref}><header><h2>{title}</h2><IconAction label={t("关闭窗口")} onClick={onClose}><X size={18} /></IconAction></header><div className="mg-drawer-body">{children}</div>{footer && <footer>{footer}</footer>}</section></div>;
}
function Empty({ title, detail, children }: { title: string; detail?: string; children?: ReactNode }) {
  return <div className="mg-empty"><h2>{title}</h2>{detail && <p>{detail}</p>}<div>{children}</div></div>;
}
const agentLabel = (agent: McpAgentId) => (agent === "codex" ? "Codex" : agent === "claude" ? "Claude Code" : "Grok");
/** 矩阵列头：Agent 图标 + 名称 + 在用数量；悬停显示它的配置文件位置。 */
export function AgentColumn({ agent, count, title }: { agent: McpAgentId; count: number; title?: string }) {
  return <div className="mg-column mg-agent-column" title={title ? `${agentLabel(agent)} · ${title}` : agentLabel(agent)}><AgentGlyph agent={agent} /><span>{agentLabel(agent)}</span><small>{count}</small></div>;
}
/** 矩阵里的一格勾选；`hint` 覆盖悬停说明（例如「未安装」「只读」）。 */
export function MatrixCell({ name, enabled, disabled, onToggle, agent = "codex", hint }: { name: string; enabled: boolean; disabled?: boolean; onToggle: () => void; agent?: McpAgentId; hint?: string }) {
  return <button className="mg-cell" type="button" aria-pressed={enabled} aria-label={`${name} · ${agentLabel(agent)}：${enabled ? t("已启用") : t("已停用")}`} title={hint ? `${agentLabel(agent)}：${hint}` : `${name} · ${agentLabel(agent)}`} disabled={disabled} onClick={onToggle}><span data-on={enabled}>{enabled && <Check size={12} strokeWidth={3} />}</span></button>;
}
async function copyValue(text: string, notify: Notify) {
  try { await navigator.clipboard.writeText(text); await notify(t("复制"), t("已复制。"), "ok"); }
  catch (e) { await notify(t("复制失败"), String(e), "failed"); }
}

/** 「全局管理 · MCP」：每个服务器一行，Codex / Claude Code / Grok 各一列开关。
 *  Codex 一列走公共配置（onSave / onDelete，停用 = enabled = false）；另外两列直接读写各自的配置文件
 *  （load_agent_mcp / save_agent_mcp，后端换算格式并在写前备份）。插件只有 Codex，仍是单列列表。 */
export function McpPage({ entries, onSave, onDelete, onRefresh, onImport, notify }: {
  entries: ContextEntry[]; onSave: (kind: "mcp" | "plugin", id: string, body: string) => Promise<boolean>;
  onDelete: (entry: ContextEntry) => Promise<boolean>; onRefresh: () => Promise<void>;
  onImport: (json: string) => Promise<boolean>; notify: Notify;
}) {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"mcp" | "plugin">("mcp");
  const [editor, setEditor] = useState<{ row?: McpRow } | null>(null);
  const [importing, setImporting] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<McpRow | null>(null);
  const [agentConfigs, setAgentConfigs] = useState<Partial<Record<"claude" | "grok", AgentMcpConfig>>>({});
  const [busy, setBusy] = useState(false); const lock = useRef(false);
  const loadAgents = async () => {
    const next: Partial<Record<"claude" | "grok", AgentMcpConfig>> = {};
    for (const agent of ["claude", "grok"] as const) {
      try {
        const result = await invoke<Result & { config?: AgentMcpConfig }>("load_agent_mcp", { agent });
        if (succeeded(result) && result.config) next[agent] = result.config;
        else await notify("MCP", result.message, "failed");
      } catch (e) { await notify("MCP", String(e), "failed"); }
    }
    setAgentConfigs(next);
  };
  useEffect(() => { void loadAgents(); }, []);
  const codexEntries = entries.filter((entry) => entry.kind === kind);
  const rows = kind === "mcp"
    ? mergeMcpRows(codexEntries.map((entry) => ({ id: entry.id, tomlBody: entry.tomlBody, enabled: entry.enabled })), { claude: agentConfigs.claude?.entries, grok: agentConfigs.grok?.entries })
    : mergeMcpRows(codexEntries.map((entry) => ({ id: entry.id, tomlBody: entry.tomlBody, enabled: entry.enabled })), {});
  const visible = rows.filter((row) => matchesManagementQuery([row.id, row.body], query));
  const agents: McpAgentId[] = kind === "mcp" ? MCP_AGENTS : ["codex"];
  const write = async (task: () => Promise<boolean>) => {
    if (lock.current) return false; lock.current = true; setBusy(true);
    try { return await task(); } catch (e) { await notify("MCP", String(e), "failed"); return false; } finally { lock.current = false; setBusy(false); }
  };
  /** 写一家：Codex 走公共配置，Claude / Grok 走各自的文件；body 为 null 表示移除。 */
  const writeAgent = async (agent: McpAgentId, id: string, body: string | null): Promise<boolean> => {
    if (agent === "codex") {
      if (body !== null) return onSave(kind, id, body);
      const entry = codexEntries.find((item) => item.id === id);
      return entry ? onDelete(entry) : true;
    }
    const result = await invoke<Result & { config?: AgentMcpConfig; backupPath?: string }>("save_agent_mcp", { agent, id, tomlBody: body });
    if (!succeeded(result)) { await notify("MCP", result.message, "failed"); return false; }
    if (result.config) setAgentConfigs((old) => ({ ...old, [agent]: result.config }));
    return true;
  };
  const toggle = (row: McpRow, agent: McpAgentId) => void write(async () => {
    const on = mcpOn(row, agent);
    // Codex 关掉是「停用」（保留条目），另外两家关掉就是从它的配置里移除。
    if (agent === "codex") return writeAgent("codex", row.id, enabledBody(row.byAgent.codex?.tomlBody ?? row.body, !on));
    return writeAgent(agent, row.id, on ? null : enabledBody(row.body, true));
  });
  const refreshAll = async () => { await onRefresh(); await loadAgents(); };
  return <div className="mg-page"><PageHeader icon={<Server size={20} strokeWidth={1.5} />} title={kind === "mcp" ? "MCP" : t("插件配置")} actions={<>
    <Action onClick={() => void refreshAll()} disabled={busy}><Download size={16} />{t("刷新本机配置")}</Action>
    <Action onClick={() => setEditor({})} disabled={busy} primary><Plus size={16} />{kind === "mcp" ? t("添加服务器") : t("添加插件")}</Action>
    <Menu><MenuItem onClick={() => void refreshAll()} disabled={busy}>{t("重新读取本机配置")}</MenuItem><MenuItem onClick={() => setImporting(true)} disabled={kind !== "mcp" || busy}>{t("导入 JSON")}</MenuItem><MenuItem onClick={() => setKind(kind === "mcp" ? "plugin" : "mcp")}>{kind === "mcp" ? t("管理插件配置") : t("返回 MCP")}</MenuItem></Menu>
  </>} />
    <div className="mg-toolbar"><SearchField value={query} onChange={setQuery} placeholder={t("搜索名称、命令或 URL")} /><span>{visible.length === rows.length ? `${rows.length} ${t("个服务器")}` : `${visible.length} / ${rows.length}`}</span>{kind === "plugin" ? <small className="mg-push">{t("插件只有 Codex 支持")}</small> : null}</div>
    <div className="mg-content"><div className="mg-matrix mg-agent-matrix" data-columns={agents.length}><div className="mg-matrix-head"><span>{kind === "mcp" ? t("服务器") : t("插件")}</span>{agents.map((agent) => <AgentColumn agent={agent} count={mcpCount(rows, agent)} key={agent} title={agentConfigs[agent as "claude" | "grok"]?.configPath} />)}<span className="mg-actions-label">{t("操作")}</span></div>
      {visible.map((row) => <div className="mg-matrix-row" key={`${kind}-${row.id}`}><div className="mg-row-main"><div><strong>{row.id}</strong><span className="mg-badge">{row.body.includes("url =") ? "HTTP" : kind === "mcp" ? "stdio" : "plugin"}</span></div><small>{mcpSummary(row.body)}</small></div>
        {agents.map((agent) => <MatrixCell agent={agent} disabled={busy} enabled={mcpOn(row, agent)} key={agent} name={row.id} onToggle={() => toggle(row, agent)} />)}
        <div className="mg-row-actions"><IconAction label={t("编辑")} disabled={busy} onClick={() => setEditor({ row })}><Edit3 size={15} /></IconAction><Menu><MenuItem onClick={() => void copyValue(row.body, notify)}>{t("复制配置")}</MenuItem><MenuItem onClick={() => setDeleteTarget(row)} danger>{t("删除")}</MenuItem></Menu></div></div>)}
      {!visible.length && <Empty title={rows.length ? t("没有匹配的服务器") : t("还没有 MCP 服务器")} detail={t("从各 Agent 读取现有配置，或添加一个服务器。")}>{query && <Action onClick={() => setQuery("")}>{t("清除搜索")}</Action>}</Empty>}
    </div></div>
    {editor && <McpEditor
      entry={editor.row ? { kind, id: editor.row.id, title: editor.row.id, tomlBody: editor.row.body, enabled: true } : undefined}
      kind={kind}
      present={editor.row ? mcpPresent(editor.row) : null}
      agents={agents}
      onClose={() => { if (!busy) setEditor(null); }}
      onSave={(id, body, targets) => write(async () => {
        // 表体可能取自某家停用的条目（带 enabled = false），先统一成启用，再按各家原来的启停状态写回；
        // 否则写到 Claude 时会被当成「停用即移除」。
        const base = enabledBody(body, true);
        for (const agent of targets) {
          const previous = editor.row?.byAgent[agent];
          const next = previous && !previous.enabled && agent !== "claude" ? enabledBody(base, false) : base;
          if (!(await writeAgent(agent, id, next))) return false;
        }
        setEditor(null);
        return true;
      })}
      notify={notify} busy={busy} />}
    {importing && <McpImport onClose={() => { if (!busy) setImporting(false); }} onImport={(json) => write(async () => { const saved = await onImport(json); if (saved) setImporting(false); return saved; })} busy={busy} />}
    {deleteTarget && <ManagementDrawer title={t("删除服务器")} onClose={() => { if (!busy) setDeleteTarget(null); }} footer={<><Action onClick={() => setDeleteTarget(null)} disabled={busy}>{t("取消")}</Action><Action primary disabled={busy} onClick={() => void write(async () => { for (const agent of mcpPresent(deleteTarget)) { if (!(await writeAgent(agent, deleteTarget.id, null))) return false; } setDeleteTarget(null); return true; })}>{t("确认删除")}</Action></>}><p>{t("删除后会从这些 Agent 的配置中移除：")}{mcpPresent(deleteTarget).map(agentLabel).join("、")}</p><strong>{deleteTarget.id}</strong></ManagementDrawer>}
  </div>;
}
function enabledBody(body: string, enabled: boolean) {
  // enabled 是根字段，必须放在第一个子表之前，不能落进 env / headers。
  // 旧版/外部配置可能用 disabled 表示停用，切换时一并移除，避免旧值覆盖新状态。
  const withoutState = body.replace(/^\s*(?:enabled|disabled)\s*=\s*(?:true|false)\s*(?:#.*)?\r?\n?/gim, "");
  return `enabled = ${enabled}\n${withoutState}`;
}
function mcpSummary(body: string) {
  return body.split("\n").filter((line) => /^(command|args|url)\s*=/.test(line)).join(" · ") || t("自定义配置");
}
type Pair = { key: string; value: string };
type McpForm = { transport: "stdio" | "http"; command: string; args: string[]; env: Pair[]; cwd: string; url: string; httpHeaders: Pair[]; bearerToken: string; startupTimeoutSec: string; enabled: boolean; extraToml: string };
const emptyMcp = (): McpForm => ({ transport: "stdio", command: "", args: [], env: [], cwd: "", url: "", httpHeaders: [], bearerToken: "", startupTimeoutSec: "", enabled: true, extraToml: "" });
function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) { return <label className="mg-field"><span>{label}</span>{children}{hint && <small>{hint}</small>}</label>; }
function PairEditor({ label, pairs, onChange }: { label: string; pairs: Pair[]; onChange: (pairs: Pair[]) => void }) {
  return <section className="mg-field"><div className="mg-field-heading"><span>{label}</span><IconAction label={t("添加字段")} onClick={() => onChange([...pairs, { key: "", value: "" }])}><Plus size={15} /></IconAction></div>{pairs.map((pair, index) => <div className="mg-pair" key={index}><input aria-label={`${label} ${t("键名")} ${index + 1}`} placeholder="KEY" value={pair.key} onChange={(e) => onChange(pairs.map((p, i) => i === index ? { ...p, key: e.target.value } : p))} /><input aria-label={`${label} ${t("值")} ${index + 1}`} placeholder={t("值")} value={pair.value} onChange={(e) => onChange(pairs.map((p, i) => i === index ? { ...p, value: e.target.value } : p))} /><IconAction label={t("移除字段")} onClick={() => onChange(pairs.filter((_, i) => i !== index))}><X size={14} /></IconAction></div>)}</section>;
}
/** `present` 为 null 表示新建：可勾选写入哪些 Agent；编辑时同步写到已有这台服务器的所有 Agent。 */
function McpEditor({ entry, kind, present, agents, onClose, onSave, notify, busy }: { entry?: ContextEntry; kind: "mcp" | "plugin"; present: McpAgentId[] | null; agents: McpAgentId[]; onClose: () => void; onSave: (id: string, body: string, targets: McpAgentId[]) => Promise<boolean>; notify: Notify; busy: boolean }) {
  const [id, setId] = useState(entry?.id || ""); const [form, setForm] = useState<McpForm>(emptyMcp);
  const [targets, setTargets] = useState<McpAgentId[]>(present ?? agents);
  const [body, setBody] = useState(entry?.tomlBody || ""); const [mode, setMode] = useState<"form" | "toml">(kind === "plugin" ? "toml" : "form");
  const [error, setError] = useState(""); const [loading, setLoading] = useState(Boolean(entry && kind === "mcp")); const [saving, setSaving] = useState(false);
  const loadBody = async (value: string) => {
    const result = await invoke<Result & { form: McpForm }>("parse_mcp_entry", { tomlBody: value });
    if (!succeeded(result)) throw new Error(result.message); setForm(result.form); setBody(value);
  };
  useEffect(() => { if (!entry || kind !== "mcp") return; let active = true; void invoke<Result & { form: McpForm }>("parse_mcp_entry", { tomlBody: entry.tomlBody }).then((result) => { if (!active) return; if (succeeded(result)) setForm(result.form); else { setError(result.message); setMode("toml"); } }).catch((e) => { if (active) { setError(String(e)); setMode("toml"); } }).finally(() => { if (active) setLoading(false); }); return () => { active = false; }; }, []);
  const [commandCheck, setCommandCheck] = useState<{ code: string; message: string; replacementCommand: string | null } | null>(null);
  const [checkingCommand, setCheckingCommand] = useState(false);
  const commandSequence = useRef(0);
  useEffect(() => { commandSequence.current++; setCommandCheck(null); setCheckingCommand(false); }, [form]);
  useEffect(() => () => { commandSequence.current++; }, []);
  const update = (patch: Partial<McpForm>) => { commandSequence.current++; setCheckingCommand(false); setCommandCheck(null); setForm((old) => ({ ...old, ...patch })); };
  const checkCommand = async () => {
    const sequence = ++commandSequence.current; setCheckingCommand(true); setCommandCheck(null);
    try {
      const result = await invoke<Result & { code: string; replacementCommand: string | null }>("inspect_mcp_startup", { form });
      if (sequence !== commandSequence.current) return;
      if (succeeded(result)) setCommandCheck(result); else setError(result.message);
    } catch (e) { if (sequence === commandSequence.current) setError(String(e)); }
    finally { if (sequence === commandSequence.current) setCheckingCommand(false); }
  };
  const switchMode = async (next: "form" | "toml") => {
    if (next === mode) return;
    try {
      if (next === "form") await loadBody(body);
      else { const result = await invoke<Result & { tomlBody: string }>("build_mcp_entry", { form }); if (!succeeded(result)) throw new Error(result.message); setBody(result.tomlBody); }
      setMode(next); setError("");
    } catch (e) { setError(String(e)); }
  };
  const save = async () => {
    if (saving || busy || loading) return; setSaving(true); setError("");
    try {
      let next = body;
      if (mode === "form") { const result = await invoke<Result & { tomlBody: string }>("build_mcp_entry", { form }); if (!succeeded(result)) throw new Error(result.message); next = result.tomlBody; }
      await onSave(id.trim(), next, targets);
    } catch (e) { setError(String(e)); await notify("MCP", String(e), "failed"); } finally { setSaving(false); }
  };
  return <ManagementDrawer title={entry ? t("编辑 MCP 服务器") : t("添加 MCP 服务器")} onClose={() => { if (!saving) onClose(); }} footer={<><Action onClick={onClose} disabled={saving || busy}>{t("取消")}</Action><Action primary disabled={!id.trim() || !targets.length || saving || busy || loading} onClick={() => void save()}>{saving || busy ? t("保存中…") : t("保存")}</Action></>}>
    <Field label={t("服务器 ID")} hint={t("唯一标识，添加后不可更改。") }><input value={id} disabled={Boolean(entry)} onChange={(e) => setId(e.target.value)} placeholder="context7" /></Field>
    {kind === "mcp" && <div className="mg-tabs"><button type="button" data-active={mode === "form"} disabled={loading || saving} onClick={() => void switchMode("form")}>{t("表单")}</button><button type="button" data-active={mode === "toml"} disabled={loading || saving} onClick={() => void switchMode("toml")}>TOML</button></div>}
    {loading ? <p role="status">{t("正在读取配置…")}</p> : mode === "toml" ? <Field label={t("配置内容")} hint={t("保留自定义字段，保存时由 Codex 配置解析器校验。") }><textarea className="mg-code" value={body} onChange={(e) => setBody(e.target.value)} spellCheck={false} rows={16} /></Field> : <>
      <section className="mg-field"><span>{t("传输类型")}</span><AppSelect ariaLabel={t("传输类型")} onChange={(next) => update({ transport: next })} options={[{ value: "stdio", label: "stdio" }, { value: "http", label: "HTTP / SSE" }] as { value: McpForm["transport"]; label: string }[]} value={form.transport} /></section>
      {form.transport === "stdio" ? <><Field label={t("启动命令")}><input value={form.command} onChange={(e) => update({ command: e.target.value })} placeholder="npx" /></Field><section className="mg-field"><Action disabled={checkingCommand || saving || busy || !form.command.trim()} onClick={() => void checkCommand()}>{checkingCommand ? t("正在检查…") : t("检查启动路径")}</Action>
        {commandCheck && <div role="status" className="mg-notice"><p>{t(commandCheck.message)}</p>{commandCheck.replacementCommand && <><code>{commandCheck.replacementCommand}</code><Action onClick={() => update({ command: commandCheck.replacementCommand! })}>{t("将新入口放入草稿")}</Action><small>{t("检查不会重启 MCP；确认草稿后再保存。")}</small></>}</div>}</section><Field label={t("命令参数")} hint={t("每行一个参数。") }><textarea rows={3} value={form.args.join("\n")} onChange={(e) => update({ args: e.target.value.split("\n") })} placeholder={'-y\n@upstash/context7-mcp'} /></Field><PairEditor label={t("环境变量")} pairs={form.env} onChange={(env) => update({ env })} /><Field label={t("工作目录")}><input value={form.cwd} onChange={(e) => update({ cwd: e.target.value })} placeholder={t("可选")} /></Field></> : <><Field label="URL"><input value={form.url} onChange={(e) => update({ url: e.target.value })} placeholder="https://example.com/mcp" /></Field><PairEditor label="HTTP Headers" pairs={form.httpHeaders} onChange={(httpHeaders) => update({ httpHeaders })} /><Field label="Bearer Token"><input type="password" value={form.bearerToken} onChange={(e) => update({ bearerToken: e.target.value })} /></Field></>}
      <Field label={t("启动超时（秒）")}><input inputMode="decimal" value={form.startupTimeoutSec} onChange={(e) => update({ startupTimeoutSec: e.target.value })} placeholder={t("默认")} /></Field>
      <div className="mg-write-to"><span>{t("保存目标")}</span>
        {present ? <strong>{present.map(agentLabel).join("、")}</strong> : <div className="mg-target-list">{agents.map((agent) => <label className="mg-check-line" key={agent}><input checked={targets.includes(agent)} onChange={(e) => { const checked = e.target.checked; setTargets((old) => checked ? [...old, agent] : old.filter((item) => item !== agent)); }} type="checkbox" /><AgentGlyph agent={agent} />{agentLabel(agent)}</label>)}</div>}
        <small>{present ? t("会同步写入已有这台服务器的所有 Agent；启停请在列表行操作。") : t("保存后立即写入勾选的 Agent，写前自动备份原配置。")}</small></div>
      {form.extraToml && <details><summary>{t("其他配置字段")}</summary><textarea className="mg-code" value={form.extraToml} onChange={(e) => update({ extraToml: e.target.value })} rows={5} /></details>}
    </>}{error && <p role="alert" className="mg-error">{error}</p>}
  </ManagementDrawer>;
}
function McpImport({ onClose, onImport, busy }: { onClose: () => void; onImport: (json: string) => Promise<boolean>; busy: boolean }) {
  const [json, setJson] = useState(""); const [preview, setPreview] = useState<(Result & { entries: { id: string; tomlBody: string }[]; warnings: string[] }) | null>(null); const [checking, setChecking] = useState(false); const [error, setError] = useState("");
  const check = async () => { setChecking(true); setError(""); try { const result = await invoke<NonNullable<typeof preview>>("preview_mcp_servers_json", { json }); setPreview(succeeded(result) ? result : null); if (!succeeded(result)) setError(result.message); } catch (e) { setError(String(e)); } finally { setChecking(false); } };
  return <ManagementDrawer title={t("导入 MCP JSON")} onClose={onClose} footer={<><Action onClick={() => void check()} disabled={!json.trim() || checking || busy}>{checking ? t("正在解析…") : t("预览")}</Action><Action onClick={() => void onImport(json)} disabled={!preview || busy} primary>{t("确认导入")}</Action></>}><p className="mg-muted">{t("粘贴 mcpServers、servers 或单个服务器配置。")}</p><Field label={t("MCP 配置 JSON")}><textarea rows={18} className="mg-code" spellCheck={false} value={json} onChange={(e) => { setJson(e.target.value); setPreview(null); }} placeholder={'{\n  "mcpServers": {\n    "context7": {\n      "command": "npx",\n      "args": ["-y", "@upstash/context7-mcp"]\n    }\n  }\n}'} /></Field>{preview && <div className="mg-notice"><strong>{t("将导入：")}{preview.entries.map((e) => e.id).join("、")}</strong>{preview.warnings.map((warning) => <p key={warning}>{warning}</p>)}</div>}{error && <p role="alert" className="mg-error">{error}</p>}</ManagementDrawer>;
}
