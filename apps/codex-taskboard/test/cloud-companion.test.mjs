import assert from "node:assert/strict";
import { mkdtemp, readFile, rm, stat } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import test from "node:test";

import { main } from "../cli/taskctl.mjs";
import { createTaskboardServer } from "../server/index.mjs";

const temporaryDirectories = [];

test.afterEach(async () => {
  while (temporaryDirectories.length > 0) {
    await rm(temporaryDirectories.pop(), { recursive: true, force: true });
  }
});

function capture() {
  let value = "";
  return {
    stream: { write(chunk) { value += chunk; } },
    text() { return value; },
    json() { return JSON.parse(value); },
  };
}

function jsonResponse(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function runCli(argv, overrides = {}) {
  const stdout = capture();
  const stderr = capture();
  const exitCode = await main(argv, {
    stdout: stdout.stream,
    stderr: stderr.stream,
    env: {},
    ...overrides,
  });
  return { exitCode, stdout, stderr };
}

async function temporaryConfigPath(name) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "taskboard-companion-"));
  temporaryDirectories.push(directory);
  return path.join(directory, name);
}

async function importCloudConfig() {
  return import("../server/cloud-config.mjs");
}

async function importCloudProxy() {
  return import("../server/cloud-proxy.mjs");
}

function memoryConfigStore(overrides = {}) {
  const state = {
    version: 1,
    remoteUrl: "https://tasks.example.test",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
    projectMappings: {},
    ...overrides,
  };
  return {
    state,
    async read() {
      return structuredClone(state);
    },
    async setProjectWorkspace(projectId, workspacePath) {
      state.projectMappings[projectId] = workspacePath;
      return structuredClone(state);
    },
  };
}

function firstLanAddress() {
  return Object.values(os.networkInterfaces())
    .flat()
    .find((entry) => entry?.family === "IPv4" && !entry.internal)?.address ?? null;
}

test("cloud config persists Basic Auth credentials and device mappings in a mode-0600 file", async () => {
  const { createCloudConfigStore } = await importCloudConfig();
  const configPath = await temporaryConfigPath("companion.json");
  const store = createCloudConfigStore({ configPath });

  await store.configure({
    remoteUrl: "https://tasks.example.test/",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
  });
  await store.setProjectWorkspace("portfolio", "/Users/alice/Documents/portfolio");

  assert.deepEqual(await store.read(), {
    version: 1,
    remoteUrl: "https://tasks.example.test",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
    projectMappings: {
      portfolio: "/Users/alice/Documents/portfolio",
    },
  });
  assert.equal((await stat(configPath)).mode & 0o777, 0o600);
  assert.deepEqual(JSON.parse(await readFile(configPath, "utf8")), await store.read());
});

test("cloud config only accepts HTTPS origins, except loopback HTTP used in development", async () => {
  const { createCloudConfigStore } = await importCloudConfig();

  for (const remoteUrl of [
    "http://tasks.example.test",
    "ftp://tasks.example.test",
    "https://tasks.example.test/path",
  ]) {
    const store = createCloudConfigStore({
      configPath: await temporaryConfigPath("invalid.json"),
    });
    await assert.rejects(
      store.configure({ remoteUrl, actorName: "Alice", sharedKey: "shared-key" }),
      (error) => error?.code === "INVALID_CLOUD_URL",
      remoteUrl,
    );
  }

  for (const remoteUrl of [
    "https://tasks.example.test",
    "http://127.0.0.1:8787",
    "http://localhost:8787",
  ]) {
    const store = createCloudConfigStore({
      configPath: await temporaryConfigPath("valid.json"),
    });
    await store.configure({ remoteUrl, actorName: "Alice", sharedKey: "shared-key" });
    assert.equal((await store.read()).remoteUrl, remoteUrl);
  }
});

test("cloud proxy replaces client identity with Basic Auth and makes exactly one upstream request", async () => {
  const { createCloudProxy } = await importCloudProxy();
  const calls = [];
  const configStore = memoryConfigStore();
  const proxy = createCloudProxy({
    configStore,
    fetch: async (url, init) => {
      calls.push({ url: url.toString(), init });
      return jsonResponse({
        task: {
          id: "REMOTE-1",
          creatorType: "agent",
          creatorId: "codex-agent",
          creatorName: "Codex Agent",
        },
      }, 201);
    },
  });

  const response = await proxy.forward(new Request(
    "http://127.0.0.1:47823/api/tasks?source=taskctl",
    {
      method: "POST",
      headers: {
        authorization: "Bearer client-supplied-token",
        "content-type": "application/json",
        "x-taskboard-client": "taskctl",
        "x-taskboard-user-id": "spoofed-user",
        "x-taskboard-user-name": "Spoofed User",
      },
      body: JSON.stringify({
        projectId: "portfolio",
        title: "Cloud issue",
        threadId: "thread-cloud",
      }),
    },
  ));

  assert.equal(response.status, 201);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url, "https://tasks.example.test/api/tasks?source=taskctl");
  assert.equal(calls[0].init.method, "POST");
  assert.equal(
    new Headers(calls[0].init.headers).get("authorization"),
    `Basic ${Buffer.from("Alice:two-person-shared-key").toString("base64")}`,
  );
  assert.equal(new Headers(calls[0].init.headers).get("x-taskboard-client"), "taskctl");
  assert.equal(new Headers(calls[0].init.headers).has("x-taskboard-user-id"), false);
  assert.equal(new Headers(calls[0].init.headers).has("x-taskboard-user-name"), false);
  assert.deepEqual(await response.json(), {
    task: {
      id: "REMOTE-1",
      creatorType: "agent",
      creatorId: "codex-agent",
      creatorName: "Codex Agent",
    },
  });
});

test("configured cloud mode fails explicitly and never falls back to the local database", async () => {
  const { createCloudProxy } = await importCloudProxy();
  let upstreamCalls = 0;
  let localFallbackCalls = 0;
  const proxy = createCloudProxy({
    configStore: memoryConfigStore(),
    fetch: async () => {
      upstreamCalls += 1;
      throw new Error("network unavailable");
    },
    localFallback: async () => {
      localFallbackCalls += 1;
      return jsonResponse({ projects: [{ id: "local" }] });
    },
  });

  await assert.rejects(
    proxy.forward(new Request("http://127.0.0.1:47823/api/projects")),
    (error) => error?.code === "REMOTE_UNAVAILABLE",
  );
  assert.equal(upstreamCalls, 1);
  assert.equal(localFallbackCalls, 0);
});

test("cloud proxy preserves upstream 401 responses and binary attachment streams", async () => {
  const { createCloudProxy } = await importCloudProxy();
  const unauthorized = new Response("invalid shared key", {
    status: 401,
    headers: { "content-type": "text/plain" },
  });
  const responses = [
    unauthorized,
    new Response(Uint8Array.from([0, 1, 2, 255]), {
      headers: { "content-type": "application/octet-stream" },
    }),
  ];
  const proxy = createCloudProxy({
    configStore: memoryConfigStore(),
    fetch: async () => responses.shift(),
  });

  const authResponse = await proxy.forward(
    new Request("http://127.0.0.1:47823/api/projects"),
  );
  assert.equal(authResponse, unauthorized);
  assert.equal(await authResponse.text(), "invalid shared key");

  const attachmentResponse = await proxy.forward(
    new Request("http://127.0.0.1:47823/api/attachments/file/content"),
  );
  assert.deepEqual(
    [...new Uint8Array(await attachmentResponse.arrayBuffer())],
    [0, 1, 2, 255],
  );
});

test("cloud routing keeps machine-specific capability endpoints in the local companion", async () => {
  const { isLocalCompanionRoute } = await importCloudProxy();

  for (const pathname of [
    "/health",
    "/api/meta",
    "/api/device-workspaces",
    "/api/workflow-capabilities",
    "/api/projects/portfolio/development-contexts",
    "/api/local/cloud-session",
    "/api/local/project-mappings/portfolio",
  ]) {
    assert.equal(isLocalCompanionRoute(pathname), true, pathname);
  }

  for (const pathname of [
    "/api/projects",
    "/api/projects/portfolio/workflow-workspace",
    "/api/tasks",
    "/api/tasks/PORTFOLIO-1",
    "/api/comments/comment-1",
    "/api/attachments/attachment-1",
    "/api/events",
  ]) {
    assert.equal(isLocalCompanionRoute(pathname), false, pathname);
  }
});

test("project creation stores workspacePath locally and never sends it to cloud", async () => {
  const { createCloudProxy } = await importCloudProxy();
  const configStore = memoryConfigStore();
  let upstreamBody;
  let upstreamCalls = 0;
  const proxy = createCloudProxy({
    configStore,
    fetch: async (_url, init) => {
      upstreamCalls += 1;
      upstreamBody = JSON.parse(init.body);
      return jsonResponse({
        project: {
          id: "portfolio",
          name: "Portfolio",
          issueCount: 0,
        },
      }, 201);
    },
  });

  const response = await proxy.forward(new Request(
    "http://127.0.0.1:47823/api/projects",
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        id: "portfolio",
        name: "Portfolio",
        workspacePath: "/Users/alice/Documents/portfolio",
      }),
    },
  ));

  assert.equal(upstreamCalls, 1);
  assert.deepEqual(upstreamBody, {
    id: "portfolio",
    name: "Portfolio",
  });
  assert.equal(
    configStore.state.projectMappings.portfolio,
    "/Users/alice/Documents/portfolio",
  );
  assert.equal(
    (await response.json()).project.workspacePath,
    "/Users/alice/Documents/portfolio",
  );
});

test("project lists overlay this device's workspace mappings and discard remote paths", async () => {
  const { createCloudProxy } = await importCloudProxy();
  const configStore = memoryConfigStore({
    projectMappings: {
      portfolio: "/Users/alice/Documents/portfolio",
    },
  });
  const proxy = createCloudProxy({
    configStore,
    fetch: async () => jsonResponse({
      projects: [
        { id: "portfolio", name: "Portfolio", workspacePath: "/server/path" },
        { id: "shared", name: "Shared", workspacePath: "/another/server/path" },
      ],
    }),
  });

  const response = await proxy.forward(
    new Request("http://127.0.0.1:47823/api/projects"),
  );
  assert.deepEqual((await response.json()).projects, [
    {
      id: "portfolio",
      name: "Portfolio",
      workspacePath: "/Users/alice/Documents/portfolio",
    },
    {
      id: "shared",
      name: "Shared",
      workspacePath: null,
    },
  ]);
});

test("task mutations do not send absolute worktree paths to cloud", async () => {
  const { createCloudProxy } = await importCloudProxy();
  let upstreamBody;
  const proxy = createCloudProxy({
    configStore: memoryConfigStore(),
    fetch: async (_url, init) => {
      upstreamBody = JSON.parse(init.body);
      return jsonResponse({ task: { id: "PORTFOLIO-1" } });
    },
  });

  await proxy.forward(new Request(
    "http://127.0.0.1:47823/api/tasks/PORTFOLIO-1",
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        version: 3,
        developmentContext: {
          type: "worktree",
          path: "/Users/alice/.codex/worktrees/portfolio-1",
          branch: "feature/portfolio-1",
        },
      }),
    },
  ));

  assert.deepEqual(upstreamBody, {
    version: 3,
    developmentContext: {
      type: "worktree",
      branch: "feature/portfolio-1",
    },
  });
});

test("workflow mutations remove structured git worktree paths without altering other path fields or text", async () => {
  const { createCloudProxy } = await importCloudProxy();
  let upstreamBody;
  const proxy = createCloudProxy({
    configStore: memoryConfigStore(),
    fetch: async (_url, init) => {
      upstreamBody = JSON.parse(init.body);
      return jsonResponse({ workflow: { projectId: "portfolio", version: 5 } });
    },
  });

  await proxy.forward(new Request(
    "http://127.0.0.1:47823/api/projects/portfolio/workflow-workspace",
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        version: 4,
        workspace: {
          version: 1,
          tabs: [{ id: "delivery", name: "Delivery" }],
          activeWorkflowId: "delivery",
          snapshots: {
            delivery: {
              nodes: [{
                id: "git",
                data: {
                  kind: "git",
                  branch: "feature/cloud",
                  gitWorktreePath: "/Users/alice/.codex/worktrees/cloud",
                  description: "Keep this text: /Users/alice/.codex/worktrees/cloud",
                  config: { path: "/third-party/runtime/path" },
                  planItems: [{
                    title: "Nested Git step",
                    gitWorktreePath: "/Users/alice/.codex/worktrees/nested",
                    path: "/third-party/nested/path",
                  }],
                },
              }],
              flow: { version: 2, root: { items: [] } },
              selectedNodeId: "git",
            },
          },
        },
      }),
    },
  ));

  const data = upstreamBody.workspace.snapshots.delivery.nodes[0].data;
  assert.equal(Object.hasOwn(data, "gitWorktreePath"), false);
  assert.equal(Object.hasOwn(data.planItems[0], "gitWorktreePath"), false);
  assert.equal(data.branch, "feature/cloud");
  assert.equal(
    data.description,
    "Keep this text: /Users/alice/.codex/worktrees/cloud",
  );
  assert.equal(data.config.path, "/third-party/runtime/path");
  assert.equal(data.planItems[0].path, "/third-party/nested/path");
});

test("two companions map the same cloud project to different local paths", async () => {
  const { createCloudConfigStore } = await importCloudConfig();
  const alice = createCloudConfigStore({
    configPath: await temporaryConfigPath("alice.json"),
  });
  const bob = createCloudConfigStore({
    configPath: await temporaryConfigPath("bob.json"),
  });
  for (const store of [alice, bob]) {
    await store.configure({
      remoteUrl: "https://tasks.example.test",
      actorName: store === alice ? "Alice" : "Bob",
      sharedKey: "two-person-shared-key",
    });
  }
  await alice.setProjectWorkspace("portfolio", "/Users/alice/Documents/portfolio");
  await bob.setProjectWorkspace("portfolio", "/Users/bob/src/portfolio");

  assert.equal(
    (await alice.read()).projectMappings.portfolio,
    "/Users/alice/Documents/portfolio",
  );
  assert.equal(
    (await bob.read()).projectMappings.portfolio,
    "/Users/bob/src/portfolio",
  );
  assert.doesNotMatch(JSON.stringify(await alice.read()), /\/Users\/bob/);
  assert.doesNotMatch(JSON.stringify(await bob.read()), /\/Users\/alice/);
});

test("configured server proxies business APIs without touching local rows and advertises polling", async () => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "taskboard-cloud-server-"));
  temporaryDirectories.push(directory);
  const configPath = path.join(directory, "companion.json");
  const { createCloudConfigStore } = await importCloudConfig();
  await createCloudConfigStore({ configPath }).configure({
    remoteUrl: "https://tasks.example.test",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
  });
  const upstreamCalls = [];
  const app = createTaskboardServer({
    dataDirectory: directory,
    cloudConfigPath: configPath,
    remoteFetch: async (url, init) => {
      upstreamCalls.push({ url: url.toString(), init });
      return jsonResponse({ tasks: [{ id: "REMOTE-1", projectId: "portfolio" }] });
    },
  });
  const address = await app.listen({ port: 0 });
  const baseUrl = `http://127.0.0.1:${address.port}`;

  try {
    const metadata = await fetch(`${baseUrl}/api/meta`).then((response) => response.json());
    assert.deepEqual(metadata, {
      mode: "cloud",
      realtime: { transport: "poll", intervalMs: 2000 },
      localCapabilities: { available: true },
      manageTaskboardSkillPath: app.options.skillPath,
    });
    const session = await fetch(`${baseUrl}/api/local/cloud-session`)
      .then((response) => response.json());
    assert.equal(Object.hasOwn(session, "sharedKey"), false);

    const response = await fetch(`${baseUrl}/api/tasks`);
    assert.equal(response.status, 200);
    assert.equal(upstreamCalls.length, 1);
    assert.equal(upstreamCalls[0].url, "https://tasks.example.test/api/tasks");
    assert.equal(app.database.listTasks({}).length, 0);
  } finally {
    await app.close();
  }
});

test("cloud mode exposes machine capabilities only to loopback while local mode keeps LAN access", async (t) => {
  const lanAddress = firstLanAddress();
  if (!lanAddress) {
    t.skip("No non-loopback IPv4 interface is available");
    return;
  }
  const directory = await mkdtemp(path.join(os.tmpdir(), "taskboard-cloud-lan-"));
  temporaryDirectories.push(directory);
  const configPath = path.join(directory, "companion.json");
  const { createCloudConfigStore } = await importCloudConfig();
  const store = createCloudConfigStore({ configPath });
  await store.configure({
    remoteUrl: "https://tasks.example.test",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
  });
  let upstreamCalls = 0;
  const app = createTaskboardServer({
    dataDirectory: directory,
    cloudConfigPath: configPath,
    remoteFetch: async () => {
      upstreamCalls += 1;
      return jsonResponse({ projects: [] });
    },
  });
  const address = await app.listen({ host: "0.0.0.0", port: 0 });
  const lanBaseUrl = `http://${lanAddress}:${address.port}`;

  try {
    for (const pathname of [
      "/api/meta",
      "/api/device-workspaces",
      "/api/workflow-capabilities",
      "/api/projects/portfolio/development-contexts",
    ]) {
      const response = await fetch(`${lanBaseUrl}${pathname}`);
      assert.equal(response.status, 403, pathname);
      assert.equal((await response.json()).error.code, "LOCAL_ONLY", pathname);
    }
    const projectResponse = await fetch(`${lanBaseUrl}/api/projects`);
    assert.equal(projectResponse.status, 403);
    assert.equal((await projectResponse.json()).error.code, "LOCAL_ONLY");
    const taskResponse = await fetch(`${lanBaseUrl}/api/tasks`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ projectId: "portfolio", title: "Must not proxy" }),
    });
    assert.equal(taskResponse.status, 403);
    assert.equal((await taskResponse.json()).error.code, "LOCAL_ONLY");
    assert.equal(upstreamCalls, 0);

    await store.clearCloud();
    const localResponse = await fetch(`${lanBaseUrl}/api/device-workspaces`);
    assert.equal(localResponse.status, 200);
    const localProjects = await fetch(`${lanBaseUrl}/api/projects`);
    assert.equal(localProjects.status, 200);
  } finally {
    await app.close();
  }
});

test("taskctl cloud login reads the shared key privately and sends it to the local companion", async () => {
  const fetchCalls = [];
  let secretReads = 0;
  let execCalled = false;
  const result = await runCli(
    [
      "cloud",
      "login",
      "--url",
      "https://tasks.example.test/",
      "--actor-name",
      "Alice",
    ],
    {
      execFile: async () => {
        execCalled = true;
        assert.fail("Basic Auth login must not run cloudflared");
      },
      readSecret: async () => {
        secretReads += 1;
        return "two-person-shared-key";
      },
      fetch: async (url, init) => {
        fetchCalls.push({ url: url.toString(), init });
        return jsonResponse({
          mode: "cloud",
          remoteUrl: "https://tasks.example.test",
          actorName: "Alice",
          authenticated: true,
        });
      },
    },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(secretReads, 1);
  assert.equal(execCalled, false);
  assert.equal(fetchCalls.length, 1);
  assert.equal(fetchCalls[0].url, "http://127.0.0.1:47823/api/local/cloud-session");
  assert.equal(fetchCalls[0].init.method, "PUT");
  assert.deepEqual(JSON.parse(fetchCalls[0].init.body), {
    remoteUrl: "https://tasks.example.test",
    actorName: "Alice",
    sharedKey: "two-person-shared-key",
  });
  assert.equal(result.stdout.text().includes("two-person-shared-key"), false);
  assert.equal(result.stderr.text().includes("two-person-shared-key"), false);
});

test("taskctl cloud status, logout, and project map use local companion endpoints", async () => {
  const calls = [];
  const fetchImplementation = async (url, init) => {
    calls.push({ url: url.toString(), init });
    if (url.pathname === "/api/local/cloud-session" && init.method === "GET") {
      return jsonResponse({
        mode: "cloud",
        remoteUrl: "https://tasks.example.test",
        actorName: "Alice",
        authenticated: true,
      });
    }
    if (url.pathname === "/api/local/cloud-session" && init.method === "DELETE") {
      return jsonResponse({ mode: "local", authenticated: false });
    }
    if (url.pathname === "/api/local/project-mappings/portfolio" && init.method === "PUT") {
      return jsonResponse({
        projectId: "portfolio",
        workspacePath: "/work/portfolio",
      });
    }
    return jsonResponse({ error: { code: "UNEXPECTED", message: url.toString() } }, 500);
  };

  const companionEnv = {
    CODEX_TASKBOARD_COMPANION_URL: "http://127.0.0.1:49000",
  };
  assert.equal((await runCli(
    ["cloud", "status"],
    { fetch: fetchImplementation, env: companionEnv },
  )).exitCode, 0);
  assert.equal((await runCli(
    ["cloud", "logout"],
    { fetch: fetchImplementation, env: companionEnv },
  )).exitCode, 0);
  assert.equal((await runCli(
    ["project", "map", "portfolio", "--workspace-path", "./portfolio"],
    { fetch: fetchImplementation, cwd: "/work", env: companionEnv },
  )).exitCode, 0);

  assert.deepEqual(calls.map(({ url, init }) => [url, init.method]), [
    ["http://127.0.0.1:49000/api/local/cloud-session", "GET"],
    ["http://127.0.0.1:49000/api/local/cloud-session", "DELETE"],
    ["http://127.0.0.1:49000/api/local/project-mappings/portfolio", "PUT"],
  ]);
  assert.deepEqual(JSON.parse(calls[2].init.body), {
    workspacePath: "/work/portfolio",
  });
});

test("taskctl accepts only loopback companion origins and supports the legacy loopback URL", async () => {
  let requestedUrl;
  const legacy = await runCli(["cloud", "status"], {
    env: { CODEX_TASKBOARD_URL: "http://localhost:49100/" },
    fetch: async (url) => {
      requestedUrl = url.toString();
      return jsonResponse({ mode: "local", authenticated: false });
    },
  });
  assert.equal(legacy.exitCode, 0);
  assert.equal(requestedUrl, "http://localhost:49100/api/local/cloud-session");

  let fetchCalled = false;
  const rejected = await runCli(["cloud", "status"], {
    env: { CODEX_TASKBOARD_COMPANION_URL: "https://tasks.example.test" },
    fetch: async () => {
      fetchCalled = true;
      return jsonResponse({});
    },
  });
  assert.equal(rejected.exitCode, 2);
  assert.equal(rejected.stderr.json().error.code, "USAGE_ERROR");
  assert.equal(fetchCalled, false);
});

test("taskctl uses an explicit companion URL for ordinary commands before the legacy URL", async () => {
  let requestedUrl;
  const result = await runCli(["project", "list"], {
    env: {
      CODEX_TASKBOARD_COMPANION_URL: "http://127.0.0.1:49200",
      CODEX_TASKBOARD_URL: "https://legacy.example.test",
    },
    fetch: async (url) => {
      requestedUrl = url.toString();
      return jsonResponse({ projects: [] });
    },
  });

  assert.equal(result.exitCode, 0);
  assert.equal(requestedUrl, "http://127.0.0.1:49200/api/projects");
});

test("without cloud configuration taskctl keeps using the local companion", async () => {
  let requestedUrl;
  let execCalled = false;
  const result = await runCli(["project", "list"], {
    env: { CODEX_THREAD_ID: "thread-local" },
    execFile: async () => {
      execCalled = true;
      assert.fail("cloud helper should not run for local commands");
    },
    fetch: async (url) => {
      requestedUrl = url.toString();
      return jsonResponse({ projects: [{ id: "local", name: "Local" }] });
    },
  });

  assert.equal(result.exitCode, 0);
  assert.equal(requestedUrl, "http://127.0.0.1:47823/api/projects");
  assert.equal(execCalled, false);
  assert.deepEqual(result.stdout.json(), {
    projects: [{ id: "local", name: "Local" }],
    schemaVersion: 2,
  });
});
