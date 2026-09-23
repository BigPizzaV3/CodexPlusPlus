/**
 * @description 供应商自定义请求头的校验与规范化（issue #1685），规则与 Rust 端 relay_headers 保持一致
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-09-23
 */

export type RelayHeaderRow = { key: string; value: string };

/** 传输层 / 逐跳头：由协议层按实际报文决定，用户不能覆盖。 */
export const FORBIDDEN_RELAY_HEADERS = [
  "host",
  "content-length",
  "transfer-encoding",
  "connection",
  "keep-alive",
  "proxy-connection",
  "te",
  "trailer",
  "upgrade",
  "expect",
];

export function isForbiddenRelayHeader(name: string): boolean {
  return FORBIDDEN_RELAY_HEADERS.includes(name.trim().toLowerCase());
}

/** 丢掉未填写的空行并压缩首尾空白。 */
export function normalizeRelayHeaders(rows: RelayHeaderRow[]): RelayHeaderRow[] {
  return rows
    .map((row) => ({ key: row.key.trim(), value: row.value.trim() }))
    .filter((row) => row.key.length > 0 || row.value.length > 0);
}

/** token 是否合法（RFC 7230 的 token 字符集）。 */
function isToken(name: string): boolean {
  return /^[!#$%&'*+\-.^_`|~0-9A-Za-z]+$/.test(name);
}

/**
 * 与 Rust 端同规则校验，返回中文提示；全部合法时返回 null。
 * 报错只回显头名称，不回显可能含凭据的值。
 */
export function relayHeadersValidationMessage(rows: RelayHeaderRow[]): string | null {
  const seen = new Set<string>();
  for (const row of normalizeRelayHeaders(rows)) {
    const name = row.key;
    if (!name) {
      return "自定义请求头存在空的名称";
    }
    if (!isToken(name)) {
      return `自定义请求头「${name}」不是合法的 HTTP 头名称`;
    }
    const lowered = name.toLowerCase();
    if (FORBIDDEN_RELAY_HEADERS.includes(lowered)) {
      return `自定义请求头「${name}」由协议层掌控，不允许覆盖（Host、Content-Length 等传输头）`;
    }
    if (seen.has(lowered)) {
      return `自定义请求头「${name}」重复配置`;
    }
    seen.add(lowered);
    if (/[\r\n]/.test(row.value)) {
      return `自定义请求头「${name}」的值不能包含换行`;
    }
  }
  return null;
}

/** 保存前使用：规范化后的列表（去掉空行、压缩空白）。 */
export function serializeRelayHeaders(rows: RelayHeaderRow[]): RelayHeaderRow[] {
  return normalizeRelayHeaders(rows);
}

/** 是否配置了显式 Authorization（后端据此决定不再注入 API Key）。 */
export function hasCustomAuthorization(rows: RelayHeaderRow[]): boolean {
  return normalizeRelayHeaders(rows).some(
    (row) => row.key.toLowerCase() === "authorization" && row.value.length > 0,
  );
}
