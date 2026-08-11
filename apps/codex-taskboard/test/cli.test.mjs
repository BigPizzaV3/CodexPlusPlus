import assert from "node:assert/strict";
import test from "node:test";

import { main, parseArgs } from "../cli/taskctl.mjs";

function capture() {
  let value = "";
  return {
    stream: { write(chunk) { value += chunk; } },
    json() { return JSON.parse(value); },
  };
}

function response(payload, status = 200) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: { "content-type": "application/json" },
  });
}

async function run(argv, fetchImplementation, overrides = {}) {
  const stdout = capture();
  const stderr = capture();
  const exitCode = await main(argv, {
    fetch: fetchImplementation,
    stdout: stdout.stream,
    stderr: stderr.stream,
    env: { CODEX_THREAD_ID: "thread-current" },
    ...overrides,
  });
  return {
    exitCode,
    stdout: exitCode === 0 ? stdout.json() : null,
    stderr: exitCode === 0 ? null : stderr.json(),
  };
}

test("parseArgs supports equals syntax and boolean --json", () => {
  assert.deepEqual(parseArgs(["issue", "list", "--project=local", "--json"]), {
    resource: "issue",
    action: "list",
    operands: [],
    options: { project: "local", json: true },
  });
});

test("project list uses the default local service and adds schemaVersion", async () => {
  const calls = [];
  const result = await run(["project", "list"], async (url, init) => {
    calls.push({ url: url.toString(), init });
    return response({ projects: [{ id: "local", name: "Local" }] });
  });

  assert.equal(result.exitCode, 0);
  assert.deepEqual(result.stdout, {
    projects: [{ id: "local", name: "Local" }],
    schemaVersion: 2,
  });
  assert.equal(calls[0].url, "http://127.0.0.1:47823/api/projects");
  assert.equal(calls[0].init.method, "GET");
  assert.equal(calls[0].init.headers["x-taskboard-client"], "taskctl");
});

test("CODEX_TASKBOARD_URL overrides the service origin", async () => {
  let requestedUrl;
  const result = await run(
    ["project", "list", "--json"],
    async (url) => {
      requestedUrl = url;
      return response({ projects: [] });
    },
    { env: { CODEX_TASKBOARD_URL: "https://tasks.example.test/" } },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(requestedUrl.toString(), "https://tasks.example.test/api/projects");
});

test("project create sends id, name, and an absolute workspace path", async () => {
  let requestBody;
  const result = await run(
    ["project", "create", "--id", "docs", "--name", "Docs", "--workspace-path", "./docs"],
    async (_url, init) => {
      requestBody = JSON.parse(init.body);
      return response({ project: { id: "docs", name: "Docs" } }, 201);
    },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(requestBody.id, "docs");
  assert.equal(requestBody.name, "Docs");
  assert.equal(requestBody.workspacePath.endsWith("/docs"), true);
});

test("issue list serializes project and status filters", async () => {
  let requestedUrl;
  const result = await run(
    ["issue", "list", "--project", "local", "--status", "todo"],
    async (url) => {
      requestedUrl = url;
      return response({ tasks: [] });
    },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(requestedUrl.searchParams.get("projectId"), "local");
  assert.equal(requestedUrl.searchParams.get("status"), "todo");
});

test("issue commands accept in-review, blocked, and canceled statuses", async () => {
  for (const status of ["in_review", "blocked", "canceled"]) {
    let requestedUrl;
    const listResult = await run(["issue", "list", "--status", status], async (url) => {
      requestedUrl = url;
      return response({ tasks: [] });
    });
    assert.equal(listResult.exitCode, 0);
    assert.equal(requestedUrl.searchParams.get("status"), status);
  }

  let createBody;
  const createResult = await run(
    ["issue", "create", "--project", "local", "--title", "Review me", "--status", "in_review"],
    async (_url, init) => {
      createBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", ...createBody, version: 1 } }, 201);
    },
  );
  assert.equal(createResult.exitCode, 0);
  assert.equal(createBody.status, "in_review");

  let moveBody;
  const moveResult = await run(
    ["issue", "move", "TASK-1", "--status", "blocked", "--if-version", "1"],
    async (_url, init) => {
      moveBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", status: "blocked", version: 2 } });
    },
  );
  assert.equal(moveResult.exitCode, 0);
  assert.equal(moveBody.status, "blocked");

  let updateBody;
  const updateResult = await run(
    ["issue", "update", "TASK-1", "--status", "canceled", "--if-version", "2"],
    async (_url, init) => {
      updateBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", status: "canceled", version: 3 } });
    },
  );
  assert.equal(updateResult.exitCode, 0);
  assert.equal(updateBody.status, "canceled");
});

test("invalid status errors list every accepted status", async () => {
  const result = await run(
    ["issue", "list", "--status", "started"],
    async () => assert.fail("fetch should not be called"),
  );

  assert.equal(result.exitCode, 2);
  assert.match(result.stderr.error.message, /in_review/);
  assert.match(result.stderr.error.message, /blocked/);
  assert.match(result.stderr.error.message, /canceled/);
});

test("issue create reads a description file and parses labels", async () => {
  let requestBody;
  const result = await run(
    [
      "issue",
      "create",
      "--project",
      "local",
      "--title",
      "Fix auth",
      "--description-file",
      "issue.md",
      "--labels",
      "bug, auth,bug",
    ],
    async (_url, init) => {
      requestBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", ...requestBody, version: 1 } }, 201);
    },
    { readFile: async () => "Acceptance criteria" },
  );

  assert.equal(result.exitCode, 0);
  assert.deepEqual(requestBody, {
    projectId: "local",
    title: "Fix auth",
    description: "Acceptance criteria",
    status: "backlog",
    priority: "none",
    labels: ["bug", "auth"],
    threadId: "thread-current",
  });
});

test("issue update sends an explicit optimistic concurrency version", async () => {
  const calls = [];
  const result = await run(
    ["issue", "update", "TASK/1", "--title", "New title", "--if-version", "7"],
    async (url, init) => {
      calls.push({ url, init });
      return response({ task: { id: "TASK/1", title: "New title", version: 8 } });
    },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(calls.length, 1);
  assert.equal(calls[0].url.pathname, "/api/tasks/TASK%2F1");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    title: "New title",
    threadId: "thread-current",
    version: 7,
  });
});

test("issue update binds one worktree context", async () => {
  let requestBody;
  const result = await run(
    [
      "issue", "update", "TASK-1",
      "--worktree-path", "../taskboard-worktree",
      "--worktree-branch", "worktree/taskboard",
      "--if-version", "4",
    ],
    async (_url, init) => {
      requestBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", ...requestBody, version: 5 } });
    },
    { cwd: "/work/repo" },
  );

  assert.equal(result.exitCode, 0);
  assert.deepEqual(requestBody, {
    developmentContext: {
      type: "worktree",
      path: "/work/taskboard-worktree",
      branch: "worktree/taskboard",
    },
    threadId: "thread-current",
    version: 4,
  });
});

test("issue update rejects simultaneous branch and worktree bindings", async () => {
  let called = false;
  const result = await run(
    ["issue", "update", "TASK-1", "--git-branch", "feature/taskboard", "--worktree-path", "../taskboard-worktree"],
    async () => {
      called = true;
      return response({});
    },
    { cwd: "/work/repo" },
  );
  assert.equal(result.exitCode, 2);
  assert.equal(called, false);
  assert.match(result.stderr.error.message, /either --git-branch or --worktree-path/);
});

test("issue move fetches the current version when --if-version is omitted", async () => {
  const calls = [];
  const result = await run(["issue", "move", "TASK-1", "--status", "done"], async (url, init) => {
    calls.push({ url, init });
    if (init.method === "GET") return response({ task: { id: "TASK-1", version: 3 } });
    return response({ task: { id: "TASK-1", status: "done", version: 4 } });
  });

  assert.equal(result.exitCode, 0);
  assert.equal(calls.length, 2);
  assert.deepEqual(JSON.parse(calls[1].init.body), {
    status: "done",
    threadId: "thread-current",
    version: 3,
  });
});

test("an explicit --thread-id overrides CODEX_THREAD_ID on issue writes", async () => {
  let requestBody;
  const result = await run(
    ["issue", "update", "TASK-1", "--title", "Attributed", "--thread-id", "thread-9", "--if-version", "2"],
    async (_url, init) => {
      requestBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", threadId: "thread-9", version: 3 } });
    },
  );

  assert.equal(result.exitCode, 0);
  assert.deepEqual(requestBody, { title: "Attributed", threadId: "thread-9", version: 2 });
  assert.equal(result.stdout.task.threadId, "thread-9");
});

test("issue restore uses the mutation thread and optimistic version", async () => {
  let requestBody;
  const result = await run(
    ["issue", "restore", "TASK-1", "--if-version", "5"],
    async (url, init) => {
      assert.equal(url.pathname, "/api/tasks/TASK-1/restore");
      requestBody = JSON.parse(init.body);
      return response({ task: { id: "TASK-1", threadId: "thread-current", version: 6 } });
    },
  );
  assert.equal(result.exitCode, 0);
  assert.deepEqual(requestBody, { threadId: "thread-current", version: 5 });
});

test("issue relation add and remove use typed relation endpoints", async () => {
  const calls = [];
  const addResult = await run(
    [
      "issue", "relation", "add", "TASK/1",
      "--type", "blocked_by",
      "--issue", "TASK/2",
      "--if-version", "4",
    ],
    async (url, init) => {
      calls.push({ url, init });
      return response({
        task: { id: "TASK/1", version: 5 },
        relatedTask: { id: "TASK/2", version: 2 },
      });
    },
  );
  const removeResult = await run(
    [
      "issue", "relation", "remove", "TASK/1",
      "--type", "related",
      "--issue", "TASK/3",
      "--if-version", "5",
    ],
    async (url, init) => {
      calls.push({ url, init });
      return response({
        task: { id: "TASK/1", version: 6 },
        relatedTask: { id: "TASK/3", version: 1 },
      });
    },
  );

  assert.equal(addResult.exitCode, 0);
  assert.equal(removeResult.exitCode, 0);
  assert.equal(calls[0].url.pathname, "/api/tasks/TASK%2F1/relations/blocked_by/TASK%2F2");
  assert.equal(calls[0].init.method, "POST");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    threadId: "thread-current",
    version: 4,
  });
  assert.equal(calls[1].url.pathname, "/api/tasks/TASK%2F1/relations/related/TASK%2F3");
  assert.equal(calls[1].init.method, "DELETE");
  assert.deepEqual(JSON.parse(calls[1].init.body), {
    threadId: "thread-current",
    version: 5,
  });
});

test("issue relation validates its action and relation type before fetching", async () => {
  for (const argv of [
    ["issue", "relation", "replace", "TASK-1", "--type", "related", "--issue", "TASK-2"],
    ["issue", "relation", "add", "TASK-1", "--type", "duplicate", "--issue", "TASK-2"],
    ["issue", "relation", "add", "TASK-1", "--type", "related"],
  ]) {
    const result = await run(argv, async () => assert.fail("fetch should not be called"));
    assert.equal(result.exitCode, 2);
    assert.equal(result.stderr.error.code, "USAGE_ERROR");
  }
});

test("comment list and add use the issue comments endpoint", async () => {
  const calls = [];
  const listResult = await run(["comment", "list", "TASK/1"], async (url, init) => {
    calls.push({ url, init });
    return response({ comments: [] });
  });
  const addResult = await run(
    ["comment", "add", "TASK/1", "--body", "Verified locally"],
    async (url, init) => {
      calls.push({ url, init });
      return response({ comment: { id: "comment-1", body: "Verified locally", version: 1 } }, 201);
    },
  );

  assert.equal(listResult.exitCode, 0);
  assert.equal(addResult.exitCode, 0);
  assert.equal(calls[0].url.pathname, "/api/tasks/TASK%2F1/comments");
  assert.equal(calls[0].init.method, "GET");
  assert.equal(calls[1].url.pathname, "/api/tasks/TASK%2F1/comments");
  assert.deepEqual(JSON.parse(calls[1].init.body), {
    body: "Verified locally",
    threadId: "thread-current",
  });
});

test("comment update and delete require an explicit version", async () => {
  const calls = [];
  const updateResult = await run(
    ["comment", "update", "comment/1", "--body", "Updated", "--if-version", "3"],
    async (url, init) => {
      calls.push({ url, init });
      return response({ comment: { id: "comment/1", body: "Updated", version: 4 } });
    },
  );
  const deleteResult = await run(
    ["comment", "delete", "comment/1", "--if-version", "4"],
    async (url, init) => {
      calls.push({ url, init });
      return response({});
    },
  );

  assert.equal(updateResult.exitCode, 0);
  assert.equal(deleteResult.exitCode, 0);
  assert.equal(calls[0].url.pathname, "/api/comments/comment%2F1");
  assert.deepEqual(JSON.parse(calls[0].init.body), {
    body: "Updated",
    threadId: "thread-current",
    version: 3,
  });
  assert.equal(calls[1].init.method, "DELETE");
  assert.deepEqual(JSON.parse(calls[1].init.body), { threadId: "thread-current", version: 4 });

  const missingVersion = await run(
    ["comment", "delete", "comment-1"],
    async () => assert.fail("fetch should not be called"),
  );
  assert.equal(missingVersion.exitCode, 2);
  assert.match(missingVersion.stderr.error.message, /--if-version/);
});

test("context current selects the project with the most specific matching workspace", async () => {
  const result = await run(
    ["context", "current", "--cwd", "/work/repo/packages/app"],
    async () => response({ projects: [
      { id: "local", name: "Local", workspacePath: null },
      { id: "repo", workspacePath: "/work/repo" },
      { id: "app", workspacePath: "/work/repo/packages/app" },
    ] }),
    { cwd: "/unused" },
  );

  assert.equal(result.exitCode, 0);
  assert.equal(result.stdout.cwd, "/work/repo/packages/app");
  assert.deepEqual(result.stdout.project, { id: "app", workspacePath: "/work/repo/packages/app" });
});

test("context current falls back to the local project", async () => {
  const result = await run(
    ["context", "current", "--cwd", "/unmatched"],
    async () => response({ projects: [
      { id: "other", workspacePath: "/work/other" },
      { id: "local", name: "Local", workspacePath: null },
    ] }),
  );

  assert.equal(result.exitCode, 0);
  assert.equal(result.stdout.project.id, "local");
});

test("issue and comment writes require Codex conversation attribution", async () => {
  const issueResult = await run(
    ["issue", "update", "TASK-1", "--title", "No attribution", "--if-version", "1"],
    async () => assert.fail("fetch should not be called"),
    { env: {} },
  );
  assert.equal(issueResult.exitCode, 2);
  assert.match(issueResult.stderr.error.message, /--thread-id or CODEX_THREAD_ID/);

  const commentResult = await run(
    ["comment", "add", "TASK-1", "--body", "No attribution"],
    async () => assert.fail("fetch should not be called"),
    { env: {} },
  );
  assert.equal(commentResult.exitCode, 2);
  assert.match(commentResult.stderr.error.message, /--thread-id or CODEX_THREAD_ID/);
});

test("manual linked-thread options and commands are no longer accepted", async () => {
  const optionResult = await run(
    ["issue", "update", "TASK-1", "--title", "Invalid", "--linked-thread-id", "thread-1"],
    async () => assert.fail("fetch should not be called"),
  );
  assert.equal(optionResult.exitCode, 2);
  assert.match(optionResult.stderr.error.message, /Unknown option --linked-thread-id/);

  const commandResult = await run(
    ["issue", "link-thread", "TASK-1", "--thread-id", "thread-1"],
    async () => assert.fail("fetch should not be called"),
  );
  assert.equal(commandResult.exitCode, 2);
  assert.match(commandResult.stderr.error.message, /Expected one of/);
});

test("API conflicts produce stable JSON on stderr and exit code 5", async () => {
  const result = await run(
    ["issue", "archive", "TASK-1", "--if-version", "1"],
    async () => response({ error: { code: "VERSION_CONFLICT", message: "Task changed" } }, 409),
  );

  assert.equal(result.exitCode, 5);
  assert.deepEqual(result.stderr, {
    schemaVersion: 2,
    error: { code: "VERSION_CONFLICT", message: "Task changed" },
  });
});

test("usage errors are stable and never call the service", async () => {
  const result = await run(
    ["issue", "create", "--project", "local"],
    async () => assert.fail("fetch should not be called"),
  );

  assert.equal(result.exitCode, 2);
  assert.equal(result.stderr.error.code, "USAGE_ERROR");
  assert.match(result.stderr.error.message, /--title/);
});
