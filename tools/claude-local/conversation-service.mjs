// ConversationService 的本地实现：StreamTimeline 长连接 + PerformAction + 其余方法兜底。
//
// 协议：Connect RPC。
//   - server-streaming（StreamTimeline / StreamConversationList / StreamPromptComparison）
//     → application/connect+proto，响应是 [flag][len][protobuf] 帧序列，flag 0x02 结束帧。
//   - unary（其余）→ application/proto，响应是裸 protobuf 消息。
//
// 会话状态由 proxy.mjs 提供；受管运行时按变更保存，完成一轮后立即落盘。

import crypto from "node:crypto";
import http from "node:http";
import https from "node:https";
import { gatewayModel, gatewayTarget, runtimeConfig } from "./runtime.mjs";
import {
  decode, get, asStr, asMsg, asInt, ld, vi,
  requestMessages, frame, endFrame,
  conversation, message, displayGroup, contentBlock,
  conversationUpdate, contentBlockAppend, streamEvent,
  streamTimelineResponse, streamError, mutationAck, performActionResponse,
  readConversationResponse, readHistoryResponse, reportViewingResponse,
  newConversationDefaultsResponse, listConversationsResponse,
  sendAndSettleResponse,
} from "./rpc.mjs";

const runtime = runtimeConfig();

/**
 * 调网关拿一轮回复的文本增量。
 * 返回 {next} 迭代器：{done:false,value:string} / {done:true}，出错 throw。
 */
function anthropicStream(messages, model, signal) {
  const queue = [];
  let notify = null;
  let done = false, failed = null;
  const push = (v) => { queue.push(v); const n = notify; notify = null; n?.(); };
  const payload = JSON.stringify({
    model: gatewayModel(runtime, model), max_tokens: 4096, stream: true, messages,
  });
  const u = gatewayTarget(runtime, "/v1/messages");
  const lib = u.protocol === "https:" ? https : http;
  const req = lib.request(
    {
      hostname: u.hostname, port: u.port || (u.protocol === "https:" ? 443 : 80),
      path: u.pathname + u.search,
      method: "POST", timeout: 300000,
      headers: {
        "content-type": "application/json",
        "x-api-key": runtime.key,
        authorization: `Bearer ${runtime.key}`,
        "anthropic-version": "2023-06-01",
        accept: "text/event-stream",
      },
    },
    (res) => {
      if (res.statusCode >= 400) {
        res.resume();
        res.on("end", () => {
          failed = new Error(`gateway HTTP ${res.statusCode}`);
          done = true; const n = notify; notify = null; n?.();
        });
        return;
      }
      let buf = "";
      res.on("data", (c) => {
        buf += c.toString("utf8");
        let i;
        while ((i = buf.indexOf("\n\n")) >= 0) {
          const ev = buf.slice(0, i); buf = buf.slice(i + 2);
          const line = ev.split("\n").find((l) => l.startsWith("data:"));
          if (!line) continue;
          const data = line.slice(5).trim();
          if (data === "[DONE]") continue;
          let d; try { d = JSON.parse(data); } catch { continue; }
          if (d.type === "content_block_delta" && d.delta?.type === "text_delta") push(d.delta.text);
          else if (d.type === "message_stop") push({ end: true });
          else if (d.type === "error") failed = new Error("gateway stream error");
        }
      });
      res.on("end", () => { done = true; const n = notify; notify = null; n?.(); });
      res.on("error", (e) => { failed = e; done = true; const n = notify; notify = null; n?.(); });
    },
  );
  req.on("timeout", () => req.destroy(new Error("gateway timeout")));
  req.on("error", (e) => { failed = e; done = true; const n = notify; notify = null; n?.(); });
  if (signal) signal.addEventListener("abort", () => { try { req.destroy(); } catch {} });
  req.end(payload);
  return {
    next: async () => {
      for (;;) {
        const v = queue.shift();
        if (v !== undefined) {
          if (v && typeof v === "object" && v.end) return { done: true };
          return { done: false, value: v };
        }
        if (done) {
          if (failed) throw failed;
          return { done: true };
        }
        await new Promise((r) => { notify = r; });
      }
    },
  };
}

async function* gatewayDeltas(messages, model, signal) {
  const it = anthropicStream(messages, model, signal);
  for (;;) {
    const r = await it.next();
    if (r.done) return;
    yield r.value;
  }
}

// ── 服务 ─────────────────────────────────────────────────────
// deps: ensureConv(id) -> conv；listConvs() -> conv[]；log(...)
// conv 约定字段：uuid/name/created_at/updated_at/turns[]/nextIndex/leafMessageId
export function createConversationService({ ensureConv, listConvs, log, onChange = () => {} }) {
  /** convId -> {res, convId, frames, closed, hb} */
  const streams = new Map();

  const writeEvent = (stream, event) => {
    if (stream.closed || stream.res.writableEnded) return;
    stream.frames++;
    try { stream.res.write(frame(streamTimelineResponse(event))); } catch {}
  };

  const endStream = (stream, trailers = {}) => {
    if (stream.closed) return;
    stream.closed = true;
    clearInterval(stream.hb);
    try { stream.res.write(endFrame(trailers)); stream.res.end(); } catch {}
    if (streams.get(stream.convId) === stream) streams.delete(stream.convId);
  };

  // ── 把 conv 的全部已知实体打成一个 ConversationUpdate ──
  // turn: {humanId,humanIndex,humanParent,prompt,asstId,asstIndex,
  //        groupId,blockId,text,complete,stopReason,created,replied,replyStarted}
  function buildStateUpdate(conv, { includeConversation = true } = {}) {
    const messages = [], groups = [], blocks = [];
    let leaf;
    for (const t of conv.turns) {
      messages.push(message({
        id: t.humanId, convId: conv.uuid, role: "user", index: t.humanIndex,
        complete: true, parent: t.humanParent || undefined, created: t.created,
      }));
      // 用户侧 group/block：和客户端乐观层同一套 id 派生（dgrp_/cblk_ 前缀）
      const hug = `dgrp_${t.humanId}`, hub = `cblk_${t.humanId}`;
      groups.push(displayGroup({ id: hug, messageId: t.humanId, index: 0, style: "inline", complete: true }));
      blocks.push(contentBlock({
        id: hub, groupId: hug, index: 0, complete: true, text: t.prompt, markdown: true,
      }));
      const done = t.complete;
      messages.push(message({
        id: t.asstId, convId: conv.uuid, role: "assistant", index: t.asstIndex,
        complete: done,
        stopReason: done ? (t.stopReason || "endTurn") : undefined,
        parent: t.humanId, created: t.created,
      }));
      groups.push(displayGroup({ id: t.groupId, messageId: t.asstId, index: 0, style: "inline", complete: done }));
      blocks.push(contentBlock({
        id: t.blockId, groupId: t.groupId, index: 0, complete: done,
        state: done ? "complete" : "running", text: t.text, markdown: true,
      }));
      leaf = t.asstId;
    }
    const convMsg = includeConversation
      ? conversation({
          id: conv.uuid,
          model: conv.model,
          title: conv.name && conv.name !== "New conversation" ? conv.name : undefined,
          status: conv.turns.some((t) => !t.complete) ? 2 : 1, // RUNNING / IDLE
          leaf,
          created: conv.created_at ? new Date(conv.created_at) : undefined,
          updated: conv.updated_at ? new Date(conv.updated_at) : undefined,
        })
      : undefined;
    const live = conv.turns.find((t) => !t.complete);
    return conversationUpdate({
      conv: convMsg, messages, groups, blocks,
      turn: live ? { trigger: live.humanId, state: 1 } : undefined,
    });
  }

  function convProto(c) {
    return conversation({
      id: c.uuid,
      model: c.model,
      title: c.name && c.name !== "New conversation" ? c.name : undefined,
      status: c.turns?.some((t) => !t.complete) ? 2 : 1,
      leaf: c.leafMessageId || undefined,
      created: c.created_at ? new Date(c.created_at) : undefined,
      updated: c.updated_at ? new Date(c.updated_at) : undefined,
    });
  }

  // ── 跑一轮回复：running 快照 → 逐 delta append → 完成快照 ──
  const replying = new Set();
  const controllers = new Map();
  async function ensureReply(conv) {
    const turn = conv.turns.find((t) => !t.complete && !t.replyStarted);
    if (!turn || replying.has(turn.asstId)) return;
    replying.add(turn.asstId);
    turn.replyStarted = true;
    const controller = new AbortController();
    controllers.set(turn.asstId, controller);
    try {
      const stream = () => streams.get(conv.uuid);
      // running 快照（全量实体，幂等）
      const s0 = stream();
      if (s0) writeEvent(s0, streamEvent({ update: buildStateUpdate(conv) }));
      let failed = null;
      try {
        const messages = [];
        for (const previous of conv.turns) {
          messages.push({ role: "user", content: previous.prompt });
          if (previous === turn) break;
          if (previous.text) messages.push({ role: "assistant", content: previous.text });
        }
        for await (const delta of gatewayDeltas(messages, turn.model, controller.signal)) {
          if (turn.complete) break;
          turn.text += delta;
          onChange();
          const s = stream();
          if (s) writeEvent(s, streamEvent({ append: contentBlockAppend(turn.blockId, delta) }));
        }
      } catch (e) {
        failed = controller.signal.aborted && turn.stopReason === "userCanceled" ? null : e;
      }
      turn.complete = true;
      turn.stopReason = turn.stopReason === "userCanceled" ? turn.stopReason : failed ? "error" : "endTurn";
      if (failed && turn.stopReason !== "userCanceled") turn.text += `\n\n[网关错误：${failed.message}]`;
      conv.updated_at = new Date().toISOString();
      onChange(true);
      // 完成快照：本轮的 message/block 置 complete + stop_reason，conversation 回 IDLE
      const upd = conversationUpdate({
        conv: conversation({
          id: conv.uuid,
          model: conv.model,
          title: conv.name !== "New conversation" ? conv.name : undefined,
          status: 1, leaf: turn.asstId,
          created: conv.created_at ? new Date(conv.created_at) : undefined,
          updated: new Date(conv.updated_at),
        }),
        messages: [message({
          id: turn.asstId, convId: conv.uuid, role: "assistant", index: turn.asstIndex,
          complete: true, stopReason: turn.stopReason, parent: turn.humanId, created: turn.created,
        })],
        groups: [displayGroup({ id: turn.groupId, messageId: turn.asstId, index: 0, style: "inline", complete: true })],
        blocks: [contentBlock({
          id: turn.blockId, groupId: turn.groupId, index: 0, complete: true,
          state: failed ? "error" : "complete", text: turn.text, markdown: true,
        })],
        turn: { trigger: turn.humanId, state: 2 }, // FINISHED
      });
      const s1 = stream();
      if (s1) writeEvent(s1, streamEvent({ update: upd }));
      if (failed) {
        const s2 = stream();
        if (s2) writeEvent(s2, streamEvent({ error: streamError("api_error", failed.message, true) }));
      }
      // 还有排队的 turn 就接着跑
      const next = conv.turns.find((t) => !t.complete && !t.replyStarted);
      if (next) ensureReply(conv).catch((e) => log?.(`reply error: ${e.message}`));
    } finally {
      replying.delete(turn.asstId);
      controllers.delete(turn.asstId);
    }
  }

  const titleFromPrompt = (text) => {
    const t = (text || "").replace(/\s+/g, " ").trim();
    return t.length > 30 ? `${t.slice(0, 30)}…` : t || "New conversation";
  };

  function recordSend(conv, send) {
    // 幂等：客户端重试同一 mutation（同 message_id）时不重复入 turn
    const dup = conv.turns.find((t) => t.humanId === (send.messageId || "") && send.messageId);
    if (dup) {
      log?.(`send dedup conv=${conv.uuid} human=${dup.humanId}`);
      return dup;
    }
    const now = new Date();
    const asstId = send.assistantMessageId || crypto.randomUUID();
    const turn = {
      humanId: send.messageId || crypto.randomUUID(),
      humanIndex: conv.nextIndex,
      humanParent: send.parentMessageId || conv.leafMessageId || undefined,
      prompt: send.text,
      asstId,
      asstIndex: conv.nextIndex + 1,
      groupId: `dgrp_${asstId}`,
      blockId: `cblk_${asstId}`,
      text: "", complete: false, stopReason: null, created: now, replyStarted: false,
      model: gatewayModel(runtime, send.model || conv.model),
    };
    conv.nextIndex += 2;
    conv.turns.push(turn);
    conv.leafMessageId = turn.asstId;
    conv.model = turn.model;
    if (!conv.name || conv.name === "New conversation") conv.name = titleFromPrompt(send.text);
    conv.updated_at = now.toISOString();
    onChange(true);
    log?.("Recorded a local conversation turn");
    return turn;
  }

  // ── 请求解析 ──
  const parseSendMessage = (msg) => {
    const f = get(msg, 2); // PerformActionRequest field2 = send_message
    if (!f || f.wire !== 2) return null;
    const sm = asMsg(f);
    return {
      messageId: asStr(get(sm, 1)),
      assistantMessageId: asStr(get(sm, 2)),
      text: asStr(get(sm, 3)) ?? "",
      parentMessageId: asStr(get(sm, 5)),
      inputMode: asInt(get(sm, 6)),
      model: asStr(get(asMsg(get(sm, 7)), 2)),
    };
  };

  const parseActionHeader = (msg) => {
    const h = asMsg(get(msg, 1)); // ActionHeader
    const mid = asMsg(get(h, 1)); // MutationId{session_id=1, version=2}
    return {
      conversationId: asStr(get(h, 2)),
      sessionId: asStr(get(mid, 1)),
      version: get(mid, 2) ? Number(get(mid, 2).val) : undefined,
    };
  };

  const unary = (res, body, type = "application/proto") => {
    res.writeHead(200, { "content-type": type, "content-length": body.length });
    res.end(body);
  };

  const streamHeaders = {
    "content-type": "application/connect+proto",
    "connect-protocol-version": "1",
    "cache-control": "no-cache",
  };

  // ── 入口：method 名 + 原始请求体 ──
  async function handle(method, req, res, rawBody) {
    const ctype = req.headers["content-type"] || "";
    const isJson = ctype.includes("json");
    // connect+proto（streaming）是信封帧；application/proto（unary）是裸消息
    const payloads = ctype.includes("connect+")
      ? requestMessages(rawBody, req.headers["connect-content-encoding"] || "")
      : [rawBody];
    const reqMsg = payloads.length && payloads[0].length ? decode(payloads[0]) : [];

    switch (method) {
      case "StreamTimeline": {
        const convId = asStr(get(reqMsg, 1)) || "";
        const conv = convId ? ensureConv(convId) : null;
        const old = streams.get(convId);
        if (old) endStream(old);
        res.writeHead(200, streamHeaders);
        const stream = { res, convId, frames: 0, closed: false, hb: null };
        streams.set(convId, stream);
        stream.hb = setInterval(() => {
          if (!stream.closed) writeEvent(stream, streamEvent({ heartbeat: stream.frames }));
        }, 15000);
        stream.hb.unref?.();
        res.on("close", () => {
          stream.closed = true;
          clearInterval(stream.hb);
          if (streams.get(convId) === stream) streams.delete(convId);
        });
        if (conv) {
          writeEvent(stream, streamEvent({ update: buildStateUpdate(conv) }));
          ensureReply(conv).catch((e) => log?.(`reply error: ${e.message}`));
        }
        return;
      }

      case "PerformAction": {
        if (process.env.CLAUDE_LOCAL_DEBUG && !runtime.managed) {
          const dump = (fs, d = 0) => fs.map((f) => `${" ".repeat(d)}f${f.no}/w${f.wire}=${f.wire === 2 ? `<${f.val.length}B>` : f.val}`).join("\n");
          log?.(`PA req fields:\n${dump(reqMsg)}`);
          const hf = asMsg(get(reqMsg, 1));
          if (hf.length) log?.(`PA header fields:\n${dump(hf, 2)}`);
          const mid = asMsg(get(hf, 1));
          if (mid.length) log?.(`PA mutation_id fields:\n${dump(mid, 4)}`);
        }
        const header = parseActionHeader(reqMsg);
        const convId = header.conversationId || "";
        const conv = convId ? ensureConv(convId) : null;
        const send = parseSendMessage(reqMsg);
        const stopField = get(reqMsg, 3); // stop_generation
        const selectedModel = get(reqMsg, 27); // SetConversationModel.model.identifier
        if (conv && send) {
          recordSend(conv, send);
          ensureReply(conv).catch((e) => log?.(`reply error: ${e.message}`));
        } else if (conv && selectedModel) {
          conv.model = gatewayModel(runtime, asStr(get(asMsg(get(asMsg(selectedModel), 1)), 2)));
          onChange(true);
          const stream = streams.get(convId);
          if (stream) writeEvent(stream, streamEvent({ update: buildStateUpdate(conv) }));
        } else if (conv && stopField) {
          const turn = conv.turns.find((t) => !t.complete);
          if (turn) {
            turn.complete = true;
            turn.stopReason = "userCanceled";
            controllers.get(turn.asstId)?.abort();
            onChange(true);
            const s = streams.get(convId);
            if (s) writeEvent(s, streamEvent({ update: buildStateUpdate(conv) }));
            log?.(`stop conv=${convId}`);
          }
        }
        const ack = mutationAck({ sessionId: header.sessionId, version: header.version, applied: true });
        return unary(res, performActionResponse(ack));
      }

      case "SendAndSettle": {
        const header = parseActionHeader(reqMsg);
        const convId = header.conversationId || "";
        const conv = convId ? ensureConv(convId) : null;
        const sendField = get(reqMsg, 2);
        if (conv && sendField && sendField.wire === 2) {
          const sm = asMsg(sendField);
          recordSend(conv, {
            messageId: asStr(get(sm, 1)),
            assistantMessageId: asStr(get(sm, 2)),
            text: asStr(get(sm, 3)) ?? "",
            parentMessageId: asStr(get(sm, 5)),
            model: asStr(get(asMsg(get(sm, 7)), 2)),
          });
          await ensureReply(conv);
          const ack = mutationAck({ sessionId: header.sessionId, version: header.version, applied: true });
          return unary(res, sendAndSettleResponse({ update: buildStateUpdate(conv), settled: true, ack }));
        }
        const ack = mutationAck({ sessionId: header.sessionId, version: header.version, applied: true });
        return unary(res, sendAndSettleResponse({ settled: true, ack }));
      }

      case "GetNewConversationDefaults":
        return unary(res, newConversationDefaultsResponse());

      case "ReadConversation": {
        const convId = asStr(get(reqMsg, 1)) || "";
        const conv = convId ? ensureConv(convId) : null;
        return unary(res, readConversationResponse(conv ? buildStateUpdate(conv) : conversationUpdate({})));
      }

      case "ReadConversationHistory": {
        const convId = asStr(get(reqMsg, 1)) || "";
        const conv = convId ? ensureConv(convId) : null;
        return unary(res, readHistoryResponse(conv ? buildStateUpdate(conv) : conversationUpdate({})));
      }

      case "ListConversations":
        return unary(res, listConversationsResponse(listConvs().map(convProto)));

      case "StreamConversationList": {
        res.writeHead(200, streamHeaders);
        // StreamConversationListResponse{update=1: ConversationListUpdate{replace_head=1, conversations=2}}
        const update = Buffer.concat([
          vi(1, 1),
          ...listConvs().map((c) => ld(2, convProto(c))),
        ]);
        res.write(frame(ld(1, update)));
        res.end(endFrame());
        return;
      }

      case "ReportViewing":
        return unary(res, reportViewingResponse());

      case "StreamPromptComparison":
        res.writeHead(200, streamHeaders);
        return res.end(endFrame());

      case "ReportPromptSent":
      case "PrepareSend":
      case "ReadCoworkSession":
      case "ListConversationMemberships":
      case "CheckSignedSourceCards":
      case "ReadComposerNotice":
      case "ActOnComposerNotice":
      case "ListAttuneModels":
      case "UpsertAttuneModel":
        // 全 optional 的空响应
        return unary(res, Buffer.alloc(0));

      default:
        log?.(`RPC unknown method: ${method}`);
        return unary(res, Buffer.alloc(0));
    }
  }

  return { handle, streams, buildStateUpdate, ensureReply, recordSend };
}
