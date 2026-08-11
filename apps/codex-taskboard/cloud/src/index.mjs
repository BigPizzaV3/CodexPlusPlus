import { normalizeWorkflowSnapshot } from "../../shared/workflow-control-flow.mjs";
import { DEFAULT_PROJECT_ID } from "../../shared/domain.mjs";

const JSON_BODY_LIMIT = 1024 * 1024;
const ATTACHMENT_BODY_LIMIT = 25 * 1024 * 1024;
const PROJECT_ID_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const TASK_STATUSES = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "blocked",
  "done",
  "canceled",
];
const TASK_PRIORITIES = ["none", "urgent", "high", "medium", "low"];
const INLINE_ATTACHMENT_TYPES = new Set([
  "application/pdf",
  "image/avif",
  "image/gif",
  "image/jpeg",
  "image/png",
  "image/webp",
  "text/plain",
]);

class ApiError extends Error {
  constructor(status, code, message, details) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

function json(status, value, headers = {}) {
  return new Response(JSON.stringify(value), {
    status,
    headers: {
      "cache-control": "no-store",
      "content-type": "application/json; charset=utf-8",
      ...headers,
    },
  });
}

function empty(status, headers = {}) {
  return new Response(null, {
    status,
    headers: { "cache-control": "no-store", ...headers },
  });
}

function methodNotAllowed(allowed) {
  throw new ApiError(405, "METHOD_NOT_ALLOWED", "Method not allowed", {
    allowed,
  });
}

function assertPlainObject(value) {
  if (value === null || typeof value !== "object" || Array.isArray(value)) {
    throw new ApiError(400, "INVALID_BODY", "Request body must be a JSON object");
  }
}

function assertAllowedKeys(value, allowed) {
  const unknown = Object.keys(value).filter((key) => !allowed.has(key));
  if (unknown.length > 0) {
    throw new ApiError(
      400,
      "UNKNOWN_FIELD",
      `Unknown field${unknown.length === 1 ? "" : "s"}: ${unknown.join(", ")}`,
    );
  }
}

function stringField(value, name, {
  required = false,
  nullable = false,
  maxLength,
} = {}) {
  if (value === undefined) {
    if (required) {
      throw new ApiError(400, "INVALID_FIELD", `'${name}' is required`);
    }
    return undefined;
  }
  if (nullable && value === null) return null;
  if (typeof value !== "string") {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      `'${name}' must be a string${nullable ? " or null" : ""}`,
    );
  }
  const normalized = value.trim();
  if (required && normalized.length === 0) {
    throw new ApiError(400, "INVALID_FIELD", `'${name}' cannot be empty`);
  }
  if (normalized.length > maxLength) {
    throw new ApiError(400, "INVALID_FIELD", `'${name}' cannot exceed ${maxLength} characters`);
  }
  return normalized;
}

function parseVersion(value, { allowZero = false } = {}) {
  if (!Number.isSafeInteger(value) || value < (allowZero ? 0 : 1)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      `'version' must be a ${allowZero ? "non-negative" : "positive"} integer`,
    );
  }
  return value;
}

function parseStatus(value, fallback) {
  const status = value ?? fallback;
  if (!TASK_STATUSES.includes(status)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      `'status' must be one of: ${TASK_STATUSES.join(", ")}`,
    );
  }
  return status;
}

function parsePriority(value, fallback) {
  const priority = value ?? fallback;
  if (!TASK_PRIORITIES.includes(priority)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'priority' must be none, urgent, high, medium, or low",
    );
  }
  return priority;
}

function parseLabels(value) {
  if (!Array.isArray(value) || value.length > 20) {
    throw new ApiError(400, "INVALID_FIELD", "'labels' must be an array with at most 20 entries");
  }
  const labels = value.map((label) => {
    if (typeof label !== "string") {
      throw new ApiError(400, "INVALID_FIELD", "Every label must be a string");
    }
    const normalized = label.trim();
    if (normalized.length === 0 || normalized.length > 64) {
      throw new ApiError(400, "INVALID_FIELD", "Labels must contain 1 to 64 characters");
    }
    return normalized;
  });
  if (new Set(labels).size !== labels.length) {
    throw new ApiError(400, "INVALID_FIELD", "Labels must be unique");
  }
  return labels;
}

function parseSortOrder(value) {
  if (
    typeof value !== "number"
    || !Number.isFinite(value)
    || Math.abs(value) > 1_000_000_000_000
  ) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'sortOrder' must be a finite number between -1000000000000 and 1000000000000",
    );
  }
  return value;
}

function parseDueDate(value) {
  const date = stringField(value, "dueDate", { nullable: true, maxLength: 10 });
  if (date !== null && date !== undefined && !/^\d{4}-\d{2}-\d{2}$/.test(date)) {
    throw new ApiError(400, "INVALID_FIELD", "'dueDate' must use YYYY-MM-DD");
  }
  return date;
}

function parseRecurrence(value) {
  if (value === null) return null;
  assertPlainObject(value);
  assertAllowedKeys(value, new Set(["interval", "unit"]));
  if (!Number.isSafeInteger(value.interval) || value.interval < 1 || value.interval > 365) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'recurrence.interval' must be an integer from 1 to 365",
    );
  }
  if (!["day", "week", "month", "year"].includes(value.unit)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'recurrence.unit' must be day, week, month, or year",
    );
  }
  return { interval: value.interval, unit: value.unit };
}

function parseDevelopmentContext(value) {
  if (value === null) return null;
  assertPlainObject(value);
  if (value.type === "branch") {
    assertAllowedKeys(value, new Set(["type", "branch"]));
    return {
      type: "branch",
      branch: stringField(value.branch, "developmentContext.branch", {
        required: true,
        maxLength: 512,
      }),
    };
  }
  if (value.type === "worktree") {
    assertAllowedKeys(value, new Set(["type", "path", "branch"]));
    const worktreePath = stringField(value.path, "developmentContext.path", {
      maxLength: 4096,
    });
    if (worktreePath?.includes("\0")) {
      throw new ApiError(
        400,
        "INVALID_FIELD",
        "'developmentContext.path' cannot contain null bytes",
      );
    }
    return {
      type: "worktree",
      branch: stringField(value.branch ?? null, "developmentContext.branch", {
        nullable: true,
        maxLength: 512,
      }),
    };
  }
  throw new ApiError(
    400,
    "INVALID_FIELD",
    "'developmentContext.type' must be branch or worktree",
  );
}

function parseThreadId(value) {
  if (value === undefined) return undefined;
  return stringField(value, "threadId", { required: true, maxLength: 256 });
}

function parseWorkflowId(value) {
  const workflowId = stringField(value, "workflowId", {
    nullable: true,
    maxLength: 128,
  });
  if (workflowId === "") {
    throw new ApiError(400, "INVALID_FIELD", "'workflowId' cannot be empty");
  }
  return workflowId;
}

function parseAssigneeTarget(value) {
  if (value === undefined) return undefined;
  if (!["current-user", "codex-agent"].includes(value)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'assigneeTarget' must be current-user or codex-agent",
    );
  }
  return value;
}

function slugify(value) {
  return value
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 64);
}

function validateProjectId(value) {
  const id = stringField(value, "id", { required: true, maxLength: 64 });
  if (!PROJECT_ID_PATTERN.test(id)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'id' must be a lowercase slug containing letters, numbers, or hyphens",
    );
  }
  return id;
}

function projectPrefix(projectId) {
  const prefix = projectId.toUpperCase().replace(/[^A-Z0-9]+/g, "");
  return (prefix || "TASK").slice(0, 12);
}

function now() {
  return new Date().toISOString();
}

function uuid() {
  return crypto.randomUUID();
}

function decodeBasicCredentials(header) {
  if (!header?.startsWith("Basic ")) return null;
  let bytes;
  try {
    const binary = atob(header.slice(6).trim());
    bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  } catch {
    return null;
  }
  let value;
  try {
    value = new TextDecoder("utf-8", { fatal: true }).decode(bytes);
  } catch {
    return null;
  }
  const separator = value.indexOf(":");
  if (separator < 1) return null;
  return {
    username: value.slice(0, separator),
    password: value.slice(separator + 1),
  };
}

function unauthorized() {
  return json(
    401,
    { error: { code: "UNAUTHORIZED", message: "Valid Basic credentials are required" } },
    { "www-authenticate": 'Basic realm="Codex Taskboard", charset="UTF-8"' },
  );
}

async function authenticate(request, env) {
  if (typeof env.TASKBOARD_SHARED_SECRET !== "string" || env.TASKBOARD_SHARED_SECRET === "") {
    throw new ApiError(
      500,
      "SERVER_MISCONFIGURED",
      "TASKBOARD_SHARED_SECRET is not configured",
    );
  }
  const credentials = decodeBasicCredentials(request.headers.get("authorization"));
  if (!credentials) return null;
  const encoder = new TextEncoder();
  const [providedSecret, configuredSecret] = await Promise.all([
    crypto.subtle.digest("SHA-256", encoder.encode(credentials.password)),
    crypto.subtle.digest("SHA-256", encoder.encode(env.TASKBOARD_SHARED_SECRET)),
  ]);
  if (!crypto.subtle.timingSafeEqual(providedSecret, configuredSecret)) return null;
  const username = stringField(credentials.username, "Basic username", {
    required: true,
    maxLength: 120,
  });
  const userId = `basic:${encodeURIComponent(username.toLowerCase())}`;
  if (request.headers.get("x-taskboard-client") === "taskctl") {
    return {
      type: "agent",
      id: `${userId}:codex-agent`,
      name: `Codex Agent (${username})`,
      avatarUrl: null,
      username,
    };
  }
  return {
    type: "user",
    id: userId,
    name: username,
    avatarUrl: null,
    username,
  };
}

function resolveAssignee(target, actor) {
  if (target === undefined || target === "current-user") return actor;
  const userId = `basic:${encodeURIComponent(actor.username.toLowerCase())}`;
  return {
    type: "agent",
    id: `${userId}:codex-agent`,
    name: `Codex Agent (${actor.username})`,
    avatarUrl: null,
  };
}

async function readJson(request) {
  const contentType = request.headers.get("content-type") ?? "";
  if (!contentType.toLowerCase().startsWith("application/json")) {
    throw new ApiError(
      415,
      "UNSUPPORTED_MEDIA_TYPE",
      "Content-Type must be application/json",
    );
  }
  const contentLength = Number(request.headers.get("content-length") ?? 0);
  if (contentLength > JSON_BODY_LIMIT) {
    throw new ApiError(413, "BODY_TOO_LARGE", "JSON body cannot exceed 1 MiB");
  }
  const text = await request.text();
  if (new TextEncoder().encode(text).byteLength > JSON_BODY_LIMIT) {
    throw new ApiError(413, "BODY_TOO_LARGE", "JSON body cannot exceed 1 MiB");
  }
  try {
    return JSON.parse(text);
  } catch {
    throw new ApiError(400, "INVALID_JSON", "Request body is not valid JSON");
  }
}

async function readAttachment(request) {
  const contentLength = Number(request.headers.get("content-length") ?? 0);
  if (contentLength > ATTACHMENT_BODY_LIMIT) {
    throw new ApiError(413, "BODY_TOO_LARGE", "Attachment cannot exceed 25 MiB");
  }
  const body = await request.arrayBuffer();
  if (body.byteLength > ATTACHMENT_BODY_LIMIT) {
    throw new ApiError(413, "BODY_TOO_LARGE", "Attachment cannot exceed 25 MiB");
  }
  return body;
}

function parseAttachmentHeaders(request) {
  const encodedFilename = request.headers.get("x-taskboard-filename");
  if (encodedFilename === null) {
    throw new ApiError(400, "INVALID_FILENAME", "X-Taskboard-Filename is required");
  }
  let filename;
  try {
    filename = decodeURIComponent(encodedFilename).trim();
  } catch {
    throw new ApiError(
      400,
      "INVALID_FILENAME",
      "Attachment filename contains invalid encoding",
    );
  }
  if (
    filename.length === 0
    || filename.length > 240
    || filename === "."
    || filename === ".."
    || /[\u0000-\u001f\u007f/\\]/.test(filename)
  ) {
    throw new ApiError(400, "INVALID_FILENAME", "Attachment filename is invalid");
  }
  const rawContentType = request.headers.get("content-type");
  const contentType = rawContentType
    ? rawContentType.split(";", 1)[0].trim().toLowerCase()
    : "application/octet-stream";
  if (
    contentType.length === 0
    || contentType.length > 200
    || !/^[!#$%&'*+.^_`|~0-9a-z-]+\/[!#$%&'*+.^_`|~0-9a-z-]+$/.test(contentType)
  ) {
    throw new ApiError(
      415,
      "UNSUPPORTED_MEDIA_TYPE",
      "Attachment Content-Type is invalid",
    );
  }
  return { filename, contentType };
}

function projectFromRow(row) {
  return {
    id: row.id,
    name: row.name,
    workspacePath: null,
    archivedAt: row.archived_at,
    issueCount: Number(row.issue_count ?? 0),
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

function developmentContextFromRow(row) {
  if (row.development_context_type === "worktree") {
    return {
      type: "worktree",
      path: null,
      branch: row.development_branch,
    };
  }
  if (row.development_context_type === "branch") {
    return { type: "branch", branch: row.development_branch };
  }
  return null;
}

function taskFromRow(row) {
  return {
    id: row.id,
    identifier: row.identifier,
    projectId: row.project_id,
    title: row.title,
    description: row.description,
    status: row.status,
    priority: row.priority,
    labels: JSON.parse(row.labels),
    sortOrder: row.sort_order,
    threadId: row.thread_id,
    creatorType: row.creator_type,
    creatorId: row.creator_id,
    creatorName: row.creator_name,
    creatorAvatarUrl: row.creator_avatar_url,
    assignee: {
      type: row.assignee_type,
      id: row.assignee_id,
      name: row.assignee_name,
      avatarUrl: row.assignee_avatar_url,
    },
    workflowId: row.workflow_id,
    developmentContext: developmentContextFromRow(row),
    dueDate: row.due_date,
    recurrence: row.recurrence_interval && row.recurrence_unit
      ? { interval: row.recurrence_interval, unit: row.recurrence_unit }
      : null,
    archivedAt: row.archived_at,
    version: row.version,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

function taskRelationSummaryFromRow(row) {
  return {
    id: row.id,
    identifier: row.identifier,
    projectId: row.project_id,
    title: row.title,
    status: row.status,
    priority: row.priority,
    threadId: row.thread_id,
    assignee: {
      type: row.assignee_type,
      id: row.assignee_id,
      name: row.assignee_name,
      avatarUrl: row.assignee_avatar_url,
    },
    archivedAt: row.archived_at,
  };
}

function commentFromRow(row, attachments = []) {
  return {
    id: row.id,
    taskId: row.task_id,
    body: row.body,
    threadId: row.thread_id,
    authorType: row.author_type,
    authorId: row.author_id,
    authorName: row.author_name,
    authorAvatarUrl: row.author_avatar_url,
    attachments,
    version: row.version,
    createdAt: row.created_at,
    updatedAt: row.updated_at,
  };
}

function attachmentFromRow(row) {
  return {
    id: row.id,
    taskId: row.task_id,
    commentId: row.comment_id,
    filename: row.filename,
    contentType: row.content_type,
    size: row.size,
    createdAt: row.created_at,
  };
}

async function all(statement) {
  return (await statement.all()).results;
}

function changed(result) {
  if (typeof result?.meta?.changes !== "number") {
    throw new Error("D1 mutation did not return change metadata");
  }
  return result.meta.changes > 0;
}

async function requireProject(env, id) {
  const row = await env.DB.prepare(
    "SELECT * FROM projects WHERE id = ? AND archived_at IS NULL",
  ).bind(id).first();
  if (!row) {
    throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${id}' does not exist`);
  }
  return row;
}

async function taskRow(env, id) {
  return env.DB.prepare(
    "SELECT * FROM tasks WHERE id = ? OR identifier = ?",
  ).bind(id, id).first();
}

async function requireTaskRow(env, id) {
  const row = await taskRow(env, id);
  if (!row) throw new ApiError(404, "TASK_NOT_FOUND", `Task '${id}' does not exist`);
  return row;
}

function assertTaskVersion(row, expectedVersion) {
  if (row.version !== expectedVersion) {
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion, actualVersion: row.version },
    );
  }
}

async function attachmentsForComment(env, commentId) {
  return (
    await all(
      env.DB.prepare(
        "SELECT * FROM attachments WHERE comment_id = ? ORDER BY created_at, id",
      ).bind(commentId),
    )
  ).map(attachmentFromRow);
}

async function hydrateComment(env, row) {
  return commentFromRow(row, await attachmentsForComment(env, row.id));
}

async function hydrateTask(env, row) {
  const task = taskFromRow(row);
  const [parent, subIssues, blockedBy, blocks, related] = await Promise.all([
    env.DB.prepare(`
      SELECT tasks.*
      FROM task_relations
      JOIN tasks ON tasks.id = task_relations.source_task_id
      WHERE task_relations.relation_type = 'parent'
        AND task_relations.target_task_id = ?
    `).bind(task.id).first(),
    all(env.DB.prepare(`
      SELECT tasks.*
      FROM task_relations
      JOIN tasks ON tasks.id = task_relations.target_task_id
      WHERE task_relations.relation_type = 'parent'
        AND task_relations.source_task_id = ?
      ORDER BY tasks.sort_order, tasks.created_at, tasks.id
    `).bind(task.id)),
    all(env.DB.prepare(`
      SELECT tasks.*
      FROM task_relations
      JOIN tasks ON tasks.id = task_relations.source_task_id
      WHERE task_relations.relation_type = 'blocks'
        AND task_relations.target_task_id = ?
      ORDER BY tasks.sort_order, tasks.created_at, tasks.id
    `).bind(task.id)),
    all(env.DB.prepare(`
      SELECT tasks.*
      FROM task_relations
      JOIN tasks ON tasks.id = task_relations.target_task_id
      WHERE task_relations.relation_type = 'blocks'
        AND task_relations.source_task_id = ?
      ORDER BY tasks.sort_order, tasks.created_at, tasks.id
    `).bind(task.id)),
    all(env.DB.prepare(`
      SELECT tasks.*
      FROM task_relations
      JOIN tasks ON tasks.id = CASE
        WHEN task_relations.source_task_id = ? THEN task_relations.target_task_id
        ELSE task_relations.source_task_id
      END
      WHERE task_relations.relation_type = 'related'
        AND (
          task_relations.source_task_id = ?
          OR task_relations.target_task_id = ?
        )
      ORDER BY tasks.sort_order, tasks.created_at, tasks.id
    `).bind(task.id, task.id, task.id)),
  ]);
  task.relations = {
    parent: parent ? taskRelationSummaryFromRow(parent) : null,
    subIssues: subIssues.map(taskRelationSummaryFromRow),
    blockedBy: blockedBy.map(taskRelationSummaryFromRow),
    blocks: blocks.map(taskRelationSummaryFromRow),
    related: related.map(taskRelationSummaryFromRow),
  };
  return task;
}

async function getTask(env, id) {
  const row = await taskRow(env, id);
  return row ? hydrateTask(env, row) : null;
}

function parseProjectCreate(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set(["id", "name", "workspacePath"]));
  const name = stringField(body.name, "name", { required: true, maxLength: 120 });
  const id = validateProjectId(body.id ?? slugify(name));
  if (body.workspacePath !== undefined && body.workspacePath !== null) {
    const workspacePath = stringField(body.workspacePath, "workspacePath", {
      required: true,
      maxLength: 4096,
    });
    if (workspacePath.includes("\0")) {
      throw new ApiError(400, "INVALID_FIELD", "'workspacePath' cannot contain null bytes");
    }
  }
  return { id, name };
}

function parseTaskCreate(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set([
    "projectId",
    "title",
    "description",
    "status",
    "priority",
    "labels",
    "sortOrder",
    "threadId",
    "assigneeTarget",
    "workflowId",
    "developmentContext",
    "dueDate",
    "recurrence",
  ]));
  const input = {
    projectId: validateProjectId(body.projectId ?? "local"),
    title: stringField(body.title, "title", { required: true, maxLength: 240 }),
    description: stringField(body.description ?? "", "description", { maxLength: 100_000 }),
    status: parseStatus(body.status, "backlog"),
    priority: parsePriority(body.priority, "none"),
    labels: body.labels === undefined ? [] : parseLabels(body.labels),
    sortOrder: body.sortOrder === undefined ? undefined : parseSortOrder(body.sortOrder),
    threadId: parseThreadId(body.threadId),
    assigneeTarget: parseAssigneeTarget(body.assigneeTarget),
    workflowId: parseWorkflowId(body.workflowId ?? null),
    developmentContext: parseDevelopmentContext(body.developmentContext ?? null),
    dueDate: parseDueDate(body.dueDate ?? null),
    recurrence: parseRecurrence(body.recurrence ?? null),
  };
  if (input.recurrence && !input.dueDate) {
    throw new ApiError(400, "INVALID_FIELD", "A recurring issue requires 'dueDate'");
  }
  return input;
}

function parseTaskPatch(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set([
    "version",
    "title",
    "description",
    "status",
    "priority",
    "labels",
    "threadId",
    "assigneeTarget",
    "workflowId",
    "developmentContext",
    "dueDate",
    "recurrence",
  ]));
  const changes = {};
  if (body.title !== undefined) {
    changes.title = stringField(body.title, "title", { required: true, maxLength: 240 });
  }
  if (body.description !== undefined) {
    changes.description = stringField(body.description, "description", { maxLength: 100_000 });
  }
  if (body.status !== undefined) changes.status = parseStatus(body.status);
  if (body.priority !== undefined) changes.priority = parsePriority(body.priority);
  if (body.labels !== undefined) changes.labels = parseLabels(body.labels);
  if (body.workflowId !== undefined) changes.workflowId = parseWorkflowId(body.workflowId);
  if (body.developmentContext !== undefined) {
    changes.developmentContext = parseDevelopmentContext(body.developmentContext);
  }
  if (body.dueDate !== undefined) changes.dueDate = parseDueDate(body.dueDate);
  if (body.recurrence !== undefined) changes.recurrence = parseRecurrence(body.recurrence);
  const assigneeTarget = parseAssigneeTarget(body.assigneeTarget);
  if (Object.keys(changes).length === 0 && assigneeTarget === undefined) {
    throw new ApiError(400, "INVALID_BODY", "PATCH requires at least one task field");
  }
  return {
    version: parseVersion(body.version),
    changes,
    threadId: parseThreadId(body.threadId),
    assigneeTarget,
  };
}

function parseMove(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set(["version", "status", "sortOrder", "threadId"]));
  return {
    version: parseVersion(body.version),
    status: parseStatus(body.status),
    sortOrder: body.sortOrder === undefined ? undefined : parseSortOrder(body.sortOrder),
    threadId: parseThreadId(body.threadId),
  };
}

function parseVersionMutation(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set(["version", "threadId"]));
  return {
    version: parseVersion(body.version),
    threadId: parseThreadId(body.threadId),
  };
}

function parseCommentCreate(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set(["body", "threadId"]));
  return {
    body: stringField(body.body ?? "", "body", { maxLength: 100_000 }),
    threadId: parseThreadId(body.threadId),
  };
}

function parseCommentPatch(body) {
  assertPlainObject(body);
  assertAllowedKeys(body, new Set(["version", "body", "threadId"]));
  if (body.body === undefined) {
    throw new ApiError(400, "INVALID_FIELD", "'body' is required");
  }
  return {
    version: parseVersion(body.version),
    body: stringField(body.body, "body", { maxLength: 100_000 }),
    threadId: parseThreadId(body.threadId),
  };
}

function parseTaskFilters(searchParams) {
  const allowed = new Set(["projectId", "status", "archived"]);
  for (const key of searchParams.keys()) {
    if (!allowed.has(key)) {
      throw new ApiError(400, "UNKNOWN_QUERY_PARAMETER", `Unknown query parameter: ${key}`);
    }
    if (searchParams.getAll(key).length > 1) {
      throw new ApiError(400, "INVALID_QUERY_PARAMETER", `'${key}' cannot be repeated`);
    }
  }
  const projectId = searchParams.get("projectId");
  const status = searchParams.get("status");
  const archived = searchParams.get("archived") ?? "false";
  if (projectId !== null) validateProjectId(projectId);
  if (status !== null) parseStatus(status);
  if (!["false", "true", "all"].includes(archived)) {
    throw new ApiError(
      400,
      "INVALID_QUERY_PARAMETER",
      "'archived' must be false, true, or all",
    );
  }
  return { projectId, status, archived };
}

function sanitizeWorkflowNodeData(value) {
  if (Array.isArray(value)) return value.map(sanitizeWorkflowNodeData);
  if (value === null || typeof value !== "object") return value;

  return Object.fromEntries(
    Object.entries(value)
      .filter(([key]) => key !== "gitWorktreePath")
      .map(([key, child]) => [key, sanitizeWorkflowNodeData(child)]),
  );
}

function parseWorkflowWorkspace(value) {
  assertPlainObject(value);
  assertAllowedKeys(value, new Set(["version", "tabs", "activeWorkflowId", "snapshots"]));
  if (value.version !== 1) {
    throw new ApiError(400, "INVALID_FIELD", "'workspace.version' must be 1");
  }
  if (!Array.isArray(value.tabs) || value.tabs.length === 0 || value.tabs.length > 100) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'workspace.tabs' must contain 1 to 100 workflows",
    );
  }
  const tabs = value.tabs.map((tab, index) => {
    assertPlainObject(tab);
    assertAllowedKeys(tab, new Set(["id", "name"]));
    return {
      id: stringField(tab.id, `workspace.tabs[${index}].id`, {
        required: true,
        maxLength: 128,
      }),
      name: stringField(tab.name, `workspace.tabs[${index}].name`, {
        required: true,
        maxLength: 120,
      }),
    };
  });
  if (new Set(tabs.map((tab) => tab.id)).size !== tabs.length) {
    throw new ApiError(400, "INVALID_FIELD", "'workspace.tabs' ids must be unique");
  }
  const activeWorkflowId = stringField(
    value.activeWorkflowId,
    "workspace.activeWorkflowId",
    { required: true, maxLength: 128 },
  );
  if (!tabs.some((tab) => tab.id === activeWorkflowId)) {
    throw new ApiError(
      400,
      "INVALID_FIELD",
      "'workspace.activeWorkflowId' must reference a workflow tab",
    );
  }
  assertPlainObject(value.snapshots);
  const snapshots = {};
  for (const tab of tabs) {
    const snapshot = value.snapshots[tab.id];
    assertPlainObject(snapshot);
    assertAllowedKeys(snapshot, new Set(["nodes", "edges", "flow", "selectedNodeId"]));
    if (!Array.isArray(snapshot.nodes) || snapshot.nodes.length > 10_000) {
      throw new ApiError(
        400,
        "INVALID_FIELD",
        `'workspace.snapshots.${tab.id}.nodes' must be an array`,
      );
    }
    if (
      snapshot.flow === undefined
      && (!Array.isArray(snapshot.edges) || snapshot.edges.length > 20_000)
    ) {
      throw new ApiError(
        400,
        "INVALID_FIELD",
        `'workspace.snapshots.${tab.id}.edges' must be an array`,
      );
    }
    if (snapshot.flow !== undefined && snapshot.edges !== undefined) {
      throw new ApiError(
        400,
        "INVALID_FIELD",
        `'workspace.snapshots.${tab.id}' cannot contain both 'flow' and 'edges'`,
      );
    }
    const selectedNodeId = stringField(
      snapshot.selectedNodeId ?? null,
      `workspace.snapshots.${tab.id}.selectedNodeId`,
      { nullable: true, maxLength: 256 },
    );
    const nodes = snapshot.nodes.map((node) => {
      if (
        node === null
        || Array.isArray(node)
        || typeof node !== "object"
        || node.data === null
        || Array.isArray(node.data)
        || typeof node.data !== "object"
      ) {
        return node;
      }
      return { ...node, data: sanitizeWorkflowNodeData(node.data) };
    });
    try {
      snapshots[tab.id] = normalizeWorkflowSnapshot({
        nodes,
        edges: snapshot.edges,
        flow: snapshot.flow,
        selectedNodeId,
      });
    } catch (error) {
      throw new ApiError(
        400,
        "INVALID_FIELD",
        `'workspace.snapshots.${tab.id}' is not a valid workflow: ${error.message}`,
      );
    }
  }
  return { version: 1, tabs, activeWorkflowId, snapshots };
}

async function listProjects(env) {
  const rows = await all(env.DB.prepare(`
    SELECT
      projects.id,
      projects.name,
      projects.workspace_path,
      projects.archived_at,
      projects.created_at,
      projects.updated_at,
      COUNT(tasks.id) AS issue_count
    FROM projects
    LEFT JOIN tasks
      ON tasks.project_id = projects.id
      AND tasks.archived_at IS NULL
    WHERE projects.archived_at IS NULL
    GROUP BY
      projects.id,
      projects.name,
      projects.workspace_path,
      projects.archived_at,
      projects.created_at,
      projects.updated_at
    ORDER BY projects.created_at, projects.id
  `));
  return rows.map(projectFromRow);
}

async function getProject(env, id) {
  const row = await env.DB.prepare(`
    SELECT
      projects.id,
      projects.name,
      projects.workspace_path,
      projects.archived_at,
      projects.created_at,
      projects.updated_at,
      COUNT(tasks.id) AS issue_count
    FROM projects
    LEFT JOIN tasks
      ON tasks.project_id = projects.id
      AND tasks.archived_at IS NULL
    WHERE projects.id = ? AND projects.archived_at IS NULL
    GROUP BY
      projects.id,
      projects.name,
      projects.workspace_path,
      projects.archived_at,
      projects.created_at,
      projects.updated_at
  `).bind(id).first();
  return row ? projectFromRow(row) : null;
}

async function createProject(env, input) {
  const timestamp = now();
  try {
    await env.DB.prepare(`
      INSERT INTO projects (
        id, name, workspace_path, next_task_number, created_at, updated_at
      ) VALUES (?, ?, NULL, 1, ?, ?)
    `).bind(input.id, input.name, timestamp, timestamp).run();
  } catch (error) {
    if (String(error.message).includes("UNIQUE constraint failed")) {
      throw new ApiError(409, "PROJECT_EXISTS", `Project '${input.id}' already exists`);
    }
    throw error;
  }
  return getProject(env, input.id);
}

async function archiveProject(env, id) {
  if (id === DEFAULT_PROJECT_ID) {
    throw new ApiError(409, "DEFAULT_PROJECT_PROTECTED", "The default project cannot be archived or deleted");
  }
  const project = await getProject(env, id);
  if (!project) throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${id}' does not exist`);

  const timestamp = now();
  const result = await env.DB.prepare(`
    UPDATE projects
    SET archived_at = ?, updated_at = ?
    WHERE id = ? AND archived_at IS NULL
  `).bind(timestamp, timestamp, id).run();
  if (!changed(result)) {
    throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${id}' does not exist`);
  }
  return { ...project, archivedAt: timestamp, updatedAt: timestamp };
}

async function deleteProject(env, id) {
  if (id === DEFAULT_PROJECT_ID) {
    throw new ApiError(409, "DEFAULT_PROJECT_PROTECTED", "The default project cannot be archived or deleted");
  }
  const project = await getProject(env, id);
  if (!project) throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${id}' does not exist`);

  const attachments = await all(env.DB.prepare(`
    SELECT attachments.*
    FROM attachments
    JOIN tasks ON tasks.id = attachments.task_id
    WHERE tasks.project_id = ?
  `).bind(id));
  const results = await env.DB.batch([
    env.DB.prepare("DELETE FROM tasks WHERE project_id = ?").bind(id),
    env.DB.prepare("DELETE FROM projects WHERE id = ?").bind(id),
  ]);
  if (!changed(results[1])) {
    throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${id}' does not exist`);
  }
  await Promise.all(attachments.map((attachment) => env.ATTACHMENTS.delete(attachment.id)));
  return project;
}

async function listTasks(env, filters) {
  const where = [];
  const values = [];
  if (filters.projectId) {
    where.push("project_id = ?");
    values.push(filters.projectId);
  }
  if (filters.status) {
    where.push("status = ?");
    values.push(filters.status);
  }
  if (filters.archived === "false") {
    where.push("archived_at IS NULL");
  } else if (filters.archived === "true") {
    where.push("archived_at IS NOT NULL");
  }
  const rows = await all(
    env.DB.prepare(`
      SELECT * FROM tasks
      ${where.length > 0 ? `WHERE ${where.join(" AND ")}` : ""}
      ORDER BY
        CASE status
          WHEN 'backlog' THEN 1
          WHEN 'todo' THEN 2
          WHEN 'in_progress' THEN 3
          WHEN 'in_review' THEN 4
          WHEN 'blocked' THEN 5
          WHEN 'done' THEN 6
          WHEN 'canceled' THEN 7
        END,
        sort_order,
        created_at,
        id
    `).bind(...values),
  );
  return Promise.all(rows.map((row) => hydrateTask(env, row)));
}

async function createTask(env, input, actor) {
  await requireProject(env, input.projectId);
  let sortOrder = input.sortOrder;
  if (sortOrder === undefined) {
    const row = await env.DB.prepare(`
      SELECT COALESCE(MAX(sort_order), 0) AS maximum
      FROM tasks
      WHERE project_id = ? AND status = ? AND archived_at IS NULL
    `).bind(input.projectId, input.status).first();
    sortOrder = row.maximum + 1000;
  }
  const id = uuid();
  const timestamp = now();
  const assignee = resolveAssignee(input.assigneeTarget, actor);
  const results = await env.DB.batch([
    env.DB.prepare(`
      INSERT INTO tasks (
        id, identifier, project_id, title, description, status, priority, labels,
        sort_order, thread_id, creator_type, creator_id, creator_name, creator_avatar_url,
        assignee_type, assignee_id, assignee_name, assignee_avatar_url,
        workflow_id, development_context_type, development_branch,
        due_date, recurrence_interval, recurrence_unit,
        archived_at, version, created_at, updated_at
      )
      SELECT
        ?,
        ? || '-' || CAST(next_task_number AS TEXT),
        projects.id,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        ?, ?, ?, ?, ?, ?, ?, ?, ?, ?,
        NULL, 1, ?, ?
      FROM projects
      WHERE projects.id = ? AND projects.archived_at IS NULL
    `).bind(
      id,
      projectPrefix(input.projectId),
      input.title,
      input.description,
      input.status,
      input.priority,
      JSON.stringify(input.labels),
      sortOrder,
      input.threadId ?? null,
      actor.type,
      actor.id,
      actor.name,
      actor.avatarUrl,
      assignee.type,
      assignee.id,
      assignee.name,
      assignee.avatarUrl,
      input.workflowId,
      input.developmentContext?.type ?? null,
      input.developmentContext?.branch ?? null,
      input.dueDate,
      input.recurrence?.interval ?? null,
      input.recurrence?.unit ?? null,
      timestamp,
      timestamp,
      input.projectId,
    ),
    env.DB.prepare(`
      UPDATE projects
      SET next_task_number = next_task_number + 1, updated_at = ?
      WHERE id = ? AND archived_at IS NULL
    `).bind(timestamp, input.projectId),
  ]);
  if (!changed(results[0]) || !changed(results[1])) {
    throw new ApiError(
      404,
      "PROJECT_NOT_FOUND",
      `Project '${input.projectId}' does not exist`,
    );
  }
  return getTask(env, id);
}

async function updateTask(env, id, input, actor) {
  const current = await requireTaskRow(env, id);
  assertTaskVersion(current, input.version);
  const currentTask = taskFromRow(current);
  const dueDate = Object.hasOwn(input.changes, "dueDate")
    ? input.changes.dueDate
    : currentTask.dueDate;
  const recurrence = Object.hasOwn(input.changes, "recurrence")
    ? input.changes.recurrence
    : currentTask.recurrence;
  if (recurrence && !dueDate) {
    throw new ApiError(400, "INVALID_FIELD", "A recurring issue requires a due date");
  }

  const assignments = [];
  const values = [];
  const columns = {
    title: "title",
    description: "description",
    status: "status",
    priority: "priority",
    labels: "labels",
    workflowId: "workflow_id",
    dueDate: "due_date",
  };
  for (const [key, value] of Object.entries(input.changes)) {
    if (key === "developmentContext") {
      assignments.push("development_context_type = ?", "development_branch = ?");
      values.push(value?.type ?? null, value?.branch ?? null);
    } else if (key === "recurrence") {
      assignments.push("recurrence_interval = ?", "recurrence_unit = ?");
      values.push(value?.interval ?? null, value?.unit ?? null);
    } else {
      assignments.push(`${columns[key]} = ?`);
      values.push(key === "labels" ? JSON.stringify(value) : value);
    }
  }
  if (input.assigneeTarget !== undefined) {
    const assignee = resolveAssignee(input.assigneeTarget, actor);
    assignments.push(
      "assignee_type = ?",
      "assignee_id = ?",
      "assignee_name = ?",
      "assignee_avatar_url = ?",
    );
    values.push(assignee.type, assignee.id, assignee.name, assignee.avatarUrl);
  }
  if (input.threadId !== undefined) {
    assignments.push("thread_id = ?");
    values.push(input.threadId);
  }
  assignments.push("version = version + 1", "updated_at = ?");
  values.push(now(), current.id, input.version);
  const result = await env.DB.prepare(`
    UPDATE tasks
    SET ${assignments.join(", ")}
    WHERE id = ? AND version = ?
  `).bind(...values).run();
  if (!changed(result)) {
    const latest = await requireTaskRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return getTask(env, current.id);
}

async function moveTask(env, id, input) {
  const current = await requireTaskRow(env, id);
  assertTaskVersion(current, input.version);
  if (current.archived_at !== null) {
    throw new ApiError(409, "TASK_ARCHIVED", "Archived tasks cannot be moved");
  }
  let sortOrder = input.sortOrder;
  if (sortOrder === undefined) {
    const row = await env.DB.prepare(`
      SELECT COALESCE(MAX(sort_order), 0) AS maximum
      FROM tasks
      WHERE project_id = ? AND status = ? AND archived_at IS NULL AND id != ?
    `).bind(current.project_id, input.status, current.id).first();
    sortOrder = row.maximum + 1000;
  }
  const result = await env.DB.prepare(`
    UPDATE tasks
    SET
      status = ?,
      sort_order = ?,
      thread_id = COALESCE(?, thread_id),
      version = version + 1,
      updated_at = ?
    WHERE id = ? AND version = ?
  `).bind(
    input.status,
    sortOrder,
    input.threadId ?? null,
    now(),
    current.id,
    input.version,
  ).run();
  if (!changed(result)) {
    const latest = await requireTaskRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return getTask(env, current.id);
}

async function archiveTask(env, id, input) {
  const current = await requireTaskRow(env, id);
  assertTaskVersion(current, input.version);
  const timestamp = now();
  const result = await env.DB.prepare(`
    UPDATE tasks
    SET
      archived_at = ?,
      thread_id = COALESCE(?, thread_id),
      version = version + 1,
      updated_at = ?
    WHERE id = ? AND version = ?
  `).bind(timestamp, input.threadId ?? null, timestamp, current.id, input.version).run();
  if (!changed(result)) {
    const latest = await requireTaskRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return getTask(env, current.id);
}

async function restoreTask(env, id, input) {
  const current = await requireTaskRow(env, id);
  assertTaskVersion(current, input.version);
  if (current.archived_at === null) {
    throw new ApiError(409, "TASK_NOT_ARCHIVED", "Only archived tasks can be restored");
  }
  const result = await env.DB.prepare(`
    UPDATE tasks
    SET
      archived_at = NULL,
      thread_id = COALESCE(?, thread_id),
      version = version + 1,
      updated_at = ?
    WHERE id = ? AND version = ?
  `).bind(input.threadId ?? null, now(), current.id, input.version).run();
  if (!changed(result)) {
    const latest = await requireTaskRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return getTask(env, current.id);
}

function relationEndpoints(type, taskId, relatedTaskId) {
  if (type === "parent") {
    return {
      relationType: "parent",
      sourceTaskId: relatedTaskId,
      targetTaskId: taskId,
    };
  }
  if (type === "blocks") {
    return {
      relationType: "blocks",
      sourceTaskId: taskId,
      targetTaskId: relatedTaskId,
    };
  }
  if (type === "blocked_by") {
    return {
      relationType: "blocks",
      sourceTaskId: relatedTaskId,
      targetTaskId: taskId,
    };
  }
  if (type === "related") {
    const [sourceTaskId, targetTaskId] = [taskId, relatedTaskId].sort();
    return { relationType: "related", sourceTaskId, targetTaskId };
  }
  throw new ApiError(
    400,
    "INVALID_FIELD",
    "'relation type' must be parent, blocks, blocked_by, or related",
  );
}

async function assertRelationTasks(env, taskId, relatedTaskId, expectedVersion) {
  const task = await requireTaskRow(env, taskId);
  const relatedTask = await requireTaskRow(env, relatedTaskId);
  assertTaskVersion(task, expectedVersion);
  if (task.id === relatedTask.id) {
    throw new ApiError(400, "SELF_RELATION", "An issue cannot be related to itself");
  }
  if (task.project_id !== relatedTask.project_id) {
    throw new ApiError(
      400,
      "CROSS_PROJECT_RELATION",
      "Issue relations must stay within one project",
    );
  }
  return { task, relatedTask };
}

async function addRelation(env, taskId, type, relatedTaskId, input) {
  const { task, relatedTask } = await assertRelationTasks(
    env,
    taskId,
    relatedTaskId,
    input.version,
  );
  const endpoints = relationEndpoints(type, task.id, relatedTask.id);
  const statements = [];
  if (endpoints.relationType === "parent") {
    const cycle = await env.DB.prepare(`
      WITH RECURSIVE ancestors(id) AS (
        SELECT source_task_id
        FROM task_relations
        WHERE relation_type = 'parent' AND target_task_id = ?
        UNION
        SELECT task_relations.source_task_id
        FROM task_relations
        JOIN ancestors ON task_relations.target_task_id = ancestors.id
        WHERE task_relations.relation_type = 'parent'
      )
      SELECT 1 AS found FROM ancestors WHERE id = ?
    `).bind(relatedTask.id, task.id).first();
    if (cycle) {
      throw new ApiError(409, "RELATION_CYCLE", "This parent would create a cycle");
    }
    const existing = await env.DB.prepare(`
      SELECT source_task_id
      FROM task_relations
      WHERE relation_type = 'parent' AND target_task_id = ?
    `).bind(task.id).first();
    if (existing?.source_task_id === relatedTask.id) {
      throw new ApiError(409, "RELATION_EXISTS", "This parent relation already exists");
    }
    if (existing) {
      statements.push(
        env.DB.prepare(`
          DELETE FROM task_relations
          WHERE relation_type = 'parent'
            AND target_task_id = ?
            AND EXISTS (
              SELECT 1 FROM tasks WHERE id = ? AND version = ?
            )
        `).bind(task.id, task.id, input.version),
      );
    }
  } else {
    const existing = await env.DB.prepare(`
      SELECT 1 AS found
      FROM task_relations
      WHERE relation_type = ? AND source_task_id = ? AND target_task_id = ?
    `).bind(
      endpoints.relationType,
      endpoints.sourceTaskId,
      endpoints.targetTaskId,
    ).first();
    if (existing) {
      throw new ApiError(409, "RELATION_EXISTS", "This issue relation already exists");
    }
  }
  statements.push(
    env.DB.prepare(`
      INSERT INTO task_relations (
        relation_type, source_task_id, target_task_id, created_at
      )
      SELECT ?, ?, ?, ?
      WHERE EXISTS (
        SELECT 1 FROM tasks WHERE id = ? AND version = ?
      )
    `).bind(
      endpoints.relationType,
      endpoints.sourceTaskId,
      endpoints.targetTaskId,
      now(),
      task.id,
      input.version,
    ),
    env.DB.prepare(`
      UPDATE tasks
      SET
        thread_id = COALESCE(?, thread_id),
        version = version + 1,
        updated_at = ?
      WHERE id = ? AND version = ?
    `).bind(input.threadId ?? null, now(), task.id, input.version),
  );
  let results;
  try {
    results = await env.DB.batch(statements);
  } catch (error) {
    const message = String(error.message);
    if (message.includes("RELATION_CYCLE")) {
      throw new ApiError(409, "RELATION_CYCLE", "This parent would create a cycle");
    }
    if (
      message.includes("UNIQUE constraint failed")
      && message.includes("task_relations")
    ) {
      throw new ApiError(409, "RELATION_EXISTS", "This issue relation already exists");
    }
    throw error;
  }
  if (!changed(results.at(-1))) {
    const latest = await requireTaskRow(env, task.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return {
    task: await getTask(env, task.id),
    relatedTask: await getTask(env, relatedTask.id),
  };
}

async function removeRelation(env, taskId, type, relatedTaskId, input) {
  const { task, relatedTask } = await assertRelationTasks(
    env,
    taskId,
    relatedTaskId,
    input.version,
  );
  const endpoints = relationEndpoints(type, task.id, relatedTask.id);
  const exists = await env.DB.prepare(`
    SELECT 1 AS found
    FROM task_relations
    WHERE relation_type = ? AND source_task_id = ? AND target_task_id = ?
  `).bind(
    endpoints.relationType,
    endpoints.sourceTaskId,
    endpoints.targetTaskId,
  ).first();
  if (!exists) {
    throw new ApiError(404, "RELATION_NOT_FOUND", "This issue relation does not exist");
  }
  const results = await env.DB.batch([
    env.DB.prepare(`
      DELETE FROM task_relations
      WHERE relation_type = ?
        AND source_task_id = ?
        AND target_task_id = ?
        AND EXISTS (
          SELECT 1 FROM tasks WHERE id = ? AND version = ?
        )
    `).bind(
      endpoints.relationType,
      endpoints.sourceTaskId,
      endpoints.targetTaskId,
      task.id,
      input.version,
    ),
    env.DB.prepare(`
      UPDATE tasks
      SET
        thread_id = COALESCE(?, thread_id),
        version = version + 1,
        updated_at = ?
      WHERE id = ? AND version = ?
    `).bind(input.threadId ?? null, now(), task.id, input.version),
  ]);
  if (!changed(results.at(-1))) {
    const latest = await requireTaskRow(env, task.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Task was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  return {
    task: await getTask(env, task.id),
    relatedTask: await getTask(env, relatedTask.id),
  };
}

async function getWorkflow(env, projectId) {
  await requireProject(env, projectId);
  const row = await env.DB.prepare(`
    SELECT project_id, workspace, version, updated_at
    FROM workflow_workspaces
    WHERE project_id = ?
  `).bind(projectId).first();
  return row
    ? {
        projectId: row.project_id,
        workspace: JSON.parse(row.workspace),
        version: row.version,
        updatedAt: row.updated_at,
      }
    : { projectId, workspace: null, version: 0, updatedAt: null };
}

async function saveWorkflow(env, projectId, expectedVersion, workspace) {
  await requireProject(env, projectId);
  const current = await env.DB.prepare(`
    SELECT version FROM workflow_workspaces WHERE project_id = ?
  `).bind(projectId).first();
  const actualVersion = current?.version ?? 0;
  if (actualVersion !== expectedVersion) {
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Workflow was changed by another client",
      { expectedVersion, actualVersion },
    );
  }
  const timestamp = now();
  if (current) {
    const result = await env.DB.prepare(`
      UPDATE workflow_workspaces
      SET workspace = ?, version = version + 1, updated_at = ?
      WHERE project_id = ? AND version = ?
    `).bind(
      JSON.stringify(workspace),
      timestamp,
      projectId,
      expectedVersion,
    ).run();
    if (!changed(result)) {
      const latest = await env.DB.prepare(`
        SELECT version FROM workflow_workspaces WHERE project_id = ?
      `).bind(projectId).first();
      throw new ApiError(
        409,
        "VERSION_CONFLICT",
        "Workflow was changed by another client",
        { expectedVersion, actualVersion: latest?.version ?? 0 },
      );
    }
  } else {
    try {
      await env.DB.prepare(`
        INSERT INTO workflow_workspaces (project_id, workspace, version, updated_at)
        VALUES (?, ?, 1, ?)
      `).bind(projectId, JSON.stringify(workspace), timestamp).run();
    } catch (error) {
      if (String(error.message).includes("UNIQUE constraint failed")) {
        const latest = await env.DB.prepare(`
          SELECT version FROM workflow_workspaces WHERE project_id = ?
        `).bind(projectId).first();
        throw new ApiError(
          409,
          "VERSION_CONFLICT",
          "Workflow was changed by another client",
          { expectedVersion, actualVersion: latest.version },
        );
      }
      throw error;
    }
  }
  return getWorkflow(env, projectId);
}

async function listComments(env, taskId) {
  const task = await requireTaskRow(env, taskId);
  const rows = await all(env.DB.prepare(`
    SELECT * FROM comments
    WHERE task_id = ?
    ORDER BY created_at, id
  `).bind(task.id));
  return Promise.all(rows.map((row) => hydrateComment(env, row)));
}

async function createComment(env, taskId, input, actor) {
  const task = await requireTaskRow(env, taskId);
  const id = uuid();
  const timestamp = now();
  await env.DB.prepare(`
    INSERT INTO comments (
      id, task_id, body, thread_id, author_type, author_id, author_name,
      author_avatar_url, version, created_at, updated_at
    ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)
  `).bind(
    id,
    task.id,
    input.body,
    input.threadId ?? null,
    actor.type,
    actor.id,
    actor.name,
    actor.avatarUrl,
    timestamp,
    timestamp,
  ).run();
  const row = await env.DB.prepare("SELECT * FROM comments WHERE id = ?").bind(id).first();
  return hydrateComment(env, row);
}

async function requireCommentRow(env, id) {
  const row = await env.DB.prepare("SELECT * FROM comments WHERE id = ?").bind(id).first();
  if (!row) {
    throw new ApiError(404, "COMMENT_NOT_FOUND", `Comment '${id}' does not exist`);
  }
  return row;
}

function assertCommentVersion(row, expectedVersion) {
  if (row.version !== expectedVersion) {
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Comment was changed by another client",
      { expectedVersion, actualVersion: row.version },
    );
  }
}

async function updateComment(env, id, input) {
  const current = await requireCommentRow(env, id);
  assertCommentVersion(current, input.version);
  const result = await env.DB.prepare(`
    UPDATE comments
    SET
      body = ?,
      thread_id = COALESCE(?, thread_id),
      version = version + 1,
      updated_at = ?
    WHERE id = ? AND version = ?
  `).bind(
    input.body,
    input.threadId ?? null,
    now(),
    current.id,
    input.version,
  ).run();
  if (!changed(result)) {
    const latest = await requireCommentRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Comment was changed by another client",
      { expectedVersion: input.version, actualVersion: latest.version },
    );
  }
  const row = await requireCommentRow(env, current.id);
  return hydrateComment(env, row);
}

async function deleteComment(env, id, expectedVersion) {
  const current = await requireCommentRow(env, id);
  assertCommentVersion(current, expectedVersion);
  const attachments = await attachmentsForComment(env, current.id);
  const result = await env.DB.prepare(`
    DELETE FROM comments WHERE id = ? AND version = ?
  `).bind(current.id, expectedVersion).run();
  if (!changed(result)) {
    const latest = await requireCommentRow(env, current.id);
    throw new ApiError(
      409,
      "VERSION_CONFLICT",
      "Comment was changed by another client",
      { expectedVersion, actualVersion: latest.version },
    );
  }
  await Promise.all(attachments.map((attachment) => env.ATTACHMENTS.delete(attachment.id)));
}

async function listTaskAttachments(env, taskId) {
  const task = await requireTaskRow(env, taskId);
  return (
    await all(env.DB.prepare(`
      SELECT * FROM attachments
      WHERE task_id = ? AND comment_id IS NULL
      ORDER BY created_at, id
    `).bind(task.id))
  ).map(attachmentFromRow);
}

async function listCommentAttachments(env, commentId) {
  await requireCommentRow(env, commentId);
  return attachmentsForComment(env, commentId);
}

async function uploadAttachment(env, ownerType, ownerId, request) {
  let taskId;
  let commentId = null;
  if (ownerType === "task") {
    taskId = (await requireTaskRow(env, ownerId)).id;
  } else {
    const comment = await requireCommentRow(env, ownerId);
    taskId = comment.task_id;
    commentId = comment.id;
  }
  const metadata = parseAttachmentHeaders(request);
  const body = await readAttachment(request);
  const id = uuid();
  await env.ATTACHMENTS.put(id, body, {
    httpMetadata: { contentType: metadata.contentType },
  });
  try {
    await env.DB.prepare(`
      INSERT INTO attachments (
        id, task_id, comment_id, filename, content_type, size, created_at
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
    `).bind(
      id,
      taskId,
      commentId,
      metadata.filename,
      metadata.contentType,
      body.byteLength,
      now(),
    ).run();
  } catch (error) {
    await env.ATTACHMENTS.delete(id);
    throw error;
  }
  const row = await env.DB.prepare("SELECT * FROM attachments WHERE id = ?").bind(id).first();
  return attachmentFromRow(row);
}

async function requireAttachment(env, id) {
  const row = await env.DB.prepare("SELECT * FROM attachments WHERE id = ?").bind(id).first();
  if (!row) {
    throw new ApiError(404, "ATTACHMENT_NOT_FOUND", `Attachment '${id}' does not exist`);
  }
  return attachmentFromRow(row);
}

async function deleteAttachment(env, id) {
  const attachment = await requireAttachment(env, id);
  await env.DB.prepare("DELETE FROM attachments WHERE id = ?").bind(attachment.id).run();
  await env.ATTACHMENTS.delete(attachment.id);
  return attachment;
}

function requireNoQuery(url, routeName) {
  if ([...url.searchParams.keys()].length > 0) {
    throw new ApiError(
      400,
      "UNKNOWN_QUERY_PARAMETER",
      `${routeName} does not accept query parameters`,
    );
  }
}

function decodePathPart(value, label) {
  let decoded;
  try {
    decoded = decodeURIComponent(value);
  } catch {
    throw new ApiError(400, "INVALID_PATH", `${label} contains invalid encoding`);
  }
  if (decoded.length === 0 || decoded.length > 128) {
    throw new ApiError(400, "INVALID_PATH", `${label} is invalid`);
  }
  return decoded;
}

async function attachmentContent(env, id, request) {
  const attachment = await requireAttachment(env, id);
  const object = await env.ATTACHMENTS.get(attachment.id);
  if (!object) {
    throw new ApiError(
      404,
      "ATTACHMENT_NOT_FOUND",
      `Attachment '${id}' does not exist`,
    );
  }
  const encodedFilename = encodeURIComponent(attachment.filename).replace(
    /['()*]/g,
    (character) => `%${character.charCodeAt(0).toString(16).toUpperCase()}`,
  );
  const canOpenInline = INLINE_ATTACHMENT_TYPES.has(attachment.contentType);
  return new Response(request.method === "HEAD" ? null : object.body, {
    status: 200,
    headers: {
      "cache-control": "private, no-store",
      "content-disposition": `${
        canOpenInline ? "inline" : "attachment"
      }; filename*=UTF-8''${encodedFilename}`,
      "content-length": String(attachment.size),
      "content-security-policy": "sandbox; default-src 'none'",
      "content-type": canOpenInline
        ? attachment.contentType
        : "application/octet-stream",
    },
  });
}

async function routeApi(request, env, actor, url) {
  const { pathname } = url;

  if (pathname === "/api/meta") {
    if (request.method !== "GET") methodNotAllowed(["GET"]);
    requireNoQuery(url, "GET /api/meta");
    return json(200, {
      mode: "cloud",
      manageTaskboardSkillPath: null,
      realtime: { transport: "poll", intervalMs: 2000 },
      localCapabilities: { available: false },
    });
  }

  if (pathname === "/api/revisions") {
    if (request.method !== "GET") methodNotAllowed(["GET"]);
    const unknown = [...url.searchParams.keys()].filter((key) => key !== "since");
    if (unknown.length > 0) {
      throw new ApiError(
        400,
        "UNKNOWN_QUERY_PARAMETER",
        `Unknown query parameter: ${unknown[0]}`,
      );
    }
    if (url.searchParams.getAll("since").length !== 1) {
      throw new ApiError(
        400,
        "INVALID_QUERY_PARAMETER",
        "'since' must be provided once",
      );
    }
    const rawSince = url.searchParams.get("since");
    if (!/^\d+$/.test(rawSince ?? "")) {
      throw new ApiError(
        400,
        "INVALID_QUERY_PARAMETER",
        "'since' must be a non-negative integer",
      );
    }
    const since = Number(rawSince);
    if (!Number.isSafeInteger(since)) {
      throw new ApiError(
        400,
        "INVALID_QUERY_PARAMETER",
        "'since' must be a non-negative integer",
      );
    }
    const revision = await env.DB.prepare(`
      SELECT revision FROM global_revision WHERE singleton = 1
    `).first("revision");
    return json(200, { changed: revision > since, revision });
  }

  if (
    pathname === "/api/device-workspaces"
    || pathname === "/api/workflow-capabilities"
    || /^\/api\/projects\/[^/]+\/development-contexts$/.test(pathname)
  ) {
    if (request.method !== "GET") methodNotAllowed(["GET"]);
    throw new ApiError(
      409,
      "LOCAL_COMPANION_REQUIRED",
      "This capability requires the local Codex companion",
    );
  }

  if (pathname === "/api/events") {
    if (request.method !== "GET") methodNotAllowed(["GET"]);
    throw new ApiError(
      409,
      "POLLING_REQUIRED",
      "Cloud collaboration uses revision polling",
    );
  }

  if (pathname === "/api/projects") {
    if (request.method === "GET") {
      requireNoQuery(url, "GET /api/projects");
      return json(200, { projects: await listProjects(env) });
    }
    if (request.method === "POST") {
      return json(201, {
        project: await createProject(env, parseProjectCreate(await readJson(request))),
      });
    }
    methodNotAllowed(["GET", "POST"]);
  }

  const projectArchiveMatch = pathname.match(/^\/api\/projects\/([^/]+)\/archive$/);
  if (projectArchiveMatch) {
    requireNoQuery(url, "Project archive route");
    const projectId = validateProjectId(
      decodePathPart(projectArchiveMatch[1], "Project id"),
    );
    if (request.method === "POST") {
      return json(200, { project: await archiveProject(env, projectId) });
    }
    methodNotAllowed(["POST"]);
  }

  const projectDeleteMatch = pathname.match(/^\/api\/projects\/([^/]+)$/);
  if (projectDeleteMatch) {
    requireNoQuery(url, "Project delete route");
    const projectId = validateProjectId(
      decodePathPart(projectDeleteMatch[1], "Project id"),
    );
    if (request.method === "DELETE") {
      await deleteProject(env, projectId);
      return empty(204);
    }
    methodNotAllowed(["DELETE"]);
  }

  const workflowMatch = pathname.match(
    /^\/api\/projects\/([^/]+)\/workflow-workspace$/,
  );
  if (workflowMatch) {
    requireNoQuery(url, "Workflow workspace routes");
    const projectId = validateProjectId(
      decodePathPart(workflowMatch[1], "Project id"),
    );
    if (request.method === "GET") {
      return json(200, { workflow: await getWorkflow(env, projectId) });
    }
    if (request.method === "PUT") {
      const body = await readJson(request);
      assertPlainObject(body);
      assertAllowedKeys(body, new Set(["version", "workspace"]));
      const version = parseVersion(body.version, { allowZero: true });
      const workspace = parseWorkflowWorkspace(body.workspace);
      return json(200, {
        workflow: await saveWorkflow(env, projectId, version, workspace),
      });
    }
    methodNotAllowed(["GET", "PUT"]);
  }

  if (pathname === "/api/tasks") {
    if (request.method === "GET") {
      return json(200, {
        tasks: await listTasks(env, parseTaskFilters(url.searchParams)),
      });
    }
    if (request.method === "POST") {
      return json(201, {
        task: await createTask(env, parseTaskCreate(await readJson(request)), actor),
      });
    }
    methodNotAllowed(["GET", "POST"]);
  }

  const relationMatch = pathname.match(
    /^\/api\/tasks\/([^/]+)\/relations\/([^/]+)\/([^/]+)$/,
  );
  if (relationMatch) {
    requireNoQuery(url, "Issue relation routes");
    const taskId = decodePathPart(relationMatch[1], "Task id");
    const type = decodePathPart(relationMatch[2], "Relation type");
    const relatedTaskId = decodePathPart(relationMatch[3], "Related task id");
    const input = parseVersionMutation(await readJson(request));
    if (request.method === "POST") {
      return json(200, await addRelation(env, taskId, type, relatedTaskId, input));
    }
    if (request.method === "DELETE") {
      return json(200, await removeRelation(env, taskId, type, relatedTaskId, input));
    }
    methodNotAllowed(["POST", "DELETE"]);
  }

  const taskCommentsMatch = pathname.match(/^\/api\/tasks\/([^/]+)\/comments$/);
  if (taskCommentsMatch) {
    requireNoQuery(url, "Comment routes");
    const taskId = decodePathPart(taskCommentsMatch[1], "Task id");
    if (request.method === "GET") {
      return json(200, { comments: await listComments(env, taskId) });
    }
    if (request.method === "POST") {
      return json(201, {
        comment: await createComment(
          env,
          taskId,
          parseCommentCreate(await readJson(request)),
          actor,
        ),
      });
    }
    methodNotAllowed(["GET", "POST"]);
  }

  const commentAttachmentsMatch = pathname.match(
    /^\/api\/comments\/([^/]+)\/attachments$/,
  );
  if (commentAttachmentsMatch) {
    requireNoQuery(url, "Attachment routes");
    const commentId = decodePathPart(commentAttachmentsMatch[1], "Comment id");
    if (request.method === "GET") {
      return json(200, {
        attachments: await listCommentAttachments(env, commentId),
      });
    }
    if (request.method === "POST") {
      return json(201, {
        attachment: await uploadAttachment(env, "comment", commentId, request),
      });
    }
    methodNotAllowed(["GET", "POST"]);
  }

  const commentMatch = pathname.match(/^\/api\/comments\/([^/]+)$/);
  if (commentMatch) {
    requireNoQuery(url, "Comment routes");
    const commentId = decodePathPart(commentMatch[1], "Comment id");
    if (request.method === "PATCH") {
      return json(200, {
        comment: await updateComment(
          env,
          commentId,
          parseCommentPatch(await readJson(request)),
        ),
      });
    }
    if (request.method === "DELETE") {
      const { version } = parseVersionMutation(await readJson(request));
      await deleteComment(env, commentId, version);
      return empty(204);
    }
    methodNotAllowed(["PATCH", "DELETE"]);
  }

  const taskAttachmentsMatch = pathname.match(
    /^\/api\/tasks\/([^/]+)\/attachments$/,
  );
  if (taskAttachmentsMatch) {
    requireNoQuery(url, "Attachment routes");
    const taskId = decodePathPart(taskAttachmentsMatch[1], "Task id");
    if (request.method === "GET") {
      return json(200, {
        attachments: await listTaskAttachments(env, taskId),
      });
    }
    if (request.method === "POST") {
      return json(201, {
        attachment: await uploadAttachment(env, "task", taskId, request),
      });
    }
    methodNotAllowed(["GET", "POST"]);
  }

  const attachmentContentMatch = pathname.match(
    /^\/api\/attachments\/([^/]+)\/content$/,
  );
  if (attachmentContentMatch) {
    requireNoQuery(url, "Attachment routes");
    if (!["GET", "HEAD"].includes(request.method)) methodNotAllowed(["GET", "HEAD"]);
    return attachmentContent(
      env,
      decodePathPart(attachmentContentMatch[1], "Attachment id"),
      request,
    );
  }

  const attachmentMatch = pathname.match(/^\/api\/attachments\/([^/]+)$/);
  if (attachmentMatch) {
    requireNoQuery(url, "Attachment routes");
    if (request.method !== "DELETE") methodNotAllowed(["DELETE"]);
    await deleteAttachment(
      env,
      decodePathPart(attachmentMatch[1], "Attachment id"),
    );
    return empty(204);
  }

  const taskMatch = pathname.match(
    /^\/api\/tasks\/([^/]+)(?:\/(archive|restore|move))?$/,
  );
  if (taskMatch) {
    const taskId = decodePathPart(taskMatch[1], "Task id");
    const action = taskMatch[2];
    requireNoQuery(url, "Task routes");
    if (!action && request.method === "GET") {
      const task = await getTask(env, taskId);
      if (!task) {
        throw new ApiError(404, "TASK_NOT_FOUND", `Task '${taskId}' does not exist`);
      }
      return json(200, { task });
    }
    if (!action && request.method === "PATCH") {
      return json(200, {
        task: await updateTask(
          env,
          taskId,
          parseTaskPatch(await readJson(request)),
          actor,
        ),
      });
    }
    if (action === "move" && request.method === "POST") {
      return json(200, {
        task: await moveTask(env, taskId, parseMove(await readJson(request))),
      });
    }
    if (action === "archive" && request.method === "POST") {
      return json(200, {
        task: await archiveTask(
          env,
          taskId,
          parseVersionMutation(await readJson(request)),
        ),
      });
    }
    if (action === "restore" && request.method === "POST") {
      return json(200, {
        task: await restoreTask(
          env,
          taskId,
          parseVersionMutation(await readJson(request)),
        ),
      });
    }
    methodNotAllowed(action ? ["POST"] : ["GET", "PATCH"]);
  }

  throw new ApiError(404, "NOT_FOUND", "API route not found");
}

function withSecurityHeaders(response) {
  const secured = new Response(response.body, response);
  secured.headers.set("x-content-type-options", "nosniff");
  secured.headers.set("referrer-policy", "no-referrer");
  return secured;
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);
    try {
      if (url.pathname === "/health") {
        if (request.method !== "GET") methodNotAllowed(["GET"]);
        return withSecurityHeaders(json(200, { status: "ok" }));
      }

      const actor = await authenticate(request, env);
      if (!actor) return withSecurityHeaders(unauthorized());

      const response = url.pathname.startsWith("/api/")
        ? await routeApi(request, env, actor, url)
        : env.ASSETS
          ? await env.ASSETS.fetch(request)
          : json(404, { error: { code: "NOT_FOUND", message: "Resource not found" } });
      return withSecurityHeaders(response);
    } catch (error) {
      if (error instanceof ApiError) {
        const payload = {
          error: { code: error.code, message: error.message },
        };
        if (error.details !== undefined) payload.error.details = error.details;
        const headers = error.status === 405 && error.details?.allowed
          ? { allow: error.details.allowed.join(", ") }
          : {};
        return withSecurityHeaders(json(error.status, payload, headers));
      }
      console.error(error);
      return withSecurityHeaders(json(500, {
        error: { code: "INTERNAL_ERROR", message: "Internal server error" },
      }));
    }
  },
};
