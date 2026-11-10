// claude-local 的 protobuf / Connect 协议层。
//
// schema 来自客户端内置 FileDescriptorProto（vendor/rpc-descriptor.js、
// vendor/c30ae4883-BX2Padv3.js 里录下来的 JSON），这里只实现用到的字段，
// 不引入 protobuf 运行时：编码走「字段号 + wire type」手写，解码走通用 wire 解析。
//
// wire type：0=varint，1=64bit，2=length-delimited，5=32bit。

// ── 编码 ─────────────────────────────────────────────────────

const utf8 = (s) => Buffer.from(String(s), "utf8");

function varintBytes(n) {
  // n: number | bigint（负数先转无符号 64 位）
  let v = typeof n === "bigint" ? n : BigInt(n);
  if (v < 0n) v = BigInt.asUintN(64, v);
  const out = [];
  while (v > 127n) {
    out.push(Number(v & 0x7fn) | 0x80);
    v >>= 7n;
  }
  out.push(Number(v));
  return Buffer.from(out);
}

function tag(field, wire) {
  return varintBytes((field << 3) | wire);
}

/** length-delimited 字段（string / bytes / 嵌套消息统一走这里） */
export function ld(field, payload) {
  const p = Buffer.isBuffer(payload) ? payload : utf8(payload);
  return Buffer.concat([tag(field, 2), varintBytes(p.length), p]);
}

/** varint 字段（int32/enum/bool；bool 传 true/false 或 0/1） */
export function vi(field, n) {
  return Buffer.concat([tag(field, 0), varintBytes(n === true ? 1 : n === false ? 0 : n)]);
}

/** int64 字段（秒级时间戳等，用 BigInt 或 number） */
export function i64(field, n) {
  return Buffer.concat([tag(field, 0), varintBytes(n)]);
}

// ── 解码 ─────────────────────────────────────────────────────

/**
 * 把一段 protobuf 字节解成 [{no, wire, val}]。
 * val: wire0 → BigInt；wire1 → 8 字节 Buffer；wire2 → Buffer；wire5 → 4 字节 Buffer。
 * 解析失败的字段跳过，不抛错（前向兼容：客户端可能带我们不认识的新字段）。
 */
export function decode(buf) {
  const fields = [];
  let i = 0;
  const rdVarint = () => {
    let v = 0n, s = 0n;
    for (;;) {
      if (i >= buf.length) throw new Error("eof");
      const b = buf[i++];
      v |= BigInt(b & 0x7f) << s;
      if (!(b & 0x80)) return v;
      s += 7n;
      if (s > 70n) throw new Error("varint too long");
    }
  };
  while (i < buf.length) {
    let key;
    try { key = rdVarint(); } catch { break; }
    const no = Number(key >> 3n), wire = Number(key & 7n);
    if (no === 0) break;
    try {
      if (wire === 0) fields.push({ no, wire, val: rdVarint() });
      else if (wire === 1) { fields.push({ no, wire, val: buf.subarray(i, i + 8) }); i += 8; }
      else if (wire === 2) {
        const len = Number(rdVarint());
        fields.push({ no, wire, val: buf.subarray(i, i + len) });
        i += len;
      } else if (wire === 5) { fields.push({ no, wire, val: buf.subarray(i, i + 4) }); i += 4; }
      else break; // wire 3/4（group）本协议用不到，遇到就停
    } catch { break; }
  }
  return fields;
}

/** 取第一个匹配字段；没有返回 undefined */
export function get(fields, no, wire) {
  return fields.find((f) => f.no === no && (wire === undefined || f.wire === wire));
}

/** 取所有匹配字段 */
export function all(fields, no) {
  return fields.filter((f) => f.no === no);
}

export const asStr = (f) => (f && f.wire === 2 ? f.val.toString("utf8") : undefined);
export const asMsg = (f) => (f && f.wire === 2 ? decode(f.val) : []);
export const asBool = (f) => (f ? f.val !== 0n : undefined);
export const asInt = (f) => (f ? Number(f.val) : undefined);
export const asBig = (f) => (f ? f.val : undefined);

// ── Connect 封装 ─────────────────────────────────────────────
// streaming：每帧 [flag:1B][len:4B 大端][payload]；flag 0x02 = 结束帧，
// 其 payload 是 JSON trailers（通常 {}）。unary：body 就是裸消息。

import zlib from "node:zlib";

/** 从 connect+proto 请求体里取出所有消息帧的 payload（跳过结束帧） */
export function requestMessages(buf, contentEncoding = "") {
  const out = [];
  const enc = contentEncoding.toLowerCase();
  const inflate = (payload) => {
    if (enc === "gzip") return zlib.gunzipSync(payload);
    if (enc === "deflate") return zlib.inflateSync(payload);
    return payload;
  };
  let i = 0;
  while (i + 5 <= buf.length) {
    const flag = buf[i];
    const len = buf.readUInt32BE(i + 1);
    let payload = buf.subarray(i + 5, i + 5 + len);
    i += 5 + len;
    if (flag & 0x02) break; // 结束帧
    if (flag & 0x01) {
      try { payload = inflate(payload); } catch { continue; }
    }
    out.push(payload);
  }
  return out;
}

/** 一个 connect 消息帧 */
export function frame(payload) {
  const head = Buffer.alloc(5);
  head[0] = 0;
  head.writeUInt32BE(payload.length, 1);
  return Buffer.concat([head, payload]);
}

/** connect 结束帧：flag=0x02 + JSON trailers */
export function endFrame(trailers = {}) {
  const payload = Buffer.from(JSON.stringify(trailers), "utf8");
  const head = Buffer.alloc(5);
  head[0] = 2;
  head.writeUInt32BE(payload.length, 1);
  return Buffer.concat([head, payload]);
}

// ── 消息构造（bard.api.v1alpha） ──────────────────────────────
// 只实现用到的字段，字段号与 streaming.proto / conversation.proto /
// content.proto / bard_api.proto 的 descriptor 一致。

export const ts = (d = new Date()) =>
  Buffer.concat([i64(1, BigInt(Math.floor(d.getTime() / 1000))), vi(2, d.getMilliseconds() * 1e6)]);

/** Conversation{id,title,status,current_leaf_message_id,created_at,updated_at} */
export function conversation({ id, title, status = 1, leaf, created, updated, model }) {
  return Buffer.concat([
    ld(1, id),
    title ? ld(2, title) : Buffer.alloc(0),
    status ? vi(3, status) : Buffer.alloc(0),
    created ? ld(4, ts(created)) : Buffer.alloc(0),
    updated ? ld(5, ts(updated)) : Buffer.alloc(0),
    model ? ld(6, ld(2, model)) : Buffer.alloc(0),
    leaf ? ld(7, leaf) : Buffer.alloc(0),
  ]);
}

const ROLE = { user: 1, assistant: 2, system: 3 };
const STOP = { endTurn: 1, maxTokens: 2, toolUse: 3, refusal: 4, userCanceled: 10, error: 11 };

/** Message{id,conversation_id,role,index,is_complete,stop_reason,parent_message_id,created_at} */
export function message({ id, convId, role, index, complete = true, stopReason, parent, created }) {
  return Buffer.concat([
    ld(1, id),
    convId ? ld(2, convId) : Buffer.alloc(0),
    vi(3, ROLE[role] ?? role),
    vi(4, index ?? 0),
    complete ? vi(6, 1) : Buffer.alloc(0),
    stopReason ? vi(7, STOP[stopReason] ?? stopReason) : Buffer.alloc(0),
    created ? ld(9, ts(created)) : Buffer.alloc(0),
    parent ? ld(11, parent) : Buffer.alloc(0),
  ]);
}

const GROUP_STYLE = { inline: 1, timeline: 2 };

/** DisplayGroup{id,message_id,index,style,is_complete} */
export function displayGroup({ id, messageId, index = 0, style = "inline", complete = true }) {
  return Buffer.concat([
    ld(1, id),
    ld(2, messageId),
    vi(3, index),
    vi(4, GROUP_STYLE[style] ?? style),
    complete ? vi(6, 1) : Buffer.alloc(0),
  ]);
}

const BLOCK_STATE = { running: 1, complete: 2, error: 3, pendingApproval: 4, cancelled: 5 };

/** ContentBlock{id=1,display_group_id=2,index=3,is_complete=4,state=8,text=9,text_format=10} */
export function contentBlock({ id, groupId, index = 0, complete = true, state, text = "", markdown = true }) {
  return Buffer.concat([
    ld(1, id),
    ld(2, groupId),
    vi(3, index),
    complete ? vi(4, 1) : Buffer.alloc(0),
    state ? vi(8, BLOCK_STATE[state] ?? state) : Buffer.alloc(0),
    text ? ld(9, text) : Buffer.alloc(0),
    markdown ? ld(10, vi(1, 2)) : Buffer.alloc(0),
  ]);
}

/** TurnDisposition{trigger_message_id,state} state: RUNNING=1 FINISHED=2 */
export const turnDisposition = (triggerId, state) =>
  Buffer.concat([ld(1, triggerId), vi(2, state)]);

/**
 * ConversationUpdate{conversation=2, messages=3, display_groups=4,
 * content_blocks=5, turn_disposition=31}
 */
export function conversationUpdate({ conv, messages = [], groups = [], blocks = [], turn, replaceAll = false }) {
  const parts = [];
  if (replaceAll) parts.push(vi(1, 1));
  if (conv) parts.push(ld(2, conv));
  for (const m of messages) parts.push(ld(3, m));
  for (const g of groups) parts.push(ld(4, g));
  for (const b of blocks) parts.push(ld(5, b));
  if (turn) parts.push(ld(31, turnDisposition(turn.trigger, turn.state)));
  return Buffer.concat(parts);
}

/** ContentBlockAppend{content_block_id=1, text_delta=2} */
export const contentBlockAppend = (blockId, delta) =>
  Buffer.concat([ld(1, blockId), ld(2, delta)]);

const STREAM_EVENT = { update: 1, append: 2, error: 3, messageLimit: 4, mutationAck: 5, heartbeat: 6 };

/** StreamError{type=1,message=2,retryable=4} */
export const streamError = (type, message, retryable = true) =>
  Buffer.concat([ld(1, type), ld(2, message), vi(4, retryable ? 1 : 0)]);

/**
 * StreamEvent —— oneof "event"：update=1 / append=2 / error=3 / heartbeat=6。
 * version=10（Version{value=1}）可选，这里不带。
 */
export function streamEvent({ update, append, error, heartbeat }) {
  if (update) return ld(STREAM_EVENT.update, update);
  if (append) return ld(STREAM_EVENT.append, append);
  if (error) return ld(STREAM_EVENT.error, error);
  if (heartbeat !== undefined) return ld(STREAM_EVENT.heartbeat, vi(1, heartbeat));
  return Buffer.alloc(0);
}

/** StreamTimelineResponse{event=1: StreamEvent} */
export const streamTimelineResponse = (event) => ld(1, event);

/** MutationId{session_id=1,version=2 i64} */
export const mutationId = (sessionId, version) =>
  Buffer.concat([sessionId ? ld(1, sessionId) : Buffer.alloc(0), version !== undefined ? i64(2, version) : Buffer.alloc(0)]);

/** MutationAck{high_water_mark=1, applied=2(oneof), error=3} */
export function mutationAck({ sessionId, version, applied = true }) {
  return Buffer.concat([
    ld(1, mutationId(sessionId, version)),
    applied ? vi(2, 1) : Buffer.alloc(0),
  ]);
}

/** PerformActionResponse{ack=1} */
export const performActionResponse = (ack) => (ack ? ld(1, ack) : Buffer.alloc(0));

/** ReadConversationResponse{outcome=1, update=2} outcome: SERVED=1 */
export const readConversationResponse = (update) =>
  Buffer.concat([vi(1, 1), update ? ld(2, update) : Buffer.alloc(0)]);

/** ReadConversationHistoryResponse{update=1, outcome=3: PAGE=1} */
export const readHistoryResponse = (update) =>
  Buffer.concat([update ? ld(1, update) : Buffer.alloc(0), vi(3, 1)]);

/** ReportViewingResponse{refresh_after_seconds=1} */
export const reportViewingResponse = () => vi(1, 0);

/**
 * GetNewConversationDefaultsResponse{regular=1, temporary=2}。
 * NewConversationDefaults{settings=5 -> ConversationSettings{...}}。
 * 字段全是 optional，给最小可用集。
 */
export function newConversationDefaultsResponse() {
  const settings = Buffer.concat([
    // ConversationSettings: permission_mode=15(i32), effort_level_token=17,
    // thinking_mode_token=16 —— 全空也能跑，客户端有自己的默认。
    Buffer.alloc(0),
  ]);
  const ncd = ld(5, settings); // NewConversationDefaults{settings}
  return Buffer.concat([ld(1, ncd), ld(2, ncd)]);
}

/** ListConversationsResponse{conversations=1 repeated Conversation, older_cursor=2} */
export const listConversationsResponse = (convs) =>
  Buffer.concat(convs.map((c) => ld(1, c)));

/** StreamConversationListResponse{update=1 -> ConversationListUpdate} */
export function conversationListUpdate(convs, { replaceHead = true } = {}) {
  const parts = [replaceHead ? vi(1, 1) : Buffer.alloc(0)];
  for (const c of convs) parts.push(ld(2, c));
  return ld(1, Buffer.concat(parts));
}

/** SendAndSettleResponse{update=1, version=2, settled=3, ack=4} */
export function sendAndSettleResponse({ update, version, settled = true, ack }) {
  return Buffer.concat([
    update ? ld(1, update) : Buffer.alloc(0),
    version ? ld(2, ld(1, version)) : Buffer.alloc(0),
    vi(3, settled ? 1 : 0),
    ack ? ld(4, ack) : Buffer.alloc(0),
  ]);
}
