const encoder = new TextEncoder();
const decoder = new TextDecoder();

const createView = document.querySelector("[data-create-view]");
const viewer = document.querySelector("[data-view-view]");
const form = document.querySelector("[data-share-form]");
const content = document.querySelector("#content");
const counter = document.querySelector("[data-counter]");
const submit = document.querySelector("[data-submit]");
const result = document.querySelector("[data-result]");
const shareUrl = document.querySelector("[data-share-url]");
const notice = document.querySelector("[data-notice]");
let sharedSession = null;

const shareId =
  new URLSearchParams(location.search).get("s") ||
  location.pathname.match(/^\/s\/([A-Za-z0-9_-]{20,32})\/?$/)?.[1];
if (shareId) {
  showShare(shareId);
} else {
  setupComposer();
}

function setupComposer() {
  content.addEventListener("input", () => {
    counter.textContent = `${content.value.length.toLocaleString()} / 900,000`;
  });

  form.addEventListener("submit", async (event) => {
    event.preventDefault();
    setNotice("");
    submit.disabled = true;
    submit.textContent = "正在加密...";

    try {
      const encrypted = await encryptText(content.value);
      const response = await fetch("/api/shares", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          encrypted: encrypted.payload,
          ttl: Number(document.querySelector("#ttl").value),
        }),
      });
      const data = await response.json();
      if (!response.ok) throw new Error(data.error || "创建失败");

      const url = new URL(location.origin);
      url.searchParams.set("s", data.id);
      url.hash = `k=${encrypted.key}`;
      shareUrl.value = url.toString();
      localStorage.setItem(`codexpp-share-delete:${data.id}`, data.deleteToken);
      result.dataset.shareId = data.id;
      result.hidden = false;
      setNotice("链接已创建。正文和密钥均未以明文发送。", "success");
    } catch (error) {
      setNotice(error instanceof Error ? error.message : "创建失败，请稍后重试。", "error");
    } finally {
      submit.disabled = false;
      submit.textContent = "加密并创建链接";
    }
  });

  document.querySelector("[data-copy]").addEventListener("click", async () => {
    await navigator.clipboard.writeText(shareUrl.value);
    setNotice("分享链接已复制。", "success");
  });

  document.querySelector("[data-delete]").addEventListener("click", async () => {
    const id = result.dataset.shareId;
    const token = localStorage.getItem(`codexpp-share-delete:${id}`);
    if (!id || !token) return setNotice("找不到此分享的撤销凭据。", "error");

    const response = await fetch(`/api/shares/${id}`, {
      method: "DELETE",
      headers: { "X-Delete-Token": token },
    });
    if (!response.ok) return setNotice("撤销失败，请稍后重试。", "error");

    localStorage.removeItem(`codexpp-share-delete:${id}`);
    result.hidden = true;
    setNotice("分享已撤销。", "success");
  });
}

async function showShare(id) {
  createView.hidden = true;
  viewer.hidden = false;
  const viewContent = document.querySelector("[data-view-content]");
  const viewMeta = document.querySelector("[data-view-meta]");
  const viewNotice = document.querySelector("[data-view-notice]");
  const importButton = document.querySelector("[data-import-session]");

  try {
    const keyValue = new URLSearchParams(location.hash.slice(1)).get("k");
    if (!keyValue) throw new Error("链接缺少解密密钥。请使用完整的分享链接。");

    const response = await fetch(`/api/shares/${id}`, { cache: "no-store" });
    const data = await response.json();
    if (!response.ok) throw new Error("分享不存在、已撤销或已过期。");

    const plaintext = await decryptText(data.encrypted, keyValue);
    try {
      const parsed = JSON.parse(plaintext);
      if (parsed?.kind === "codex-rollout" && typeof parsed.content === "string") {
        sharedSession = parsed;
        viewContent.textContent = rolloutToMarkdown(parsed.content, parsed.title);
        importButton.hidden = false;
        importButton.addEventListener("click", importSharedSession, { once: true });
      } else if (parsed?.kind === "codex-session" && Array.isArray(parsed.messages)) {
        sharedSession = parsed;
        viewContent.textContent = sessionToMarkdown(parsed);
        importButton.hidden = false;
        importButton.addEventListener("click", importSharedSession, { once: true });
      } else {
        viewContent.textContent = plaintext;
      }
    } catch {
      viewContent.textContent = plaintext;
    }
    viewMeta.textContent = `有效期至 ${new Date(data.expiresAt).toLocaleString()}`;
  } catch (error) {
    viewMeta.textContent = "无法打开分享";
    viewNotice.textContent = error instanceof Error ? error.message : "解密失败。";
    viewNotice.dataset.type = "error";
  }
}

function sessionToMarkdown(session) {
  const title = String(session.title || "未命名会话").trim() || "未命名会话";
  const messages = session.messages.map((message) => {
    const role = message.role === "user" ? "用户" : message.role === "assistant" ? "助手" : "消息";
    return `### ${role}\n\n${String(message.text || "").trim()}`;
  }).filter(Boolean);
  return `# ${title}\n\n- 会话 ID：\`${session.session_id || ""}\`\n\n${messages.join("\n\n")}`;
}

function rolloutToMarkdown(content, title) {
  const messages = [];
  for (const line of content.split(/\r?\n/)) {
    try {
      const payload = JSON.parse(line).payload;
      if (payload?.type !== "message" || !["user", "assistant"].includes(payload.role)) continue;
      const text = Array.isArray(payload.content)
        ? payload.content.map((part) => part?.text || "").join("\n").trim()
        : "";
      if (text) messages.push(`### ${payload.role === "user" ? "用户" : "助手"}\n\n${text}`);
    } catch {
      // Ignore malformed or non-message lines in the viewer; the native importer validates them.
    }
  }
  return `# ${String(title || "未命名会话").trim() || "未命名会话"}\n\n${messages.join("\n\n")}`;
}

async function importSharedSession() {
  if (!sharedSession) return;
  const payload = { type: "codexpp-import-session", session: sharedSession };
  if (window.opener && !window.opener.closed) {
    window.opener.postMessage(payload, "*");
    const viewNotice = document.querySelector("[data-view-notice]");
    viewNotice.textContent = "已发送到 Codex++，正在创建新的导入会话。";
    viewNotice.dataset.type = "success";
    return;
  }
  try {
    const text = sharedSession.kind === "codex-rollout"
      ? rolloutToMarkdown(sharedSession.content, sharedSession.title)
      : sessionToMarkdown(sharedSession);
    await navigator.clipboard.writeText(text);
    const viewNotice = document.querySelector("[data-view-notice]");
    viewNotice.textContent = "当前页面不是从 Codex++ 打开的，完整会话内容已复制。请在 Codex++ 新对话中粘贴。";
    viewNotice.dataset.type = "success";
  } catch {
    const viewNotice = document.querySelector("[data-view-notice]");
    viewNotice.textContent = "无法连接到 Codex++，请直接复制页面中的完整会话内容。";
    viewNotice.dataset.type = "error";
  }
}

async function encryptText(value) {
  const key = await crypto.subtle.generateKey({ name: "AES-GCM", length: 256 }, true, ["encrypt"]);
  const iv = crypto.getRandomValues(new Uint8Array(12));
  const ciphertext = await crypto.subtle.encrypt({ name: "AES-GCM", iv }, key, encoder.encode(value));
  const exportedKey = await crypto.subtle.exportKey("raw", key);
  return {
    key: toBase64Url(new Uint8Array(exportedKey)),
    payload: {
      v: 1,
      iv: toBase64Url(iv),
      ciphertext: toBase64Url(new Uint8Array(ciphertext)),
    },
  };
}

async function decryptText(payload, keyValue) {
  if (payload?.v !== 1) throw new Error("不支持此分享的数据格式。");
  const key = await crypto.subtle.importKey(
    "raw",
    fromBase64Url(keyValue),
    { name: "AES-GCM" },
    false,
    ["decrypt"],
  );
  const plaintext = await crypto.subtle.decrypt(
    { name: "AES-GCM", iv: fromBase64Url(payload.iv) },
    key,
    fromBase64Url(payload.ciphertext),
  );
  return decoder.decode(plaintext);
}

function toBase64Url(bytes) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function fromBase64Url(value) {
  const normalized = value.replace(/-/g, "+").replace(/_/g, "/");
  const padded = normalized.padEnd(Math.ceil(normalized.length / 4) * 4, "=");
  const binary = atob(padded);
  return Uint8Array.from(binary, (character) => character.charCodeAt(0));
}

function setNotice(message, type = "") {
  notice.textContent = message;
  notice.dataset.type = type;
}
