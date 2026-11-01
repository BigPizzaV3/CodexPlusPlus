/**
 * 拓展的管理器面板（codexpp.json 的 `manager.page`）：拓展自己的 HTML 放进沙箱 iframe（只给 allow-scripts，
 * 没有同源、弹窗、顶层跳转），页面里拿不到 Tauri 接口，也不能联网（下面的 CSP）；只能经 postMessage 调
 * 宿主开放的几样：本拓展的配置、本拓展的常驻后台、本拓展 assets/ 里的资源、提示条、主题。
 *
 * 消息格式：页面 → 宿主 `{ __codexPlusManager: 1, id, method, params }`，宿主 → 页面
 * `{ __codexPlusManager: 1, id, ok, value | error }`；宿主主动推的事件 `{ __codexPlusManager: 1, event, value }`。
 */

export type ManagerTheme = "dark" | "light";

export type ManagerBridgeDeps = {
  config(): Record<string, unknown>;
  saveConfig(values: Record<string, unknown>): Promise<boolean>;
  backendCall(method: string, params: unknown): Promise<unknown>;
  asset(path: string): Promise<string>;
  toast(message: string): void;
  resize(height: number): void;
  theme(): ManagerTheme;
};

export type ManagerRequest = { __codexPlusManager: 1; id: number; method: string; params?: unknown };
export type ManagerReply = { __codexPlusManager: 1; id: number; ok: boolean; value?: unknown; error?: string };

export const MANAGER_PANEL_MIN_HEIGHT = 80;
export const MANAGER_PANEL_MAX_HEIGHT = 2000;

export function isManagerRequest(data: unknown): data is ManagerRequest {
  const message = data as Partial<ManagerRequest> | null;
  return !!message && message.__codexPlusManager === 1 && typeof message.id === "number" && typeof message.method === "string";
}

const record = (value: unknown): Record<string, unknown> =>
  value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {};

/** 处理一条页面请求，返回给页面的回复。未知方法、参数不对都回 ok: false，不抛错。 */
export async function handleManagerRequest(request: ManagerRequest, deps: ManagerBridgeDeps): Promise<ManagerReply> {
  const reply = (value: unknown): ManagerReply => ({ __codexPlusManager: 1, id: request.id, ok: true, value });
  const fail = (error: unknown): ManagerReply => ({
    __codexPlusManager: 1, id: request.id, ok: false, error: error instanceof Error ? error.message : String(error),
  });
  const params = record(request.params);
  try {
    switch (request.method) {
      case "config.get":
        return reply(deps.config());
      case "config.set": {
        const values = record(params.values);
        if (!(await deps.saveConfig(values))) return fail("配置没有保存（类型不符或拓展未声明该配置项）");
        return reply(deps.config());
      }
      case "backend.call": {
        const method = typeof params.method === "string" ? params.method.trim() : "";
        if (!method) return fail("backend.call 需要方法名");
        return reply(await deps.backendCall(method, params.params ?? null));
      }
      case "asset": {
        const path = typeof params.path === "string" ? params.path : "";
        if (!path) return fail("asset 需要包内路径");
        return reply(await deps.asset(path));
      }
      case "toast":
        deps.toast(String(params.message ?? "").slice(0, 300));
        return reply(null);
      case "resize": {
        const height = Number(params.height);
        if (!Number.isFinite(height)) return fail("resize 需要高度");
        deps.resize(Math.min(MANAGER_PANEL_MAX_HEIGHT, Math.max(MANAGER_PANEL_MIN_HEIGHT, Math.ceil(height))));
        return reply(null);
      }
      case "theme":
        return reply(deps.theme());
      default:
        return fail(`不支持的方法：${request.method}`);
    }
  } catch (error) {
    return fail(error);
  }
}

/** 面板页里禁止联网与加载外部资源：资源经 asset() 取 data URL，网络请求交给拓展自己的常驻后台。 */
export const MANAGER_PANEL_CSP = [
  "default-src 'none'",
  "script-src 'unsafe-inline'",
  "style-src 'unsafe-inline'",
  "img-src data: blob:",
  "font-src data:",
  "media-src data: blob:",
].join("; ");

/** 注入到面板页最前面的引导脚本：定义 window.codexPlusManager，并在内容高度变化时通知宿主。 */
export const MANAGER_PANEL_BOOTSTRAP = `(() => {
  let seq = 0;
  const pending = new Map();
  const listeners = { theme: new Set(), config: new Set() };
  const request = (method, params) => new Promise((resolve, reject) => {
    const id = ++seq;
    pending.set(id, { resolve, reject });
    parent.postMessage({ __codexPlusManager: 1, id, method, params }, "*");
  });
  addEventListener("message", (event) => {
    const data = event.data;
    if (event.source !== parent || !data || data.__codexPlusManager !== 1) return;
    if (data.event) {
      if (data.event === "theme") document.documentElement.dataset.theme = data.value;
      (listeners[data.event] || new Set()).forEach((handler) => { try { handler(data.value); } catch (error) { console.error(error); } });
      return;
    }
    const entry = pending.get(data.id);
    if (!entry) return;
    pending.delete(data.id);
    if (data.ok) entry.resolve(data.value); else entry.reject(new Error(data.error || "调用失败"));
  });
  const on = (name) => (handler) => { listeners[name].add(handler); return () => listeners[name].delete(handler); };
  window.codexPlusManager = Object.freeze({
    config: Object.freeze({ get: () => request("config.get"), set: (values) => request("config.set", { values }), onChange: on("config") }),
    backend: Object.freeze({ call: (method, params) => request("backend.call", { method, params }) }),
    asset: (path) => request("asset", { path }),
    toast: (message) => request("toast", { message }),
    theme: () => request("theme"),
    onTheme: on("theme"),
  });
  const report = () => request("resize", { height: document.documentElement.scrollHeight }).catch(() => {});
  addEventListener("load", report);
  if (typeof ResizeObserver === "function") {
    const observer = new ResizeObserver(report);
    const watch = () => document.body ? observer.observe(document.body) : setTimeout(watch, 0);
    watch();
  }
})();`;

/** 把拓展的 HTML 包成 iframe 的 srcdoc：CSP 与引导脚本放在 head 最前面，页面自己的 CSP 只能更严。 */
export function managerPanelSrcdoc(html: string, theme: ManagerTheme): string {
  const head = `<meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="${MANAGER_PANEL_CSP}">`
    + `<script>${MANAGER_PANEL_BOOTSTRAP}</script><script>document.documentElement.dataset.theme=${JSON.stringify(theme)}</script>`;
  if (/<head[^>]*>/i.test(html)) return html.replace(/<head[^>]*>/i, (tag) => `${tag}${head}`);
  if (/<html[^>]*>/i.test(html)) return html.replace(/<html[^>]*>/i, (tag) => `${tag}<head>${head}</head>`);
  return `<!doctype html><html><head>${head}</head><body>${html}</body></html>`;
}
