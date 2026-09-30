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
  | "/user-scripts/list";

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
  /** 当前脚本的标识，形如 `user:my-script.js`。 */
  readonly script: { readonly key: string };

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

interface Window {
  codexPlus: CodexPlusExtensionApi;
}
