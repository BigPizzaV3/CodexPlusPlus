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
  | "/session/export"
  | "/thread-usage-history"
  | "/archived-thread"
  | "/export-markdown"
  | "/user-scripts/list"
  | "/cdp/targets"
  | "/cdp/evaluate";

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
  onActivate(context: {
    row: HTMLElement;
    session_id: string;
    close: () => void;
  }): void;
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
    options?: { id?: string; scriptKey?: string },
  ): CodexPlusExtensionContext;

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
}

interface CodexPlusExtensionDefinition {
  id?: string;
  apply(ctx: CodexPlusExtensionContext): void | Promise<void>;
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

  /** 立即执行 fn；它返回的函数在卸载时调用。返回值可提前撤销。 */
  effect(fn: () => void | (() => void)): CodexPlusDispose;

  registerRowAction(definition: CodexPlusRowActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerNavEntry(definition: CodexPlusNavEntryDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  /** 定义形态同 codexPlus.registerMenuItem：开关给 onChange，按钮给 onActivate。 */
  registerMenuItem(definition: Record<string, unknown>, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerPage(definition: CodexPlusPageDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerComposerAction(definition: CodexPlusComposerActionDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;
  registerCommand(definition: CodexPlusCommandDefinition, options?: CodexPlusRegisterOptions): CodexPlusDispose;

  /** 拓展间事件；`codex/*` 是 Codex 会话事件（见 CodexPlusCodexEvent）。内核广播 `extension/apply`、`extension/dispose`，参数为 `{ id, scriptKey }`。 */
  on(event: string, handler: (...args: any[]) => void): CodexPlusDispose;
  emit(event: string, ...args: unknown[]): void;

  /** 往名为 name 的中间件链挂一个中间件，order 小的先执行。 */
  intercept<T = unknown>(name: string, handler: CodexPlusInterceptor<T>, options?: { order?: number }): CodexPlusDispose;
  /** 跑一条中间件链；没有中间件时原样返回 value。 */
  waterfall<T = unknown>(name: string, value: T): T | Promise<T>;

  listen(target: EventTarget, type: string, handler: EventListenerOrEventListenerObject, options?: boolean | AddEventListenerOptions): CodexPlusDispose;
  setTimeout(handler: (...args: any[]) => void, delay?: number, ...args: unknown[]): CodexPlusDispose;
  setInterval(handler: (...args: any[]) => void, delay?: number, ...args: unknown[]): CodexPlusDispose;

  /** 拓展包只能调 codexpp.json `permissions.routes` 里声明的路由。 */
  call<T = unknown>(route: CodexPlusExtensionRoute, payload?: Record<string, unknown>, options?: CodexPlusCallOptions): Promise<T>;
  toast(message: string, options?: CodexPlusToastOptions): { ok: boolean; value?: CodexPlusDispose };
  fail(error: unknown): void;
  /** 主动卸载。重复调用无效果。 */
  dispose(): void;
}

interface Window {
  codexPlus: CodexPlusExtensionApi;
}
