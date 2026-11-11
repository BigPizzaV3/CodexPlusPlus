import { invoke } from "@tauri-apps/api/core";
import { Check, ChevronDown, Download, Eye, EyeOff, Loader2, Play, Plus, RefreshCw, ShieldCheck, Square, Trash2, X } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import {
  IMPORT_SOURCES,
  SOURCE_LABELS,
  WINDOW_PRESETS,
  claudeState,
  emptyProfile,
  formatWindow,
  hostOf,
  isOk,
  modelLabel,
  normalizeModels,
  profileProblem,
  setupSteps,
  type ClaudeConfig,
  type ClaudeStatus,
  type CommandResult,
  type UpstreamBook,
  type UpstreamModel,
  type UpstreamProfile,
} from "./model";

type Tab = "upstream" | "claude";
type Notice = { text: string; tone: "ok" | "error" } | null;
type ClaudeAction = "launch" | "stop" | "trust" | "untrust" | "prepare" | "sync_sessions";

const errorText = (cause: unknown) => (cause instanceof Error ? cause.message : String(cause));

export function ClaudeMomApp() {
  const [book, setBook] = useState<UpstreamBook>({ profiles: [], activeId: null });
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [draft, setDraft] = useState<UpstreamProfile | null>(null);
  const [status, setStatus] = useState<ClaudeStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [notice, setNotice] = useState<Notice>(null);
  const [tab, setTab] = useState<Tab>("upstream");
  const pending = useRef(false);

  const say = (text: string, tone: "ok" | "error" = "ok") => setNotice(text ? { text, tone } : null);

  const acceptBook = useCallback((result: CommandResult<UpstreamBook>, keepSelection?: string | null) => {
    const next = { profiles: result.profiles ?? [], activeId: result.activeId ?? null };
    setBook(next);
    const wanted = keepSelection ?? null;
    const target = next.profiles.find((profile) => profile.id === wanted) ?? next.profiles.find((profile) => profile.id === next.activeId) ?? next.profiles[0] ?? null;
    setSelectedId(target?.id ?? null);
    setDraft(target ? { ...target } : null);
  }, []);

  const refreshStatus = useCallback(async () => {
    try {
      const result = await invoke<CommandResult<ClaudeStatus>>("claude_local_status");
      if (isOk(result)) setStatus(result);
      else say(result.message, "error");
    } catch (cause) {
      say(errorText(cause), "error");
    }
  }, []);

  // 启动：读上游（同时切到完整功能模式、把选中的上游交给本地服务），再读 Claude 状态。
  useEffect(() => {
    void (async () => {
      try {
        const result = await invoke<CommandResult<UpstreamBook>>("claude_mom_load");
        acceptBook(result);
        if (!isOk(result)) say(result.message, "error");
      } catch (cause) {
        say(errorText(cause), "error");
      }
      await refreshStatus();
    })();
  }, [acceptBook, refreshStatus]);

  // Claude 开着或本地服务在跑时每 3 秒刷新一次状态；闲着不轮询（检测要查进程和证书）。
  useEffect(() => {
    if ((!status?.running && !status?.claudeRunning) || busy) return;
    const timer = setInterval(() => void refreshStatus(), 3000);
    return () => clearInterval(timer);
  }, [status?.running, status?.claudeRunning, busy, refreshStatus]);

  /** 同一时间只做一件事；结束后刷新状态。 */
  const run = async <T,>(label: string, task: () => Promise<T>): Promise<T | undefined> => {
    if (pending.current) return undefined;
    pending.current = true;
    setBusy(label);
    try {
      return await task();
    } catch (cause) {
      say(errorText(cause), "error");
      return undefined;
    } finally {
      pending.current = false;
      setBusy(null);
    }
  };

  const claudeAction = (action: ClaudeAction) =>
    run(action, async () => {
      const result = await invoke<CommandResult<ClaudeStatus>>("claude_local_action", { action });
      if (isOk(result)) setStatus(result);
      say(result.message, isOk(result) ? "ok" : "error");
    });

  const configure = (patch: Partial<ClaudeConfig>) =>
    run("configure", async () => {
      if (!status) return;
      const config = { ...status.config, ...patch, enabled: true };
      const result = await invoke<CommandResult<ClaudeStatus>>("claude_local_configure", { config });
      if (isOk(result)) setStatus(result);
      else say(result.message, "error");
    });

  const bookCall = (label: string, command: string, args: Record<string, unknown>, keep?: string | null) =>
    run(label, async () => {
      const result = await invoke<CommandResult<UpstreamBook>>(command, args);
      acceptBook(result, keep);
      say(result.message, isOk(result) ? "ok" : "error");
      await refreshStatus();
      return result;
    });

  const active = book.profiles.find((profile) => profile.id === book.activeId) ?? null;
  const dirty = useMemo(() => {
    if (!draft) return false;
    const saved = book.profiles.find((profile) => profile.id === draft.id);
    return !saved || JSON.stringify(saved) !== JSON.stringify(draft);
  }, [book, draft]);

  const select = (id: string) => {
    const profile = book.profiles.find((item) => item.id === id);
    if (!profile) return;
    setSelectedId(id);
    setDraft({ ...profile });
    setTab("upstream");
  };

  const addProfile = () => {
    setSelectedId(null);
    setDraft(emptyProfile());
    setTab("upstream");
  };

  const saveDraft = async (activate: boolean) => {
    if (!draft) return;
    const problem = profileProblem(draft);
    if (problem) return say(problem, "error");
    const profile = { ...draft, models: normalizeModels(draft.models, draft.model) };
    const result = await bookCall("save", "claude_mom_save_profile", { profile, activate }, draft.id || undefined);
    // 新增的条目保存后才有 id：选中刚存的那条（列表最后一条或同名那条）。
    if (result && isOk(result) && !draft.id) {
      const created = [...(result.profiles ?? [])].reverse().find((item) => item.baseUrl === profile.baseUrl.trim().replace(/\/+$/, ""));
      if (created) {
        setSelectedId(created.id);
        setDraft({ ...created });
      }
    }
  };

  const fetchModels = () =>
    run("models", async () => {
      if (!draft) return;
      const result = await invoke<CommandResult<{ models: UpstreamModel[] }>>("claude_mom_fetch_models", { baseUrl: draft.baseUrl, apiKey: draft.apiKey });
      if (!isOk(result)) return say(result.message, "error");
      const ids = result.models.map((model) => model.id);
      setDraft((current) => {
        if (!current) return current;
        const model = current.model || ids[0] || "";
        const window = result.models.find((item) => item.id === model)?.contextWindow ?? current.contextWindow;
        const names = Object.fromEntries(result.models.filter((item) => item.displayName).map((item) => [item.id, item.displayName as string]));
        return { ...current, models: normalizeModels(ids, model), model, contextWindow: window ?? null, modelNames: { ...current.modelNames, ...names } };
      });
      say(result.message);
    });

  const state = claudeState(status, busy === "launch" || busy === "stop" ? busy : null);
  const steps = setupSteps(status, active);
  const canLaunch = Boolean(status?.supported && status.nodeAvailable && status.caTrusted && active && !profileProblem(active));
  const showStop = Boolean(status?.running || status?.localProcess);

  return (
    <div className="cm-shell">
      <aside className="cm-side">
        <div className="cm-side-head" data-tauri-drag-region>
          <span className="cm-brand">ClaudeMom</span>
        </div>
        <div className="cm-side-label">上游</div>
        <nav className="cm-list" aria-label="上游">
          {book.profiles.map((profile) => (
            <button
              aria-current={profile.id === selectedId ? "true" : undefined}
              className="cm-item"
              key={profile.id}
              onClick={() => select(profile.id)}
              onDoubleClick={() => void bookCall("activate", "claude_mom_set_active", { id: profile.id }, profile.id)}
              title="双击设为当前"
              type="button"
            >
              <span className="cm-radio" data-on={profile.id === book.activeId || undefined} />
              <span className="cm-item-text">
                <span className="cm-item-name">{profile.name || hostOf(profile.baseUrl)}</span>
                <span className="cm-item-sub">{profile.model ? modelLabel(profile, profile.model) : hostOf(profile.baseUrl)}</span>
              </span>
              {profile.source !== "manual" ? <span className="cm-tag">{SOURCE_LABELS[profile.source] ?? profile.source}</span> : null}
            </button>
          ))}
          {draft && !draft.id ? (
            <div className="cm-item" aria-current="true">
              <span className="cm-radio" />
              <span className="cm-item-text"><span className="cm-item-name">新上游</span><span className="cm-item-sub">未保存</span></span>
            </div>
          ) : null}
          {!book.profiles.length && !draft ? <p className="cm-empty">还没有上游。手动添加，或从 cc-switch / Magpie / Codex++ 导入。</p> : null}
        </nav>
        <div className="cm-side-foot">
          <button className="cm-btn cm-btn-quiet" onClick={addProfile} type="button"><Plus />添加</button>
          <ImportMenu disabled={busy !== null} onPick={(source) => void bookCall("import", "claude_mom_import", { source }, selectedId)} />
        </div>
      </aside>

      <main className="cm-main">
        <header className="cm-toolbar" data-tauri-drag-region>
          <span className="cm-dot" data-tone={state.tone} />
          <div className="cm-toolbar-text" data-tauri-drag-region>
            <strong>Claude Desktop · {state.text}</strong>
            <span>
              {status?.supported ? `版本 ${status.version || "未知"}` : "需要先安装官方 Claude Desktop"}
              {active ? ` · ${active.name} · ${active.model ? modelLabel(active, active.model) : "未选模型"} · 窗口 ${formatWindow(active.contextWindow)}` : ""}
            </span>
          </div>
          <div className="cm-toolbar-actions">
            <button aria-label="重新检测" className="cm-icon" disabled={busy !== null} onClick={() => void run("detect", refreshStatus)} title="重新检测" type="button">
              <RefreshCw className={busy === "detect" ? "cm-spin" : undefined} />
            </button>
            {showStop ? (
              <button className="cm-btn" disabled={busy !== null} onClick={() => void claudeAction("stop")} type="button"><Square />停止</button>
            ) : null}
            <button className="cm-btn cm-btn-primary" disabled={busy !== null || !canLaunch || Boolean(status?.claudeRunning)} onClick={() => void claudeAction("launch")} title={status?.claudeRunning ? "先完全退出 Claude 再启动" : undefined} type="button">
              {busy === "launch" ? <Loader2 className="cm-spin" /> : <Play />}启动 Claude
            </button>
          </div>
        </header>

        {steps.length || status?.needsRestart || status?.warning ? (
          <div className="cm-banners">
            {steps.map((step) => (
              <div className="cm-banner" key={step.id}>
                <span>{step.text}</span>
                {step.id === "trust" ? (
                  <button className="cm-btn cm-btn-small" disabled={busy !== null} onClick={() => void claudeAction("trust")} type="button"><ShieldCheck />信任证书</button>
                ) : step.id === "upstream" && !draft ? (
                  <button className="cm-btn cm-btn-small" onClick={addProfile} type="button"><Plus />添加上游</button>
                ) : null}
              </div>
            ))}
            {status?.needsRestart ? <div className="cm-banner" data-tone="info"><span>设置变了：完全退出 Claude 后重新启动生效。</span></div> : null}
            {status?.warning && !steps.length ? <div className="cm-banner"><span>{status.warning}</span></div> : null}
          </div>
        ) : null}

        <div className="cm-tabs" role="tablist">
          <button aria-selected={tab === "upstream"} onClick={() => setTab("upstream")} role="tab" type="button">上游</button>
          <button aria-selected={tab === "claude"} onClick={() => setTab("claude")} role="tab" type="button">Claude 设置</button>
        </div>

        <section className="cm-body">
          {tab === "upstream" ? (
            draft ? (
              <UpstreamEditor
                active={draft.id !== "" && draft.id === book.activeId}
                busy={busy}
                dirty={dirty}
                draft={draft}
                onActivate={() => void bookCall("activate", "claude_mom_set_active", { id: draft.id }, draft.id)}
                onChange={setDraft}
                onDelete={() => void bookCall("delete", "claude_mom_delete_profile", { id: draft.id })}
                onDiscard={() => {
                  const saved = book.profiles.find((profile) => profile.id === draft.id);
                  if (saved) setDraft({ ...saved });
                  else acceptBook({ ...book, status: "ok", message: "" });
                }}
                onFetchModels={() => void fetchModels()}
                onSave={(activate) => void saveDraft(activate)}
              />
            ) : (
              <div className="cm-placeholder">
                <p>选一组上游，或者新建一组。</p>
                <button className="cm-btn" onClick={addProfile} type="button"><Plus />添加上游</button>
              </div>
            )
          ) : (
            <ClaudeSettings busy={busy} onAction={(action) => void claudeAction(action)} onConfigure={(patch) => void configure(patch)} status={status} />
          )}
        </section>

        <footer className="cm-status">
          <span>{status?.port ? `本地服务 · 端口 ${status.port}` : "本地服务未运行"}</span>
          <span>{book.profiles.length} 组上游</span>
          {notice ? (
            <span className="cm-notice" data-tone={notice.tone} role={notice.tone === "error" ? "alert" : "status"}>
              {notice.text}
              <button aria-label="关闭提示" onClick={() => setNotice(null)} type="button"><X /></button>
            </span>
          ) : null}
        </footer>
      </main>
    </div>
  );
}

function ImportMenu({ disabled, onPick }: { disabled: boolean; onPick: (source: string) => void }) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
    const close = (event: MouseEvent) => {
      if (!ref.current?.contains(event.target as Node)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", close);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("mousedown", close);
      document.removeEventListener("keydown", escape);
    };
  }, [open]);
  return (
    <div className="cm-menu-anchor" ref={ref}>
      <button aria-expanded={open} className="cm-btn cm-btn-quiet" disabled={disabled} onClick={() => setOpen((value) => !value)} type="button">
        <Download />导入<ChevronDown className="cm-chevron" />
      </button>
      {open ? (
        <div className="cm-menu" role="menu">
          {IMPORT_SOURCES.map((source) => (
            <button key={source.id} onClick={() => { setOpen(false); onPick(source.id); }} role="menuitem" type="button">
              <strong>{source.label}</strong>
              <span>{source.detail}</span>
            </button>
          ))}
        </div>
      ) : null}
    </div>
  );
}

function Field({ label, hint, children }: { label: string; hint?: string; children: ReactNode }) {
  return (
    <label className="cm-field">
      <span className="cm-field-label">{label}</span>
      <span className="cm-field-control">{children}</span>
      {hint ? <span className="cm-field-hint">{hint}</span> : null}
    </label>
  );
}

function UpstreamEditor({ draft, active, dirty, busy, onChange, onSave, onActivate, onDelete, onDiscard, onFetchModels }: {
  draft: UpstreamProfile;
  active: boolean;
  dirty: boolean;
  busy: string | null;
  onChange: (next: UpstreamProfile) => void;
  onSave: (activate: boolean) => void;
  onActivate: () => void;
  onDelete: () => void;
  onDiscard: () => void;
  onFetchModels: () => void;
}) {
  const [showKey, setShowKey] = useState(false);
  const [newModel, setNewModel] = useState("");
  const set = (patch: Partial<UpstreamProfile>) => onChange({ ...draft, ...patch });
  const preset = WINDOW_PRESETS.find((item) => item.value === draft.contextWindow);
  const listId = `cm-models-${draft.id || "new"}`;
  const addModel = () => {
    const model = newModel.trim();
    if (!model) return;
    set({ models: normalizeModels([...draft.models, model]), model: draft.model || model });
    setNewModel("");
  };
  return (
    <div className="cm-editor">
      <div className="cm-editor-head">
        <input aria-label="名称" className="cm-title-input" onChange={(event) => set({ name: event.currentTarget.value })} placeholder={hostOf(draft.baseUrl) || "新上游"} value={draft.name} />
        {draft.source !== "manual" ? <span className="cm-tag">来自 {SOURCE_LABELS[draft.source] ?? draft.source}</span> : null}
        {active ? <span className="cm-tag" data-tone="ok"><Check />当前</span> : null}
      </div>

      <div className="cm-form">
        <Field hint="和 ANTHROPIC_BASE_URL 一样，写到根地址即可" label="Base URL">
          <input autoComplete="off" onChange={(event) => set({ baseUrl: event.currentTarget.value })} placeholder="https://api.example.com" spellCheck={false} value={draft.baseUrl} />
        </Field>
        <Field label="Key">
          <input autoComplete="off" onChange={(event) => set({ apiKey: event.currentTarget.value })} placeholder="sk-…" spellCheck={false} type={showKey ? "text" : "password"} value={draft.apiKey} />
          <button aria-label={showKey ? "隐藏 Key" : "显示 Key"} className="cm-icon" onClick={() => setShowKey((value) => !value)} type="button">{showKey ? <EyeOff /> : <Eye />}</button>
        </Field>
        <Field hint={draft.model && draft.modelNames?.[draft.model] ? `${draft.modelNames[draft.model]}。在 Claude 里选 Opus / Sonnet 等自带条目时也用它` : "在 Claude 里选 Opus / Sonnet 等自带条目时也用它"} label="默认模型">
          <input list={listId} onChange={(event) => set({ model: event.currentTarget.value })} placeholder="claude-opus-4-6" spellCheck={false} value={draft.model} />
          <button className="cm-btn cm-btn-small" disabled={busy !== null || !draft.baseUrl.trim()} onClick={onFetchModels} title="从 {Base URL}/v1/models 拉取" type="button">
            {busy === "models" ? <Loader2 className="cm-spin" /> : <RefreshCw />}拉取模型
          </button>
        </Field>
        <Field hint="标题生成等小任务用；留空同默认模型" label="快速模型">
          <input list={listId} onChange={(event) => set({ smallModel: event.currentTarget.value })} placeholder={draft.model || "同默认模型"} spellCheck={false} value={draft.smallModel} />
        </Field>
        <datalist id={listId}>{draft.models.map((model) => <option key={model} label={modelLabel(draft, model)} value={model} />)}</datalist>
        <Field label="上下文窗口">
          <select onChange={(event) => set({ contextWindow: event.currentTarget.value === "" ? null : Number(event.currentTarget.value) })} value={draft.contextWindow == null ? "" : String(draft.contextWindow)}>
            {WINDOW_PRESETS.map((item) => <option key={item.label} value={item.value == null ? "" : String(item.value)}>{item.label}</option>)}
            {!preset && draft.contextWindow ? <option value={String(draft.contextWindow)}>{formatWindow(draft.contextWindow)}</option> : null}
          </select>
        </Field>
        <div className="cm-field">
          <span className="cm-field-label">模型列表</span>
          <div className="cm-chips">
            {draft.models.map((model) => (
              <span className="cm-chip" data-primary={model === draft.model || undefined} key={model}>
                <button onClick={() => set({ model })} title={`${model}（点一下设为默认）`} type="button">{modelLabel(draft, model)}</button>
                <button aria-label={`移除 ${model}`} onClick={() => set({ models: draft.models.filter((item) => item !== model), model: draft.model === model ? "" : draft.model })} type="button"><X /></button>
              </span>
            ))}
            <input
              aria-label="添加模型"
              className="cm-chip-input"
              onChange={(event) => setNewModel(event.currentTarget.value)}
              onKeyDown={(event) => { if (event.key === "Enter") { event.preventDefault(); addModel(); } }}
              placeholder="添加模型，回车"
              spellCheck={false}
              value={newModel}
            />
          </div>
          <span className="cm-field-hint">这些会出现在 Claude 的模型选择器里；点一个设为默认。</span>
        </div>
      </div>

      <div className="cm-editor-foot">
        {draft.id ? <button className="cm-btn cm-btn-danger" disabled={busy !== null} onClick={onDelete} type="button"><Trash2 />删除</button> : null}
        <span className="cm-spacer" />
        {dirty ? <button className="cm-btn cm-btn-quiet" disabled={busy !== null} onClick={onDiscard} type="button">放弃修改</button> : null}
        {!dirty && draft.id && !active ? <button className="cm-btn" disabled={busy !== null} onClick={onActivate} type="button">设为当前</button> : null}
        {dirty ? <button className="cm-btn" disabled={busy !== null} onClick={() => onSave(false)} type="button">保存</button> : null}
        {dirty ? <button className="cm-btn cm-btn-primary" disabled={busy !== null} onClick={() => onSave(true)} type="button">保存并使用</button> : null}
      </div>
    </div>
  );
}

function Toggle({ label, detail, checked, disabled, onChange }: { label: string; detail?: string; checked: boolean; disabled?: boolean; onChange: (next: boolean) => void }) {
  return (
    <label className="cm-toggle-row">
      <span className="cm-toggle-text">
        <span>{label}</span>
        {detail ? <span className="cm-field-hint">{detail}</span> : null}
      </span>
      <input checked={checked} disabled={disabled} onChange={(event) => onChange(event.currentTarget.checked)} type="checkbox" />
      <span aria-hidden="true" className="cm-switch" />
    </label>
  );
}

function ClaudeSettings({ status, busy, onConfigure, onAction }: {
  status: ClaudeStatus | null;
  busy: string | null;
  onConfigure: (patch: Partial<ClaudeConfig>) => void;
  onAction: (action: ClaudeAction) => void;
}) {
  if (!status) return <div className="cm-placeholder"><Loader2 className="cm-spin" /></div>;
  const config = status.config;
  const disabled = busy !== null || !status.supported;
  const text = (key: "displayName" | "email", placeholder: string, label: string, type = "text") => (
    <Field label={label}>
      <input
        defaultValue={config[key] ?? ""}
        disabled={disabled}
        key={config[key] ?? ""}
        onBlur={(event) => { if (event.currentTarget.value !== (config[key] ?? "")) onConfigure({ [key]: event.currentTarget.value }); }}
        onKeyDown={(event) => { if (event.key === "Enter") event.currentTarget.blur(); }}
        placeholder={placeholder}
        type={type}
      />
    </Field>
  );
  return (
    <div className="cm-settings">
      <div className="cm-group">
        <h3>功能</h3>
        <Toggle checked={config.localization} disabled={disabled} label="界面汉化" onChange={(next) => onConfigure({ localization: next })} />
        <Toggle checked={config.voiceInput} detail={status.dictationReady ? undefined : "本地听写模型还没准备好"} disabled={disabled} label="原生听写" onChange={(next) => onConfigure({ voiceInput: next })} />
        <Toggle checked={config.computerUse} disabled={disabled} label="电脑控制" onChange={(next) => onConfigure({ computerUse: next })} />
        <Toggle checked={config.backgroundMode} disabled={disabled} label="后台模式" onChange={(next) => onConfigure({ backgroundMode: next })} />
        <Toggle checked={config.hideUnroutedModels ?? true} detail="Claude 自带的 Opus / Sonnet 等条目会被换成默认模型" disabled={disabled} label="只显示上游的模型" onChange={(next) => onConfigure({ hideUnroutedModels: next })} />
      </div>
      <div className="cm-group">
        <h3>账号显示</h3>
        <p className="cm-field-hint">只改 Claude 里显示的信息，下次启动生效。</p>
        {text("displayName", "Local User", "用户名")}
        {text("email", "local@claude.local", "邮箱", "email")}
        <Field label="套餐">
          <select disabled={disabled} onChange={(event) => onConfigure({ plan: event.currentTarget.value })} value={config.plan || "ultra"}>
            {["free", "pro", "max", "ultra"].map((plan) => <option key={plan} value={plan}>{plan[0].toUpperCase() + plan.slice(1)}</option>)}
          </select>
        </Field>
      </div>
      <div className="cm-group">
        <h3>会话</h3>
        <Toggle checked={config.sessionSync === true} detail="启动前同步 3p 模式与完整功能模式的 Code 会话" disabled={disabled} label="自动同步会话" onChange={(next) => onConfigure({ sessionSync: next })} />
        <div className="cm-row-actions">
          <button className="cm-btn cm-btn-small" disabled={disabled || status.claudeRunning} onClick={() => onAction("sync_sessions")} type="button">立即同步</button>
        </div>
      </div>
      <div className="cm-group">
        <h3>本地证书</h3>
        <p className="cm-field-hint">{status.caTrusted ? "已信任。" : status.caReady ? "已准备，还没信任。" : "还没准备。"}本地服务用它接管 Claude 的请求，只在本机有效。</p>
        <div className="cm-row-actions">
          {!status.caReady ? <button className="cm-btn cm-btn-small" disabled={busy !== null} onClick={() => onAction("prepare")} type="button">准备证书</button> : null}
          {status.caTrusted
            ? <button className="cm-btn cm-btn-small" disabled={busy !== null} onClick={() => onAction("untrust")} type="button">取消信任</button>
            : <button className="cm-btn cm-btn-small" disabled={busy !== null} onClick={() => onAction("trust")} type="button"><ShieldCheck />信任证书</button>}
        </div>
      </div>
    </div>
  );
}
