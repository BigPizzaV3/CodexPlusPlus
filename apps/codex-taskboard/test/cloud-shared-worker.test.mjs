import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import { createCloudWorkerHarness } from "./helpers/cloud-worker-harness.mjs";

let cloud;
const alice = "Alice";
const bob = "Bob";

before(async () => {
  cloud = await createCloudWorkerHarness();
});

after(async () => {
  await cloud?.dispose();
});

async function createProject(id, actorName = alice) {
  return cloud.request("/api/projects", {
    method: "POST",
    actorName,
    json: {
      id,
      name: id.toUpperCase(),
      workspacePath: `/Users/${actorName.toLowerCase()}/${id}`,
    },
  });
}

async function createTask(projectId, title, actorName = alice, extra = {}) {
  return cloud.request("/api/tasks", {
    method: "POST",
    actorName,
    json: {
      projectId,
      title,
      description: "",
      status: "backlog",
      priority: "none",
      labels: [],
      ...extra,
    },
  });
}

test("Basic authentication protects static assets, APIs, and attachment content", async () => {
  for (const pathname of ["/", "/api/projects", "/api/attachments/missing/content"]) {
    const missing = await cloud.request(pathname);
    assert.equal(missing.response.status, 401);
    assert.match(missing.response.headers.get("www-authenticate") ?? "", /^Basic\b/i);

    const invalid = await cloud.request(pathname, {
      actorName: alice,
      password: "wrong",
    });
    assert.equal(invalid.response.status, 401);
    assert.match(invalid.response.headers.get("www-authenticate") ?? "", /^Basic\b/i);
  }
});

test("Basic authentication requires an exact shared-secret match", async () => {
  const accepted = await cloud.request("/api/projects", { actorName: alice });
  assert.equal(accepted.response.status, 200);

  const lastCharacter = cloud.sharedSecret.at(-1);
  const sameLengthWrongSecret = `${cloud.sharedSecret.slice(0, -1)}${
    lastCharacter === "x" ? "y" : "x"
  }`;
  assert.equal(sameLengthWrongSecret.length, cloud.sharedSecret.length);
  const rejected = await cloud.request("/api/projects", {
    actorName: alice,
    password: sameLengthWrongSecret,
  });
  assert.equal(rejected.response.status, 401);
  assert.match(rejected.response.headers.get("www-authenticate") ?? "", /^Basic\b/i);
});

test("the Basic username becomes the trusted actor while the shared password grants access", async () => {
  const project = await createProject("alpha");
  assert.equal(project.response.status, 201);
  assert.equal(project.body.project.workspacePath, null);

  const userTask = await createTask("alpha", "Created in browser", alice);
  assert.equal(userTask.response.status, 201);
  assert.equal(userTask.body.task.creatorType, "user");
  assert.equal(userTask.body.task.creatorName, alice);
  assert.match(userTask.body.task.creatorId, /^basic:/);

  const agentTask = await cloud.request("/api/tasks", {
    method: "POST",
    actorName: bob,
    headers: { "x-taskboard-client": "taskctl" },
    json: {
      projectId: "alpha",
      title: "Created through taskctl",
      status: "backlog",
      priority: "none",
      labels: [],
    },
  });
  assert.equal(agentTask.response.status, 201);
  assert.equal(agentTask.body.task.creatorType, "agent");
  assert.match(agentTask.body.task.creatorName, /Codex Agent/);
  assert.match(agentTask.body.task.creatorName, /Bob/);
});

test("projects, tasks, comments, relations, and workflows preserve the current API contract", async () => {
  const parent = await createTask("alpha", "Parent");
  const child = await createTask("alpha", "Child");
  const relation = await cloud.request(
    `/api/tasks/${child.body.task.id}/relations/parent/${parent.body.task.id}`,
    {
      method: "POST",
      actorName: alice,
      json: { version: child.body.task.version },
    },
  );
  assert.equal(relation.response.status, 200);
  assert.equal(relation.body.task.relations.parent.id, parent.body.task.id);

  const comment = await cloud.request(`/api/tasks/${child.body.task.id}/comments`, {
    method: "POST",
    actorName: bob,
    json: { body: "Review note" },
  });
  assert.equal(comment.response.status, 201);
  assert.equal(comment.body.comment.authorName, bob);

  const workspace = {
    version: 1,
    tabs: [{ id: "delivery", name: "Delivery" }],
    activeWorkflowId: "delivery",
    snapshots: {
      delivery: {
        nodes: [],
        flow: { version: 2, root: { items: [] } },
        selectedNodeId: null,
      },
    },
  };
  const saved = await cloud.request("/api/projects/alpha/workflow-workspace", {
    method: "PUT",
    actorName: alice,
    json: { version: 0, workspace },
  });
  assert.equal(saved.response.status, 200);
  assert.equal(saved.body.workflow.version, 1);

  const listed = await cloud.request("/api/tasks?projectId=alpha&archived=false", {
    actorName: alice,
  });
  assert.equal(listed.response.status, 200);
  assert.ok(listed.body.tasks.some((task) => task.id === child.body.task.id));
});

test("concurrent issue creation has unique identifiers and stale writes return 409", async () => {
  const created = await Promise.all(
    Array.from({ length: 25 }, (_, index) => createTask("alpha", `Concurrent ${index}`)),
  );
  for (const result of created) assert.equal(result.response.status, 201);
  const identifiers = created.map((result) => result.body.task.identifier);
  assert.equal(new Set(identifiers).size, identifiers.length);
  const numbers = identifiers
    .map((identifier) => Number(identifier.slice(identifier.lastIndexOf("-") + 1)))
    .sort((left, right) => left - right);
  numbers.forEach((number, index) => {
    assert.equal(number, numbers[0] + index);
  });

  const task = created[0].body.task;
  const winner = await cloud.request(`/api/tasks/${task.id}`, {
    method: "PATCH",
    actorName: alice,
    json: { version: task.version, title: "Winner" },
  });
  assert.equal(winner.response.status, 200);

  const stale = await cloud.request(`/api/tasks/${task.id}`, {
    method: "PATCH",
    actorName: bob,
    json: { version: task.version, title: "Stale" },
  });
  assert.equal(stale.response.status, 409);
  assert.equal(stale.body.error.code, "VERSION_CONFLICT");
  assert.deepEqual(stale.body.error.details, {
    expectedVersion: task.version,
    actualVersion: winner.body.task.version,
  });
});

test("a failed task insert rolls back its reserved project identifier", async () => {
  await createProject("atomic-counter");
  await cloud.db.exec(
    "CREATE TRIGGER fail_task_insert BEFORE INSERT ON tasks WHEN NEW.title = 'Fail counter' BEGIN SELECT RAISE(ABORT, 'intentional task insert failure'); END;",
  );
  const failed = await createTask("atomic-counter", "Fail counter");
  assert.equal(failed.response.status, 500);

  const counter = await cloud.db.prepare(
    "SELECT next_task_number FROM projects WHERE id = 'atomic-counter'",
  ).first("next_task_number");
  assert.equal(counter, 1);

  const succeeded = await createTask("atomic-counter", "First real issue");
  assert.equal(succeeded.response.status, 201);
  assert.equal(succeeded.body.task.identifier, "ATOMICCOUNTE-1");
});

test("archived tasks are excluded from project issue counts", async () => {
  const project = await createProject("archive-count");
  assert.equal(project.response.status, 201);
  const task = await createTask("archive-count", "Archive me");
  const before = await cloud.request("/api/projects", { actorName: alice });
  assert.equal(
    before.body.projects.find((candidate) => candidate.id === "archive-count").issueCount,
    1,
  );

  const archived = await cloud.request(`/api/tasks/${task.body.task.id}/archive`, {
    method: "POST",
    actorName: alice,
    json: { version: task.body.task.version },
  });
  assert.equal(archived.response.status, 200);
  const after = await cloud.request("/api/projects", { actorName: alice });
  assert.equal(
    after.body.projects.find((candidate) => candidate.id === "archive-count").issueCount,
    0,
  );
});

test("R2 attachment upload, download, delete, and D1 failure compensation form one closed lifecycle", async () => {
  const task = await createTask("alpha", "Attachment owner");
  const uploaded = await cloud.request(`/api/tasks/${task.body.task.id}/attachments`, {
    method: "POST",
    actorName: alice,
    headers: {
      "content-type": "text/plain",
      "x-taskboard-filename": encodeURIComponent("evidence.txt"),
    },
    body: "attachment body",
  });
  assert.equal(uploaded.response.status, 201);
  const attachment = uploaded.body.attachment;
  const downloaded = await cloud.request(`/api/attachments/${attachment.id}/content`, {
    actorName: bob,
  });
  assert.equal(downloaded.response.status, 200);
  assert.equal(downloaded.body, "attachment body");

  const deleted = await cloud.request(`/api/attachments/${attachment.id}`, {
    method: "DELETE",
    actorName: alice,
  });
  assert.equal(deleted.response.status, 204);
  assert.equal((await cloud.listAttachmentKeys()).length, 0);

  await cloud.db.exec(
    "CREATE TRIGGER fail_attachment_insert BEFORE INSERT ON attachments WHEN NEW.filename = 'fail.txt' BEGIN SELECT RAISE(ABORT, 'intentional attachment metadata failure'); END;",
  );
  const beforeKeys = await cloud.listAttachmentKeys();
  const failed = await cloud.request(`/api/tasks/${task.body.task.id}/attachments`, {
    method: "POST",
    actorName: alice,
    headers: {
      "content-type": "text/plain",
      "x-taskboard-filename": encodeURIComponent("fail.txt"),
    },
    body: "must be compensated",
  });
  assert.equal(failed.response.status, 500);
  assert.deepEqual(await cloud.listAttachmentKeys(), beforeKeys);
});

test("the global revision is monotonic and lets clients poll only when data changed", async () => {
  const initial = await cloud.request("/api/revisions?since=0", { actorName: alice });
  assert.equal(initial.response.status, 200);
  const baseline = initial.body.revision;

  const unchanged = await cloud.request(`/api/revisions?since=${baseline}`, {
    actorName: alice,
  });
  assert.equal(unchanged.body.changed, false);

  await createTask("alpha", "Revision mutation");
  const changed = await cloud.request(`/api/revisions?since=${baseline}`, {
    actorName: bob,
  });
  assert.equal(changed.body.changed, true);
  assert.ok(changed.body.revision > baseline);

  const current = await cloud.request(`/api/revisions?since=${changed.body.revision}`, {
    actorName: alice,
  });
  assert.equal(current.body.changed, false);
});

test("cloud-only local capability routes return an explicit companion requirement", async () => {
  const meta = await cloud.request("/api/meta", { actorName: alice });
  assert.equal(meta.response.status, 200);
  assert.equal(meta.body.mode, "cloud");
  assert.deepEqual(meta.body.realtime, { transport: "poll", intervalMs: 2000 });
  assert.equal(meta.body.localCapabilities.available, false);

  for (const pathname of [
    "/api/device-workspaces",
    "/api/workflow-capabilities",
    "/api/projects/alpha/development-contexts",
  ]) {
    const result = await cloud.request(pathname, { actorName: alice });
    assert.equal(result.response.status, 409);
    assert.equal(result.body.error.code, "LOCAL_COMPANION_REQUIRED");
  }
});

test("task lifecycle keeps optimistic versions and never persists a worktree path", async () => {
  await createProject("task-lifecycle");
  const created = await createTask("task-lifecycle", "Lifecycle", alice, {
    developmentContext: { type: "worktree", branch: "feature/shared" },
  });
  assert.equal(created.response.status, 201);
  assert.deepEqual(created.body.task.developmentContext, {
    type: "worktree",
    path: null,
    branch: "feature/shared",
  });

  const moved = await cloud.request(`/api/tasks/${created.body.task.id}/move`, {
    method: "POST",
    actorName: alice,
    json: {
      version: created.body.task.version,
      status: "in_progress",
    },
  });
  assert.equal(moved.response.status, 200);
  assert.equal(moved.body.task.status, "in_progress");

  const archived = await cloud.request(`/api/tasks/${created.body.task.id}/archive`, {
    method: "POST",
    actorName: alice,
    json: { version: moved.body.task.version },
  });
  assert.equal(archived.response.status, 200);
  assert.ok(archived.body.task.archivedAt);

  const restored = await cloud.request(`/api/tasks/${created.body.task.id}/restore`, {
    method: "POST",
    actorName: alice,
    json: { version: archived.body.task.version },
  });
  assert.equal(restored.response.status, 200);
  assert.equal(restored.body.task.archivedAt, null);
});

test("relation direction, deletion, and parent-cycle checks match the local contract", async () => {
  await createProject("relation-parity");
  const blocker = await createTask("relation-parity", "Blocker");
  const blocked = await createTask("relation-parity", "Blocked");
  const blockedBy = await cloud.request(
    `/api/tasks/${blocked.body.task.id}/relations/blocked_by/${blocker.body.task.id}`,
    {
      method: "POST",
      actorName: alice,
      json: { version: blocked.body.task.version },
    },
  );
  assert.equal(blockedBy.response.status, 200);
  assert.equal(blockedBy.body.task.relations.blockedBy[0].id, blocker.body.task.id);
  assert.equal(blockedBy.body.relatedTask.relations.blocks[0].id, blocked.body.task.id);

  const removed = await cloud.request(
    `/api/tasks/${blocked.body.task.id}/relations/blocked_by/${blocker.body.task.id}`,
    {
      method: "DELETE",
      actorName: alice,
      json: { version: blockedBy.body.task.version },
    },
  );
  assert.equal(removed.response.status, 200);
  assert.deepEqual(removed.body.task.relations.blockedBy, []);

  const child = await createTask("relation-parity", "Child");
  const parent = await cloud.request(
    `/api/tasks/${child.body.task.id}/relations/parent/${blocker.body.task.id}`,
    {
      method: "POST",
      actorName: alice,
      json: { version: child.body.task.version },
    },
  );
  assert.equal(parent.response.status, 200);
  const cycle = await cloud.request(
    `/api/tasks/${blocker.body.task.id}/relations/parent/${child.body.task.id}`,
    {
      method: "POST",
      actorName: alice,
      json: { version: blocker.body.task.version },
    },
  );
  assert.equal(cycle.response.status, 409);
  assert.equal(cycle.body.error.code, "RELATION_CYCLE");
});

test("concurrent inverse parent writes cannot create a cycle", async () => {
  await createProject("concurrent-parent-cycle");
  const first = await createTask("concurrent-parent-cycle", "First");
  const second = await createTask("concurrent-parent-cycle", "Second");

  const [firstResult, secondResult] = await Promise.all([
    cloud.request(
      `/api/tasks/${first.body.task.id}/relations/parent/${second.body.task.id}`,
      {
        method: "POST",
        actorName: alice,
        json: { version: first.body.task.version },
      },
    ),
    cloud.request(
      `/api/tasks/${second.body.task.id}/relations/parent/${first.body.task.id}`,
      {
        method: "POST",
        actorName: bob,
        json: { version: second.body.task.version },
      },
    ),
  ]);

  assert.deepEqual(
    [firstResult.response.status, secondResult.response.status].sort(),
    [200, 409],
  );
  const conflict = firstResult.response.status === 409 ? firstResult : secondResult;
  assert.equal(conflict.body.error.code, "RELATION_CYCLE");

  const rows = await cloud.db.prepare(`
    SELECT source_task_id, target_task_id
    FROM task_relations
    WHERE relation_type = 'parent'
      AND source_task_id IN (?, ?)
      AND target_task_id IN (?, ?)
  `).bind(
    first.body.task.id,
    second.body.task.id,
    first.body.task.id,
    second.body.task.id,
  ).all();
  assert.equal(rows.results.length, 1);

  const third = await createTask("concurrent-parent-cycle", "Third");
  const fourth = await createTask("concurrent-parent-cycle", "Fourth");
  const inserted = await Promise.allSettled([
    cloud.db.prepare(`
      INSERT INTO task_relations (
        relation_type, source_task_id, target_task_id, created_at
      ) VALUES ('parent', ?, ?, ?)
    `).bind(fourth.body.task.id, third.body.task.id, new Date().toISOString()).run(),
    cloud.db.prepare(`
      INSERT INTO task_relations (
        relation_type, source_task_id, target_task_id, created_at
      ) VALUES ('parent', ?, ?, ?)
    `).bind(third.body.task.id, fourth.body.task.id, new Date().toISOString()).run(),
  ]);
  assert.equal(inserted.filter((result) => result.status === "fulfilled").length, 1);
});

test("workflow conflicts and comment attachment cleanup preserve shared-state boundaries", async () => {
  await createProject("shared-boundaries");
  const workspace = {
    version: 1,
    tabs: [{ id: "delivery", name: "Delivery" }],
    activeWorkflowId: "delivery",
    snapshots: {
      delivery: {
        nodes: [],
        flow: { version: 2, root: { items: [] } },
        selectedNodeId: null,
      },
    },
  };
  const saved = await cloud.request(
    "/api/projects/shared-boundaries/workflow-workspace",
    {
      method: "PUT",
      actorName: alice,
      json: { version: 0, workspace },
    },
  );
  assert.equal(saved.response.status, 200);
  const stale = await cloud.request(
    "/api/projects/shared-boundaries/workflow-workspace",
    {
      method: "PUT",
      actorName: bob,
      json: { version: 0, workspace },
    },
  );
  assert.equal(stale.response.status, 409);
  assert.deepEqual(stale.body.error.details, {
    expectedVersion: 0,
    actualVersion: 1,
  });

  const task = await createTask("shared-boundaries", "Comment owner");
  const created = await cloud.request(`/api/tasks/${task.body.task.id}/comments`, {
    method: "POST",
    actorName: bob,
    json: { body: "Initial" },
  });
  const attachment = await cloud.request(
    `/api/comments/${created.body.comment.id}/attachments`,
    {
      method: "POST",
      actorName: bob,
      headers: {
        "content-type": "text/plain",
        "x-taskboard-filename": encodeURIComponent("comment.txt"),
      },
      body: "comment attachment",
    },
  );
  assert.equal(attachment.response.status, 201);

  const updated = await cloud.request(`/api/comments/${created.body.comment.id}`, {
    method: "PATCH",
    actorName: bob,
    json: { version: created.body.comment.version, body: "Updated" },
  });
  assert.equal(updated.response.status, 200);
  assert.equal(updated.body.comment.attachments.length, 1);

  const deleted = await cloud.request(`/api/comments/${created.body.comment.id}`, {
    method: "DELETE",
    actorName: bob,
    json: { version: updated.body.comment.version },
  });
  assert.equal(deleted.response.status, 204);
  assert.deepEqual(await cloud.listAttachmentKeys(), []);
});

test("workflow saves never persist or return a local Git worktree path", async () => {
  await createProject("workflow-runtime-boundary");
  const localPath = "/Users/alice/source/workflow-runtime-boundary-worktree";
  const nestedLocalPath = "/Users/alice/source/nested-plan-worktree";
  const workspace = {
    version: 1,
    tabs: [{ id: "delivery", name: "Delivery" }],
    activeWorkflowId: "delivery",
    snapshots: {
      delivery: {
        nodes: [
          {
            id: "trigger",
            position: { x: 0, y: 0 },
            data: { kind: "issue-trigger" },
          },
          {
            id: "git",
            position: { x: 0, y: 120 },
            data: {
              kind: "git",
              gitOperation: "create-worktree",
              gitBranchName: "feature/shared-workflow",
              gitWorktreePath: localPath,
              additionalInstructions: `Do not rewrite this note: ${localPath}`,
              planItem: {
                gitWorktreePath: nestedLocalPath,
                path: nestedLocalPath,
                branch: "feature/nested-plan",
                notes: `Keep this nested note: ${nestedLocalPath}`,
              },
            },
          },
        ],
        flow: {
          version: 2,
          root: {
            items: [
              { type: "step", nodeId: "trigger" },
              { type: "step", nodeId: "git" },
            ],
          },
        },
        selectedNodeId: "git",
      },
    },
  };

  const saved = await cloud.request(
    "/api/projects/workflow-runtime-boundary/workflow-workspace",
    {
      method: "PUT",
      actorName: alice,
      json: { version: 0, workspace },
    },
  );
  assert.equal(saved.response.status, 200);
  const savedData = saved.body.workflow.workspace.snapshots.delivery.nodes[1].data;
  assert.equal(savedData.gitWorktreePath, undefined);
  assert.equal(savedData.gitBranchName, "feature/shared-workflow");
  assert.equal(savedData.additionalInstructions, `Do not rewrite this note: ${localPath}`);
  assert.equal(savedData.planItem.gitWorktreePath, undefined);
  assert.equal(savedData.planItem.path, nestedLocalPath);
  assert.equal(savedData.planItem.branch, "feature/nested-plan");
  assert.equal(savedData.planItem.notes, `Keep this nested note: ${nestedLocalPath}`);

  const row = await cloud.db.prepare(`
    SELECT workspace FROM workflow_workspaces WHERE project_id = ?
  `).bind("workflow-runtime-boundary").first();
  assert.doesNotMatch(row.workspace, /"gitWorktreePath"/);
  assert.match(row.workspace, new RegExp(localPath));
  const persistedData = JSON.parse(row.workspace).snapshots.delivery.nodes[1].data;
  assert.equal(persistedData.gitWorktreePath, undefined);
  assert.equal(persistedData.gitBranchName, "feature/shared-workflow");
  assert.equal(persistedData.additionalInstructions, `Do not rewrite this note: ${localPath}`);
  assert.equal(persistedData.planItem.gitWorktreePath, undefined);
  assert.equal(persistedData.planItem.path, nestedLocalPath);
  assert.equal(persistedData.planItem.branch, "feature/nested-plan");
  assert.equal(persistedData.planItem.notes, `Keep this nested note: ${nestedLocalPath}`);
});
