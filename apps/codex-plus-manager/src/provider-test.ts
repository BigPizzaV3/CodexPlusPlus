/** 供应商池的「测试」：后端结果的类型、耗时格式和按失败分类给出的排查提示。
 *
 * 后端只回标识、名称和末 4 位来说明「这次用的是哪个 Key」，Key 原文不进界面。
 * 这里不直接翻译（便于 node 测试）：提示以「中文原文 + 参数」返回，界面用 `t` / `tf` 显示。 */

/** 一条待翻译的提示：中文原文（带 `{0}` 占位时用 `tf`）和参数。 */
export type ProbeHint = { text: string; args: string[] };

const hint = (text: string, args: string[] = []): ProbeHint => ({ text, args });

/** `test_gateway_provider` 的结果（与 Rust 的 `ProbeResult` 一一对应）。 */
export type ProviderProbeResult = {
  ok: boolean;
  /** 失败的大类：auth / not_found / rate_limited / quota / timeout / connect / server / client / stream / other；成功时为空。 */
  category: string;
  /** 上游的 HTTP 状态码；没收到响应时为空。 */
  httpStatus: number | null;
  model: string;
  /** 实际和上游说的协议：responses / chat / anthropic。 */
  wireApi: string;
  firstTokenMs: number | null;
  totalMs: number;
  reply: string;
  replyTruncated: boolean;
  error: string;
  keyId: string;
  keyName: string;
  keyTail: string;
};

/** 毫秒数显示：1 秒以内按毫秒，以上按一位小数的秒。 */
export function formatProbeMs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !Number.isFinite(ms)) return "—";
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

export function probeWireLabel(wire: string): string {
  if (wire === "chat") return "Chat Completions";
  if (wire === "anthropic") return "Anthropic Messages";
  return "Responses";
}

/** 这次用的是哪个 Key（名称 + 末 4 位）；不用 Key 的供应商、Key 太短时为空。 */
export function probeKeyHint(result: Pick<ProviderProbeResult, "keyName" | "keyTail">): ProbeHint | null {
  if (!result.keyTail) return null;
  return result.keyName
    ? hint("本次用的是 Key「{0}」（末 4 位 {1}）。", [result.keyName, result.keyTail])
    : hint("本次用的是末 4 位为 {0} 的 Key。", [result.keyTail]);
}

/** 失败时的排查提示，按后端给的分类；认证失败时附上这次用的 Key。成功返回空。 */
export function probeHints(result: ProviderProbeResult, account = false): ProbeHint[] {
  if (result.ok) return [];
  switch (result.category) {
    case "auth": {
      const hints = [account
        ? hint("登录凭据无效或已过期：请在详情底部点「重新登录」。")
        : hint("Key 无效、已过期，或没有这个模型的权限：核对 Key 是否填对，并在供应商后台确认它已启用。")];
      const key = probeKeyHint(result);
      if (key) hints.push(key);
      return hints;
    }
    case "not_found":
      return [hint("模型名不对，或上游没有这个模型：用「从上游获取」刷新模型列表，或换一个模型再试。")];
    case "rate_limited":
      return [hint("被上游限速，或额度已用完：稍等一会儿再试，或到供应商后台查看用量与余额。")];
    case "quota":
      return [hint("额度或余额已用完：充值、等额度重置，或换一个 Key。")];
    case "timeout":
      return [hint("等待超时：检查 Base URL 和代理是否正确可用；上游过忙时也会这样。")];
    case "connect":
      return [hint("连不上上游：检查 Base URL 和代理设置（这家供应商的代理或系统代理），确认网络能访问这个地址。")];
    case "server":
      return [hint("上游服务出错：多半是上游临时故障，稍后再试。")];
    case "stream":
      return [hint("上游开始回复后又报错或断开：见下方的错误原文。")];
    default:
      return [hint("请求没有成功：见下方的错误原文。")];
  }
}
