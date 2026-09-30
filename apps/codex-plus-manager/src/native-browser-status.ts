export type BrowserConnection = {
  state: string;
  browsers: { family: string; headerEnabled: boolean | null }[];
  failedChecks: number;
};

export type NativeBrowserDiagnostics = {
  compatibility: { state: string; detail: string };
  connection: BrowserConnection;
};

export function browserConnectionLabel(connection: BrowserConnection): string {
  switch (connection.state) {
    case "available": {
      const names = [...new Set(connection.browsers.map(browser =>
        browser.family === "edge" ? "Edge" : browser.family === "chrome" ? "Chrome" : ""))].filter(Boolean);
      return names.length ? `${names.join(" / ")} 浏览器可用` : "浏览器状态检测失败";
    }
    case "disconnected": return "浏览器未连接";
    case "checking": return "正在检测浏览器连接";
    case "unsupported": return "此平台不支持浏览器连接检测";
    default: return "浏览器状态检测失败";
  }
}

export function browserHeaderLabel(enabled: boolean | null): string {
  return enabled === true ? "请求标识：已开启（扩展报告）"
    : enabled === false ? "请求标识：已关闭（扩展报告）"
    : "请求标识：扩展未提供状态";
}

export function nativeBrowserStatusLabel(state: string): string {
  switch (state) {
    case "disabled": return "兼容选项未启用";
    case "not_started": return "等待下次启动 Codex++；保存设置不会自动重启";
    case "waiting_for_runtime": return "等待原生插件生成运行时";
    case "prepared": return "兼容服务文件已准备，等待原生工作进程加载";
    case "restored": return "服务文件已恢复；扩展保留的请求标识不受此操作影响";
    case "runtime_unverified": return "未应用旧版兼容补丁：当前运行时版本不匹配";
    case "blocked": return "兼容补丁文件检查或恢复受阻，已保留文件";
    case "unsupported": return "此平台未提供 Edge / Chrome 请求标识兼容补丁";
    case "stale": return "状态已过期；需启动本版本 Codex++ 后重新检查";
    default: return "无法读取兼容补丁状态";
  }
}
