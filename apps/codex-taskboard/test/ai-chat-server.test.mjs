import assert from "node:assert/strict";
import { chmod, mkdtemp, mkdir, realpath, rm, writeFile } from "node:fs/promises";
import { request as httpRequest } from "node:http";
import os from "node:os";
import path from "node:path";
import { test } from "node:test";

import { createTaskboardServer } from "../server/index.mjs";

async function createServerFixture(host = "127.0.0.1") {
  const directory = await mkdtemp(path.join(os.tmpdir(), "taskboard-ai-server-"));
  const workspacePath = path.join(directory, "workspace");
  await mkdir(workspacePath);
  const workspace = await realpath(workspacePath);
  const codexExecutable = path.join(directory, "fake-codex.mjs");
  await writeFile(codexExecutable, `#!/usr/bin/env node
const args = process.argv.slice(2);
if (args[0] === "debug") {
  process.stdout.write('{"models":[{"slug":"gpt-real","display_name":"GPT Real","description":"","default_reasoning_level":"low","supported_reasoning_levels":[{"effort":"low"},{"effort":"high"}],"service_tiers":[]}]}');
} else if (args[0] === "app-server") {
  process.stdin.setEncoding("utf8"); let buffer="";
  process.stdin.on("data", chunk => { buffer += chunk; let i;
    while ((i=buffer.indexOf("\\n"))>=0) { const line=buffer.slice(0,i); buffer=buffer.slice(i+1);
      if (!line.trim()) continue; const message=JSON.parse(line);
      if (message.id===1) process.stdout.write('{"id":1,"result":{}}\\n');
      if (message.id===2) process.stdout.write('{"id":2,"result":{"data":[{"skills":[{"name":"real-skill","enabled":true,"scope":"repo","interface":null}]}]}}\\n');
    }
  });
} else {
  process.stdin.resume();
  process.stdin.on("end", () => {
    process.stdout.write('{"type":"thread.started","thread_id":"session-1"}\\n');
    process.stdout.write('{"type":"item.completed","item":{"type":"agent_message","text":"ok"}}\\n');
    process.stdout.write('{"type":"turn.completed"}\\n');
  });
}
`);
  await chmod(codexExecutable, 0o755);
  const codexStatePath = path.join(directory, "codex-state.json");
  await writeFile(codexStatePath, JSON.stringify({
    "local-projects": { local: { rootPaths: [workspace] } },
  }));
  const app = createTaskboardServer({
    dataDirectory: directory,
    codexExecutable,
    codexStatePath,
    skillPath: "/fixture/manage-taskboard/SKILL.md",
  });
  const address = await app.listen({ host, port: 0 });
  return {
    app,
    baseUrl: `http://127.0.0.1:${address.port}`,
    directory,
    workspace,
    async close() {
      await app.close();
      await rm(directory, { recursive: true, force: true });
    },
  };
}

function privateLanAddress() {
  return Object.values(os.networkInterfaces())
    .flat()
    .find((entry) => {
      if (entry?.family !== "IPv4" || entry.internal) return false;
      const [first, second] = entry.address.split(".").map(Number);
      return first === 10
        || (first === 172 && second >= 16 && second <= 31)
        || (first === 192 && second === 168)
        || (first === 169 && second === 254);
    })?.address;
}

async function requestFrom(address, port, pathname) {
  return new Promise((resolve, reject) => {
    const outgoing = httpRequest({
      host: address,
      port,
      path: pathname,
      headers: { host: `${address}:${port}` },
    }, (response) => {
      const chunks = [];
      response.on("data", (chunk) => chunks.push(chunk));
      response.on("end", () => resolve({
        status: response.statusCode,
        body: JSON.parse(Buffer.concat(chunks).toString("utf8")),
      }));
    });
    outgoing.on("error", reject);
    outgoing.end();
  });
}

async function request(baseUrl, pathname, options = {}) {
  const response = await fetch(`${baseUrl}${pathname}`, {
    ...options,
    headers: { "content-type": "application/json", ...options.headers },
    body: options.body === undefined ? undefined : JSON.stringify(options.body),
  });
  const text = await response.text();
  return { response, body: text ? JSON.parse(text) : undefined };
}

test("loopback AI API freezes server-owned origin and rejects injected execution fields", async () => {
  const fixture = await createServerFixture();
  try {
    const meta = await request(fixture.baseUrl, "/api/meta");
    assert.equal(meta.body.capabilities.localAiChat, true);
    const catalog = await request(fixture.baseUrl, "/api/local/ai/catalog?projectId=local");
    assert.equal(catalog.response.status, 200);
    assert.equal(catalog.body.models[0].slug, "gpt-real");
    assert.equal(catalog.body.skills[0].id, "real-skill");

    const injected = await request(fixture.baseUrl, "/api/local/ai/threads", {
      method: "POST",
      body: { projectId: "local", workspacePath: "/tmp/evil", argv: ["--dangerously-bypass-approvals-and-sandbox"] },
    });
    assert.equal(injected.response.status, 400);
    assert.equal(injected.body.error.code, "UNKNOWN_FIELD");

    const created = await request(fixture.baseUrl, "/api/local/ai/threads", {
      method: "POST",
      body: {
        projectId: "local",
        model: "gpt-real",
        reasoningEffort: "high",
        sandbox: "read-only",
      },
    });
    assert.equal(created.response.status, 201);
    assert.equal(created.body.thread.origin.workspacePath, fixture.workspace);
    const threadId = created.body.thread.id;

    const invalidSkill = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}/turns`, {
      method: "POST",
      body: { message: "hello", skillIds: ["invented-skill"] },
    });
    assert.equal(invalidSkill.response.status, 400);
    assert.equal(invalidSkill.body.error.code, "INVALID_SKILL");

    const turn = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}/turns`, {
      method: "POST",
      body: { message: "hello", skillIds: ["real-skill"] },
    });
    assert.equal(turn.response.status, 202);
    assert.equal(turn.body.run.threadId, threadId);

    let snapshot;
    for (let index = 0; index < 100; index += 1) {
      snapshot = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}`);
      if (snapshot.body.runs[0]?.status !== "running") break;
      await new Promise((resolve) => setTimeout(resolve, 20));
    }
    assert.equal(snapshot.body.thread.codexThreadId, "session-1");
    assert.equal(snapshot.body.events.some((event) => event.content === "ok"), true);
  } finally {
    await fixture.close();
  }
});

test("danger-full-access requires confirmation on every turn and thread settings are validated", async () => {
  const fixture = await createServerFixture();
  try {
    const created = await request(fixture.baseUrl, "/api/local/ai/threads", {
      method: "POST",
      body: {
        projectId: "local",
        model: "gpt-real",
        reasoningEffort: "low",
        sandbox: "danger-full-access",
      },
    });
    assert.equal(created.response.status, 201);
    const threadId = created.body.thread.id;
    const denied = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}/turns`, {
      method: "POST",
      body: { message: "hello" },
    });
    assert.equal(denied.response.status, 400);
    assert.equal(denied.body.error.code, "DANGER_CONFIRMATION_REQUIRED");
    const allowed = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}/turns`, {
      method: "POST",
      body: { message: "hello", dangerFullAccessConfirmed: true },
    });
    assert.equal(allowed.response.status, 202);

    const invalidModel = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}`, {
      method: "PATCH",
      body: { model: "invented-model", reasoningEffort: "high" },
    });
    assert.equal(invalidModel.response.status, 400);
    assert.equal(invalidModel.body.error.code, "INVALID_MODEL");
  } finally {
    await fixture.close();
  }
});

test("thread management, interrupt and query contracts stay narrow", async () => {
  const fixture = await createServerFixture();
  try {
    const created = await request(fixture.baseUrl, "/api/local/ai/threads", {
      method: "POST",
      body: { projectId: "local", title: "Original" },
    });
    const threadId = created.body.thread.id;

    const list = await request(fixture.baseUrl, "/api/local/ai/threads");
    assert.equal(list.response.status, 200);
    assert.equal(list.body.threads.some((thread) => thread.id === threadId), true);

    const unknownQuery = await request(fixture.baseUrl, "/api/local/ai/threads?projectId=local");
    assert.equal(unknownQuery.response.status, 400);
    assert.equal(unknownQuery.body.error.code, "UNKNOWN_QUERY_PARAMETER");

    const updated = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}`, {
      method: "PATCH",
      body: { title: "Renamed", sandbox: "workspace-write" },
    });
    assert.equal(updated.response.status, 200);
    assert.equal(updated.body.thread.title, "Renamed");

    const interruptedMissing = await request(fixture.baseUrl, "/api/local/ai/runs/missing/interrupt", {
      method: "POST",
    });
    assert.equal(interruptedMissing.response.status, 404);

    const removed = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}`, {
      method: "DELETE",
    });
    assert.equal(removed.response.status, 204);
    const missing = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}`);
    assert.equal(missing.response.status, 404);
  } finally {
    await fixture.close();
  }
});

test("local AI routes reject private-LAN clients while ordinary API routes remain available", async (context) => {
  const address = privateLanAddress();
  if (!address) {
    context.skip("No private LAN interface is available");
    return;
  }
  const fixture = await createServerFixture("0.0.0.0");
  const port = fixture.app.server.address().port;
  try {
    const projects = await requestFrom(address, port, "/api/projects");
    assert.equal(projects.status, 200);
    const metadata = await requestFrom(address, port, "/api/meta");
    assert.equal(metadata.status, 200);
    assert.equal(metadata.body.capabilities.localAiChat, false);
    const ai = await requestFrom(address, port, "/api/local/ai/threads");
    assert.equal(ai.status, 403);
    assert.equal(ai.body.error.code, "LOCAL_AI_LOOPBACK_REQUIRED");
  } finally {
    await fixture.close();
  }
});

test("AI SSE is live-only and thread snapshots remain the durable source", async () => {
  const fixture = await createServerFixture();
  try {
    const created = await request(fixture.baseUrl, "/api/local/ai/threads", {
      method: "POST",
      body: { projectId: "local" },
    });
    const threadId = created.body.thread.id;
    const controller = new AbortController();
    const response = await fetch(`${fixture.baseUrl}/api/local/ai/threads/${threadId}/events`, {
      signal: controller.signal,
    });
    assert.equal(response.status, 200);
    const reader = response.body.getReader();
    let connected = "";
    while (!connected.includes("event: ai.event")) {
      const chunk = await reader.read();
      assert.equal(chunk.done, false);
      connected += new TextDecoder().decode(chunk.value);
    }
    assert.match(connected, /connected/);
    const turn = await request(fixture.baseUrl, `/api/local/ai/threads/${threadId}/turns`, {
      method: "POST",
      body: { message: "hello" },
    });
    assert.equal(turn.response.status, 202);
    let streamed = "";
    while (!streamed.includes("ai.event")) {
      const chunk = await reader.read();
      assert.equal(chunk.done, false);
      streamed += new TextDecoder().decode(chunk.value);
    }
    assert.match(streamed, /event: ai\.(event|run)/);
    controller.abort();
  } finally {
    await fixture.close();
  }
});

test("server close stops accepting requests before AI shutdown completes", async () => {
  const fixture = await createServerFixture();
  let appClosed = false;
  try {
    let releaseAiClose;
    const aiCloseGate = new Promise((resolve) => {
      releaseAiClose = resolve;
    });
    fixture.app.aiChat.close = () => aiCloseGate;

    const closing = fixture.app.close();
    await new Promise((resolve) => setTimeout(resolve, 20));
    const acceptedDuringClose = await fetch(`${fixture.baseUrl}/health`)
      .then(() => true, () => false);
    releaseAiClose();
    await closing;
    appClosed = true;

    assert.equal(acceptedDuringClose, false);
  } finally {
    if (appClosed) {
      await rm(fixture.directory, { recursive: true, force: true });
    } else {
      await fixture.close();
    }
  }
});
