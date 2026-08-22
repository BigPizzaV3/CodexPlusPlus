const MAX_BODY_BYTES = 23_000_000;
const MAX_CIPHERTEXT_LENGTH = 22_500_000;
const ALLOWED_TTLS = new Set([86_400, 604_800, 2_592_000]);

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (request.method === "OPTIONS" && url.pathname.startsWith("/api/shares")) {
      return new Response(null, { status: 204, headers: corsHeaders() });
    }

    if (url.pathname === "/api/shares" && request.method === "POST") {
      return createShare(request, env);
    }

    const match = url.pathname.match(/^\/api\/shares\/([A-Za-z0-9_-]{20,32})$/);
    if (match && request.method === "GET") {
      return getShare(match[1], env);
    }
    if (match && request.method === "DELETE") {
      return deleteShare(request, match[1], env);
    }

    if (request.method !== "GET" && request.method !== "HEAD") {
      return json({ error: "Method not allowed" }, 405);
    }

    if (url.pathname.startsWith("/s/")) {
      // Fetch the root document to avoid Pages redirecting /index.html to /,
      // which would discard the share id from the browser URL.
      const indexUrl = new URL("/", url);
      const indexResponse = await env.ASSETS.fetch(new Request(indexUrl, request));
      return new Response(indexResponse.body, {
        status: indexResponse.status,
        headers: indexResponse.headers,
      });
    }

    return env.ASSETS.fetch(request);
  },
};

async function createShare(request, env) {
  if (!isJsonRequest(request)) {
    return json({ error: "Content-Type must be application/json" }, 415);
  }

  const contentLength = Number(request.headers.get("content-length") || 0);
  if (contentLength > MAX_BODY_BYTES) {
    return json({ error: "Share is too large" }, 413);
  }

  let body;
  try {
    body = await request.json();
  } catch {
    return json({ error: "Invalid JSON" }, 400);
  }

  const ttl = Number(body.ttl);
  const encrypted = body.encrypted;
  if (
    !ALLOWED_TTLS.has(ttl) ||
    !encrypted ||
    encrypted.v !== 1 ||
    typeof encrypted.iv !== "string" ||
    !/^[A-Za-z0-9_-]{16}$/.test(encrypted.iv) ||
    typeof encrypted.ciphertext !== "string" ||
    encrypted.ciphertext.length < 24 ||
    encrypted.ciphertext.length > MAX_CIPHERTEXT_LENGTH ||
    !/^[A-Za-z0-9_-]+$/.test(encrypted.ciphertext)
  ) {
    return json({ error: "Invalid encrypted payload" }, 400);
  }

  const id = randomToken(16);
  const deleteToken = randomToken(24);
  const deleteTokenHash = await sha256(deleteToken);
  const now = Date.now();
  const record = {
    encrypted,
    deleteTokenHash,
    createdAt: now,
    expiresAt: now + ttl * 1000,
  };

  await env.SHARES.put(`share:${id}`, JSON.stringify(record), {
    expirationTtl: ttl,
  });

  return json({ id, deleteToken, expiresAt: record.expiresAt }, 201);
}

async function getShare(id, env) {
  const record = await env.SHARES.get(`share:${id}`, "json");
  if (!record || record.expiresAt <= Date.now()) {
    return json({ error: "Share not found or expired" }, 404);
  }

  return json(
    {
      encrypted: record.encrypted,
      createdAt: record.createdAt,
      expiresAt: record.expiresAt,
    },
    200,
    { "Cache-Control": "private, no-store" },
  );
}

async function deleteShare(request, id, env) {
  const deleteToken = request.headers.get("x-delete-token");
  if (!deleteToken || deleteToken.length > 64) {
    return json({ error: "Invalid delete token" }, 401);
  }

  const record = await env.SHARES.get(`share:${id}`, "json");
  if (!record) {
    return new Response(null, { status: 204, headers: corsHeaders() });
  }

  const suppliedHash = await sha256(deleteToken);
  if (!timingSafeEqual(suppliedHash, record.deleteTokenHash)) {
    return json({ error: "Invalid delete token" }, 403);
  }

  await env.SHARES.delete(`share:${id}`);
  return new Response(null, { status: 204, headers: corsHeaders() });
}

function isJsonRequest(request) {
  return (request.headers.get("content-type") || "")
    .toLowerCase()
    .startsWith("application/json");
}

function randomToken(byteLength) {
  const bytes = crypto.getRandomValues(new Uint8Array(byteLength));
  return base64Url(bytes);
}

async function sha256(value) {
  const bytes = new TextEncoder().encode(value);
  const digest = await crypto.subtle.digest("SHA-256", bytes);
  return base64Url(new Uint8Array(digest));
}

function base64Url(bytes) {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

function timingSafeEqual(left, right) {
  if (typeof left !== "string" || typeof right !== "string" || left.length !== right.length) {
    return false;
  }
  let result = 0;
  for (let index = 0; index < left.length; index += 1) {
    result |= left.charCodeAt(index) ^ right.charCodeAt(index);
  }
  return result === 0;
}

function json(body, status, extraHeaders = {}) {
  return new Response(JSON.stringify(body), {
    status,
    headers: {
      "Content-Type": "application/json; charset=utf-8",
      "Cache-Control": "no-store",
      "X-Content-Type-Options": "nosniff",
      ...corsHeaders(),
      ...extraHeaders,
    },
  });
}

function corsHeaders() {
  return {
    "Access-Control-Allow-Origin": "*",
    "Access-Control-Allow-Methods": "GET, POST, DELETE, OPTIONS",
    "Access-Control-Allow-Headers": "Content-Type, X-Delete-Token",
  };
}
