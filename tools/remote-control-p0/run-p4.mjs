// 远程控制 P4 端到端：发图片 / 文件、新建会话时浏览文件夹。
// 核对：上传后路径在上传目录里；发消息带图后上游真的收到 input_image、带文件后上游消息里引用了该文件；
// 视图里能看到附件，网页能取回原图；上传目录外的路径被拒；目录浏览只列文件夹、标出 git 仓库。
//
// 前提同 run-p3：P1_PORT=57399 P1_TOKEN=… P1_WORK=/tmp/codex-p0/work2 P1_MOCK_LOG=… node run-p4.mjs
import fs from "node:fs";
import path from "node:path";

const PORT = Number(process.env.P1_PORT || 57399);
const TOKEN = process.env.P1_TOKEN;
const WORK = process.env.P1_WORK;
const MOCK_LOG = process.env.P1_MOCK_LOG;
if (!TOKEN || !WORK || !MOCK_LOG) {
  console.error("需要 P1_TOKEN、P1_WORK、P1_MOCK_LOG");
  process.exit(2);
}
const BASE = `http://127.0.0.1:${PORT}`;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const results = [];
function record(name, ok, detail = "") {
  results.push({ name, ok });
  console.log(`${ok ? "✓" : "✗"} ${name}${detail ? `  (${detail})` : ""}`);
}
async function api(method, route, body, headers = {}) {
  const response = await fetch(`${BASE}/api${route}`, {
    method,
    headers: { Authorization: `Bearer ${TOKEN}`, ...(body && !(body instanceof Uint8Array) ? { "Content-Type": "application/json" } : {}), ...headers },
    body: body instanceof Uint8Array ? body : body ? JSON.stringify(body) : undefined,
  });
  const payload = await response.json().catch(() => ({}));
  return { status: response.status, ...payload };
}
const upload = (bytes, name, type) => api("POST", "/uploads", bytes, { "Content-Type": type, "X-File-Name": encodeURIComponent(name) });
const lastRequest = () => JSON.parse(fs.readFileSync(MOCK_LOG, "utf8").trim().split("\n").at(-1));
async function waitIdle(threadId, predicate, ms = 30000) {
  const deadline = Date.now() + ms;
  while (Date.now() < deadline) {
    const view = await api("GET", `/threads/${threadId}`);
    if (view.ok && view.data.runtime !== "active" && predicate(view.data)) return view.data;
    await sleep(500);
  }
  return null;
}

// 1×1 红色 PNG
const PNG = Uint8Array.from(Buffer.from("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==", "base64"));

try {
  // 目录浏览
  const home = await api("GET", "/fs/dirs");
  record("目录浏览从主目录开始", home.ok && !!home.data.path && home.data.path === home.data.home, home.data?.path);
  // 目录浏览保留走过来的路径写法（不把 /tmp 解析成 /private/tmp），和会话记录的 cwd 一致。
  const realWork = WORK;
  const listed = await api("GET", `/fs/dirs?path=${encodeURIComponent(path.dirname(WORK))}`);
  const entry = listed.data?.entries?.find((item) => item.path === realWork);
  record("能列出子文件夹", listed.ok && !!entry, `${listed.data?.entries?.length} 个`);
  record("只列文件夹、不列隐藏的", listed.ok && listed.data.entries.every((item) => !item.name.startsWith(".")));
  fs.mkdirSync(path.join(WORK, "repo-probe", ".git"), { recursive: true });
  const git = await api("GET", `/fs/dirs?path=${encodeURIComponent(WORK)}`);
  record("标出 git 仓库", git.data?.entries?.find((item) => item.name === "repo-probe")?.git === true);
  record("相对路径被拒", !(await api("GET", "/fs/dirs?path=tmp")).ok);

  // 上传
  const image = await upload(PNG, "截图.png", "image/png");
  record("上传图片", image.ok && image.data.kind === "image" && image.data.path.includes("remote-uploads"), image.error ?? image.data.path);
  const noExt = await upload(PNG, "IMG_0001", "image/jpeg");
  record("没扩展名的照片按类型补上", noExt.ok && noExt.data.name.endsWith(".jpg"), noExt.data?.name);
  const doc = await upload(Uint8Array.from(Buffer.from("远程上传的说明文件\n第二行\n")), "../../说明.md", "text/markdown");
  record("上传文件，名字里的路径被去掉", doc.ok && doc.data.kind === "file" && doc.data.name === "说明.md", doc.error ?? doc.data.name);
  record("空文件被拒", !(await upload(new Uint8Array(), "empty.txt", "text/plain")).ok);

  const raw = await fetch(`${BASE}/api/uploads/raw?path=${encodeURIComponent(image.data.path)}&token=${TOKEN}`);
  const rawBytes = new Uint8Array(await raw.arrayBuffer());
  record("网页能取回原图", raw.status === 200 && raw.headers.get("content-type") === "image/png" && rawBytes.length === PNG.length);
  const outside = await fetch(`${BASE}/api/uploads/raw?path=${encodeURIComponent("/etc/hosts")}&token=${TOKEN}`);
  record("上传目录外的文件取不到", outside.status === 400);

  // 发带附件的消息（新会话，后台执行）
  const created = await api("POST", "/threads", { cwd: WORK, name: `P4 附件 ${new Date().toISOString().slice(11, 19)}` });
  const threadId = created.data.threadId;
  const sentImage = await api("POST", `/threads/${threadId}/messages`, { text: "看看这张图", attachments: [{ path: image.data.path, name: image.data.name, kind: "image" }] });
  record("发带图片的消息", sentImage.ok, sentImage.error ?? sentImage.data.kind);
  const imageView = await waitIdle(threadId, (view) => view.turns.at(-1)?.items.some((item) => item.type === "agentMessage"));
  const request = lastRequest();
  record("上游收到图片", request.images === 1 && request.user.includes("看看这张图"), JSON.stringify({ images: request.images, user: request.user }));
  const userItem = imageView?.turns.flatMap((turn) => turn.items).find((item) => item.type === "userMessage" && item.attachments?.some((attachment) => attachment.path === image.data.path));
  record("视图里能看到图片附件", userItem?.attachments?.[0]?.kind === "image" && userItem.attachments[0].path === image.data.path, JSON.stringify(userItem?.attachments));

  const sentFile = await api("POST", `/threads/${threadId}/messages`, { text: "", attachments: [{ path: doc.data.path, name: doc.data.name, kind: "file" }] });
  record("只发文件不写字也行", sentFile.ok, sentFile.error ?? "");
  await waitIdle(threadId, (view) => view.turns.length >= 2 && view.turns.at(-1).items.some((item) => item.type === "agentMessage"));
  const fileRequest = lastRequest();
  record("上游消息里引用了该文件", fileRequest.files.some((file) => doc.data.path.endsWith(file.replace(/^remote-uploads\//, ""))), JSON.stringify(fileRequest.files));

  const bad = await api("POST", `/threads/${threadId}/messages`, { text: "x", attachments: [{ path: "/etc/hosts", kind: "file" }] });
  record("消息里塞上传目录外的路径被拒", !bad.ok, bad.error);
  const empty = await api("POST", `/threads/${threadId}/messages`, { text: "  " });
  record("没字没附件被拒", !empty.ok, empty.error);
  fs.rmSync(path.join(WORK, "repo-probe"), { recursive: true, force: true });
} catch (error) {
  record("运行中断", false, error.message);
} finally {
  const failed = results.filter((item) => !item.ok);
  console.log(`\n${results.length - failed.length}/${results.length} 通过`);
  process.exit(failed.length ? 1 : 0);
}
