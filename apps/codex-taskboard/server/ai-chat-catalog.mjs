import { execFile, spawn } from "node:child_process";
import { readFile, realpath, stat } from "node:fs/promises";
import path from "node:path";
import { promisify } from "node:util";

import { ApiError } from "./database.mjs";

const execFileAsync = promisify(execFile);
const CATALOG_TIMEOUT_MS = 10_000;
const CATALOG_MAX_BUFFER = 2 * 1024 * 1024;

async function existingDirectory(value) {
  if (typeof value !== "string" || !path.isAbsolute(value.trim())) return null;
  try {
    const resolved = await realpath(value.trim());
    return (await stat(resolved)).isDirectory() ? resolved : null;
  } catch {
    return null;
  }
}

export async function loadDeviceWorkspaces(codexStatePath, database) {
  const workspaces = new Map();
  let localProjects = {};
  try {
    const state = JSON.parse(await readFile(codexStatePath, "utf8"));
    if (
      state?.["local-projects"]
      && typeof state["local-projects"] === "object"
      && !Array.isArray(state["local-projects"])
    ) {
      localProjects = state["local-projects"];
    }
  } catch {}

  for (const [projectId, project] of Object.entries(localProjects)) {
    if (!Array.isArray(project?.rootPaths)) continue;
    for (const rootPath of project.rootPaths) {
      const workspacePath = await existingDirectory(rootPath);
      if (!workspacePath) continue;
      workspaces.set(projectId, workspacePath);
      break;
    }
  }

  for (const project of await database.listProjects()) {
    if (workspaces.has(project.id)) continue;
    const workspacePath = await existingDirectory(project.workspacePath);
    if (workspacePath) workspaces.set(project.id, workspacePath);
  }
  return workspaces;
}

export async function resolveAiWorkspace(projectId, codexStatePath, database) {
  const project = await database.getProject(projectId);
  if (!project) {
    throw new ApiError(404, "PROJECT_NOT_FOUND", `Project '${projectId}' does not exist`);
  }
  const workspaces = await loadDeviceWorkspaces(codexStatePath, database);
  const workspacePath = workspaces.get(projectId);
  if (!workspacePath) {
    throw new ApiError(
      409,
      "PROJECT_WORKSPACE_UNAVAILABLE",
      `Project '${projectId}' has no available device workspace`,
    );
  }
  return {
    workspacePath,
    addDirectories: [...new Set(workspaces.values())].filter((candidate) => candidate !== workspacePath),
    project,
  };
}

function sanitizeModels(value) {
  if (!Array.isArray(value)) throw new Error("Codex returned an invalid model catalog");
  return value.flatMap((model) => {
    if (
      !model
      || typeof model !== "object"
      || (model.visibility !== undefined && model.visibility !== "list")
      || typeof model.slug !== "string"
      || !model.slug.trim()
    ) {
      return [];
    }
    const slug = model.slug.trim();
    const efforts = Array.isArray(model.supported_reasoning_levels)
      ? [...new Set(model.supported_reasoning_levels.flatMap((level) => (
          typeof level?.effort === "string" && level.effort.trim() ? [level.effort.trim()] : []
        )))]
      : [];
    const serviceTiers = Array.isArray(model.service_tiers)
      ? model.service_tiers.flatMap((tier) => (
          typeof tier?.id === "string"
          && tier.id.trim()
          && typeof tier.name === "string"
          && tier.name.trim()
            ? [{ id: tier.id.trim(), name: tier.name.trim() }]
            : []
        ))
      : [];
    return [{
      slug,
      displayName: typeof model.display_name === "string" && model.display_name.trim()
        ? model.display_name.trim()
        : slug,
      description: typeof model.description === "string" ? model.description : "",
      defaultReasoningEffort: typeof model.default_reasoning_level === "string"
        ? model.default_reasoning_level.trim()
        : "",
      supportedReasoningEfforts: efforts,
      serviceTiers,
    }];
  });
}

function listSkills(codexExecutable, workspacePath, processEnv) {
  return new Promise((resolve, reject) => {
    const child = spawn(codexExecutable, ["app-server", "--stdio"], {
      cwd: workspacePath,
      env: processEnv,
      stdio: ["pipe", "pipe", "ignore"],
    });
    let buffer = "";
    let settled = false;
    const timeout = setTimeout(
      () => finish(new Error("Timed out while reading Codex skills")),
      CATALOG_TIMEOUT_MS,
    );

    function finish(error, value) {
      if (settled) return;
      settled = true;
      clearTimeout(timeout);
      child.stdin.end();
      child.kill("SIGTERM");
      if (error) reject(error);
      else resolve(value);
    }

    function send(message) {
      child.stdin.write(`${JSON.stringify(message)}\n`);
    }

    function handleMessage(message) {
      if (message?.id === 1) {
        if (message.error) return finish(new Error("Codex app-server rejected initialization"));
        send({ method: "initialized" });
        send({
          id: 2,
          method: "skills/list",
          params: { cwds: [workspacePath], forceReload: false },
        });
        return;
      }
      if (message?.id !== 2) return;
      if (message.error) return finish(new Error("Codex app-server could not list skills"));
      finish(null, Array.isArray(message.result?.data) ? message.result.data : []);
    }

    child.stdout.setEncoding("utf8");
    child.stdout.on("data", (chunk) => {
      buffer += chunk;
      if (buffer.length > CATALOG_MAX_BUFFER) {
        finish(new Error("Codex skills response exceeded the catalog size limit"));
        return;
      }
      let newlineIndex = buffer.indexOf("\n");
      while (newlineIndex >= 0 && !settled) {
        const line = buffer.slice(0, newlineIndex).trim();
        buffer = buffer.slice(newlineIndex + 1);
        if (line) {
          try {
            handleMessage(JSON.parse(line));
          } catch {}
        }
        newlineIndex = buffer.indexOf("\n");
      }
    });
    child.stdin.on("error", (error) => finish(error));
    child.once("error", (error) => finish(error));
    child.once("exit", (code, signal) => {
      if (!settled) {
        finish(new Error(`Codex app-server exited before listing skills (${signal || code})`));
      }
    });
    child.once("spawn", () => {
      send({
        id: 1,
        method: "initialize",
        params: {
          clientInfo: { name: "codex-taskboard", version: "0.1.0" },
          capabilities: { experimentalApi: true },
        },
      });
    });
  });
}

function sanitizeSkills(entries) {
  const unique = new Map();
  for (const entry of entries) {
    if (!Array.isArray(entry?.skills)) continue;
    for (const skill of entry.skills) {
      if (
        !skill
        || typeof skill !== "object"
        || skill.enabled === false
        || typeof skill.name !== "string"
        || !skill.name.trim()
      ) {
        continue;
      }
      const id = skill.name.trim();
      if (unique.has(id)) continue;
      const displayName = typeof skill.interface?.displayName === "string"
        ? skill.interface.displayName.trim()
        : "";
      unique.set(id, {
        id,
        label: displayName || id,
        description: typeof skill.description === "string" ? skill.description.trim() : "",
        path: typeof skill.path === "string" ? skill.path.trim() : "",
        scope: ["user", "repo", "system", "admin"].includes(skill.scope) ? skill.scope : "user",
      });
    }
  }
  return [...unique.values()].sort((left, right) => left.label.localeCompare(right.label));
}

export async function discoverAiCatalog({
  codexExecutable,
  codexStatePath,
  database,
  projectId,
  processEnv,
}) {
  const { workspacePath } = await resolveAiWorkspace(projectId, codexStatePath, database);
  const [modelResult, skillEntries] = await Promise.all([
    execFileAsync(codexExecutable, ["debug", "models"], {
      cwd: workspacePath,
      env: processEnv,
      encoding: "utf8",
      timeout: CATALOG_TIMEOUT_MS,
      maxBuffer: CATALOG_MAX_BUFFER,
    }),
    listSkills(codexExecutable, workspacePath, processEnv),
  ]);
  const modelCatalog = JSON.parse(modelResult.stdout);
  return {
    models: sanitizeModels(modelCatalog?.models),
    skills: sanitizeSkills(skillEntries),
    sandboxes: ["read-only", "workspace-write", "danger-full-access"],
  };
}
