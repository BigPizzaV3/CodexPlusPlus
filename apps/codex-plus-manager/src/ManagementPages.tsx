/** 页面结构、尺寸参考 cc-switch v7 (MIT), Jason Young / farion1231。
 * https://github.com/farion1231/cc-switch — Codex 写入使用本仓库的原有后端。
 */
import { useEffect, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { ArrowLeft, ArrowRight, BookOpen, Check, ChevronDown, Copy, Download, Edit3, ExternalLink, Folder, History, MoreHorizontal, Plus, RefreshCw, Search, Server, Trash2, X } from "lucide-react";
import { t } from "./i18n";
import { matchesManagementQuery, sessionGroups, skillRepoKey, type ManagedSession } from "./management-model";
import "./ManagementPages.css";

type Result = { status: string; message: string };
type Notify = (title: string, message: string, status?: "ok" | "failed" | "pending" | "notFound" | "unsupported" | "notApplicable") => Promise<void>;
type ContextEntry = { kind: "mcp" | "skill" | "plugin"; id: string; title: string; tomlBody: string; enabled: boolean };
const succeeded = (result: Result) => result.status === "ok";
const codexIcon = new URL("./assets/agents/chatgpt.svg", import.meta.url).href;

function Action({ children, onClick, disabled, primary = false, title }: { children: ReactNode; onClick: () => void; disabled?: boolean; primary?: boolean; title?: string }) {
  return <button type="button" className={`mg-button ${primary ? "mg-primary" : ""}`} onClick={onClick} disabled={disabled} title={title}>{children}</button>;
}
function IconAction({ children, onClick, label, disabled }: { children: ReactNode; onClick: () => void; label: string; disabled?: boolean }) {
  return <button type="button" className="mg-icon-button" onClick={onClick} aria-label={label} title={label} disabled={disabled}>{children}</button>;
}
export function PageHeader({ icon, title, actions }: { icon: ReactNode; title: string; actions?: ReactNode }) {
  return <header className="mg-header" data-tauri-drag-region><div className="mg-header-title" data-tauri-drag-region>{icon}<h1 data-tauri-drag-region>{title}</h1></div><div className="mg-header-actions">{actions}</div></header>;
}
function SearchField({ value, onChange, placeholder }: { value: string; onChange: (value: string) => void; placeholder: string }) {
  return <div className="mg-search"><Search size={15} /><input aria-label={placeholder} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />{value && <IconAction label={t("清除搜索")} onClick={() => onChange("")}><X size={13} /></IconAction>}</div>;
}
function Menu({ label = t("更多操作"), children }: { label?: string; children: ReactNode }) {
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
function MenuItem({ children, onClick, disabled, danger }: { children: ReactNode; onClick: () => void; disabled?: boolean; danger?: boolean }) {
  return <button type="button" role="menuitem" className={danger ? "mg-danger" : ""} disabled={disabled} onClick={onClick}>{children}</button>;
}
export function ManagementDrawer({ title, onClose, children, footer }: { title: string; onClose: () => void; children: ReactNode; footer?: ReactNode }) {
  const ref = useRef<HTMLElement>(null);
  const closeRef = useRef(onClose); closeRef.current = onClose;
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const root = ref.current;
    root?.querySelector<HTMLElement>("button, input, textarea, select")?.focus();
    const keyboard = (e: KeyboardEvent) => {
      if (e.key === "Escape") { e.preventDefault(); closeRef.current(); }
      if (e.key !== "Tab" || !root) return;
      const nodes = [...root.querySelectorAll<HTMLElement>('button:not(:disabled), input:not(:disabled), textarea:not(:disabled), select:not(:disabled), [tabindex="0"]')];
      const first = nodes[0]; const last = nodes.at(-1);
      if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
      else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
    };
    document.addEventListener("keydown", keyboard);
    return () => { document.removeEventListener("keydown", keyboard); previous?.focus(); };
  }, []);
  return <div className="mg-drawer-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}><section className="mg-drawer" role="dialog" aria-modal="true" aria-label={title} ref={ref}><header><h2>{title}</h2><IconAction label={t("关闭窗口")} onClick={onClose}><X size={18} /></IconAction></header><div className="mg-drawer-body">{children}</div>{footer && <footer>{footer}</footer>}</section></div>;
}
function Empty({ title, detail, children }: { title: string; detail?: string; children?: ReactNode }) {
  return <div className="mg-empty"><h2>{title}</h2>{detail && <p>{detail}</p>}<div>{children}</div></div>;
}
function CodexColumn({ count, total, children }: { count: number; total: number; children?: ReactNode }) {
  return <div className="mg-column" title={`Codex · ${count}/${total}`}><img src={codexIcon} alt="Codex" /><small>{count}</small>{children}</div>;
}
function MatrixCell({ name, enabled, disabled, onToggle }: { name: string; enabled: boolean; disabled?: boolean; onToggle: () => void }) {
  return <button className="mg-cell" type="button" aria-pressed={enabled} aria-label={`${name} · Codex：${enabled ? t("已启用") : t("已停用")}`} title={`${name} · Codex`} disabled={disabled} onClick={onToggle}><span data-on={enabled}>{enabled && <Check size={12} strokeWidth={3} />}</span></button>;
}
async function copyValue(text: string, notify: Notify) {
  try { await navigator.clipboard.writeText(text); await notify(t("复制"), t("已复制。"), "ok"); }
  catch (e) { await notify(t("复制失败"), String(e), "failed"); }
}

export function McpPage({ entries, onSave, onDelete, onRefresh, onImport, notify }: {
  entries: ContextEntry[]; onSave: (kind: "mcp" | "plugin", id: string, body: string) => Promise<boolean>;
  onDelete: (entry: ContextEntry) => Promise<boolean>; onRefresh: () => Promise<void>;
  onImport: (json: string) => Promise<boolean>; notify: Notify;
}) {
  const [query, setQuery] = useState("");
  const [kind, setKind] = useState<"mcp" | "plugin">("mcp");
  const [editor, setEditor] = useState<{ entry?: ContextEntry } | null>(null);
  const [importing, setImporting] = useState(false);
  const [deleteTarget, setDeleteTarget] = useState<ContextEntry | null>(null);
  const [busy, setBusy] = useState(false); const lock = useRef(false);
  const all = entries.filter((entry) => entry.kind === kind);
  const visible = all.filter((entry) => matchesManagementQuery([entry.id, entry.title, entry.tomlBody], query));
  const write = async (task: () => Promise<boolean>) => {
    if (lock.current) return false; lock.current = true; setBusy(true);
    try { return await task(); } finally { lock.current = false; setBusy(false); }
  };
  const bulk = async (enabled: boolean) => {
    await write(async () => {
      for (const entry of visible.filter((item) => item.enabled !== enabled)) {
        const parsed = await invoke<Result & { form: McpForm }>("parse_mcp_entry", { tomlBody: entry.tomlBody });
        if (!succeeded(parsed)) { await notify("MCP", parsed.message, "failed"); return false; }
        const built = await invoke<Result & { tomlBody: string }>("build_mcp_entry", { form: { ...parsed.form, enabled } });
        if (!succeeded(built) || !(await onSave(kind, entry.id, built.tomlBody))) return false;
      }
      return true;
    }).catch((e) => notify("MCP", String(e), "failed"));
  };
  return <div className="mg-page"><PageHeader icon={<Server size={20} strokeWidth={1.5} />} title={kind === "mcp" ? "MCP" : t("插件配置")} actions={<>
    <Action onClick={() => void onRefresh()} disabled={busy}><Download size={16} />{t("刷新本机配置")}</Action>
    <Action onClick={() => setEditor({})} disabled={busy} primary><Plus size={16} />{kind === "mcp" ? t("添加服务器") : t("添加插件")}</Action>
    <Menu><MenuItem onClick={() => void onRefresh()} disabled={busy}>{t("重新读取本机配置")}</MenuItem><MenuItem onClick={() => setImporting(true)} disabled={kind !== "mcp" || busy}>{t("导入 JSON")}</MenuItem><MenuItem onClick={() => setKind(kind === "mcp" ? "plugin" : "mcp")}>{kind === "mcp" ? t("管理插件配置") : t("返回 MCP")}</MenuItem></Menu>
  </>} />
    <div className="mg-toolbar"><SearchField value={query} onChange={setQuery} placeholder={t("搜索名称、命令或 URL")} /><span>{visible.length === all.length ? `${all.length} ${t("个服务器")}` : `${visible.length} / ${all.length}`}</span><small className="mg-push">{all.filter((entry) => entry.enabled).length} {t("个已启用")}</small></div>
    <div className="mg-content"><div className="mg-matrix"><div className="mg-matrix-head"><span>{kind === "mcp" ? t("服务器") : t("插件")}</span><span className="mg-actions-label">{t("操作")}</span></div>
      {visible.map((entry) => <div className="mg-matrix-row" key={`${kind}-${entry.id}`}><div className="mg-row-main"><div><strong>{entry.title || entry.id}</strong>{entry.title && entry.title !== entry.id && <code>{entry.id}</code>}<span className="mg-badge">{entry.tomlBody.includes("url =") ? "HTTP" : kind === "mcp" ? "stdio" : "plugin"}</span></div><small>{mcpSummary(entry.tomlBody)}</small></div><div className="mg-row-actions"><button className="mg-inline-action" type="button" disabled={busy} onClick={() => void write(() => onSave(kind, entry.id, enabledBody(entry.tomlBody, !entry.enabled)))}>{entry.enabled ? t("停用") : t("启用")}</button><IconAction label={t("编辑")} disabled={busy} onClick={() => setEditor({ entry })}><Edit3 size={15} /></IconAction><Menu><MenuItem onClick={() => void copyValue(entry.tomlBody, notify)}>{t("复制配置")}</MenuItem><MenuItem onClick={() => setDeleteTarget(entry)} danger>{t("删除")}</MenuItem></Menu></div></div>)}
      {!visible.length && <Empty title={all.length ? t("没有匹配的服务器") : t("还没有 MCP 服务器")} detail={t("从 Codex 导入现有配置，或添加一个服务器。")}>{query && <Action onClick={() => setQuery("")}>{t("清除搜索")}</Action>}</Empty>}
    </div></div>
    {editor && <McpEditor entry={editor.entry} kind={kind} onClose={() => { if (!busy) setEditor(null); }} onSave={(id, body) => write(async () => { const saved = await onSave(kind, id, body); if (saved) setEditor(null); return saved; })} notify={notify} busy={busy} />}
    {importing && <McpImport onClose={() => { if (!busy) setImporting(false); }} onImport={(json) => write(async () => { const saved = await onImport(json); if (saved) setImporting(false); return saved; })} busy={busy} />}
    {deleteTarget && <ManagementDrawer title={t("删除服务器")} onClose={() => { if (!busy) setDeleteTarget(null); }} footer={<><Action onClick={() => setDeleteTarget(null)} disabled={busy}>{t("取消")}</Action><Action primary disabled={busy} onClick={() => void write(async () => { const deleted = await onDelete(deleteTarget); if (deleted) setDeleteTarget(null); return deleted; })}>{t("确认删除")}</Action></>}><p>{t("删除后会从 Codex 配置中移除：")}</p><strong>{deleteTarget.title || deleteTarget.id}</strong></ManagementDrawer>}
  </div>;
}
function enabledBody(body: string, enabled: boolean) {
  // enabled 是根字段，必须放在第一个子表之前，不能落进 env / headers。
  return `enabled = ${enabled}\n${body.replace(/^enabled\s*=\s*(?:true|false)\s*\n?/m, "")}`;
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
function McpEditor({ entry, kind, onClose, onSave, notify, busy }: { entry?: ContextEntry; kind: "mcp" | "plugin"; onClose: () => void; onSave: (id: string, body: string) => Promise<boolean>; notify: Notify; busy: boolean }) {
  const [id, setId] = useState(entry?.id || ""); const [form, setForm] = useState<McpForm>(emptyMcp);
  const [body, setBody] = useState(entry?.tomlBody || ""); const [mode, setMode] = useState<"form" | "toml">(kind === "plugin" ? "toml" : "form");
  const [error, setError] = useState(""); const [loading, setLoading] = useState(Boolean(entry && kind === "mcp")); const [saving, setSaving] = useState(false);
  const loadBody = async (value: string) => {
    const result = await invoke<Result & { form: McpForm }>("parse_mcp_entry", { tomlBody: value });
    if (!succeeded(result)) throw new Error(result.message); setForm(result.form); setBody(value);
  };
  useEffect(() => { if (!entry || kind !== "mcp") return; let active = true; void invoke<Result & { form: McpForm }>("parse_mcp_entry", { tomlBody: entry.tomlBody }).then((result) => { if (!active) return; if (succeeded(result)) setForm(result.form); else { setError(result.message); setMode("toml"); } }).catch((e) => { if (active) { setError(String(e)); setMode("toml"); } }).finally(() => { if (active) setLoading(false); }); return () => { active = false; }; }, []);
  const update = (patch: Partial<McpForm>) => setForm((old) => ({ ...old, ...patch }));
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
      await onSave(id.trim(), next);
    } catch (e) { setError(String(e)); await notify("MCP", String(e), "failed"); } finally { setSaving(false); }
  };
  return <ManagementDrawer title={entry ? t("编辑 MCP 服务器") : t("添加 MCP 服务器")} onClose={() => { if (!saving) onClose(); }} footer={<><Action onClick={onClose} disabled={saving || busy}>{t("取消")}</Action><Action primary disabled={!id.trim() || saving || busy || loading} onClick={() => void save()}>{saving || busy ? t("保存中…") : t("保存")}</Action></>}>
    <Field label={t("服务器 ID")} hint={t("唯一标识，添加后不可更改。") }><input value={id} disabled={Boolean(entry)} onChange={(e) => setId(e.target.value)} placeholder="context7" /></Field>
    {kind === "mcp" && <div className="mg-tabs"><button type="button" data-active={mode === "form"} disabled={loading || saving} onClick={() => void switchMode("form")}>{t("表单")}</button><button type="button" data-active={mode === "toml"} disabled={loading || saving} onClick={() => void switchMode("toml")}>TOML</button></div>}
    {loading ? <p role="status">{t("正在读取配置…")}</p> : mode === "toml" ? <Field label={t("配置内容")} hint={t("保留自定义字段，保存时由 Codex 配置解析器校验。") }><textarea className="mg-code" value={body} onChange={(e) => setBody(e.target.value)} spellCheck={false} rows={16} /></Field> : <>
      <Field label={t("传输类型")}><select value={form.transport} onChange={(e) => update({ transport: e.target.value as McpForm["transport"] })}><option value="stdio">stdio</option><option value="http">HTTP / SSE</option></select></Field>
      {form.transport === "stdio" ? <><Field label={t("启动命令")}><input value={form.command} onChange={(e) => update({ command: e.target.value })} placeholder="npx" /></Field><Field label={t("命令参数")} hint={t("每行一个参数。") }><textarea rows={3} value={form.args.join("\n")} onChange={(e) => update({ args: e.target.value.split("\n") })} placeholder={'-y\n@upstash/context7-mcp'} /></Field><PairEditor label={t("环境变量")} pairs={form.env} onChange={(env) => update({ env })} /><Field label={t("工作目录")}><input value={form.cwd} onChange={(e) => update({ cwd: e.target.value })} placeholder={t("可选")} /></Field></> : <><Field label="URL"><input value={form.url} onChange={(e) => update({ url: e.target.value })} placeholder="https://example.com/mcp" /></Field><PairEditor label="HTTP Headers" pairs={form.httpHeaders} onChange={(httpHeaders) => update({ httpHeaders })} /><Field label="Bearer Token"><input type="password" value={form.bearerToken} onChange={(e) => update({ bearerToken: e.target.value })} /></Field></>}
      <Field label={t("启动超时（秒）")}><input inputMode="decimal" value={form.startupTimeoutSec} onChange={(e) => update({ startupTimeoutSec: e.target.value })} placeholder={t("默认")} /></Field>
      <div className="mg-write-to"><span>{t("保存目标")}</span><strong>{t("当前 Codex 配置")}</strong><small>{t("保存后立即落盘；启停请在列表行操作。")}</small></div>
      {form.extraToml && <details><summary>{t("其他配置字段")}</summary><textarea className="mg-code" value={form.extraToml} onChange={(e) => update({ extraToml: e.target.value })} rows={5} /></details>}
    </>}{error && <p role="alert" className="mg-error">{error}</p>}
  </ManagementDrawer>;
}
function McpImport({ onClose, onImport, busy }: { onClose: () => void; onImport: (json: string) => Promise<boolean>; busy: boolean }) {
  const [json, setJson] = useState(""); const [preview, setPreview] = useState<(Result & { entries: { id: string; tomlBody: string }[]; warnings: string[] }) | null>(null); const [checking, setChecking] = useState(false); const [error, setError] = useState("");
  const check = async () => { setChecking(true); setError(""); try { const result = await invoke<NonNullable<typeof preview>>("preview_mcp_servers_json", { json }); setPreview(succeeded(result) ? result : null); if (!succeeded(result)) setError(result.message); } catch (e) { setError(String(e)); } finally { setChecking(false); } };
  return <ManagementDrawer title={t("导入 MCP JSON")} onClose={onClose} footer={<><Action onClick={() => void check()} disabled={!json.trim() || checking || busy}>{checking ? t("正在解析…") : t("预览")}</Action><Action onClick={() => void onImport(json)} disabled={!preview || busy} primary>{t("确认导入")}</Action></>}><p className="mg-muted">{t("粘贴 mcpServers、servers 或单个服务器配置。")}</p><Field label={t("MCP 配置 JSON")}><textarea rows={18} className="mg-code" spellCheck={false} value={json} onChange={(e) => { setJson(e.target.value); setPreview(null); }} placeholder={'{\n  "mcpServers": {\n    "context7": {\n      "command": "npx",\n      "args": ["-y", "@upstash/context7-mcp"]\n    }\n  }\n}'} /></Field>{preview && <div className="mg-notice"><strong>{t("将导入：")}{preview.entries.map((e) => e.id).join("、")}</strong>{preview.warnings.map((warning) => <p key={warning}>{warning}</p>)}</div>}{error && <p role="alert" className="mg-error">{error}</p>}</ManagementDrawer>;
}
