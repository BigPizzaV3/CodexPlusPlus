import assert from "node:assert/strict";
import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import {
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import { afterEach, test } from "node:test";
import { promisify } from "node:util";

import { Miniflare } from "miniflare";
import {
  createCloudBindingMigrationAdapters,
  createCloudD1ImportSql,
  createCloudMigrationBundle,
  importCloudMigrationBundle,
  readCloudMigrationBundle,
  runCli,
  writeCloudMigrationBundle,
} from "../scripts/migrate-to-cloud.mjs";
import { createCloudWorkerHarness } from "./helpers/cloud-worker-harness.mjs";

const fixtures = [];
const timestamp = "2026-07-24T12:00:00.000Z";
const execFile = promisify(execFileCallback);
const projectRoot = path.resolve(import.meta.dirname, "..");
const wranglerExecutable = path.join(projectRoot, "node_modules", ".bin", "wrangler");
const wranglerConfig = path.join(projectRoot, "wrangler.jsonc");

afterEach(async () => {
  while (fixtures.length > 0) {
    const fixture = fixtures.pop();
    fixture.database.close();
    await rm(fixture.directory, { recursive: true, force: true });
  }
});

async function createMigrationFixture({
  missingAttachmentId = null,
  foreignKeyViolation = false,
} = {}) {
  const directory = await mkdtemp(path.join(os.tmpdir(), "taskboard-cloud-migration-"));
  const databasePath = path.join(directory, "taskboard.sqlite");
  const attachmentsDirectory = path.join(directory, "attachments");
  const database = new DatabaseSync(databasePath);
  const fixture = {
    directory,
    database,
    databasePath,
    attachmentsDirectory,
    attachmentContents: {
      "attachment-a": Buffer.from("alpha issue attachment\n"),
      "attachment-a-comment": Buffer.from("alpha comment attachment\n"),
      "attachment-b": Buffer.from("beta attachment\n"),
    },
  };
  fixtures.push(fixture);

  database.exec(`
    PRAGMA foreign_keys = ON;
    PRAGMA journal_mode = WAL;
    PRAGMA wal_autocheckpoint = 0;

    CREATE TABLE projects (
      id TEXT PRIMARY KEY,
      name TEXT NOT NULL,
      workspace_path TEXT,
      next_task_number INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE tasks (
      id TEXT PRIMARY KEY,
      identifier TEXT NOT NULL UNIQUE,
      project_id TEXT NOT NULL REFERENCES projects(id),
      title TEXT NOT NULL,
      description TEXT NOT NULL DEFAULT '',
      status TEXT NOT NULL,
      priority TEXT NOT NULL,
      labels TEXT NOT NULL DEFAULT '[]',
      sort_order REAL NOT NULL,
      thread_id TEXT,
      creator_type TEXT NOT NULL,
      creator_id TEXT NOT NULL,
      creator_name TEXT NOT NULL,
      creator_avatar_url TEXT,
      assignee_type TEXT NOT NULL,
      assignee_id TEXT NOT NULL,
      assignee_name TEXT NOT NULL,
      assignee_avatar_url TEXT,
      workflow_id TEXT,
      git_branch TEXT,
      worktree_path TEXT,
      worktree_branch TEXT,
      due_date TEXT,
      recurrence_interval INTEGER,
      recurrence_unit TEXT,
      archived_at TEXT,
      version INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE comments (
      id TEXT PRIMARY KEY,
      task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      body TEXT NOT NULL,
      thread_id TEXT,
      author_type TEXT NOT NULL,
      author_id TEXT NOT NULL,
      author_name TEXT NOT NULL,
      author_avatar_url TEXT,
      version INTEGER NOT NULL,
      created_at TEXT NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE attachments (
      id TEXT PRIMARY KEY,
      task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      comment_id TEXT REFERENCES comments(id) ON DELETE CASCADE,
      filename TEXT NOT NULL,
      content_type TEXT NOT NULL,
      size INTEGER NOT NULL,
      created_at TEXT NOT NULL
    );

    CREATE TABLE workflow_workspaces (
      project_id TEXT PRIMARY KEY REFERENCES projects(id) ON DELETE CASCADE,
      workspace TEXT NOT NULL,
      version INTEGER NOT NULL,
      updated_at TEXT NOT NULL
    );

    CREATE TABLE task_relations (
      relation_type TEXT NOT NULL,
      source_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      target_task_id TEXT NOT NULL REFERENCES tasks(id) ON DELETE CASCADE,
      created_at TEXT NOT NULL,
      PRIMARY KEY (relation_type, source_task_id, target_task_id)
    );

    CREATE TABLE local_cloud_session (
      access_token TEXT NOT NULL,
      created_at TEXT NOT NULL
    );
  `);
  database.exec("PRAGMA wal_checkpoint(TRUNCATE)");

  database.exec(`
    BEGIN IMMEDIATE;

    INSERT INTO projects VALUES
      (
        'alpha',
        'Alpha',
        '/Users/alice/source/alpha',
        3,
        '${timestamp}',
        '${timestamp}'
      ),
      (
        'beta',
        'Beta',
        '/Users/bob/source/beta',
        2,
        '${timestamp}',
        '${timestamp}'
      );

    INSERT INTO tasks VALUES
      (
        'task-a1',
        'ALPHA-1',
        'alpha',
        'Alpha one',
        'First alpha issue',
        'todo',
        'high',
        '["cloud"]',
        1000,
        'thread-alpha',
        'user',
        'alice',
        'Alice',
        NULL,
        'user',
        'alice',
        'Alice',
        NULL,
        'workflow-alpha',
        NULL,
        '/Users/alice/source/alpha-worktree',
        'feature/cloud-share',
        NULL,
        NULL,
        NULL,
        NULL,
        2,
        '${timestamp}',
        '${timestamp}'
      ),
      (
        'task-a2',
        'ALPHA-2',
        'alpha',
        'Alpha two',
        '',
        'in_progress',
        'none',
        '[]',
        2000,
        NULL,
        'agent',
        'codex-agent',
        'Codex Agent',
        NULL,
        'agent',
        'codex-agent',
        'Codex Agent',
        NULL,
        NULL,
        'feature/plain-branch',
        NULL,
        NULL,
        NULL,
        NULL,
        NULL,
        NULL,
        1,
        '${timestamp}',
        '${timestamp}'
      ),
      (
        'task-b1',
        'BETA-1',
        'beta',
        'Beta one',
        '',
        'backlog',
        'low',
        '[]',
        1000,
        NULL,
        'user',
        'bob',
        'Bob',
        NULL,
        'user',
        'bob',
        'Bob',
        NULL,
        NULL,
        NULL,
        '/Users/bob/source/beta-worktree',
        'feature/beta-worktree',
        NULL,
        NULL,
        NULL,
        NULL,
        1,
        '${timestamp}',
        '${timestamp}'
      );

    INSERT INTO comments VALUES
      (
        'comment-a',
        'task-a1',
        'Alpha comment',
        'thread-alpha-comment',
        'user',
        'alice',
        'Alice',
        NULL,
        1,
        '${timestamp}',
        '${timestamp}'
      ),
      (
        'comment-b',
        'task-b1',
        'Beta comment',
        NULL,
        'user',
        'bob',
        'Bob',
        NULL,
        1,
        '${timestamp}',
        '${timestamp}'
      );

    INSERT INTO attachments VALUES
      (
        'attachment-a',
        'task-a1',
        NULL,
        'alpha.txt',
        'text/plain',
        23,
        '${timestamp}'
      ),
      (
        'attachment-a-comment',
        'task-a1',
        'comment-a',
        'alpha-comment.txt',
        'text/plain',
        25,
        '${timestamp}'
      ),
      (
        'attachment-b',
        'task-b1',
        NULL,
        'beta.txt',
        'text/plain',
        16,
        '${timestamp}'
      );

    INSERT INTO workflow_workspaces VALUES
      (
        'alpha',
        '{"version":1,"tabs":[{"id":"delivery","name":"Delivery","nodes":[{"data":{"gitWorktreePath":"/Users/alice/source/alpha-worktree","workspacePath":"/api/request/workspace","cwd":"/api/request/cwd","workingDirectory":"/api/request/working-directory","additionalInstructions":"Keep this workflow note"}}]}]}',
        1,
        '${timestamp}'
      );

    INSERT INTO task_relations VALUES
      ('parent', 'task-a1', 'task-a2', '${timestamp}');

    INSERT INTO local_cloud_session VALUES
      ('cf-access-super-secret', '${timestamp}');

    COMMIT;
  `);
  if (foreignKeyViolation) {
    database.exec(`
      PRAGMA foreign_keys = OFF;
      INSERT INTO task_relations VALUES
        ('blocks', 'missing-task', 'task-a1', '${timestamp}');
      PRAGMA foreign_keys = ON;
    `);
  }

  await mkdir(attachmentsDirectory, { recursive: true });
  for (const [id, contents] of Object.entries(fixture.attachmentContents)) {
    if (id !== missingAttachmentId) {
      await writeFile(path.join(attachmentsDirectory, id), contents);
    }
  }

  return fixture;
}

function expectedProjectCounts() {
  return {
    alpha: {
      projects: 1,
      tasks: 2,
      comments: 1,
      attachments: 2,
      task_relations: 1,
      workflow_workspaces: 1,
    },
    beta: {
      projects: 1,
      tasks: 1,
      comments: 1,
      attachments: 1,
      task_relations: 0,
      workflow_workspaces: 0,
    },
  };
}

function createD1Adapter(
  bundle,
  {
    initialCounts = {},
    importedCounts = bundle.counts.byProject,
    failImport = false,
  } = {},
) {
  const calls = [];
  const state = { tables: null };
  return {
    calls,
    state,
    async importTables(tables) {
      const staged = structuredClone(tables);
      calls.push(staged);
      if (failImport) throw new Error("Atomic D1 import failed");
      state.tables = staged;
    },
    async countByProject() {
      return structuredClone(state.tables ? importedCounts : initialCounts);
    },
  };
}

function createR2Adapter({ tamperHeadFor = null } = {}) {
  const objects = new Map();
  const deleted = [];
  return {
    objects,
    deleted,
    async put(key, body, options) {
      const bytes = Buffer.from(body);
      objects.set(key, {
        bytes,
        size: bytes.byteLength,
        customMetadata: structuredClone(options?.customMetadata ?? {}),
      });
    },
    async head(key) {
      const object = objects.get(key);
      if (!object) return null;
      if (key === tamperHeadFor) {
        return {
          size: object.size,
          customMetadata: { ...object.customMetadata, sha256: "0".repeat(64) },
        };
      }
      return {
        size: object.size,
        customMetadata: structuredClone(object.customMetadata),
      };
    },
    async delete(key) {
      deleted.push(key);
      objects.delete(key);
    },
  };
}

test("migration snapshots live WAL data, counts each project, and strips local execution state", async () => {
  const fixture = await createMigrationFixture();
  const wal = await stat(`${fixture.databasePath}-wal`);
  assert.ok(wal.size > 32, "fixture data must still be present in the WAL");

  const mainFileOnly = path.join(fixture.directory, "main-file-only.sqlite");
  await copyFile(fixture.databasePath, mainFileOnly);
  const staleDatabase = new DatabaseSync(mainFileOnly);
  assert.equal(
    staleDatabase.prepare("SELECT COUNT(*) AS count FROM projects").get().count,
    0,
    "copying only the main file should prove the test data has not been checkpointed",
  );
  staleDatabase.close();

  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });

  assert.equal(bundle.schemaVersion, 1);
  assert.deepEqual(bundle.counts.byProject, expectedProjectCounts());
  assert.deepEqual(
    bundle.tables.projects.map((project) => project.id).sort(),
    ["alpha", "beta"],
  );
  assert.equal(bundle.tables.projects.every((project) => project.workspace_path === null), true);

  const alphaWorktreeTask = bundle.tables.tasks.find((task) => task.id === "task-a1");
  assert.equal(alphaWorktreeTask.worktree_path, null);
  assert.equal(alphaWorktreeTask.worktree_branch, "feature/cloud-share");

  const alphaBranchTask = bundle.tables.tasks.find((task) => task.id === "task-a2");
  assert.equal(alphaBranchTask.git_branch, "feature/plain-branch");

  const workflow = JSON.parse(bundle.tables.workflow_workspaces[0].workspace);
  assert.equal(workflow.tabs[0].nodes[0].data.gitWorktreePath, null);
  assert.equal(workflow.tabs[0].nodes[0].data.workspacePath, "/api/request/workspace");
  assert.equal(workflow.tabs[0].nodes[0].data.cwd, "/api/request/cwd");
  assert.equal(
    workflow.tabs[0].nodes[0].data.workingDirectory,
    "/api/request/working-directory",
  );
  assert.equal(
    workflow.tabs[0].nodes[0].data.additionalInstructions,
    "Keep this workflow note",
  );

  const serializedBundle = JSON.stringify(bundle);
  assert.doesNotMatch(serializedBundle, /cf-access-super-secret/);
  assert.doesNotMatch(serializedBundle, /\/Users\/(?:alice|bob)\//);
  assert.equal(Object.hasOwn(bundle.tables, "local_cloud_session"), false);
});

test("migration records attachment bytes, SHA-256, and actual size", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });

  assert.equal(bundle.attachments.length, 3);
  for (const attachment of bundle.attachments) {
    const expectedContents = fixture.attachmentContents[attachment.id];
    assert.deepEqual(Buffer.from(attachment.body), expectedContents);
    assert.equal(attachment.size, expectedContents.byteLength);
    assert.equal(
      attachment.sha256,
      createHash("sha256").update(expectedContents).digest("hex"),
    );
    assert.equal(path.isAbsolute(attachment.objectKey), false);
  }
});

test("migration fails before import when an attachment referenced by SQLite is missing", async () => {
  const fixture = await createMigrationFixture({ missingAttachmentId: "attachment-a-comment" });

  await assert.rejects(
    createCloudMigrationBundle({
      databasePath: fixture.databasePath,
      attachmentsDirectory: fixture.attachmentsDirectory,
    }),
    /attachment-a-comment.*missing|missing.*attachment-a-comment/i,
  );
});

test("migration rejects a snapshot with foreign key violations", async () => {
  const fixture = await createMigrationFixture({ foreignKeyViolation: true });

  await assert.rejects(
    createCloudMigrationBundle({
      databasePath: fixture.databasePath,
      attachmentsDirectory: fixture.attachmentsDirectory,
    }),
    /foreign key|foreign_key/i,
  );
});

test("cloud import calls D1 and R2 adapters, then verifies project counts and object hashes", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const d1 = createD1Adapter(bundle);
  const r2 = createR2Adapter();

  const result = await importCloudMigrationBundle(bundle, { d1, r2 });

  assert.equal(d1.calls.length, 1);
  assert.deepEqual(Object.keys(d1.calls[0]), [
    "projects",
    "tasks",
    "comments",
    "task_relations",
    "attachments",
    "workflow_workspaces",
  ]);
  assert.deepEqual(result.counts.byProject, expectedProjectCounts());
  assert.equal(result.attachments.verified, 3);

  for (const attachment of bundle.attachments) {
    const object = r2.objects.get(attachment.objectKey);
    assert.deepEqual(object.bytes, Buffer.from(attachment.body));
    assert.equal(object.size, attachment.size);
    assert.equal(object.customMetadata.sha256, attachment.sha256);
  }
});

test("cloud import rejects D1 count drift and R2 hash drift", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });

  const wrongCounts = structuredClone(bundle.counts.byProject);
  wrongCounts.alpha.tasks -= 1;
  const committedD1 = createD1Adapter(bundle, { importedCounts: wrongCounts });
  const committedR2 = createR2Adapter();
  await assert.rejects(
    importCloudMigrationBundle(bundle, {
      d1: committedD1,
      r2: committedR2,
    }),
    /count.*alpha|alpha.*count/i,
  );
  assert.notEqual(committedD1.state.tables, null);
  assert.equal(committedR2.deleted.length, 0, "committed D1 must retain matching R2 objects");
  assert.equal(committedR2.objects.size, bundle.attachments.length);

  const driftD1 = createD1Adapter(bundle);
  const driftR2 = createR2Adapter({ tamperHeadFor: bundle.attachments[0].objectKey });
  await assert.rejects(
    importCloudMigrationBundle(bundle, {
      d1: driftD1,
      r2: driftR2,
    }),
    /sha-?256|hash/i,
  );
  assert.equal(driftD1.calls.length, 0, "R2 must verify before D1 becomes visible");
  assert.deepEqual(
    driftR2.deleted.sort(),
    bundle.attachments.map((attachment) => attachment.objectKey).sort(),
  );
});

test("cloud import is atomic in D1 and cleans staged R2 objects when D1 rejects", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const d1 = createD1Adapter(bundle, { failImport: true });
  const r2 = createR2Adapter();

  await assert.rejects(
    importCloudMigrationBundle(bundle, { d1, r2 }),
    /atomic D1 import failed/i,
  );

  assert.equal(d1.calls.length, 1);
  assert.equal(d1.state.tables, null);
  assert.equal(r2.objects.size, 0);
  assert.deepEqual(
    r2.deleted.sort(),
    bundle.attachments.map((attachment) => attachment.objectKey).sort(),
  );
});

test("D1 binding import uses one JSON statement per table for 100+ rows", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const template = bundle.tables.tasks[0];
  const tables = structuredClone(bundle.tables);
  tables.tasks = Array.from({ length: 125 }, (_, index) => ({
    ...structuredClone(template),
    id: `bulk-task-${index}`,
    identifier: `BULK-${index + 1}`,
    sort_order: index + 1,
  }));
  const batches = [];
  const d1 = {
    prepare(sql) {
      return {
        bind(...values) {
          return { sql, values };
        },
      };
    },
    async batch(statements) {
      batches.push(statements);
    },
  };

  const adapters = createCloudBindingMigrationAdapters({
    d1,
    r2: createR2Adapter(),
  });
  await adapters.d1.importTables(tables);

  assert.equal(batches.length, 1);
  assert.equal(batches[0].length, 6);
  for (const statement of batches[0]) assert.match(statement.sql, /json_each\(\?\)/);
  assert.equal(JSON.parse(batches[0][1].values[0]).length, 125);
});

test("Wrangler D1 SQL chunks large tables below the remote statement byte limit", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const template = bundle.tables.tasks[0];
  const tables = structuredClone(bundle.tables);
  tables.tasks = Array.from({ length: 160 }, (_, index) => ({
    ...structuredClone(template),
    id: `large-task-${String(index).padStart(3, "0")}`,
    identifier: `LARGE-${index + 1}`,
    description: `多字节-${index}-`.repeat(160),
    sort_order: index + 1,
  }));

  const sql = createCloudD1ImportSql(tables);
  assert.ok(Buffer.byteLength(sql, "utf8") > 100_000);
  const statements = sql
    .split(/;\n(?=INSERT INTO )/)
    .map((statement) => statement.endsWith(";") ? statement : `${statement};`);
  assert.ok(
    statements.filter((statement) => statement.startsWith('INSERT INTO "tasks"')).length > 1,
  );
  for (const statement of statements) {
    assert.ok(
      Buffer.byteLength(statement, "utf8") < 90_000,
      `statement was ${Buffer.byteLength(statement, "utf8")} bytes`,
    );
  }
  assert.ok(sql.indexOf("large-task-000") < sql.indexOf("large-task-159"));

  const oversized = structuredClone(tables);
  oversized.tasks = [{
    ...structuredClone(template),
    id: "oversized-task",
    identifier: "OVERSIZED-1",
    description: "界".repeat(40_000),
  }];
  assert.throws(
    () => createCloudD1ImportSql(oversized),
    /single row.*90,?000 bytes/i,
  );
});

test("real D1 batch atomically imports a bundle containing local and maps development fields", async () => {
  const fixture = await createMigrationFixture();
  fixture.database.exec(`
    PRAGMA foreign_keys = OFF;
    UPDATE projects SET id = 'local' WHERE id = 'alpha';
    UPDATE tasks SET project_id = 'local' WHERE project_id = 'alpha';
    UPDATE workflow_workspaces SET project_id = 'local' WHERE project_id = 'alpha';
    PRAGMA foreign_keys = ON;
  `);
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  assert.ok(bundle.tables.projects.some((project) => project.id === "local"));

  const failingCloud = await createCloudWorkerHarness();
  try {
    await failingCloud.db.prepare(`
      CREATE TRIGGER fail_migration_comment
      BEFORE INSERT ON comments
      BEGIN
        SELECT RAISE(ABORT, 'intentional migration failure');
      END;
    `).run();
    await assert.rejects(
      importCloudMigrationBundle(
        bundle,
        createCloudBindingMigrationAdapters({
          d1: failingCloud.db,
          r2: failingCloud.attachments,
        }),
      ),
      /intentional migration failure/i,
    );
    assert.equal(
      (await failingCloud.db.prepare("SELECT COUNT(*) AS count FROM projects").first()).count,
      0,
    );
    assert.deepEqual(await failingCloud.listAttachmentKeys(), []);
  } finally {
    await failingCloud.dispose();
  }

  const cloud = await createCloudWorkerHarness();
  try {
    const result = await importCloudMigrationBundle(
      bundle,
      createCloudBindingMigrationAdapters({
        d1: cloud.db,
        r2: cloud.attachments,
      }),
    );
    assert.deepEqual(result.counts.byProject, bundle.counts.byProject);
    assert.deepEqual(
      (await cloud.db.prepare("SELECT id FROM projects ORDER BY id").all()).results,
      [{ id: "beta" }, { id: "local" }],
    );
    assert.deepEqual(
      await cloud.db.prepare(`
        SELECT development_context_type, development_branch
        FROM tasks
        WHERE id = 'task-a1'
      `).first(),
      {
        development_context_type: "worktree",
        development_branch: "feature/cloud-share",
      },
    );
    assert.deepEqual(
      await cloud.db.prepare(`
        SELECT development_context_type, development_branch
        FROM tasks
        WHERE id = 'task-a2'
      `).first(),
      {
        development_context_type: "branch",
        development_branch: "feature/plain-branch",
      },
    );
    const taskColumns = await cloud.db.prepare("PRAGMA table_info(tasks)").all();
    assert.equal(taskColumns.results.some((column) => column.name === "worktree_path"), false);

    const migratedAttachment = bundle.attachments.find(
      (attachment) => attachment.id === "attachment-a",
    );
    const downloaded = await cloud.request(
      `/api/attachments/${encodeURIComponent(migratedAttachment.id)}/content`,
      { actorName: "Alice" },
    );
    assert.equal(downloaded.response.status, 200);
    assert.equal(downloaded.body, fixture.attachmentContents[migratedAttachment.id].toString());

    const deleted = await cloud.request(
      `/api/attachments/${encodeURIComponent(migratedAttachment.id)}`,
      { method: "DELETE", actorName: "Alice" },
    );
    assert.equal(deleted.response.status, 204);
    assert.equal(await cloud.attachments.head(migratedAttachment.id), null);
  } finally {
    await cloud.dispose();
  }
});

test("cloud import refuses non-empty D1 and pre-existing R2 targets", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });

  const nonEmptyD1 = createD1Adapter(bundle, {
    initialCounts: { alpha: expectedProjectCounts().alpha },
  });
  const untouchedR2 = createR2Adapter();
  await assert.rejects(
    importCloudMigrationBundle(bundle, { d1: nonEmptyD1, r2: untouchedR2 }),
    /D1.*not empty|non-empty.*D1/i,
  );
  assert.equal(nonEmptyD1.calls.length, 0);
  assert.equal(untouchedR2.objects.size, 0);

  const occupiedR2 = createR2Adapter();
  const occupiedKey = bundle.attachments[0].objectKey;
  occupiedR2.objects.set(occupiedKey, {
    bytes: Buffer.from("existing"),
    size: 8,
    customMetadata: { sha256: createHash("sha256").update("existing").digest("hex") },
  });
  const emptyD1 = createD1Adapter(bundle);
  await assert.rejects(
    importCloudMigrationBundle(bundle, { d1: emptyD1, r2: occupiedR2 }),
    /R2.*already exists|already exists.*R2/i,
  );
  assert.equal(emptyD1.calls.length, 0);
  assert.deepEqual(occupiedR2.objects.get(occupiedKey).bytes, Buffer.from("existing"));
  assert.equal(occupiedR2.deleted.length, 0);
});

test("bundle validation binds attachment size and object key to SQLite metadata", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });

  const wrongSize = structuredClone(bundle);
  wrongSize.tables.attachments[0].size += 1;
  await assert.rejects(
    importCloudMigrationBundle(wrongSize, {
      d1: createD1Adapter(wrongSize),
      r2: createR2Adapter(),
    }),
    /metadata.*size|size.*metadata/i,
  );

  const wrongObjectKey = structuredClone(bundle);
  wrongObjectKey.attachments[0].objectKey = "different-attachment";
  await assert.rejects(
    importCloudMigrationBundle(wrongObjectKey, {
      d1: createD1Adapter(wrongObjectKey),
      r2: createR2Adapter(),
    }),
    /object key/i,
  );

  const wrongProject = structuredClone(bundle);
  wrongProject.attachments[0].projectId = "beta";
  await assert.rejects(
    importCloudMigrationBundle(wrongProject, {
      d1: createD1Adapter(wrongProject),
      r2: createR2Adapter(),
    }),
    /task project|project.*task/i,
  );
});

test("versioned bundle round-trips through private manifest, data, and attachment files", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const outputDirectory = path.join(fixture.directory, "cloud-bundle-v1");

  await writeCloudMigrationBundle(bundle, outputDirectory);

  assert.equal((await stat(outputDirectory)).mode & 0o777, 0o700);
  assert.equal((await stat(path.join(outputDirectory, "data"))).mode & 0o777, 0o700);
  assert.equal(
    (await stat(path.join(outputDirectory, "attachments"))).mode & 0o777,
    0o700,
  );
  assert.equal(
    (await stat(path.join(outputDirectory, "manifest.json"))).mode & 0o777,
    0o600,
  );

  const restored = await readCloudMigrationBundle(outputDirectory);
  assert.deepEqual(restored.counts, bundle.counts);
  assert.equal(JSON.stringify(restored.tables), JSON.stringify(bundle.tables));
  assert.deepEqual(
    restored.attachments.map(({ id, body }) => [id, Buffer.from(body)]),
    bundle.attachments.map(({ id, body }) => [id, Buffer.from(body)]),
  );
});

test("CLI import and verify require and use an explicit adapter module", async () => {
  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const outputDirectory = path.join(fixture.directory, "cloud-bundle-v1");
  await writeCloudMigrationBundle(bundle, outputDirectory);
  const d1 = createD1Adapter(bundle);
  const r2 = createR2Adapter();
  const output = [];
  const options = {
    stdout: { write: (value) => output.push(value) },
    loadAdapters: async (modulePath) => {
      assert.equal(modulePath, "./cloud-adapter.mjs");
      return { d1, r2 };
    },
  };

  await runCli(
    ["import", "--bundle", outputDirectory, "--adapter", "./cloud-adapter.mjs"],
    options,
  );
  await runCli(
    ["verify", "--bundle", outputDirectory, "--adapter", "./cloud-adapter.mjs"],
    options,
  );
  assert.equal(output.length, 2);
  await assert.rejects(
    runCli(["verify", "--bundle", outputDirectory], options),
    /--adapter/,
  );
});

test("Wrangler adapter requires remote opt-in and keeps transfer files private", async () => {
  const {
    createCloudMigrationAdapters,
    createWranglerCloudAdapters,
  } = await import("../scripts/wrangler-cloud-adapter.mjs");
  assert.throws(
    () => createWranglerCloudAdapters({
      remote: true,
      environment: {},
    }),
    /TASKBOARD_MIGRATION_REMOTE=1/,
  );

  const fixture = await createMigrationFixture();
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  const oversizedBundle = structuredClone(bundle);
  oversizedBundle.tables.tasks[0].description = "界".repeat(40_000);
  assert.throws(
    () => createCloudMigrationAdapters({
      bundle: oversizedBundle,
      command: "import",
    }),
    /single row.*90,?000 bytes/i,
  );
  const calls = [];
  const transferFiles = [];
  const downloadedBody = Buffer.from("downloaded R2 body");
  const adapters = createWranglerCloudAdapters({
    remote: true,
    environment: { TASKBOARD_MIGRATION_REMOTE: "1" },
    runCommand: async (_executable, args) => {
      calls.push(args);
      const fileIndex = args.indexOf("--file");
      if (fileIndex !== -1) {
        const filename = args[fileIndex + 1];
        transferFiles.push(filename);
        if (args[0] === "r2" && args[2] === "get") {
          await writeFile(filename, downloadedBody);
        }
      }
      if (args.includes("--json")) return { stdout: '[{"results":[]}]' };
      return { stdout: "" };
    },
  });

  try {
    await adapters.d1.importTables(bundle.tables);
    assert.deepEqual(await adapters.d1.countByProject(), {});
    await adapters.r2.put("attachment-a", Buffer.from("upload"));
    assert.deepEqual(await adapters.r2.head("attachment-a"), {
      size: downloadedBody.byteLength,
      customMetadata: {
        sha256: createHash("sha256").update(downloadedBody).digest("hex"),
      },
    });
    await adapters.r2.delete("attachment-a");

    assert.deepEqual(
      calls.map((args) => args.slice(0, 3)),
      [
        ["d1", "execute", "codex-taskboard-db"],
        ["d1", "execute", "codex-taskboard-db"],
        ["r2", "object", "put"],
        ["r2", "object", "get"],
        ["r2", "object", "delete"],
      ],
    );
    for (const args of calls) {
      assert.ok(args.includes("--remote"));
      assert.ok(!args.includes("--local"));
      assert.ok(!args.includes("--persist-to"));
    }
    for (const filename of transferFiles) {
      assert.equal((await stat(filename)).mode & 0o777, 0o600);
      assert.equal((await stat(path.dirname(filename))).mode & 0o777, 0o700);
    }
  } finally {
    await adapters.cleanup();
  }
});

test("one-time Wrangler adapter migrates and verifies local persistence without remote access", async () => {
  const {
    createWranglerCloudAdapters,
  } = await import("../scripts/wrangler-cloud-adapter.mjs");
  assert.throws(
    () => createWranglerCloudAdapters({
      remote: true,
      environment: {},
    }),
    /TASKBOARD_MIGRATION_REMOTE=1/,
  );

  const fixture = await createMigrationFixture();
  fixture.database.exec(`
    PRAGMA foreign_keys = OFF;
    UPDATE projects SET id = 'local' WHERE id = 'alpha';
    UPDATE tasks SET project_id = 'local' WHERE project_id = 'alpha';
    UPDATE workflow_workspaces SET project_id = 'local' WHERE project_id = 'alpha';
    PRAGMA foreign_keys = ON;
  `);
  const insertLargeTask = fixture.database.prepare(`
    INSERT INTO tasks
    SELECT ?, ?, project_id, ?, ?, status, priority, labels, ?, thread_id,
      creator_type, creator_id, creator_name, creator_avatar_url,
      assignee_type, assignee_id, assignee_name, assignee_avatar_url,
      workflow_id, git_branch, worktree_path, worktree_branch, due_date,
      recurrence_interval, recurrence_unit, archived_at, version, created_at,
      updated_at
    FROM tasks WHERE id = 'task-a1'
  `);
  for (let index = 0; index < 160; index += 1) {
    insertLargeTask.run(
      `wrangler-large-task-${index}`,
      `LOCAL-${index + 100}`,
      `Wrangler large task ${index}`,
      `多字节-${index}-`.repeat(160),
      index + 10_000,
    );
  }
  const bundle = await createCloudMigrationBundle({
    databasePath: fixture.databasePath,
    attachmentsDirectory: fixture.attachmentsDirectory,
  });
  assert.ok(Buffer.byteLength(createCloudD1ImportSql(bundle.tables), "utf8") > 100_000);
  const bundleDirectory = path.join(fixture.directory, "wrangler-bundle-v1");
  await writeCloudMigrationBundle(bundle, bundleDirectory);
  const adapterPath = path.join(projectRoot, "scripts", "wrangler-cloud-adapter.mjs");

  async function applyMigrations(persistTo) {
    await execFile(wranglerExecutable, [
      "d1",
      "migrations",
      "apply",
      "codex-taskboard-db",
      "--local",
      "--persist-to",
      persistTo,
      "--config",
      wranglerConfig,
    ], { cwd: projectRoot });
  }

  async function runMigration(command, directory, persistTo) {
    return execFile(process.execPath, [
      "scripts/migrate-to-cloud.mjs",
      command,
      "--bundle",
      directory,
      "--adapter",
      adapterPath,
    ], {
      cwd: projectRoot,
      env: {
        ...process.env,
        TASKBOARD_MIGRATION_PERSIST_TO: persistTo,
        TASKBOARD_MIGRATION_CONFIG: wranglerConfig,
      },
    });
  }

  const persistTo = path.join(fixture.directory, "wrangler-success");
  await applyMigrations(persistTo);
  await runMigration("import", bundleDirectory, persistTo);
  await runMigration("verify", bundleDirectory, persistTo);

  const configuredDatabaseId = JSON.parse(
    await readFile(wranglerConfig, "utf8"),
  ).d1_databases[0].database_id;
  const miniflare = new Miniflare({
    modules: true,
    scriptPath: path.join(projectRoot, "cloud", "src", "index.mjs"),
    modulesRoot: projectRoot,
    compatibilityDate: "2026-07-24",
    bindings: {
      TASKBOARD_ENVIRONMENT: "production",
      TASKBOARD_SHARED_SECRET: "migration-test-secret",
    },
    d1Databases: { DB: configuredDatabaseId },
    r2Buckets: { ATTACHMENTS: "codex-taskboard-attachments" },
    defaultPersistRoot: path.join(persistTo, "v3"),
    d1Persist: path.join(persistTo, "v3", "d1"),
    r2Persist: true,
  });
  try {
    await miniflare.ready;
    const authorization = `Basic ${Buffer.from("Alice:migration-test-secret").toString("base64")}`;
    const attachment = bundle.attachments[0];
    const downloaded = await miniflare.dispatchFetch(
      `https://taskboard.example.test/api/attachments/${attachment.id}/content`,
      { headers: { authorization } },
    );
    assert.equal(downloaded.status, 200);
    assert.deepEqual(
      Buffer.from(await downloaded.arrayBuffer()),
      fixture.attachmentContents[attachment.id],
    );
    const deleted = await miniflare.dispatchFetch(
      `https://taskboard.example.test/api/attachments/${attachment.id}`,
      { method: "DELETE", headers: { authorization } },
    );
    assert.equal(deleted.status, 204);
    assert.equal(
      await (await miniflare.getR2Bucket("ATTACHMENTS")).head(attachment.id),
      null,
    );
  } finally {
    await miniflare.dispose();
  }

  const failingBundle = structuredClone(bundle);
  failingBundle.tables.tasks[1].identifier = failingBundle.tables.tasks[0].identifier;
  const failingDirectory = path.join(fixture.directory, "wrangler-failing-bundle-v1");
  await writeCloudMigrationBundle(failingBundle, failingDirectory);
  const failingPersistTo = path.join(fixture.directory, "wrangler-failure");
  await applyMigrations(failingPersistTo);
  await assert.rejects(
    runMigration("import", failingDirectory, failingPersistTo),
    /UNIQUE constraint failed|D1_EXEC_ERROR/i,
  );

  const failureAdapters = createWranglerCloudAdapters({
    persistTo: failingPersistTo,
    configPath: wranglerConfig,
  });
  try {
    assert.deepEqual(await failureAdapters.d1.countByProject(), {});
    for (const attachment of bundle.attachments) {
      assert.equal(await failureAdapters.r2.head(attachment.id), null);
    }
  } finally {
    await failureAdapters.cleanup();
  }
});
