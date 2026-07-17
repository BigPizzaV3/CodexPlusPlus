/// VLM 测试结果 -> 通俗文案映射。tr 为翻译回调（zh + 可选插值参数），
/// 由调用方注入（App 传 t/tf，测试传 identity），保持本模块纯函数无 @/ 依赖。
export type TranslateFn = (zh: string, params?: string[]) => string;

export function vlmTestTranslation(
  status: string,
  httpCode: number | undefined,
  durationMs: number,
  tr: TranslateFn,
): string {
  const secs = (durationMs / 1000).toFixed(1);
  switch (status) {
    case "ok":
      return tr("\u2705 识别成功（耗时 {0}s）", [secs]);
    case "http_error":
      if (httpCode === 401) return tr("\u274C 认证失败：API Key 可能不正确（HTTP 401）");
      if (httpCode === 404) return tr("\u274C 接口不存在：Base URL 或模型名有误（HTTP 404）");
      if (httpCode === 429) return tr("\u274C 被限流，稍后再试（HTTP 429）");
      return tr("\u274C 服务返回错误（HTTP {0}）", [String(httpCode ?? "?")]);
    case "timeout":
      return tr("\u274C 请求超时：VLM 响应过慢或网络不通");
    case "send_error":
      return tr("\u274C 发送失败：网络错误，检查 Base URL 是否可达");
    case "json_error":
      return tr("\u274C 返回内容解析失败：上游返回的不是有效 JSON");
    case "no_text":
      return tr("\u274C 返回中未找到描述文本：模型可能不支持视觉或返回格式异常");
    case "parse_error":
      return tr("\u274C 批量描述解析失败（单图测试不应触发）");
    case "client_error":
      return tr("\u274C HTTP 客户端构建失败");
    default:
      return tr("\u274C 未知错误");
  }
}
