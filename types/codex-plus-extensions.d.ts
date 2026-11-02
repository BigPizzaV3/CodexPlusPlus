/**
 * Codex++ 拓展接口类型定义。
 *
 * 用法：在脚本开头加一行引用即可获得补全与类型检查。
 *
 *   /// <reference types="./codex-plus-extensions" />
 *
 * 这些声明与 assets/inject/renderer-inject/91-extension-api.js 一一对应。
 * 接口变更时两边必须同时改；契约测试会守住路由白名单与挂载点。
 */

/** 提示的样式类型，对应右下角提示的边框配色。 */
type CodexPlusToastType = "info" | "success" | "warn" | "error";

/** 当前开放给拓展调用的后端路由。新增路由需在接口层显式决定。 */
type CodexPlusExtensionRoute =
  | "/diagnostics/log"
  | "/thread-usage-history"
  | "/archived-thread"
  | "/export-markdown"
  | "/user-scripts/list"
  | "/cdp/targets"
  | "/cdp/evaluate"
  | "/proxy-status"
  | "/thread-usage-live"
  | "/backend/status"
  | "/settings/public"
  | "/model-catalog/public";

interface CodexPlusToastOptions {
  type?: CodexPlusToastType;
}

interface CodexPlusCallOptions {
  /** 超时毫秒数，默认 26000。超时只放弃等待，服务端任务仍会跑完。 */
  timeout?: number;
}

interface CodexPlusRegisterOptions {
  /** 显式指定归属脚本。默认取当前正在初始化的脚本，一般不用传。 */
  scriptKey?: string;
  /** 自定义 id 片段；不传则自动生成。 */
  id?: string;
  /** 多个拓展项之间的排序，默认按注册顺序。内置项永远排在拓展项之前。 */
  order?: number;
  /** 图标栏入口的可访问标签，默认取 title。 */
  navLabel?: string;
  icon?: string;
}

interface CodexPlusRowActionDefinition {
  label: string;
  /** 单个字形或 SVG 字符串。 */
  icon?: string;
  /** "menu"（默认）进「更多操作」菜单；"inline" 画成行上常驻的图标按钮。 */
  placement?: "menu" | "inline";
  /** 出现在哪：默认 ["sidebar"]；加上 "archived" 也画在归档页的行上。 */
  surfaces?: Array<"sidebar" | "archived">;
  /** 给行内按钮 / 归档页按钮加的类名。 */
  className?: string;
  /** 行内按钮建好、套原生外观之前调用；按钮随行重建，只设属性、不挂监听。 */
  onMount?(context: { button: HTMLButtonElement; row: HTMLElement; session_id: string }): void;
  /** 点击时调用。菜单项与 render 二选一。 */
  onActivate?(context: {
    row: HTMLElement;
    session_id: string;
    title: string;
    /** 是不是归档页的行。 */
    archived: boolean;
    /** 归档页的行未必读得到会话 id：按标题问 Codex++ 解析。 */
    resolve(): Promise<{ session_id: string; title: string }>;
    close: () => void;
    button?: HTMLElement;
    event?: Event;
  }): void;
  /** 自绘菜单项：往菜单里的一个容器画（例如 React 组件）；可返回清理函数，菜单拆掉时调用。 */
  render?(context: {
    container: HTMLElement;
    row: HTMLElement;
    session_id: string;
    title: string;
    close: () => void;
  }): void | (() => void);
}

interface CodexPlusPageDefinition {
  title: string;
  icon?: string;
  /**
   * 每次打开页面都会重新调用，且容器会被重建。
   * 不要缓存 container，也不要假设上一次插入的节点还在。
   */
  render(context: {
    container: HTMLElement;
    close: () => void;
    script: string;
  }): void;
}

interface CodexPlusNavEntryDefinition {
  label: string;
  icon?: string;
  onActivate?(): void;
}

/** 注册项的注销函数。调用后该 UI 项从所有宿主移除。 */
type CodexPlusDispose = () => void;

interface CodexPlusExtensionApi {
  /** 适配层版本。 */
  readonly version: string;
  /** 接口契约版本。后端契约变化时增加，脚本可据此提示用户升级。 */
  readonly apiVersion: number;
  /** 当前脚本的标识，形如 `user:my-script.js`；拓展包附带清单，单文件脚本为 null。 */
  readonly script: { readonly key: string; readonly manifest: CodexPlusExtensionManifest | null };

  /**
   * 创建一个拓展实例。ctx 上的每次注册都随实例卸载按逆序自动撤销；
   * 在脚本同步初始化期间调用时，热重载会自动卸载旧实例。
   */
  extension(
    definition: ((ctx: CodexPlusExtensionContext) => void | Promise<void>) | CodexPlusExtensionDefinition,
    options?: { id?: string; scriptKey?: string; inject?: CodexPlusInject },
  ): CodexPlusExtensionContext | CodexPlusExtensionFiber;

  /** 当前已提供的服务与提供方。取值请在拓展里 inject 或 ctx.get。 */
  services(): Array<{ name: string; owner: string }>;
  /** 当前打开的 Codex 会话 id；首页、新对话还没有 id 时为空串。 */
  currentSessionId(): string;
  /** 内置拓展的现状：Codex++ 自带功能也挂在拓展内核上，可停用、可被 replaces 顶替。 */
  builtins(): Array<{ id: string; name: string; active: boolean; disabled: boolean; replacedBy: string | null }>;

  /** @deprecated 早期名称，等同于 {@link CodexPlusExtensionApi.extension}。 */
  plugin: CodexPlusExtensionApi["extension"];

  /** 稳定的类名与属性契约。这些值一旦发布不再更名。 */
  readonly constants: {
    readonly pageClass: string;
    readonly pageNavAttribute: string;
    readonly railSelector: string;
    readonly railDestinationSelector: string;
    readonly actionGroupClass: string;
    readonly moreMenuClass: string;
    readonly toastClass: string;
    readonly extensionAttribute: string;
  };

  /** 右下角提示。最多同时 3 条，返回的函数可提前关闭。 */
  toast(message: string, options?: CodexPlusToastOptions): { ok: boolean; value?: CodexPlusDispose };

  /** 调用后端白名单路由，失败时 reject。 */
  call<T = unknown>(
    route: CodexPlusExtensionRoute,
    payload?: Record<string, unknown>,
    options?: CodexPlusCallOptions,
  ): Promise<T>;

  /** 在输入区（发送按钮那一行）加一个图标按钮。 */
  registerComposerAction(
    definition: CodexPlusComposerActionDefinition,
    options?: CodexPlusRegisterOptions,
  ): CodexPlusDispose;

  /** 注册斜杠命令：输入框里「/name 参数」回车时调用。 */
  registerCommand(definition: CodexPlusCommandDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;

  /** 在每条对话消息的操作行（复制、编辑按钮旁）加一个图标按钮。 */
  registerMessageAction(definition: CodexPlusMessageActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;

  /** 在每条对话消息的正文下方挂一块内容。 */
  registerMessageDecoration(definition: CodexPlusMessageDecorationDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;

  /** 注册右侧面板：顶栏右侧出现开关，打开时主区域让出面板宽度。 */
  registerSidePanel(definition: CodexPlusSidePanelDefinition, options?: CodexPlusRegisterOptions): CodexPlusSidePanelHandle;
  /** 往 Codex 界面的固定位置放内容（输入框上方、对话区顶部 / 底部、侧栏顶部 / 底部、标题栏）。 */
  registerSlot(definition: CodexPlusSlotDefinition, options?: CodexPlusRegisterOptions): CodexPlusSlotHandle;
  registerInputTrigger(definition: CodexPlusInputTriggerDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerShortcut(definition: CodexPlusShortcutDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerTheme(definition: CodexPlusThemeDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  ask(question: CodexPlusAskQuestion): Promise<{ value: unknown; text: string | null } | null>;
  registerFloatingTab(definition: CodexPlusFloatingTabDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerToolView(definition: CodexPlusToolViewDefinition, options?: CodexPlusRegisterOptions): CodexPlusSlotHandle;
  registerSettingsSection(definition: CodexPlusSettingsSectionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;

  /** 在会话行「更多操作」里加一项。 */
  registerRowAction(
    definition: CodexPlusRowActionDefinition,
    options?: CodexPlusRegisterOptions,
  ): CodexPlusDispose;

  /** 加一个图标栏入口。一般不用手写——registerPage 会自动配一个。 */
  registerNavEntry(
    definition: CodexPlusNavEntryDefinition,
    options?: CodexPlusRegisterOptions,
  ): CodexPlusDispose;

  /** 注册一个整页视图，并自动配一个图标栏入口。 */
  registerPage(
    definition: CodexPlusPageDefinition,
    options?: CodexPlusRegisterOptions,
  ): CodexPlusDispose;

  /**
   * 注册清理函数，热重载时逆序执行。
   *
   * 注意：只要有一个已加载脚本没注册清理函数，热重载就会回退到整页刷新。
   * 注册清理函数能让所有人的重载体验更好。
   */
  onCleanup(cleanup: () => void): void;

  /**
   * 主动上报失败。
   *
   * 脚本初始化期间的同步错误由 Codex++ 自动捕获，但 `await` 之后的异步错误不会，
   * 需要用这个方法上报，管理页才能显示出来。
   */
  fail(error: unknown): void;
}

interface CodexPlusCommandDefinition {
  /** 字母、数字、_、-，最长 32。 */
  name: string;
  description?: string;
  run(args: string, context: {
    editor: HTMLElement;
    insertText(text: string): boolean;
    /** 用 text 替换输入框内容。 */
    setText(text: string): boolean;
    toast(message: string, options?: CodexPlusToastOptions): void;
  }): void | Promise<void>;
}

interface CodexPlusComposerActionDefinition {
  label: string;
  /** 单个字形或 SVG 字符串。 */
  icon?: string;
  onActivate(context: {
    /** 当前输入框（contenteditable）。 */
    editor: HTMLElement | null;
    /** 把文字插入当前输入框光标处。 */
    insertText(text: string): boolean;
    close(): void;
  }): void;
}

/** 交给消息插槽的消息信息。每次回调现取，不要缓存。 */
interface CodexPlusMessage {
  role: "user" | "assistant";
  /** 助手消息是 Codex 的消息 id（msg_…），用户消息是所在轮次的单元 key。 */
  id: string;
  /** 所在轮次 id。 */
  turnId: string;
  /** 所在会话 id；取不到时为空串。 */
  conversationId: string;
  /** 消息正文的纯文本（流式输出时是当前已输出的部分）。 */
  text: string;
  /** 消息节点。只读用，节点随虚拟化滚动被 Codex 重建，不要缓存或改它的结构。 */
  element: HTMLElement;
}

interface CodexPlusMessageActionDefinition {
  label: string;
  /** 单个字形或 SVG 字符串。 */
  icon?: string;
  /** 只在这些角色的消息上出现，省略表示全部。 */
  roles?: Array<"user" | "assistant"> | "user" | "assistant";
  /** 返回 false 时这条消息不显示按钮。 */
  visible?(message: CodexPlusMessage): boolean;
  onActivate(context: { message: CodexPlusMessage; close(): void }): void;
}

interface CodexPlusMessageDecorationDefinition {
  roles?: Array<"user" | "assistant"> | "user" | "assistant";
  /** 返回 false 时这条消息不挂装饰。 */
  visible?(message: CodexPlusMessage): boolean;
  /** 往 container 里画；消息文字变化（流式输出）时清空 container 重新调用。留空则不占位。 */
  render(context: { message: CodexPlusMessage; container: HTMLElement }): void;
}

interface CodexPlusSidePanelDefinition {
  title: string;
  /** 顶栏开关的图标：单个字形或 SVG 字符串。 */
  icon?: string;
  /** 面板宽度，240~720，默认 360。 */
  width?: number;
  /** 每次打开都调用；可返回清理函数，关闭时调用。 */
  render(context: { container: HTMLElement; close(): void }): void | (() => void);
}

type CodexPlusSlotName =
  | "composer.above" | "composer.below" | "composer.left" | "composer.right" | "composer.model" | "home.above" | "settings.nav"
  | "thread.top" | "thread.bottom" | "turn.after" | "activity.after" | "header.title" | "titlebar" | "approval"
  | "sidebar.top" | "sidebar.bottom" | "sidebar.section" | "sessionRow.prefix" | "sessionRow.suffix" | "projectRow.suffix"
  | "overlay";

interface CodexPlusSlotDefinition {
  slot: CodexPlusSlotName;
  /** 越小越靠前。 */
  order?: number;
  /** 容器新建时调用（Codex 重建 DOM 后会再调）；可返回清理函数。容器留空则不占位。 */
  /**
   * turn.after 多给 turnKey；sessionRow.* 多给 sessionId、title；activity.after 多给 kind、itemIds、text；
   * approval 多给 kind（form / card）；projectRow.suffix 多给 projectId、label；sidebar.section 多给 section。
   */
  render(context: {
    container: HTMLElement; slot: CodexPlusSlotName; turnKey?: string; sessionId?: string; title?: string;
    kind?: "command" | "toolCall" | "fileChange" | "reasoning" | "plan" | "error" | "other" | "form" | "card"; itemIds?: string[]; text?: string;
    projectId?: string; label?: string; section?: string; sampleClass?: string;
  }): void | (() => void);
}

interface CodexPlusFloatingTabDefinition {
  title: string;
  /** 单个字形或 SVG 字符串。 */
  icon?: string;
  order?: number;
  render(context: { container: HTMLElement; close(): void }): void | (() => void);
}

interface CodexPlusToolCall {
  kind: "command" | "toolCall" | "fileChange" | "reasoning" | "plan" | "error" | "other";
  itemIds: string[];
  text: string;
  sessionId: string;
  callId: string | null;
  name: string;
  input: unknown;
  output: unknown;
}

interface CodexPlusToolViewDefinition {
  /** 工具名：字符串（整名或正则字符串）或正则。不给时不按工具名过滤。 */
  tools?: Array<string | RegExp>;
  kinds?: CodexPlusToolCall["kind"][];
  order?: number;
  render(context: { container: HTMLElement; call: CodexPlusToolCall }): void | (() => void);
}

interface CodexPlusSettingsSectionDefinition {
  title: string;
  icon?: string;
  order?: number;
  render(context: { container: HTMLElement; close: () => void }): void | (() => void);
}

/** 撤销函数，另带 refresh()：强制所有位置重画该项。 */
type CodexPlusSlotHandle = CodexPlusDispose & { refresh(): void };

/** 撤销函数，另带开合控制。同一时间只开一个面板。 */
type CodexPlusSidePanelHandle = CodexPlusDispose & {
  open(): boolean;
  close(): boolean;
  toggle(): boolean;
};

/** 按拓展 id 隔离的本地存储，值按 JSON 存取。 */
interface CodexPlusExtensionStorage {
  get<T = unknown>(key: string, fallback?: T): T;
  set(key: string, value: unknown): void;
  remove(key: string): void;
  keys(): string[];
}

/**
 * Codex 会话事件：`ctx.on("codex/<method>", (params, { method }) => …)`。
 * 已转发的 method：thread/started、turn/started、turn/completed、item/started、item/completed、
 * item/agentMessage/delta、error。params 是 Codex app-server 通知的原样参数。
 */
type CodexPlusCodexEvent =
  | "codex/thread/started"
  | "codex/turn/started"
  | "codex/turn/completed"
  | "codex/item/started"
  | "codex/item/completed"
  | "codex/item/agentMessage/delta"
  | "codex/error";

/** `ctx.intercept("codex/request", …)` 的值：发往 Codex app-server 的请求。只能改 params，必须同步返回。 */
interface CodexPlusCodexRequest {
  method: string;
  params: any;
}

/** `codexpp.json` 中下发给页面的字段。 */
interface CodexPlusExtensionManifest {
  id: string;
  name: string | null;
  version: string;
  description: string;
  engines: { codexpp: number };
  permissions: { routes: string[] };
  config: Record<string, unknown>;
  /** 包内 assets/ 资源：包内路径 → data URL。 */
  assets?: Record<string, string>;
  /** 包内 locale/<语言>.json 的内容。 */
  locales?: Record<string, Record<string, unknown>>;
}

/** 依赖的服务：数组都是必需依赖；对象写法里 `{ required: false }` 是可选依赖（缺了也应用，变了照样重新应用）。 */
type CodexPlusInject = string[] | Record<string, { required?: boolean } | true>;

interface CodexPlusExtensionDefinition {
  id?: string;
  /** 依赖的服务。声明后 extension() 返回生命周期句柄：必需依赖齐全才应用，提供方消失时卸载、回来时重新应用。 */
  inject?: CodexPlusInject;
  apply(ctx: CodexPlusExtensionContext): void | Promise<void>;
}

/** 声明了 inject 的拓展的生命周期句柄。 */
interface CodexPlusExtensionFiber {
  readonly id: string;
  readonly scriptKey: string;
  readonly inject: string[];
  readonly state: "pending" | "active" | "failed" | "disposed";
  /** 当前实例；挂起时为 null。 */
  readonly ctx: CodexPlusExtensionContext | null;
  dispose(): void;
}

/** 会话查询返回的会话。 */
interface CodexPlusSession {
  id: string;
  title: string;
  cwd: string;
  model: string;
  createdAt: number | null;
  updatedAt: number | null;
  archived: boolean;
  /** 首条用户消息的前 200 字。 */
  preview: string;
  gitBranch: string;
}

/** 会话事件日志里的一条。 */
interface CodexPlusSessionEvent {
  type: "turn/start" | "user/message" | "assistant/message" | "assistant/reasoning" | "tool/call" | "tool/result";
  seq: number;
  time: string;
  data: {
    turn: string;
    id?: string;
    text?: string;
    phase?: string | null;
    name?: string;
    callId?: string;
    input?: string;
    output?: string;
  };
}

/** 内置的只读会话查询服务（inject: ["sessionQuery"]；后台与模型工具的上下文里也有）。 */
interface CodexPlusSessionQuery {
  listSessions(options?: { archived?: boolean; limit?: number; offset?: number }): Promise<CodexPlusSession[]>;
  filterSessions(filters: { id?: string; cwd?: string; archived?: boolean; createdAfter?: number; createdBefore?: number; limit?: number }): Promise<CodexPlusSession[]>;
  readSession(id: string): Promise<{ session: CodexPlusSession; events: CodexPlusSessionEvent[] }>;
  readTitle(id: string): Promise<string>;
  listEvents(id: string, options?: { types?: string[]; offset?: number; limit?: number; fromEnd?: boolean }): Promise<CodexPlusSessionEvent[]>;
  filterEvents(id: string, filters?: { types?: string[]; text?: string; limit?: number }): Promise<CodexPlusSessionEvent[]>;
  readEvent(query: { sessionId: string; seq: number; before?: number; after?: number }): Promise<CodexPlusSessionEvent[]>;
  searchSessions(query: { query: string; archived?: boolean; limit?: number; cursor?: string | null }): Promise<{ items: CodexPlusSession[]; nextCursor: string | null }>;
  searchEvents(query: { query: string; sessionId?: string; types?: string[]; archived?: boolean; limit?: number; cursor?: string | null }): Promise<{ items: Array<{ sessionId: string; event: CodexPlusSessionEvent }>; nextCursor: string | null }>;
  /** seq 及之后的完整事件（增量读）。 */
  readEventsSince(query: { sessionId: string; seq: number; limit?: number }): Promise<CodexPlusSessionEvent[]>;
  /** 把事件日志折叠成状态。 */
  project<S>(id: string, reducer: { init?: S | (() => S); apply(state: S, event: CodexPlusSessionEvent): S | undefined }): Promise<S>;
  /** 订阅新事件（默认从当前末尾开始），返回撤销函数。 */
  observeSession(id: string, handler: (event: CodexPlusSessionEvent) => void, options?: { intervalMs?: number; fromSeq?: number }): CodexPlusDispose;
  /** 订阅全部会话的新事件，返回撤销函数。后台里可另给 onStatus。 */
  observeAll(handler: (event: CodexPlusSessionEvent, meta: { sessionId: string }) => void, options?: { intervalMs?: number; onStatus?: (status: CodexPlusAgentStatus) => void }): CodexPlusDispose;
}

/** `agent/status` 事件的参数。 */
interface CodexPlusAgentStatus {
  sessionId: string;
  status: "running" | "idle" | "aborted";
  turn?: string;
  reason?: string | null;
}

/** 内置的会话写入服务（inject: ["sessions"]）。 */
interface CodexPlusSessions {
  list(options?: { limit?: number; cursor?: string; archived?: boolean }): Promise<{ data: unknown[]; nextCursor?: string | null }>;
  fork(id: string, options?: Record<string, unknown>): Promise<{ id: string; forkedFromId: string | null; [key: string]: unknown }>;
  append(id: string, items: string | { role?: "user" | "assistant" | "developer"; text: string } | Array<string | { role?: string; text: string } | Record<string, unknown>>): Promise<{ appended: number }>;
  startTurn(id: string, input: string | Array<{ type: "text"; text: string }>, options?: Record<string, unknown>): Promise<{ id: string; status: string; [key: string]: unknown }>;
  interrupt(id: string, turnId: string): Promise<unknown>;
  /** 新建会话；给了 input 时紧接着发起第一轮。 */
  create(input?: string | Array<{ type: "text"; text: string }>, options?: Record<string, unknown>): Promise<{ thread: { id: string; [key: string]: unknown } | null; turn: unknown }>;
  rename(id: string, name: string): Promise<unknown>;
  /** 往正在跑的一轮插话。 */
  steer(id: string, input: string | Array<{ type: "text"; text: string }>, turnId?: string): Promise<unknown>;
  compact(id: string): Promise<unknown>;
}

/** 拓展调模型（经 Codex++ 协议代理，用当前供应商）。 */
interface CodexPlusLlm {
  complete(options: string | {
    input: string | Array<{ role?: "user" | "assistant" | "developer" | "system"; content: string | unknown[] } | string>;
    model?: string; instructions?: string; reasoningEffort?: string; maxOutputTokens?: number; temperature?: number;
  }): Promise<{ text: string; model: string | null; usage: Record<string, unknown> | null }>;
  listModels(): Promise<{ models: Array<{ id: string; name: string | null; contextWindow: number | null }>; default: string | null }>;
}

interface CodexPlusInputTriggerDefinition {
  /** 符号开头的短前缀，如 "#"、"%"。@ $ / 是 Codex 自己的，注册会报错。 */
  trigger: string;
  items(query: string, context: { editor: HTMLElement }): Array<CodexPlusInputTriggerItem> | Promise<Array<CodexPlusInputTriggerItem>>;
}

interface CodexPlusInputTriggerItem {
  label: string;
  description?: string;
  /** 选中后替换「触发符 + 查询词」的文字，默认用 label。 */
  insert?: string;
  onSelect?(): void;
}

interface CodexPlusShortcutDefinition {
  /** 如 "Mod+Shift+K"；Mod 在 macOS 是 ⌘，其余是 Ctrl。必须带修饰键（F1~F12 除外）。 */
  key: string;
  run(context: { event: KeyboardEvent }): void | Promise<void>;
  /** 焦点在输入框里时也触发，默认 true。 */
  allowInInput?: boolean;
  description?: string;
}

interface CodexPlusThemeDefinition {
  id: string;
  name?: string | Record<string, string>;
  /** CSS 变量，写进 :root。 */
  variables?: Record<string, string>;
  css?: string;
}

interface CodexPlusThemes {
  list(): Array<{ id: string; name: string; extension: string }>;
  readonly current: string | null;
  /** 切换主题；null 恢复默认。 */
  apply(id: string | null): string | null;
}

interface CodexPlusAskQuestion {
  title?: string | Record<string, string>;
  message?: string | Record<string, string>;
  /** 按钮，最后一个是默认（回车）。 */
  options?: Array<string | { value: unknown; label: string }>;
  input?: { placeholder?: string; default?: string; multiline?: boolean };
  timeoutMs?: number;
}

/** 多语言。查找顺序：当前语言 → 主语言 → en，先在命名空间、再在 common，最后返回 key。 */
interface CodexPlusLocale {
  /** 当前语言（Codex 页面的 lang，如 zh-CN）。 */
  readonly current: string;
  register(ns: string, dicts: Record<string, Record<string, unknown>>): CodexPlusDispose;
  register(ns: string, lang: string, dict: Record<string, unknown>): CodexPlusDispose;
  bind(ns: string): (key: string, params?: Record<string, unknown>) => string;
  /** 在本拓展的命名空间翻译（包内 locale/*.json 自动登记在这里）。 */
  t(key: string, params?: Record<string, unknown>): string;
  /** 解析 { en, zh, ... } 形式的多语言文本。 */
  resolve(text: string | Record<string, string>): string;
}

/** waterfall 中间件：调 next(v) 交给下一个，不调即短路。 */
type CodexPlusInterceptor<T = unknown> = (value: T, next: (value?: T) => T | Promise<T>) => T | Promise<T>;

interface CodexPlusExtensionContext {
  readonly id: string;
  readonly scriptKey: string;
  readonly manifest: CodexPlusExtensionManifest | null;
  readonly disposed: boolean;
  readonly storage: CodexPlusExtensionStorage;
  /** 常驻后台（清单 `backend`）：call 调它导出的方法，on 订阅它 emit 的事件。 */
  readonly backend: {
    /** 首次调用时启动后台进程。超时默认 20 秒，最多 24 秒。 */
    call<T = unknown>(method: string, params?: unknown, options?: { timeout?: number }): Promise<T>;
    /** 订阅后台事件（"*" 收全部）；不会启动后台，后台起来后才开始收到。 */
    on(event: string, handler: (payload: unknown, meta: { event: string }) => void): CodexPlusDispose;
  };
  /** 生效配置：清单 `config` 的默认值叠加用户设置的值。只读；改配置会热重载拓展。 */
  readonly config: Readonly<Record<string, unknown>>;
  /** 清单 `instances` 里的哪一份；没声明实例时为 null。 */
  readonly instance: { id: string; name: string } | null;
  /**
   * 配置原地更新：管理器保存配置后先问这里，handler 返回 true 表示接住了（ctx.config 换成新值、不重载）；
   * 这个拓展只要有一个实例没接，就照常整体热重载。
   */
  onConfigUpdate(handler: (next: Readonly<Record<string, unknown>>, previous: Readonly<Record<string, unknown>>) => boolean): CodexPlusDispose;
  /** 包内 assets/ 资源（图片、音频、字体）的 data URL；找不到返回空串。 */
  asset(path: string): string;
  /** 多语言（随实例卸载自动撤销登记）。 */
  readonly locale: CodexPlusLocale;
  /** inject 了 sessionQuery 时可用。其它 inject 的服务同样以服务名挂在 ctx 上。 */
  readonly sessionQuery?: CodexPlusSessionQuery;
  /** inject 了 sessions 时可用：新建、分叉、改名、追加条目、发起 / 插话 / 中断一轮、压缩。 */
  readonly sessions?: CodexPlusSessions;
  /** 调模型。 */
  readonly llm: CodexPlusLlm;
  /** 主题。 */
  readonly themes: CodexPlusThemes;
  /** 所属生命周期句柄（仅声明了 inject 的拓展）。 */
  readonly fiber?: CodexPlusExtensionFiber;

  /** 提供服务：服务名全局唯一，重名抛错；随实例卸载撤销，依赖它的拓展随之卸载。 */
  provide(name: string, value: unknown): CodexPlusDispose;
  /** 取服务，不存在时为 undefined（可选依赖）。 */
  get<T = unknown>(name: string): T | undefined;
  /** 子作用域：deps 齐全时以子 ctx 调用 apply；服务消失时卸载、回来时重新应用。 */
  inject(deps: CodexPlusInject, apply: (ctx: CodexPlusExtensionContext) => void | Promise<void>): CodexPlusExtensionFiber;

  /** 立即执行 fn；它返回的函数在卸载时调用。返回值可提前撤销。 */
  effect(fn: () => void | (() => void)): CodexPlusDispose;

  registerRowAction(definition: CodexPlusRowActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerNavEntry(definition: CodexPlusNavEntryDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  /** 定义形态同 codexPlus.registerMenuItem：开关给 onChange，按钮给 onActivate。 */
  registerMenuItem(definition: Record<string, unknown>, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerPage(definition: CodexPlusPageDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerComposerAction(definition: CodexPlusComposerActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerCommand(definition: CodexPlusCommandDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerMessageAction(definition: CodexPlusMessageActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerMessageDecoration(definition: CodexPlusMessageDecorationDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerSidePanel(definition: CodexPlusSidePanelDefinition, options?: CodexPlusRegisterOptions): CodexPlusSidePanelHandle;
  /** 往 Codex 界面的固定位置放内容（输入框上方、对话区顶部 / 底部、侧栏顶部 / 底部、标题栏）。 */
  registerSlot(definition: CodexPlusSlotDefinition, options?: CodexPlusRegisterOptions): CodexPlusSlotHandle;
  registerInputTrigger(definition: CodexPlusInputTriggerDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerShortcut(definition: CodexPlusShortcutDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerTheme(definition: CodexPlusThemeDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  /** 弹对话框问用户；取消或超时为 null。 */
  ask(question: CodexPlusAskQuestion): Promise<{ value: unknown; text: string | null } | null>;
  registerFloatingTab(definition: CodexPlusFloatingTabDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerToolView(definition: CodexPlusToolViewDefinition, options?: CodexPlusRegisterOptions): CodexPlusSlotHandle;
  registerSettingsSection(definition: CodexPlusSettingsSectionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  /** 当前打开的 Codex 会话 id；首页、新对话还没有 id 时为空串。 */
  currentSessionId(): string;

  /** 拓展间事件；内置还有 `session/event`、`agent/status`、`theme/change`、`extensions/changed`、`settings/loaded`（`{ reason: "startup" | "heartbeat", changedKeys? }`，心跳时 changedKeys 是这次变了的设置字段名）、`settings/changed`（`{ key, value, persisted }`）、`relayProfile/switched`（`{ previous, profile }`）、`modelCatalog/changed`（`{ profile }`，profile 形状同 `/settings/public` 的 activeProfile）。`codex/*` 是 Codex 会话事件（见 CodexPlusCodexEvent）。内核广播 `extension/apply`、`extension/dispose`，参数为 `{ id, scriptKey }`。`ready` / `dispose` 只属于本实例：apply 完成后 / 卸载时调用。 */
  on(event: string, handler: (...args: any[]) => void): CodexPlusDispose;
  emit(event: string, ...args: unknown[]): void;

  /** 往名为 name 的中间件链挂一个中间件，order 小的先执行。 */
  intercept<T = unknown>(name: string, handler: CodexPlusInterceptor<T>, options?: { order?: number }): CodexPlusDispose;
  /** 跑一条中间件链；没有中间件时原样返回 value。 */
  waterfall<T = unknown>(name: string, value: T): T | Promise<T>;

  listen(target: EventTarget, type: string, handler: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): CodexPlusDispose;
  setTimeout(handler: (...args: any[]) => void, delay?: number, ...args: unknown[]): CodexPlusDispose;
  setInterval(handler: (...args: any[]) => void, delay?: number, ...args: unknown[]): CodexPlusDispose;
  /**
   * 跟着 Codex++ 的页面扫描跑（Codex 界面变了时调用，按帧合并、页面隐藏时不跑）。phase：deferred（默认）或 light（每次变化都跑）。
   * 插进页面的节点必须带 data-codex-plus-ext 属性。返回的撤销函数带 run()：立即跑一次。
   */
  onScan(handler: () => void, options?: { phase?: "deferred" | "light" }): CodexPlusDispose & { run(): void };

  /** 拓展包只能调 codexpp.json `permissions.routes` 里声明的路由。 */
  call<T = unknown>(route: CodexPlusExtensionRoute, payload?: Record<string, unknown>, options?: CodexPlusCallOptions): Promise<T>;
  toast(message: string, options?: CodexPlusToastOptions): { ok: boolean; value?: CodexPlusDispose };
  fail(error: unknown): void;
  /** 主动卸载。重复调用无效果。 */
  dispose(): void;
}

/** 管理器新增只读数据的返回结构；不包含供应商地址、凭据、磁盘路径或任务内容。 */
interface CodexPlusManagerPublicData {
  gatewayProviders: { providers: Array<{
    id: string; name: string; enabled: boolean; protocol: string; models: string[];
    modelWindows: Record<string, number>; contextWindow: number | null;
    keyRpm: number | null; keyConcurrency: number | null;
    price: CodexPlusManagerModelPrice | null; modelPrices: Record<string, CodexPlusManagerModelPrice>;
  }> };
  gatewayRouting: {
    groups: CodexPlusManagerRouteGroup[];
    autoGroups: Array<{ group: CodexPlusManagerRouteGroup; hidden: boolean; customized: boolean }>;
  };
  gatewayHealth: {
    scope: "managerProcess";
    providers: Array<{ relayId: string; breakerOpen: boolean; remainingMs: number | null; consecutiveErrors: number }>;
    speeds: Array<{ relayId: string; model: string; ms: number; samples: number }>;
    agents: Array<{ agent: string; requests: number; rerouted: number; errors: number; inFlight: string[] }>;
  };
  gatewayQuota: {
    scope: "managerProcess"; cachedOnly: true;
    providers: Array<{ id: string; name: string; quota: null | {
      windows: Array<{ windowMinutes: number | null; usedPercent: number; resetsAtMs: number | null }>;
      updatedAtMs: number;
    } }>;
  };
  installedSkills: { skills: Array<{
    id: string; name: string; enabled: boolean; bundled: boolean; managed: boolean; agents: string[]; contentHash: string;
  }> };
  /** 仅 Codex++ 市场托管的插件，不是全部 Codex 插件。 */
  installedPlugins: { plugins: Array<{ id: string; name: string; version: string; source: "public" | "full"; enabled: boolean }> };
  /** 已落盘快照，不刷新租约，不读取未 checkpoint 的 WAL。 */
  agentCollaboration: {
    enabled: boolean;
    agents: Array<{ id: string; name: string; enabled: boolean; model: string; maxConcurrentTasks: number }>;
    tasks: Array<{ id: string; agentId: string; status: string; createdAtMs: number; updatedAtMs: number; cancelRequested: boolean }>;
    total: number;
  };
}
interface CodexPlusManagerModelPrice { currency: string; input: number; cachedInput: number; output: number }
interface CodexPlusManagerRouteGroup {
  id: string; name: string; mode: string; sticky: string;
  members: Array<{ model: string; weight: number }>; hideMembers: boolean; pick: string;
}

/** 管理器面板接口：本拓展的配置、后台、资源，以及清单声明过的只读公共数据。 */
interface CodexPlusManagerPanelApi {
  config: {
    get(): Promise<Record<string, unknown>>;
    /** 只能写清单 config 里声明过的项；保存后返回最新配置。 */
    set(values: Record<string, unknown>): Promise<Record<string, unknown>>;
    onChange(handler: (config: Record<string, unknown>) => void): () => void;
  };
  backend: {
    /** 调本拓展常驻后台导出的方法，超时 30 秒。 */
    call<T = unknown>(method: string, params?: unknown): Promise<T>;
  };
  /** 本拓展 assets/ 里资源的 data URL。 */
  asset(path: string): Promise<string>;
  toast(message: string): Promise<null>;
  theme(): Promise<"dark" | "light">;
  onTheme(handler: (theme: "dark" | "light") => void): () => void;
  /** 读清单 `manager.data` 里声明过的只读管理器数据。 */
  data<K extends keyof CodexPlusManagerPublicData>(name: K, params?: Record<string, unknown>): Promise<CodexPlusManagerPublicData[K]>;
  data(name: "requestFlow" | "requestLogs" | "gatewayUsage" | "localSessions", params?: Record<string, unknown>): Promise<unknown>;
  /** 跳到管理器的另一个页面：路由 id（如 "hub"），或 "ext:<拓展 key>" 打开另一个拓展页面。 */
  navigate(target: string): Promise<null>;
}

interface Window {
  codexPlus: CodexPlusExtensionApi;
  /** 只在管理器面板页里存在。 */
  codexPlusManager?: CodexPlusManagerPanelApi;
}
