import path from "node:path";

import { normalizeCloudUrl } from "./cloud-config.mjs";

const LOCAL_COMPANION_ROUTES = new Set([
  "/health",
  "/api/meta",
  "/api/device-workspaces",
  "/api/workflow-capabilities",
  "/api/local/cloud-session",
]);

export class CloudProxyError extends Error {
  constructor(status, code, message, details) {
    super(message);
    this.name = "CloudProxyError";
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

export function isLocalCompanionRoute(pathname) {
  return LOCAL_COMPANION_ROUTES.has(pathname)
    || pathname.startsWith("/api/local/")
    || /^\/api\/projects\/[^/]+\/development-contexts$/.test(pathname);
}

function basicAuthorization(actorName, sharedKey) {
  return `Basic ${Buffer.from(`${actorName}:${sharedKey}`, "utf8").toString("base64")}`;
}

function removeGitWorktreePaths(value) {
  if (Array.isArray(value)) {
    for (const item of value) removeGitWorktreePaths(item);
    return;
  }
  if (value === null || typeof value !== "object") return;
  for (const [key, item] of Object.entries(value)) {
    if (key === "gitWorktreePath") {
      delete value[key];
    } else {
      removeGitWorktreePaths(item);
    }
  }
}

async function prepareRequest(request) {
  const url = new URL(request.url);
  let projectWorkspace = null;
  let body = request.body;
  const isJson = request.headers.get("content-type")?.includes("application/json");
  const isProjectCreate = request.method === "POST" && url.pathname === "/api/projects";
  const isTaskMutation = (
    (request.method === "POST" && url.pathname === "/api/tasks")
    || (request.method === "PATCH" && /^\/api\/tasks\/[^/]+$/.test(url.pathname))
  );
  const isWorkflowMutation = request.method === "PUT"
    && /^\/api\/projects\/[^/]+\/workflow-workspace$/.test(url.pathname);

  if (isJson && (isProjectCreate || isTaskMutation || isWorkflowMutation)) {
    let payload;
    try {
      payload = await request.clone().json();
    } catch {
      throw new CloudProxyError(400, "INVALID_JSON", "Request body must contain valid JSON");
    }
    if (payload === null || typeof payload !== "object" || Array.isArray(payload)) {
      throw new CloudProxyError(400, "INVALID_BODY", "Request body must be a JSON object");
    }
    if (isProjectCreate && Object.hasOwn(payload, "workspacePath")) {
      if (typeof payload.workspacePath === "string") {
        if (!path.isAbsolute(payload.workspacePath)) {
          throw new CloudProxyError(
            400,
            "INVALID_PROJECT_MAPPING",
            "Project workspacePath must be absolute",
          );
        }
        projectWorkspace = {
          projectId: typeof payload.id === "string" ? payload.id : null,
          workspacePath: payload.workspacePath,
        };
      }
      delete payload.workspacePath;
    }
    if (isTaskMutation && payload.developmentContext?.type === "worktree") {
      payload.developmentContext = {
        type: "worktree",
        ...(payload.developmentContext.branch === undefined
          ? {}
          : { branch: payload.developmentContext.branch }),
      };
    }
    if (isWorkflowMutation) removeGitWorktreePaths(payload.workspace);
    body = JSON.stringify(payload);
  }

  return { body, projectWorkspace };
}

async function localizeTask(task, resolveDevelopmentContext) {
  if (!task || typeof task !== "object" || task.developmentContext?.type !== "worktree") {
    return task;
  }
  const cloudContext = {
    type: "worktree",
    ...(task.developmentContext.branch === undefined
      ? {}
      : { branch: task.developmentContext.branch }),
  };
  const localContext = resolveDevelopmentContext
    ? await resolveDevelopmentContext(task.projectId, cloudContext)
    : null;
  return {
    ...task,
    developmentContext: localContext ?? { ...cloudContext, path: null },
  };
}

async function localizeResponse(
  response,
  { readConfig, setProjectWorkspace, projectWorkspace, resolveDevelopmentContext },
) {
  if (response.status === 401) return response;
  if (!response.headers.get("content-type")?.includes("application/json")) return response;
  const payload = await response.json();

  if (response.ok && projectWorkspace) {
    const projectId = projectWorkspace.projectId ?? payload.project?.id;
    if (projectId) {
      await setProjectWorkspace(projectId, projectWorkspace.workspacePath);
    }
  }

  const config = await readConfig();
  if (Array.isArray(payload.projects)) {
    payload.projects = payload.projects.map((project) => ({
      ...project,
      workspacePath: config.projectMappings[project.id] ?? null,
    }));
  }
  if (payload.project && typeof payload.project === "object") {
    payload.project = {
      ...payload.project,
      workspacePath: config.projectMappings[payload.project.id] ?? null,
    };
  }
  if (payload.task) {
    payload.task = await localizeTask(payload.task, resolveDevelopmentContext);
  }
  if (Array.isArray(payload.tasks)) {
    const contexts = new Map();
    const resolveOnce = resolveDevelopmentContext
      ? (projectId, context) => {
        const key = `${projectId ?? ""}\0${context.branch ?? ""}`;
        if (!contexts.has(key)) {
          contexts.set(key, resolveDevelopmentContext(projectId, context));
        }
        return contexts.get(key);
      }
      : null;
    payload.tasks = await Promise.all(
      payload.tasks.map((task) => localizeTask(task, resolveOnce)),
    );
  }

  const headers = new Headers(response.headers);
  headers.delete("content-length");
  headers.delete("content-encoding");
  return new Response(JSON.stringify(payload), {
    status: response.status,
    statusText: response.statusText,
    headers,
  });
}

export function createCloudProxy({
  configStore,
  getConfig,
  fetch: fetchImplementation = globalThis.fetch,
  resolveDevelopmentContext,
}) {
  const readConfig = getConfig ?? (() => configStore.read());
  const setProjectWorkspace = configStore?.setProjectWorkspace?.bind(configStore);

  return {
    async forward(request) {
      const config = await readConfig();
      if (!config?.remoteUrl || !config.actorName || !config.sharedKey) {
        throw new CloudProxyError(
          409,
          "CLOUD_NOT_CONFIGURED",
          "Cloud collaboration is not configured",
        );
      }
      let remoteUrl;
      try {
        remoteUrl = normalizeCloudUrl(config.remoteUrl);
      } catch (error) {
        throw new CloudProxyError(
          500,
          "INVALID_CLOUD_CONFIG",
          error instanceof Error ? error.message : String(error),
        );
      }

      const sourceUrl = new URL(request.url);
      const upstreamUrl = new URL(
        `${sourceUrl.pathname}${sourceUrl.search}`,
        `${remoteUrl}/`,
      );
      const headers = new Headers(request.headers);
      headers.delete("authorization");
      headers.delete("host");
      headers.delete("connection");
      headers.delete("transfer-encoding");
      for (const name of [...headers.keys()]) {
        if (name.toLowerCase().startsWith("x-taskboard-user-")) headers.delete(name);
      }
      headers.set("authorization", basicAuthorization(config.actorName, config.sharedKey));

      const prepared = await prepareRequest(request);
      if (prepared.projectWorkspace && !setProjectWorkspace) {
        throw new CloudProxyError(
          500,
          "PROJECT_MAPPING_UNAVAILABLE",
          "Local project mapping storage is unavailable",
        );
      }
      if (typeof prepared.body === "string") headers.delete("content-length");
      const init = {
        method: request.method,
        headers,
        redirect: "manual",
      };
      if (request.method !== "GET" && request.method !== "HEAD" && prepared.body !== null) {
        init.body = prepared.body;
        if (typeof prepared.body !== "string") init.duplex = "half";
      }

      let response;
      try {
        response = await fetchImplementation(upstreamUrl, init);
      } catch (error) {
        throw new CloudProxyError(
          502,
          "REMOTE_UNAVAILABLE",
          `Cannot reach cloud taskboard at ${remoteUrl}`,
          error instanceof Error ? error.message : String(error),
        );
      }
      return localizeResponse(response, {
        readConfig,
        setProjectWorkspace,
        projectWorkspace: prepared.projectWorkspace,
        resolveDevelopmentContext,
      });
    },
  };
}
