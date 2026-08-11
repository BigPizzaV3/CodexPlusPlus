#!/usr/bin/env node

import { createHash } from "node:crypto";
import {
  chmod,
  lstat,
  mkdir,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { DatabaseSync } from "node:sqlite";

const SCHEMA_VERSION = 1;
const WRANGLER_D1_STATEMENT_MAX_BYTES = 90_000;
const TABLE_ORDER = [
  "projects",
  "tasks",
  "comments",
  "task_relations",
  "attachments",
  "workflow_workspaces",
];
const LOCAL_WORKFLOW_PATH_FIELDS = new Set(["gitWorktreePath"]);
const SORT_FIELDS = {
  projects: ["id"],
  tasks: ["project_id", "identifier", "id"],
  comments: ["task_id", "created_at", "id"],
  task_relations: ["source_task_id", "target_task_id", "relation_type"],
  attachments: ["task_id", "comment_id", "created_at", "id"],
  workflow_workspaces: ["project_id"],
};
function compareValues(left, right) {
  if (left === right) return 0;
  if (left == null) return -1;
  if (right == null) return 1;
  return String(left) < String(right) ? -1 : 1;
}

function sortRows(table, rows) {
  const fields = SORT_FIELDS[table];
  return rows.sort((left, right) => {
    for (const field of fields) {
      const comparison = compareValues(left[field], right[field]);
      if (comparison !== 0) return comparison;
    }
    return 0;
  });
}

function sqliteString(value) {
  return `'${String(value).replaceAll("'", "''")}'`;
}

function sanitizeWorkflowValue(value) {
  if (Array.isArray(value)) return value.map(sanitizeWorkflowValue);
  if (!value || typeof value !== "object") return value;

  return Object.fromEntries(
    Object.entries(value).map(([key, child]) => [
      key,
      LOCAL_WORKFLOW_PATH_FIELDS.has(key) ? null : sanitizeWorkflowValue(child),
    ]),
  );
}

function sanitizeWorkflowRow(row) {
  let workspace;
  try {
    workspace = JSON.parse(row.workspace);
  } catch {
    throw new Error(`Workflow workspace for project '${row.project_id}' is not valid JSON`);
  }
  return {
    ...row,
    workspace: JSON.stringify(sanitizeWorkflowValue(workspace)),
  };
}

function buildProjectCounts(tables) {
  const counts = {};
  for (const project of tables.projects) {
    counts[project.id] = {
      projects: 1,
      tasks: 0,
      comments: 0,
      attachments: 0,
      task_relations: 0,
      workflow_workspaces: 0,
    };
  }

  const taskProjects = new Map();
  for (const task of tables.tasks) {
    if (!counts[task.project_id]) {
      throw new Error(`Task '${task.id}' references unknown project '${task.project_id}'`);
    }
    taskProjects.set(task.id, task.project_id);
    counts[task.project_id].tasks += 1;
  }
  for (const comment of tables.comments) {
    const projectId = taskProjects.get(comment.task_id);
    if (!projectId) throw new Error(`Comment '${comment.id}' references unknown task '${comment.task_id}'`);
    counts[projectId].comments += 1;
  }
  for (const attachment of tables.attachments) {
    const projectId = taskProjects.get(attachment.task_id);
    if (!projectId) {
      throw new Error(`Attachment '${attachment.id}' references unknown task '${attachment.task_id}'`);
    }
    counts[projectId].attachments += 1;
  }
  for (const relation of tables.task_relations) {
    const projectId = taskProjects.get(relation.source_task_id);
    if (!projectId) {
      throw new Error(
        `Task relation references unknown source task '${relation.source_task_id}'`,
      );
    }
    counts[projectId].task_relations += 1;
  }
  for (const workspace of tables.workflow_workspaces) {
    if (!counts[workspace.project_id]) {
      throw new Error(`Workflow workspace references unknown project '${workspace.project_id}'`);
    }
    counts[workspace.project_id].workflow_workspaces += 1;
  }

  return Object.fromEntries(
    Object.entries(counts).sort(([left], [right]) => (left < right ? -1 : 1)),
  );
}

function assertSafeAttachmentId(id) {
  if (
    typeof id !== "string"
    || id.length === 0
    || id === "."
    || id === ".."
    || id.includes("/")
    || id.includes("\\")
    || id.includes("\0")
  ) {
    throw new Error(`Attachment id '${id}' is not safe to migrate`);
  }
}

function attachmentObjectKey(attachmentId) {
  return attachmentId;
}

async function readAttachmentPayloads(tables, attachmentsDirectory) {
  const taskProjects = new Map(tables.tasks.map((task) => [task.id, task.project_id]));
  const payloads = [];

  for (const attachment of tables.attachments) {
    assertSafeAttachmentId(attachment.id);
    const sourcePath = path.join(attachmentsDirectory, attachment.id);
    let file;
    try {
      file = await lstat(sourcePath);
    } catch (error) {
      if (error?.code === "ENOENT") {
        throw new Error(`Attachment '${attachment.id}' is missing from '${attachmentsDirectory}'`);
      }
      throw error;
    }
    if (!file.isFile()) {
      throw new Error(`Attachment '${attachment.id}' is not a regular file`);
    }

    const body = await readFile(sourcePath);
    if (attachment.size !== body.byteLength) {
      throw new Error(
        `Attachment '${attachment.id}' size mismatch: SQLite=${attachment.size}, file=${body.byteLength}`,
      );
    }
    const projectId = taskProjects.get(attachment.task_id);
    payloads.push({
      id: attachment.id,
      projectId,
      objectKey: attachmentObjectKey(attachment.id),
      size: body.byteLength,
      sha256: createHash("sha256").update(body).digest("hex"),
      body,
    });
  }

  return payloads;
}

async function readSnapshot(databasePath) {
  const snapshotDirectory = await mkdtemp(path.join(os.tmpdir(), "taskboard-cloud-snapshot-"));
  const snapshotPath = path.join(snapshotDirectory, "taskboard.sqlite");
  let source;
  let snapshot;
  try {
    source = new DatabaseSync(databasePath, { readOnly: true });
    source.exec(`PRAGMA busy_timeout = 5000; VACUUM INTO ${sqliteString(snapshotPath)}`);
    source.close();
    source = null;

    snapshot = new DatabaseSync(snapshotPath, { readOnly: true });
    const integrity = snapshot.prepare("PRAGMA integrity_check").all();
    if (
      integrity.length !== 1
      || String(Object.values(integrity[0])[0]).toLowerCase() !== "ok"
    ) {
      throw new Error("SQLite snapshot failed PRAGMA integrity_check");
    }
    const foreignKeyViolations = snapshot.prepare("PRAGMA foreign_key_check").all();
    if (foreignKeyViolations.length > 0) {
      throw new Error(
        `SQLite snapshot failed PRAGMA foreign_key_check (${foreignKeyViolations.length} violation(s))`,
      );
    }
    const tables = Object.fromEntries(
      TABLE_ORDER.map((table) => [
        table,
        sortRows(table, snapshot.prepare(`SELECT * FROM "${table}"`).all()),
      ]),
    );
    snapshot.close();
    snapshot = null;
    return tables;
  } finally {
    snapshot?.close();
    source?.close();
    await rm(snapshotDirectory, { recursive: true, force: true });
  }
}

function assertCountsMatch(expected, actual) {
  const expectedProjects = Object.keys(expected).sort();
  const actualProjects = Object.keys(actual ?? {}).sort();
  if (JSON.stringify(expectedProjects) !== JSON.stringify(actualProjects)) {
    throw new Error(
      `Cloud count mismatch: expected projects ${expectedProjects.join(", ")}, got ${actualProjects.join(", ")}`,
    );
  }
  for (const projectId of expectedProjects) {
    for (const table of TABLE_ORDER) {
      const expectedCount = Number(expected[projectId][table]);
      const actualCount = Number(actual[projectId]?.[table]);
      if (actualCount !== expectedCount) {
        throw new Error(
          `Cloud count mismatch for project '${projectId}' table '${table}': expected ${expectedCount}, got ${actualCount}`,
        );
      }
    }
  }
}

function validateBundle(bundle) {
  if (!bundle || bundle.schemaVersion !== SCHEMA_VERSION) {
    throw new Error(`Unsupported cloud migration schema version '${bundle?.schemaVersion}'`);
  }
  for (const table of TABLE_ORDER) {
    if (!Array.isArray(bundle.tables?.[table])) {
      throw new Error(`Cloud migration bundle is missing table '${table}'`);
    }
  }
  if (!Array.isArray(bundle.attachments)) {
    throw new Error("Cloud migration bundle is missing attachment payloads");
  }

  const calculatedCounts = buildProjectCounts(bundle.tables);
  assertCountsMatch(calculatedCounts, bundle.counts?.byProject);
  const attachmentRows = new Map(bundle.tables.attachments.map((row) => [row.id, row]));
  const attachmentPayloads = new Map(
    bundle.attachments.map((attachment) => [attachment.id, attachment]),
  );
  if (
    attachmentRows.size !== bundle.tables.attachments.length
    || attachmentPayloads.size !== bundle.attachments.length
    || attachmentRows.size !== attachmentPayloads.size
  ) {
    throw new Error("Cloud migration attachment metadata and payload counts do not match");
  }
  const taskProjects = new Map(
    bundle.tables.tasks.map((task) => [task.id, task.project_id]),
  );
  for (const row of attachmentRows.values()) {
    const attachment = attachmentPayloads.get(row.id);
    if (!attachment) {
      throw new Error(`Attachment metadata '${row.id}' has no payload`);
    }
    const body = Buffer.from(attachment.body);
    const sha256 = createHash("sha256").update(body).digest("hex");
    if (body.byteLength !== attachment.size) {
      throw new Error(`Attachment '${attachment.id}' size verification failed`);
    }
    if (sha256 !== attachment.sha256) {
      throw new Error(`Attachment '${attachment.id}' SHA-256 verification failed`);
    }
    if (Number(row.size) !== attachment.size) {
      throw new Error(`Attachment '${attachment.id}' metadata size does not match its payload`);
    }
    const projectId = taskProjects.get(row.task_id);
    if (attachment.projectId !== projectId) {
      throw new Error(
        `Attachment '${attachment.id}' project does not match its task project`,
      );
    }
    if (attachment.objectKey !== attachmentObjectKey(attachment.id)) {
      throw new Error(`Attachment '${attachment.id}' object key must match its id`);
    }
    if (
      typeof attachment.objectKey !== "string"
      || path.posix.isAbsolute(attachment.objectKey)
      || attachment.objectKey.split("/").includes("..")
    ) {
      throw new Error(`Attachment '${attachment.id}' has an unsafe object key`);
    }
  }
}

export async function createCloudMigrationBundle({
  databasePath,
  attachmentsDirectory,
}) {
  const tables = await readSnapshot(databasePath);
  tables.projects = tables.projects.map((project) => ({
    ...project,
    workspace_path: null,
  }));
  tables.tasks = tables.tasks.map((task) => ({
    ...task,
    worktree_path: null,
  }));
  tables.workflow_workspaces = tables.workflow_workspaces.map(sanitizeWorkflowRow);

  return {
    schemaVersion: SCHEMA_VERSION,
    createdAt: new Date().toISOString(),
    counts: {
      byProject: buildProjectCounts(tables),
    },
    tables,
    attachments: await readAttachmentPayloads(tables, attachmentsDirectory),
  };
}

const CLOUD_COLUMNS = {
  projects: ["id", "name", "workspace_path", "next_task_number", "created_at", "updated_at"],
  tasks: [
    "id", "identifier", "project_id", "title", "description", "status", "priority", "labels",
    "sort_order", "thread_id", "creator_type", "creator_id", "creator_name",
    "creator_avatar_url", "assignee_type", "assignee_id", "assignee_name",
    "assignee_avatar_url", "workflow_id", "development_context_type", "development_branch",
    "due_date", "recurrence_interval", "recurrence_unit", "archived_at", "version",
    "created_at", "updated_at",
  ],
  comments: [
    "id", "task_id", "body", "thread_id", "author_type", "author_id", "author_name",
    "author_avatar_url", "version", "created_at", "updated_at",
  ],
  task_relations: ["relation_type", "source_task_id", "target_task_id", "created_at"],
  attachments: ["id", "task_id", "comment_id", "filename", "content_type", "size", "created_at"],
  workflow_workspaces: ["project_id", "workspace", "version", "updated_at"],
};

function cloudTaskRow(task) {
  const isWorktree = task.worktree_branch != null;
  return {
    ...task,
    development_context_type: isWorktree
      ? "worktree"
      : task.git_branch != null
        ? "branch"
        : null,
    development_branch: isWorktree ? task.worktree_branch : task.git_branch,
  };
}

function cloudRows(table, rows) {
  return table === "tasks" ? rows.map(cloudTaskRow) : rows;
}

function insertTableSql(table) {
  const columns = CLOUD_COLUMNS[table];
  return `INSERT INTO "${table}" (${columns.map((column) => `"${column}"`).join(", ")})
       SELECT ${columns.map((column) => `json_extract(value, '$.${column}')`).join(", ")}
       FROM json_each(?)`;
}

export function createCloudD1ImportPlan(tables) {
  return TABLE_ORDER.map((table) => {
    const columns = CLOUD_COLUMNS[table];
    const values = cloudRows(table, tables[table]).map((row) => (
      Object.fromEntries(columns.map((column) => [column, row[column] ?? null]))
    ));
    return { table, sql: insertTableSql(table), json: JSON.stringify(values) };
  });
}

function inlineD1InsertStatement(table, values) {
  const json = JSON.stringify(values);
  if (json.includes("\0")) throw new Error("D1 migration JSON cannot contain null bytes");
  return `${insertTableSql(table).replace("?", sqliteString(json))};`;
}

export function createCloudD1ImportSql(tables) {
  const statements = [];
  for (const { table, json } of createCloudD1ImportPlan(tables)) {
    const values = JSON.parse(json);
    if (values.length === 0) {
      statements.push(inlineD1InsertStatement(table, values));
      continue;
    }

    let chunk = [];
    for (const row of values) {
      const candidate = [...chunk, row];
      const statement = inlineD1InsertStatement(table, candidate);
      if (Buffer.byteLength(statement, "utf8") < WRANGLER_D1_STATEMENT_MAX_BYTES) {
        chunk = candidate;
        continue;
      }
      if (chunk.length === 0) {
        const identity = row.id ?? row.project_id ?? "unknown";
        throw new Error(
          `D1 import single row '${table}:${identity}' exceeds 90,000 bytes`,
        );
      }
      statements.push(inlineD1InsertStatement(table, chunk));
      chunk = [row];
      if (
        Buffer.byteLength(
          inlineD1InsertStatement(table, chunk),
          "utf8",
        ) >= WRANGLER_D1_STATEMENT_MAX_BYTES
      ) {
        const identity = row.id ?? row.project_id ?? "unknown";
        throw new Error(
          `D1 import single row '${table}:${identity}' exceeds 90,000 bytes`,
        );
      }
    }
    statements.push(inlineD1InsertStatement(table, chunk));
  }
  return statements.join("\n");
}

export const CLOUD_PROJECT_COUNTS_SQL = `
  SELECT p.id AS project_id, 1 AS projects,
    (SELECT COUNT(*) FROM tasks t WHERE t.project_id = p.id) AS tasks,
    (SELECT COUNT(*) FROM comments c JOIN tasks t ON t.id = c.task_id
      WHERE t.project_id = p.id) AS comments,
    (SELECT COUNT(*) FROM attachments a JOIN tasks t ON t.id = a.task_id
      WHERE t.project_id = p.id) AS attachments,
    (SELECT COUNT(*) FROM task_relations r JOIN tasks t ON t.id = r.source_task_id
      WHERE t.project_id = p.id) AS task_relations,
    (SELECT COUNT(*) FROM workflow_workspaces w
      WHERE w.project_id = p.id) AS workflow_workspaces
  FROM projects p ORDER BY p.id
`;

export function createCloudBindingMigrationAdapters({ d1, r2 }) {
  return {
    d1: {
      async importTables(tables) {
        await d1.batch(
          createCloudD1ImportPlan(tables).map(({ sql, json }) => (
            d1.prepare(sql).bind(json)
          )),
        );
      },
      async countByProject() {
        const result = await d1.prepare(CLOUD_PROJECT_COUNTS_SQL).all();
        return Object.fromEntries(result.results.map((row) => [
          row.project_id,
          Object.fromEntries(TABLE_ORDER.map((table) => [table, Number(row[table])])),
        ]));
      },
    },
    r2: {
      put: (key, body, options) => r2.put(key, body, options),
      head: (key) => r2.head(key),
      delete: (key) => r2.delete(key),
    },
  };
}

async function verifyR2Attachments(bundle, r2) {
  let verified = 0;
  for (const attachment of bundle.attachments) {
    const object = await r2.head(attachment.objectKey);
    if (!object) throw new Error(`R2 object '${attachment.objectKey}' is missing`);
    if (Number(object.size) !== attachment.size) {
      throw new Error(`R2 size verification failed for attachment '${attachment.id}'`);
    }
    if (object.customMetadata?.sha256 !== attachment.sha256) {
      throw new Error(`R2 SHA-256 hash verification failed for attachment '${attachment.id}'`);
    }
    verified += 1;
  }
  return verified;
}

export async function verifyCloudMigrationBundle(bundle, { d1, r2 }) {
  validateBundle(bundle);
  const counts = await d1.countByProject();
  assertCountsMatch(bundle.counts.byProject, counts);
  const verified = await verifyR2Attachments(bundle, r2);
  return {
    counts: { byProject: structuredClone(bundle.counts.byProject) },
    attachments: { verified },
  };
}

export async function importCloudMigrationBundle(bundle, { d1, r2 }) {
  validateBundle(bundle);

  const existingCounts = await d1.countByProject();
  if (Object.keys(existingCounts).length > 0) {
    throw new Error("Cloud migration target D1 is not empty");
  }
  for (const attachment of bundle.attachments) {
    if (await r2.head(attachment.objectKey)) {
      throw new Error(`R2 object '${attachment.objectKey}' already exists`);
    }
  }

  const attachmentRows = new Map(bundle.tables.attachments.map((row) => [row.id, row]));
  const uploadedKeys = [];
  let d1Committed = false;
  try {
    for (const attachment of bundle.attachments) {
      const row = attachmentRows.get(attachment.id);
      await r2.put(attachment.objectKey, Buffer.from(attachment.body), {
        customMetadata: { sha256: attachment.sha256 },
        httpMetadata: { contentType: row.content_type },
      });
      uploadedKeys.push(attachment.objectKey);
    }
    const verified = await verifyR2Attachments(bundle, r2);

    await d1.importTables(bundle.tables);
    d1Committed = true;
    const counts = await d1.countByProject();
    assertCountsMatch(bundle.counts.byProject, counts);
    return {
      counts: { byProject: structuredClone(bundle.counts.byProject) },
      attachments: { verified },
    };
  } catch (error) {
    if (!d1Committed && typeof r2.delete === "function") {
      const cleanup = await Promise.allSettled(uploadedKeys.map((key) => r2.delete(key)));
      const failures = cleanup.filter((result) => result.status === "rejected");
      if (failures.length > 0) {
        throw new AggregateError(
          [error, ...failures.map((result) => result.reason)],
          `${error.message}; cleanup failed for ${failures.length} R2 object(s)`,
        );
      }
    }
    throw error;
  }
}

function bundleFile(outputDirectory, relativePath) {
  const resolvedRoot = path.resolve(outputDirectory);
  const resolved = path.resolve(resolvedRoot, relativePath);
  if (!resolved.startsWith(`${resolvedRoot}${path.sep}`)) {
    throw new Error(`Unsafe bundle path '${relativePath}'`);
  }
  return resolved;
}

async function writePrivateFile(filename, contents) {
  await writeFile(filename, contents, { mode: 0o600, flag: "wx" });
  await chmod(filename, 0o600);
}

export async function writeCloudMigrationBundle(bundle, outputDirectory) {
  validateBundle(bundle);
  const parent = path.dirname(path.resolve(outputDirectory));
  await mkdir(parent, { recursive: true });
  await mkdir(outputDirectory, { mode: 0o700 });
  try {
    const dataDirectory = path.join(outputDirectory, "data");
    const attachmentsDirectory = path.join(outputDirectory, "attachments");
    await mkdir(dataDirectory, { mode: 0o700 });
    await mkdir(attachmentsDirectory, { mode: 0o700 });

    const tableFiles = {};
    for (const table of TABLE_ORDER) {
      const relativePath = `data/${table}.json`;
      await writePrivateFile(
        bundleFile(outputDirectory, relativePath),
        `${JSON.stringify(bundle.tables[table], null, 2)}\n`,
      );
      tableFiles[table] = {
        file: relativePath,
        rows: bundle.tables[table].length,
      };
    }

    const attachmentFiles = [];
    for (const attachment of bundle.attachments) {
      const relativePath = `attachments/${encodeURIComponent(attachment.id)}`;
      await writePrivateFile(
        bundleFile(outputDirectory, relativePath),
        Buffer.from(attachment.body),
      );
      attachmentFiles.push({
        id: attachment.id,
        projectId: attachment.projectId,
        objectKey: attachment.objectKey,
        file: relativePath,
        size: attachment.size,
        sha256: attachment.sha256,
      });
    }

    const manifest = {
      schemaVersion: SCHEMA_VERSION,
      createdAt: bundle.createdAt,
      counts: bundle.counts,
      tables: tableFiles,
      attachments: attachmentFiles,
    };
    await writePrivateFile(
      path.join(outputDirectory, "manifest.json"),
      `${JSON.stringify(manifest, null, 2)}\n`,
    );
    return path.resolve(outputDirectory);
  } catch (error) {
    await rm(outputDirectory, { recursive: true, force: true });
    throw error;
  }
}

async function readJsonFile(filename, label) {
  try {
    return JSON.parse(await readFile(filename, "utf8"));
  } catch (error) {
    throw new Error(`Cannot read ${label}: ${error.message}`);
  }
}

export async function readCloudMigrationBundle(inputDirectory) {
  const manifest = await readJsonFile(
    path.join(inputDirectory, "manifest.json"),
    "cloud migration manifest",
  );
  if (manifest.schemaVersion !== SCHEMA_VERSION) {
    throw new Error(`Unsupported cloud migration schema version '${manifest.schemaVersion}'`);
  }

  const tables = {};
  for (const table of TABLE_ORDER) {
    const entry = manifest.tables?.[table];
    if (!entry?.file) throw new Error(`Cloud migration manifest is missing table '${table}'`);
    const rows = await readJsonFile(
      bundleFile(inputDirectory, entry.file),
      `table '${table}'`,
    );
    if (!Array.isArray(rows) || rows.length !== entry.rows) {
      throw new Error(`Cloud migration row count mismatch for table '${table}'`);
    }
    tables[table] = rows;
  }

  const attachments = [];
  for (const entry of manifest.attachments ?? []) {
    const body = await readFile(bundleFile(inputDirectory, entry.file));
    attachments.push({ ...entry, body });
  }
  const bundle = {
    schemaVersion: manifest.schemaVersion,
    createdAt: manifest.createdAt,
    counts: manifest.counts,
    tables,
    attachments,
  };
  validateBundle(bundle);
  return bundle;
}

function parseOptions(args, allowed, required) {
  const options = {};
  for (let index = 0; index < args.length; index += 2) {
    const flag = args[index];
    const value = args[index + 1];
    if (!flag?.startsWith("--") || value == null || value.startsWith("--")) {
      throw new Error(`Expected a value after '${flag ?? ""}'`);
    }
    const name = flag.slice(2);
    if (!allowed.has(name)) throw new Error(`Unknown option '--${name}'`);
    if (Object.hasOwn(options, name)) throw new Error(`Duplicate option '--${name}'`);
    options[name] = value;
  }
  for (const name of required) {
    if (!options[name]) throw new Error(`Missing required option '--${name}'`);
  }
  return options;
}

const HELP = `Usage:
  node scripts/migrate-to-cloud.mjs export --database <sqlite> --attachments <directory> --output <directory>
  node scripts/migrate-to-cloud.mjs import --bundle <directory> --adapter <module>
  node scripts/migrate-to-cloud.mjs verify --bundle <directory> --adapter <module>

The bundled Wrangler adapter uses Wrangler authentication without reading or
storing credentials. Remote operations require TASKBOARD_MIGRATION_REMOTE=1.`;

async function loadAdapterModule(modulePath, context) {
  const loaded = await import(pathToFileURL(path.resolve(modulePath)).href);
  const factory = loaded.createCloudMigrationAdapters ?? loaded.default;
  if (typeof factory !== "function") {
    throw new Error(
      "Adapter module must export createCloudMigrationAdapters or a default factory",
    );
  }
  return factory(context);
}

export async function runCli(
  argv,
  {
    stdout = process.stdout,
    loadAdapters = loadAdapterModule,
  } = {},
) {
  const [command, ...args] = argv;
  if (!command || command === "--help" || command === "help") {
    stdout.write(`${HELP}\n`);
    return 0;
  }

  if (command === "export") {
    const options = parseOptions(
      args,
      new Set(["database", "attachments", "output"]),
      ["database", "attachments", "output"],
    );
    const bundle = await createCloudMigrationBundle({
      databasePath: options.database,
      attachmentsDirectory: options.attachments,
    });
    const output = await writeCloudMigrationBundle(bundle, options.output);
    stdout.write(`${JSON.stringify({ output, counts: bundle.counts })}\n`);
    return 0;
  }

  if (command !== "import" && command !== "verify") {
    throw new Error(`Unknown command '${command}'`);
  }
  const options = parseOptions(
    args,
    new Set(["bundle", "adapter"]),
    ["bundle", "adapter"],
  );
  const bundle = await readCloudMigrationBundle(options.bundle);
  const adapters = await loadAdapters(options.adapter, { bundle, command });
  if (!adapters?.d1 || !adapters?.r2) {
    throw new Error("Adapter factory must return d1 and r2 adapters");
  }
  if (command === "import" && typeof adapters.d1.importTables !== "function") {
    throw new Error("Import adapter must provide atomic d1.importTables");
  }
  if (command === "import" && typeof adapters.r2.delete !== "function") {
    throw new Error("Import adapter must provide r2.delete for failed-migration cleanup");
  }
  try {
    const result = command === "import"
      ? await importCloudMigrationBundle(bundle, adapters)
      : await verifyCloudMigrationBundle(bundle, adapters);
    stdout.write(`${JSON.stringify(result)}\n`);
  } finally {
    await adapters.cleanup?.();
  }
  return 0;
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  runCli(process.argv.slice(2)).catch((error) => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
