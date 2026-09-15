export function nativeBrowserStatusLabel(state: string): string {
  switch (state) {
    case "disabled": return "兼容选项未启用";
    case "not_started": return "等待下次启动 Codex++；保存设置不会自动重启";
    case "waiting_for_runtime": return "等待原生插件生成运行时";
    case "prepared": return "服务已准备，等待新原生工作进程加载；尚未验收浏览器功能";
    case "restored": return "服务文件已恢复；扩展保留的请求标识不受此操作影响";
    case "blocked": return "运行时不支持、选择不唯一或存在文件冲突；未启用兼容";
    case "unsupported": return "此平台不支持原生 Edge / Chrome 兼容";
    case "stale": return "状态已过期；需启动本版本 Codex++ 后重新检查";
    default: return "无法读取兼容状态；不代表浏览器已可用";
  }
}
