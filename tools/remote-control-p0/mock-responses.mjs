// 假的 Responses 上游，按最后一条用户消息里的标记决定行为：
//   RUN  → 调 exec_command 申请提权执行 `touch <work>/approved.txt`，触发命令审批
//   ASK  → 调 request_user_input 问一个选择题
//   SLOW → 慢速流式输出 40 段，用来测插话与中断
//   其他 → 回「收到：<原文>」
// 用法：node mock-responses.mjs <port> <work-dir>
import http from "node:http";

const PORT = Number(process.argv[2] || 18765);
const WORK = process.argv[3] || process.cwd();
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const usage = { input_tokens: 100, input_tokens_details: { cached_tokens: 0 }, output_tokens: 10, output_tokens_details: { reasoning_tokens: 0 }, total_tokens: 110 };
let seq = 0;

const textOf = (item) => (typeof item?.content === "string" ? item.content : (item?.content || []).map((c) => c.text || "").join(""));

async function stream(res, events, gap = 0) {
  res.writeHead(200, { "content-type": "text/event-stream", "cache-control": "no-cache" });
  for (const event of events) {
    res.write(`event: ${event.type}\ndata: ${JSON.stringify(event)}\n\n`);
    if (gap) await sleep(gap);
  }
  res.end();
}

function message(id, text, chunks = [text]) {
  const msgId = `msg_${id}`;
  return [
    { type: "response.created", response: { id: `resp_${id}` } },
    { type: "response.output_item.added", output_index: 0, item: { id: msgId, type: "message", role: "assistant", content: [] } },
    ...chunks.map((delta) => ({ type: "response.output_text.delta", item_id: msgId, output_index: 0, content_index: 0, delta })),
    { type: "response.output_item.done", output_index: 0, item: { id: msgId, type: "message", role: "assistant", content: [{ type: "output_text", text }] } },
    { type: "response.completed", response: { id: `resp_${id}`, status: "completed", usage } },
  ];
}

function toolCall(id, name, args) {
  const item = { id: `fc_${id}`, type: "function_call", call_id: `call_${id}`, name, arguments: JSON.stringify(args) };
  return [
    { type: "response.created", response: { id: `resp_${id}` } },
    { type: "response.output_item.added", output_index: 0, item: { ...item, arguments: "" } },
    { type: "response.output_item.done", output_index: 0, item },
    { type: "response.completed", response: { id: `resp_${id}`, status: "completed", usage } },
  ];
}

http.createServer(async (req, res) => {
  let body = "";
  for await (const chunk of req) body += chunk;
  if (req.method === "GET" && req.url.startsWith("/v1/models")) {
    res.writeHead(200, { "content-type": "application/json" });
    return res.end(JSON.stringify({ object: "list", data: [{ id: "mock-model", object: "model" }] }));
  }
  if (req.method !== "POST" || !req.url.includes("/responses")) {
    res.writeHead(404);
    return res.end();
  }
  const id = ++seq;
  let json = {};
  try {
    json = JSON.parse(body);
  } catch {}
  const input = Array.isArray(json.input) ? json.input : [];
  const last = input[input.length - 1];
  const userText = textOf([...input].reverse().find((item) => item.role === "user"));

  if (last?.type === "function_call_output") {
    const output = typeof last.output === "string" ? last.output : JSON.stringify(last.output);
    return stream(res, message(id, `工具结果：${output.slice(0, 160)}`));
  }
  if (/\bRUN\b/.test(userText)) {
    return stream(res, toolCall(id, "exec_command", {
      cmd: `touch ${WORK}/approved.txt && echo created`,
      workdir: WORK,
      sandbox_permissions: "require_escalated",
      justification: "远程审批测试：在工作目录创建 approved.txt",
    }));
  }
  if (/\bASK\b/.test(userText)) {
    return stream(res, toolCall(id, "request_user_input", {
      questions: [{ id: "color", header: "颜色", question: "你喜欢哪个颜色？", options: [{ label: "红色", description: "热情" }, { label: "蓝色", description: "冷静" }] }],
    }));
  }
  if (/\bSLOW\b/.test(userText)) {
    const chunks = Array.from({ length: 40 }, (_, i) => `第${i + 1}段 `);
    return stream(res, message(id, chunks.join(""), chunks), 500);
  }
  return stream(res, message(id, `收到：${userText.slice(0, 60)}`));
}).listen(PORT, "127.0.0.1", () => console.log(`mock responses listening on ${PORT}`));
