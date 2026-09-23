#!/usr/bin/env node

import { realpathSync } from "node:fs";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { normalizeCloudUrl } from "../server/cloud-config.mjs";
import {
  DEFAULT_PROJECT_ID,
  TASK_STATUSES,
  isTaskPriority,
  isTaskStatus,
} from "../shared/domain.mjs";

export const SCHEMA_VERSION = 2;
export const DEFAULT_API_URL = "http://127.0.0.1:47823";

const BOOLEAN_OPTIONS = new Set(["json"]);

const COMMAND_OPTIONS = new Map([
  ["project list", new Set(["json"])],
  ["project create", new Set(["id", "name", "workspace-path", "json"])],
  ["project map", new Set(["workspace-path", "json"])],
  ["cloud login", new Set(["url", "actor-name", "json"])],
  ["cloud status", new Set(["json"])],
  ["cloud logout", new Set(["json"])],
  ["issue list", new Set(["project", "status", "json"])],
  ["issue get", new Set(["json"])],
  [
    "issue create",
    new Set([
      "project",
      "title",
      "description",
      "description-file",
      "status",
      "priority",
      "labels",
      "thread-id",
      "git-branch",
      "worktree-path",
      "worktree-branch",
      "due-date",
      "recurrence-interval",
      "recurrence-unit",
      "json",
    ]),
  ],
  [
    "issue update",
    new Set([
      "title",
      "description",
      "description-file",
      "status",
      "priority",
      "labels",
      "thread-id",
      "git-branch",
      "worktree-path",
      "worktree-branch",
      "due-date",
      "recurrence-interval",
      "recurrence-unit",
      "if-version",
      "json",
    ]),
  ],
  ["issue move", new Set(["status", "thread-id", "if-version", "json"])],
  ["issue archive", new Set(["thread-id", "if-version", "json"])],
  ["issue restore", new Set(["thread-id", "if-version", "json"])],
  ["issue relation", new Set(["type", "issue", "thread-id", "if-version", "json"])],
  ["comment list", new Set(["json"])],
  ["comment add", new Set(["body", "thread-id", "json"])],
  ["comment update", new Set(["body", "thread-id", "if-version", "json"])],
  ["comment delete", new Set(["thread-id", "if-version", "json"])],
  ["attachment download", new Set(["output", "json"])],
  ["context current", new Set(["cwd", "json"])],
]);

class TaskctlError extends Error {
  constructor(message, { code = "TASKCTL_ERROR", exitCode = 2, details } = {}) {
    super(message);
    this.name = "TaskctlError";
    this.code = code;
    this.exitCode = exitCode;
    this.details = details;
  }
}

export function parseArgs(argv) {
  if (!Array.isArray(argv)) {
    throw new TypeError("argv must be an array");
  }

  const positionals = [];
  const options = {};

  for (let index = 0; index < argv.length; index += 1) {
    const token = argv[index];
    if (token === "--") {
      positionals.push(...argv.slice(index + 1));
      break;
    }

    if (!token.startsWith("--")) {
      positionals.push(token);
      continue;
    }

    const equalsIndex = token.indexOf("=");
    const name = token.slice(2, equalsIndex === -1 ? undefined : equalsIndex);
    if (!name) {
      throw usageError("Invalid empty option");
    }

    if (Object.hasOwn(options, name)) {
      throw usageError(`Option --${name} may only be specified once`);
    }

    if (BOOLEAN_OPTIONS.has(name)) {
      if (equalsIndex !== -1) {
        throw usageError(`Option --${name} does not accept a value`);
      }
      options[name] = true;
      continue;
    }

    if (equalsIndex !== -1) {
      options[name] = token.slice(equalsIndex + 1);
      continue;
    }

    const value = argv[index + 1];
    if (value === undefined || value.startsWith("--")) {
      throw usageError(`Option --${name} requires a value`);
    }
    options[name] = value;
    index += 1;
  }

  return {
    resource: positionals[0],
    action: positionals[1],
    operands: positionals.slice(2),
    options,
  };
}

export async function main(argv = process.argv.slice(2), overrides = {}) {
  const stdout = overrides.stdout ?? process.stdout;
  const stderr = overrides.stderr ?? process.stderr;

  try {
    const parsed = parseArgs(argv);
    const result = await execute(parsed, overrides);
    writeJson(stdout, { ...result, schemaVersion: SCHEMA_VERSION });
    return 0;
  } catch (error) {
    const normalized = normalizeError(error);
    const payload = {
      schemaVersion: SCHEMA_VERSION,
      error: {
        code: normalized.code,
        message: normalized.message,
      },
    };
    if (normalized.details !== undefined) {
      payload.error.details = normalized.details;
    }
    writeJson(stderr, payload);
    return normalized.exitCode;
  }
}

async function execute(parsed, overrides) {
  const command = `${parsed.resource ?? ""} ${parsed.action ?? ""}`.trim();
  const allowedOptions = COMMAND_OPTIONS.get(command);
  if (!allowedOptions) {
    throw usageError(
      "Expected one of: project list/create/map, cloud login/status/logout, issue list/get/create/update/move/archive/restore/relation, comment list/add/update/delete, attachment download, context current",
    );
  }
  validateOptions(parsed.options, allowedOptions);

  const env = overrides.env ?? process.env;
  const usesCompanionControl = command.startsWith("cloud ") || command === "project map";
  const api = createApiClient(overrides, {
    baseUrl: usesCompanionControl || env.CODEX_TASKBOARD_COMPANION_URL !== undefined
      ? resolveCompanionUrl(env)
      : undefined,
  });
  switch (command) {
    case "project list":
      expectOperandCount(parsed, 0);
      return api.request("GET", "/api/projects");
    case "project create":
      expectOperandCount(parsed, 0);
      return api.request("POST", "/api/projects", {
        ...optionalField("id", parsed.options.id),
        name: requiredOption(parsed.options, "name"),
        ...optionalField(
          "workspacePath",
          parsed.options["workspace-path"] === undefined
            ? undefined
            : resolveInputPath(parsed.options["workspace-path"], overrides),
        ),
      });
    case "project map":
      expectOperandCount(parsed, 1);
      return api.request(
        "PUT",
        `/api/local/project-mappings/${encodeURIComponent(parsed.operands[0])}`,
        {
          workspacePath: resolveInputPath(
            requiredOption(parsed.options, "workspace-path"),
            overrides,
          ),
        },
      );
    case "cloud login":
      expectOperandCount(parsed, 0);
      return cloudLogin(
        api,
        requiredOption(parsed.options, "url"),
        requiredOption(parsed.options, "actor-name"),
        overrides,
      );
    case "cloud status":
      expectOperandCount(parsed, 0);
      return api.request("GET", "/api/local/cloud-session");
    case "cloud logout":
      expectOperandCount(parsed, 0);
      return api.request("DELETE", "/api/local/cloud-session");
    case "issue list":
      expectOperandCount(parsed, 0);
      return listIssues(api, parsed.options);
    case "issue get":
      expectOperandCount(parsed, 1);
      return api.request("GET", taskPath(parsed.operands[0]));
    case "issue create":
      expectOperandCount(parsed, 0);
      return createIssue(api, parsed.options, overrides);
    case "issue update":
      expectOperandCount(parsed, 1);
      return updateIssue(api, parsed.operands[0], parsed.options, overrides);
    case "issue move":
      expectOperandCount(parsed, 1);
      return moveIssue(api, parsed.operands[0], parsed.options, overrides);
    case "issue archive":
      expectOperandCount(parsed, 1);
      return archiveIssue(api, parsed.operands[0], parsed.options, overrides, "archive");
    case "issue restore":
      expectOperandCount(parsed, 1);
      return archiveIssue(api, parsed.operands[0], parsed.options, overrides, "restore");
    case "issue relation":
      expectOperandCount(parsed, 2);
      return mutateIssueRelation(
        api,
        parsed.operands[0],
        parsed.operands[1],
        parsed.options,
        overrides,
      );
    case "comment list":
      expectOperandCount(parsed, 1);
      return api.request("GET", `${taskPath(parsed.operands[0])}/comments`);
    case "comment add":
      expectOperandCount(parsed, 1);
      return api.request("POST", `${taskPath(parsed.operands[0])}/comments`, {
        body: requiredOption(parsed.options, "body"),
        threadId: resolveThreadId(parsed.options, overrides),
      });
    case "comment update":
      expectOperandCount(parsed, 1);
      return api.request("PATCH", commentPath(parsed.operands[0]), {
        body: requiredOption(parsed.options, "body"),
        threadId: resolveThreadId(parsed.options, overrides),
        version: explicitVersion(parsed.options["if-version"]),
      });
    case "comment delete":
      expectOperandCount(parsed, 1);
      return api.request("DELETE", commentPath(parsed.operands[0]), {
        threadId: resolveThreadId(parsed.options, overrides),
        version: explicitVersion(parsed.options["if-version"]),
      });
    case "attachment download":
      expectOperandCount(parsed, 1);
      return downloadAttachment(api, parsed.operands[0], parsed.options, overrides);
    case "context current":
      expectOperandCount(parsed, 0);
      return currentContext(api, parsed.options, overrides);
    default:
      throw usageError(`Unsupported command: ${command}`);
  }
}

function createApiClient(overrides, { baseUrl: explicitBaseUrl } = {}) {
  const fetchImplementation = overrides.fetch ?? globalThis.fetch;
  if (typeof fetchImplementation !== "function") {
    throw new TaskctlError("fetch is not available", {
      code: "CLIENT_UNAVAILABLE",
      exitCode: 3,
    });
  }

  const env = overrides.env ?? process.env;
  const baseUrl = normalizeBaseUrl(explicitBaseUrl ?? env.CODEX_TASKBOARD_URL ?? DEFAULT_API_URL);

  return {
    async request(method, pathname, body) {
      let response;
      try {
        response = await fetchImplementation(new URL(pathname, `${baseUrl}/`), {
          method,
          headers: {
            accept: "application/json",
            "x-taskboard-client": "taskctl",
            ...(body === undefined ? {} : { "content-type": "application/json" }),
          },
          ...(body === undefined ? {} : { body: JSON.stringify(body) }),
        });
      } catch (error) {
        throw new TaskctlError(`Cannot reach taskboard service at ${baseUrl}`, {
          code: "SERVICE_UNAVAILABLE",
          exitCode: 3,
          details: error instanceof Error ? error.message : String(error),
        });
      }

      const payload = await readResponse(response);
      if (!response.ok) {
        const apiError = extractApiError(payload, response.status);
        throw new TaskctlError(apiError.message, {
          code: apiError.code,
          exitCode: response.status === 409 ? 5 : 4,
          details: apiError.details,
        });
      }
      if (!payload || typeof payload !== "object" || Array.isArray(payload)) {
        throw new TaskctlError("Taskboard service returned an invalid JSON response", {
          code: "INVALID_RESPONSE",
          exitCode: 4,
        });
      }
      return payload;
    },
    async download(pathname) {
      let response;
      try {
        response = await fetchImplementation(new URL(pathname, `${baseUrl}/`), {
          headers: {
            accept: "*/*",
            "x-taskboard-client": "taskctl",
          },
        });
      } catch (error) {
        throw new TaskctlError(`Cannot reach taskboard service at ${baseUrl}`, {
          code: "SERVICE_UNAVAILABLE",
          exitCode: 3,
          details: error instanceof Error ? error.message : String(error),
        });
      }

      if (!response.ok) {
        const payload = await readResponse(response);
        const apiError = extractApiError(payload, response.status);
        throw new TaskctlError(apiError.message, {
          code: apiError.code,
          exitCode: response.status === 409 ? 5 : 4,
          details: apiError.details,
        });
      }

      const bytes = new Uint8Array(await response.arrayBuffer());
      return {
        bytes,
        contentType: response.headers.get("content-type"),
        size: Number(response.headers.get("content-length")) || bytes.byteLength,
      };
    },
  };
}

async function downloadAttachment(api, attachmentId, options, overrides) {
  const output = resolveInputPath(requiredOption(options, "output"), overrides);
  const downloaded = await api.download(attachmentContentPath(attachmentId));
  const write = overrides.writeFile ?? writeFile;
  try {
    await write(output, downloaded.bytes);
  } catch (error) {
    throw new TaskctlError(`Cannot write attachment file: ${output}`, {
      code: "FILE_WRITE_FAILED",
      exitCode: 2,
      details: error instanceof Error ? error.message : String(error),
    });
  }
  return {
    attachmentId,
    output,
    contentType: downloaded.contentType,
    size: downloaded.size,
  };
}

async function cloudLogin(api, rawUrl, actorName, overrides) {
  let remoteUrl;
  try {
    remoteUrl = normalizeCloudUrl(rawUrl);
  } catch (error) {
    throw new TaskctlError(error instanceof Error ? error.message : String(error), {
      code: error?.code ?? "INVALID_CLOUD_URL",
      exitCode: 2,
    });
  }
  const sharedKey = overrides.readSecret
    ? await overrides.readSecret()
    : await readSecretFromInput(
      overrides.stdin ?? process.stdin,
      overrides.stderr ?? process.stderr,
    );
  if (typeof sharedKey !== "string" || !sharedKey) {
    throw usageError("Cloud shared key cannot be empty");
  }
  return api.request("PUT", "/api/local/cloud-session", {
    remoteUrl,
    actorName,
    sharedKey,
  });
}

async function readSecretFromInput(input, output) {
  if (!input.isTTY) {
    let value = "";
    for await (const chunk of input) value += chunk;
    return value.replace(/\r?\n$/, "");
  }

  return new Promise((resolve, reject) => {
    let value = "";
    const wasRaw = input.isRaw;
    const wasPaused = input.isPaused();
    const finish = (error) => {
      input.off("data", onData);
      input.setRawMode(wasRaw);
      if (wasPaused) input.pause();
      output.write("\n");
      if (error) reject(error);
      else resolve(value);
    };
    const onData = (chunk) => {
      for (const character of String(chunk)) {
        if (character === "\r" || character === "\n") return finish();
        if (character === "\u0003") {
          return finish(new TaskctlError("Cloud login canceled", {
            code: "CANCELED",
            exitCode: 2,
          }));
        }
        if (character === "\u007f" || character === "\b") {
          value = value.slice(0, -1);
        } else {
          value += character;
        }
      }
    };
    output.write("Shared key: ");
    input.setRawMode(true);
    input.setEncoding("utf8");
    input.resume();
    input.on("data", onData);
  });
}

async function listIssues(api, options) {
  if (options.status !== undefined) {
    assertStatus(options.status);
  }
  const search = new URLSearchParams();
  if (options.project !== undefined) search.set("projectId", options.project);
  if (options.status !== undefined) search.set("status", options.status);
  const query = search.size > 0 ? `?${search}` : "";
  return api.request("GET", `/api/tasks${query}`);
}

async function createIssue(api, options, overrides) {
  const status = options.status ?? "backlog";
  const priority = options.priority ?? "none";
  assertStatus(status);
  assertPriority(priority);

  const developmentContext = developmentContextFromOptions(options, overrides);
  const recurrence = recurrenceFromOptions(options);
  const threadId = resolveThreadId(options, overrides);
  return api.request("POST", "/api/tasks", {
    projectId: requiredOption(options, "project"),
    title: requiredOption(options, "title"),
    description: await resolveDescription(options, overrides),
    status,
    priority,
    labels: parseLabels(options.labels),
    threadId,
    ...optionalField("developmentContext", developmentContext),
    ...optionalField("dueDate", options["due-date"]),
    ...optionalField("recurrence", recurrence),
  });
}

async function updateIssue(api, taskId, options, overrides) {
  if (options.status !== undefined) assertStatus(options.status);
  if (options.priority !== undefined) assertPriority(options.priority);

  const developmentContext = developmentContextFromOptions(options, overrides);
  const recurrence = recurrenceFromOptions(options);
  const threadId = resolveThreadId(options, overrides);
  const patch = {
    ...optionalField("title", options.title),
    ...optionalField("status", options.status),
    ...optionalField("priority", options.priority),
    ...optionalField("labels", options.labels === undefined ? undefined : parseLabels(options.labels)),
    ...optionalField("developmentContext", developmentContext),
    ...optionalField("dueDate", options["due-date"]),
    ...optionalField("recurrence", recurrence),
  };
  if (options.description !== undefined || options["description-file"] !== undefined) {
    patch.description = await resolveDescription(options, overrides);
  }

  if (Object.keys(patch).length === 0) {
    throw usageError("issue update requires at least one field to update");
  }
  patch.threadId = threadId;
  patch.version = await resolveVersion(api, taskId, options["if-version"]);
  return api.request("PATCH", taskPath(taskId), patch);
}

async function moveIssue(api, taskId, options, overrides) {
  const status = requiredOption(options, "status");
  assertStatus(status);
  const threadId = resolveThreadId(options, overrides);
  return api.request("POST", `${taskPath(taskId)}/move`, {
    status,
    threadId,
    version: await resolveVersion(api, taskId, options["if-version"]),
  });
}

async function archiveIssue(api, taskId, options, overrides, action) {
  const threadId = resolveThreadId(options, overrides);
  return api.request("POST", `${taskPath(taskId)}/${action}`, {
    threadId,
    version: await resolveVersion(api, taskId, options["if-version"]),
  });
}

async function mutateIssueRelation(api, action, taskId, options, overrides) {
  if (action !== "add" && action !== "remove") {
    throw usageError("issue relation action must be add or remove");
  }
  const type = requiredOption(options, "type");
  if (!["parent", "blocks", "blocked_by", "related"].includes(type)) {
    throw usageError("--type must be parent, blocks, blocked_by, or related");
  }
  const relatedTaskId = requiredOption(options, "issue");
  const threadId = resolveThreadId(options, overrides);
  const version = await resolveVersion(api, taskId, options["if-version"]);
  return api.request(
    action === "add" ? "POST" : "DELETE",
    `${taskPath(taskId)}/relations/${type}/${encodeURIComponent(relatedTaskId)}`,
    { threadId, version },
  );
}

async function currentContext(api, options, overrides) {
  const cwd = path.resolve(options.cwd ?? overrides.cwd ?? process.cwd());
  const response = await api.request("GET", "/api/projects");
  const projects = Array.isArray(response.projects) ? response.projects : [];
  const matchingProjects = projects
    .filter((candidate) => workspaceContains(candidate?.workspacePath, cwd))
    .sort((left, right) => right.workspacePath.length - left.workspacePath.length);
  const project = matchingProjects[0]
    ?? projects.find((candidate) => candidate?.id === DEFAULT_PROJECT_ID)
    ?? projects[0]
    ?? null;
  return { cwd, project };
}

function workspaceContains(workspacePath, cwd) {
  if (typeof workspacePath !== "string" || workspacePath.length === 0) return false;
  const relative = path.relative(path.resolve(workspacePath), cwd);
  return relative === "" || (!relative.startsWith(`..${path.sep}`) && relative !== ".." && !path.isAbsolute(relative));
}

function resolveInputPath(value, overrides) {
  return path.resolve(overrides.cwd ?? process.cwd(), value);
}

async function resolveVersion(api, taskId, rawVersion) {
  if (rawVersion !== undefined) {
    const version = Number(rawVersion);
    if (!Number.isSafeInteger(version) || version < 1) {
      throw usageError("--if-version must be a positive integer");
    }
    return version;
  }

  const response = await api.request("GET", taskPath(taskId));
  const version = response.task?.version;
  if (!Number.isSafeInteger(version) || version < 1) {
    throw new TaskctlError("Taskboard service returned a task without a valid version", {
      code: "INVALID_RESPONSE",
      exitCode: 4,
    });
  }
  return version;
}

async function resolveDescription(options, overrides) {
  if (options.description !== undefined && options["description-file"] !== undefined) {
    throw usageError("Use either --description or --description-file, not both");
  }
  if (options["description-file"] === undefined) {
    return options.description ?? "";
  }

  const read = overrides.readFile ?? readFile;
  try {
    return await read(options["description-file"], "utf8");
  } catch (error) {
    throw new TaskctlError(`Cannot read description file: ${options["description-file"]}`, {
      code: "FILE_READ_FAILED",
      exitCode: 2,
      details: error instanceof Error ? error.message : String(error),
    });
  }
}

function parseLabels(rawLabels) {
  if (rawLabels === undefined || rawLabels === "") return [];
  return [...new Set(rawLabels.split(",").map((label) => label.trim()).filter(Boolean))];
}

function developmentContextFromOptions(options, overrides) {
  const branch = options["git-branch"];
  const worktreePath = options["worktree-path"];
  const worktreeBranch = options["worktree-branch"];
  if (branch !== undefined && (worktreePath !== undefined || worktreeBranch !== undefined)) {
    throw usageError("Use either --git-branch or --worktree-path/--worktree-branch, not both");
  }
  if (worktreeBranch !== undefined && worktreePath === undefined) {
    throw usageError("--worktree-branch requires --worktree-path");
  }
  if (branch !== undefined) return { type: "branch", branch };
  if (worktreePath !== undefined) {
    return {
      type: "worktree",
      path: resolveInputPath(worktreePath, overrides),
      branch: worktreeBranch ?? null,
    };
  }
  return undefined;
}

function recurrenceFromOptions(options) {
  const rawInterval = options["recurrence-interval"];
  const unit = options["recurrence-unit"];
  if (rawInterval === undefined && unit === undefined) return undefined;
  if (rawInterval === undefined || unit === undefined) {
    throw usageError("Use --recurrence-interval and --recurrence-unit together");
  }
  const interval = Number(rawInterval);
  if (!Number.isSafeInteger(interval) || interval < 1 || interval > 365) {
    throw usageError("--recurrence-interval must be an integer from 1 to 365");
  }
  if (!["day", "week", "month", "year"].includes(unit)) {
    throw usageError("--recurrence-unit must be day, week, month, or year");
  }
  return { interval, unit };
}

function resolveThreadId(options, overrides) {
  const env = overrides.env ?? process.env;
  const value = options["thread-id"] ?? env.CODEX_THREAD_ID;
  if (typeof value !== "string" || value.trim().length === 0) {
    throw usageError("Codex conversation attribution requires --thread-id or CODEX_THREAD_ID");
  }
  const threadId = value.trim();
  if (threadId.length > 256) {
    throw usageError("--thread-id and CODEX_THREAD_ID cannot exceed 256 characters");
  }
  return threadId;
}

function requiredOption(options, name) {
  const value = options[name];
  if (value === undefined || value === "") {
    throw usageError(`Missing required option --${name}`);
  }
  return value;
}

function optionalField(name, value) {
  return value === undefined ? {} : { [name]: value };
}

function validateOptions(options, allowedOptions) {
  for (const name of Object.keys(options)) {
    if (!allowedOptions.has(name)) {
      throw usageError(`Unknown option --${name}`);
    }
  }
}

function expectOperandCount(parsed, expected) {
  if (parsed.operands.length !== expected) {
    throw usageError(
      expected === 0
        ? `${parsed.resource} ${parsed.action} does not accept positional arguments`
        : `${parsed.resource} ${parsed.action} requires exactly ${expected} positional ${
            expected === 1 ? "argument" : "arguments"
          }`,
    );
  }
}

function assertStatus(status) {
  if (!isTaskStatus(status)) {
    throw usageError(`Invalid status: ${status}. Expected one of: ${TASK_STATUSES.join(", ")}`);
  }
}

function assertPriority(priority) {
  if (!isTaskPriority(priority)) {
    throw usageError(`Invalid priority: ${priority}`);
  }
}

function taskPath(taskId) {
  if (!taskId) throw usageError("Missing issue id");
  return `/api/tasks/${encodeURIComponent(taskId)}`;
}

function commentPath(commentId) {
  if (!commentId) throw usageError("Missing comment id");
  return `/api/comments/${encodeURIComponent(commentId)}`;
}

function attachmentContentPath(attachmentId) {
  if (!attachmentId) throw usageError("Missing attachment id");
  return `/api/attachments/${encodeURIComponent(attachmentId)}/content`;
}

function explicitVersion(rawVersion) {
  if (rawVersion === undefined) throw usageError("Missing required option --if-version");
  const version = Number(rawVersion);
  if (!Number.isSafeInteger(version) || version < 1) {
    throw usageError("--if-version must be a positive integer");
  }
  return version;
}

function normalizeBaseUrl(rawUrl) {
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw usageError("CODEX_TASKBOARD_URL must be a valid URL");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") {
    throw usageError("CODEX_TASKBOARD_URL must use http or https");
  }
  url.pathname = url.pathname.replace(/\/$/, "");
  url.search = "";
  url.hash = "";
  return url.toString().replace(/\/$/, "");
}

function resolveCompanionUrl(env) {
  const rawUrl = env.CODEX_TASKBOARD_COMPANION_URL
    ?? env.CODEX_TASKBOARD_URL
    ?? DEFAULT_API_URL;
  let url;
  try {
    url = new URL(rawUrl);
  } catch {
    throw usageError("Local companion URL must be a valid URL");
  }
  const isLoopback = url.hostname === "localhost"
    || url.hostname === "127.0.0.1"
    || url.hostname === "[::1]";
  if (
    !isLoopback
    || (url.protocol !== "http:" && url.protocol !== "https:")
    || url.username
    || url.password
    || (url.pathname !== "/" && url.pathname !== "")
    || url.search
    || url.hash
  ) {
    throw usageError("Local companion URL must be a loopback HTTP or HTTPS origin");
  }
  return url.origin;
}

async function readResponse(response) {
  const text = await response.text();
  if (!text) return {};
  try {
    return JSON.parse(text);
  } catch {
    throw new TaskctlError("Taskboard service returned invalid JSON", {
      code: "INVALID_RESPONSE",
      exitCode: 4,
    });
  }
}

function extractApiError(payload, status) {
  if (payload?.error && typeof payload.error === "object") {
    return {
      code: payload.error.code ?? `HTTP_${status}`,
      message: payload.error.message ?? `Taskboard service returned HTTP ${status}`,
      details: payload.error.details,
    };
  }
  return {
    code: payload?.code ?? `HTTP_${status}`,
    message:
      payload?.message ??
      (typeof payload?.error === "string" ? payload.error : `Taskboard service returned HTTP ${status}`),
    details: payload?.details,
  };
}

function normalizeError(error) {
  if (error instanceof TaskctlError) return error;
  return new TaskctlError(error instanceof Error ? error.message : String(error), {
    code: "INTERNAL_ERROR",
    exitCode: 1,
  });
}

function usageError(message) {
  return new TaskctlError(message, { code: "USAGE_ERROR", exitCode: 2 });
}

function writeJson(stream, payload) {
  stream.write(`${JSON.stringify(payload)}\n`);
}

const entrypoint = process.argv[1] ? realpathSync(process.argv[1]) : "";
if (entrypoint === realpathSync(fileURLToPath(import.meta.url))) {
  process.exitCode = await main();
}
