import { access, mkdtemp, readFile, readdir, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { Miniflare } from "miniflare";

const PROJECT_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../..");
const ENTRY_PATH = path.join(PROJECT_ROOT, "cloud", "src", "index.mjs");
const MIGRATIONS_DIRECTORY = path.join(PROJECT_ROOT, "cloud", "migrations");

async function requireCloudImplementation() {
  const missing = [];
  for (const filename of [ENTRY_PATH, MIGRATIONS_DIRECTORY]) {
    try {
      await access(filename);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
      missing.push(path.relative(PROJECT_ROOT, filename));
    }
  }
  if (missing.length > 0) {
    throw new Error(`Cloud implementation is missing:\n${missing.join("\n")}`);
  }
}

async function migrationPaths() {
  const filenames = (await readdir(MIGRATIONS_DIRECTORY))
    .filter((filename) => filename.endsWith(".sql"))
    .sort();
  if (filenames.length === 0) throw new Error("Cloud implementation has no D1 migrations");
  return filenames.map((filename) => path.join(MIGRATIONS_DIRECTORY, filename));
}

export async function createCloudWorkerHarness({
  sharedSecret = "two-person-shared-secret",
} = {}) {
  await requireCloudImplementation();
  const persistenceRoot = await mkdtemp(path.join(os.tmpdir(), "taskboard-cloud-worker-"));
  const miniflare = new Miniflare({
    modules: true,
    scriptPath: ENTRY_PATH,
    modulesRoot: PROJECT_ROOT,
    compatibilityDate: "2026-07-24",
    bindings: {
      TASKBOARD_ENVIRONMENT: "production",
      TASKBOARD_SHARED_SECRET: sharedSecret,
    },
    d1Databases: { DB: "taskboard-test" },
    r2Buckets: { ATTACHMENTS: "taskboard-test-attachments" },
    defaultPersistRoot: persistenceRoot,
    d1Persist: true,
    r2Persist: true,
  });

  try {
    await miniflare.ready;
    const db = await miniflare.getD1Database("DB");
    for (const migrationPath of await migrationPaths()) {
      await db.exec(await readFile(migrationPath, "utf8"));
    }
    const attachments = await miniflare.getR2Bucket("ATTACHMENTS");

    async function request(pathname, {
      actorName,
      password = sharedSecret,
      json,
      headers: inputHeaders,
      ...init
    } = {}) {
      const headers = new Headers(inputHeaders);
      if (actorName !== undefined) {
        headers.set(
          "authorization",
          `Basic ${Buffer.from(`${actorName}:${password}`, "utf8").toString("base64")}`,
        );
      }
      if (json !== undefined) headers.set("content-type", "application/json");
      const response = await miniflare.dispatchFetch(
        new URL(pathname, "https://taskboard.example.test"),
        {
          ...init,
          headers,
          body: json === undefined ? init.body : JSON.stringify(json),
        },
      );
      const contentType = response.headers.get("content-type") ?? "";
      const body = response.status === 204
        ? undefined
        : contentType.includes("application/json")
          ? await response.clone().json()
          : await response.clone().text();
      return { response, body };
    }

    return {
      attachments,
      db,
      miniflare,
      request,
      sharedSecret,
      async listAttachmentKeys() {
        return (await attachments.list()).objects.map((object) => object.key).sort();
      },
      async dispose() {
        await miniflare.dispose();
        await rm(persistenceRoot, { recursive: true, force: true });
      },
    };
  } catch (error) {
    await miniflare.dispose();
    await rm(persistenceRoot, { recursive: true, force: true });
    throw error;
  }
}
