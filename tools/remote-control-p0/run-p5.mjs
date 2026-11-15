// 远程控制 P5 端到端：桌面镜像。
// 核对：连上后收到 JPEG 画面帧和尺寸；窗口在后台也保持可见、有焦点；鼠标点到 Codex 输入框能聚焦；中文整段写进去；退格删一个字；
// Cmd+A（带 selectAll 命令）+ 退格清空；非法输入被拒；只读设备连不上；局域网 wss 也能用。
//
// 前提同 run-p3（隔离桌面开着调试端口 9333）：
//   P1_PORT=57399 P1_TOKEN=… P3_CDP_PORT=9333 node run-p5.mjs
// 局域网那一项需要远程桥带 P1_LAN_PORT / P1_STATE / P1_PAIR_ROLES=view,full 启动，另给 P2_LAN_PORT、P2_DIR。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";

const PORT = Number(process.env.P1_PORT || 57399);
const TOKEN = process.env.P1_TOKEN;
const CDP_PORT = Number(process.env.P3_CDP_PORT || 9333);
const LAN_PORT = Number(process.env.P2_LAN_PORT || 0);
const DIR = process.env.P2_DIR;
if (!TOKEN) {
  console.error("需要 P1_TOKEN");
  process.exit(2);
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = [];
function record(name, ok, detail = "") {
  results.push({ name, ok });
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
}

/** 直接在 Codex 页面里求值（只用来核对结果，镜像本身不经过这里）。 */
async function codex(expression) {
  const targets = await (await fetch(`http://127.0.0.1:${CDP_PORT}/json/list`)).json();
  const page = targets.find((target) => target.type === "page" && target.url.startsWith("app://-/index.html"));
  const ws = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve) => ws.addEventListener("open", resolve, { once: true }));
  const result = await new Promise((resolve) => {
    ws.addEventListener("message", (event) => {
      const message = JSON.parse(event.data);
      if (message.id === 1) resolve(message.result?.result?.value);
    });
    ws.send(JSON.stringify({ id: 1, method: "Runtime.evaluate", params: { expression, returnByValue: true } }));
  });
  ws.close();
  return result;
}
// 页面上可能有多个编辑器（首页、各会话），读当前看得见的那个。
const editorText = () => codex(`([...document.querySelectorAll(".ProseMirror")].find((e) => e.getBoundingClientRect().height > 0)?.innerText || "").replace(/\\n+$/, "")`);

function openMirror(url) {
  const socket = new WebSocket(url);
  socket.binaryType = "arraybuffer";
  const box = { socket, frames: 0, firstFrame: null, meta: null, messages: [] };
  const waiters = new Set();
  socket.addEventListener("message", (event) => {
    if (typeof event.data === "string") box.messages.push(JSON.parse(event.data));
    else {
      box.frames += 1;
      box.firstFrame ||= new Uint8Array(event.data);
    }
    const last = box.messages.at(-1);
    if (last?.type === "meta") box.meta = last.meta;
    for (const waiter of [...waiters]) waiter();
  });
  box.until = (predicate, ms = 8000) => new Promise((resolve) => {
    const check = () => {
      const value = predicate(box);
      if (value) { clearTimeout(timer); waiters.delete(check); resolve(value); }
    };
    const timer = setTimeout(() => { waiters.delete(check); resolve(null); }, ms);
    waiters.add(check);
    check();
  });
  // 被拒的握手（401 / 403，没升级成 WebSocket）在 Node 里只报 error，不一定有 close。
  box.closed = new Promise((resolve) => { socket.addEventListener("close", resolve); socket.addEventListener("error", resolve); });
  box.opened = false;
  socket.addEventListener("open", () => { box.opened = true; });
  box.send = (message) => socket.send(JSON.stringify(message));
  return box;
}

try {
  const mirror = openMirror(`ws://127.0.0.1:${PORT}/api/mirror?token=${TOKEN}`);
  const ready = await mirror.until((box) => box.messages.find((message) => message.type === "ready" || message.type === "error"));
  record("连上 Codex 桌面", ready?.type === "ready", ready?.title ?? ready?.error);
  mirror.send({ type: "start", width: 1280, height: 820 });
  await mirror.until((box) => box.frames >= 1 && box.meta);
  const jpeg = mirror.firstFrame;
  record("收到 JPEG 画面帧", !!jpeg && jpeg[0] === 0xff && jpeg[1] === 0xd8, `${mirror.frames} 帧，${jpeg?.length ?? 0} 字节`);
  record("收到页面尺寸", mirror.meta?.deviceWidth > 0 && mirror.meta?.deviceHeight > 0, JSON.stringify(mirror.meta));

  // 镜像期间：Codex 窗口在后台 / 被挡住 / 最小化，页面也当自己可见、有焦点（不然不出画面、收不到键盘）
  const pageState = await codex(`document.visibilityState + "/" + document.hasFocus()`);
  record("镜像期间 Codex 页面保持可见、有焦点", pageState === "visible/true", pageState);

  // 点击：先让输入框失焦，再从镜像点它的中心
  await codex(`document.activeElement?.blur(); window.getSelection()?.removeAllRanges(); true`);
  const rect = await codex(`(r => ({ x: r.x + r.width / 2, y: r.y + r.height / 2 }))([...document.querySelectorAll(".ProseMirror")].find((e) => e.getBoundingClientRect().height > 0).getBoundingClientRect())`);
  mirror.send({ type: "mouse", event: "mousePressed", x: rect.x, y: rect.y, button: "left", buttons: 1, clickCount: 1 });
  mirror.send({ type: "mouse", event: "mouseReleased", x: rect.x, y: rect.y, button: "left", buttons: 0, clickCount: 1 });
  await sleep(400);
  record("鼠标点击聚焦 Codex 输入框", await codex(`!!document.activeElement?.closest?.(".ProseMirror")`));

  // 文字：中文整段（输入法拼好后发来的就是这样一段）
  mirror.send({ type: "text", text: "镜像输入：中文、English 混排 ✅" });
  await sleep(400);
  const typed = await editorText();
  record("中文整段写进输入框", typed === "镜像输入：中文、English 混排 ✅", JSON.stringify(typed));

  mirror.send({ type: "key", event: "rawKeyDown", key: "Backspace", code: "Backspace", keyCode: 8 });
  mirror.send({ type: "key", event: "keyUp", key: "Backspace", code: "Backspace", keyCode: 8 });
  await sleep(300);
  const afterBackspace = await editorText();
  record("退格删一个字", afterBackspace === "镜像输入：中文、English 混排 ", JSON.stringify(afterBackspace));

  mirror.send({ type: "key", event: "rawKeyDown", key: "a", code: "KeyA", keyCode: 65, modifiers: 4, commands: ["selectAll"] });
  mirror.send({ type: "key", event: "keyUp", key: "a", code: "KeyA", keyCode: 65, modifiers: 4 });
  mirror.send({ type: "key", event: "rawKeyDown", key: "Backspace", code: "Backspace", keyCode: 8 });
  mirror.send({ type: "key", event: "keyUp", key: "Backspace", code: "Backspace", keyCode: 8 });
  await sleep(300);
  record("Cmd+A 全选后退格清空", (await editorText()) === "", JSON.stringify(await editorText()));

  mirror.send({ type: "text", text: "第一行" });
  mirror.send({ type: "key", event: "rawKeyDown", key: "Enter", code: "Enter", keyCode: 13, modifiers: 8 });
  mirror.send({ type: "key", event: "keyUp", key: "Enter", code: "Enter", keyCode: 13, modifiers: 8 });
  mirror.send({ type: "text", text: "第二行" });
  await sleep(300);
  const twoLines = await editorText();
  record("Shift+Enter 换行", /第一行\n+第二行/.test(twoLines), JSON.stringify(twoLines));

  // 滚轮：不报错即可（画面里没有可滚动内容也不会出错）
  const warningsBefore = mirror.messages.filter((message) => message.type === "warning").length;
  mirror.send({ type: "wheel", x: 600, y: 300, deltaX: 0, deltaY: 120 });
  mirror.send({ type: "key", event: "keyPress", key: "x" });
  mirror.send({ type: "key", event: "rawKeyDown", key: "a", commands: ["deleteEverything"] });
  mirror.send({ type: "mouse", event: "mousePressed", x: "abc", y: 1, button: "left" });
  mirror.send({ type: "text", text: "x".repeat(20001) });
  mirror.send({ type: "exec", script: "alert(1)" });
  const warnings = await mirror.until((box) => box.messages.filter((message) => message.type === "warning").length >= warningsBefore + 5 && box);
  record("非法输入全部被拒（不认识的事件、命令、坐标、超长文字、脚本）", !!warnings,
    mirror.messages.filter((message) => message.type === "warning").slice(-5).map((message) => message.error).join(" / "));

  // 清理输入框
  mirror.send({ type: "key", event: "rawKeyDown", key: "a", code: "KeyA", keyCode: 65, modifiers: 4, commands: ["selectAll"] });
  mirror.send({ type: "key", event: "rawKeyDown", key: "Backspace", code: "Backspace", keyCode: 8 });
  await sleep(200);
  mirror.socket.close();

  // 没令牌、错令牌
  const anonymous = openMirror(`ws://127.0.0.1:${PORT}/api/mirror`);
  const anonymousClosed = await Promise.race([anonymous.closed.then(() => true), sleep(3000).then(() => false)]);
  record("没令牌连不上", anonymousClosed && !anonymous.opened);

  // 局域网：只读设备被拒、完全控制设备经 wss 能看到画面
  if (LAN_PORT && DIR) {
    process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";
    const lanIp = Object.values(os.networkInterfaces()).flat().find((item) => item?.family === "IPv4" && !item.internal)?.address;
    const pair = async (role) => {
      for (let i = 0; i < 50; i += 1) {
        const text = fs.existsSync(path.join(DIR, "pairing-code.txt")) ? fs.readFileSync(path.join(DIR, "pairing-code.txt"), "utf8").trim() : "";
        const [code, codeRole] = text.split(" ");
        if (code && codeRole === role) {
          const response = await fetch(`https://${lanIp}:${LAN_PORT}/api/pair`, { method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify({ code, name: `镜像 ${role}` }) });
          return (await response.json()).data?.token;
        }
        await sleep(200);
      }
      return null;
    };
    const viewToken = await pair("view");
    const viewer = openMirror(`wss://${lanIp}:${LAN_PORT}/api/mirror?token=${viewToken}`);
    const viewerClosed = await Promise.race([viewer.closed.then(() => true), sleep(3000).then(() => false)]);
    record("只读设备不能用桌面镜像", !!viewToken && viewerClosed && !viewer.opened);
    const fullToken = await pair("full");
    const lan = openMirror(`wss://${lanIp}:${LAN_PORT}/api/mirror?token=${fullToken}`);
    await lan.until((box) => box.messages.some((message) => message.type === "ready"));
    lan.send({ type: "start", width: 800, height: 500 });
    const lanFrame = await lan.until((box) => box.frames >= 1 && box);
    record("完全控制设备经局域网 wss 收到画面", !!lanFrame, `${lan.frames} 帧`);
    lan.socket.close();
  }
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  process.exit(failed.length ? 1 : 0);
}
