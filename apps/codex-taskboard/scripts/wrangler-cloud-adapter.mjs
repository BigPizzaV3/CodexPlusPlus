import { execFile as execFileCallback } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmod,
  mkdtemp,
  readFile,
  rm,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";

import {
  CLOUD_PROJECT_COUNTS_SQL,
  createCloudD1ImportSql,
} from "./migrate-to-cloud.mjs";

const execFile = promisify(execFileCallback);
const projectRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultWrangler = path.join(projectRoot, "node_modules", ".bin", "wrangler");

function parseD1Results(stdout) {
  const parsed = JSON.parse(stdout);
  const resultSets = Array.isArray(parsed) ? parsed : [parsed];
  return resultSets.flatMap((result) => result.results ?? result.result?.[0]?.results ?? []);
}

function missingR2Object(error) {
  const output = `${error?.message ?? ""}\n${error?.stdout ?? ""}\n${error?.stderr ?? ""}`;
  return /not found|does not exist|NoSuchKey|10007/i.test(output);
}

export function createWranglerCloudAdapters({
  remote,
  persistTo,
  configPath,
  wranglerExecutable = defaultWrangler,
  database = "codex-taskboard-db",
  bucket = "codex-taskboard-attachments",
  preparedImportSql,
  environment = process.env,
  runCommand = execFile,
} = {}) {
  const remoteEnabled = environment.TASKBOARD_MIGRATION_REMOTE === "1";
  const useRemote = remote ?? remoteEnabled;
  if (useRemote && !remoteEnabled) {
    throw new Error("Remote migration requires TASKBOARD_MIGRATION_REMOTE=1");
  }

  const resolvedPersistTo = persistTo ?? environment.TASKBOARD_MIGRATION_PERSIST_TO;
  if (!useRemote && !resolvedPersistTo) {
    throw new Error(
      "Local migration requires TASKBOARD_MIGRATION_PERSIST_TO",
    );
  }
  const resolvedConfig = path.resolve(
    configPath
      ?? environment.TASKBOARD_MIGRATION_CONFIG
      ?? path.join(projectRoot, "wrangler.jsonc"),
  );
  const modeArguments = useRemote
    ? ["--remote"]
    : ["--local", "--persist-to", path.resolve(resolvedPersistTo)];
  const temporaryDirectory = mkdtemp(
    path.join(os.tmpdir(), "taskboard-wrangler-migration-"),
  ).then(async (directory) => {
    await chmod(directory, 0o700);
    return directory;
  });
  let commandQueue = Promise.resolve();
  let sequence = 0;

  function run(args) {
    const result = commandQueue.then(() => runCommand(wranglerExecutable, args, {
      cwd: projectRoot,
      encoding: "utf8",
      maxBuffer: 16 * 1024 * 1024,
    }));
    commandQueue = result.catch(() => {});
    return result;
  }

  async function privateFile(extension, contents) {
    const directory = await temporaryDirectory;
    const filename = path.join(directory, `${sequence += 1}${extension}`);
    await writeFile(filename, contents, { flag: "wx", mode: 0o600 });
    await chmod(filename, 0o600);
    return filename;
  }

  const d1 = {
    async importTables(tables) {
      const sqlPath = await privateFile(
        ".sql",
        `${preparedImportSql ?? createCloudD1ImportSql(tables)}\n`,
      );
      await run([
        "d1",
        "execute",
        database,
        ...modeArguments,
        "--file",
        sqlPath,
        "--yes",
        "--config",
        resolvedConfig,
      ]);
    },
    async countByProject() {
      const result = await run([
        "d1",
        "execute",
        database,
        ...modeArguments,
        "--command",
        CLOUD_PROJECT_COUNTS_SQL,
        "--json",
        "--config",
        resolvedConfig,
      ]);
      return Object.fromEntries(parseD1Results(result.stdout).map((row) => [
        row.project_id,
        {
          projects: Number(row.projects),
          tasks: Number(row.tasks),
          comments: Number(row.comments),
          task_relations: Number(row.task_relations),
          attachments: Number(row.attachments),
          workflow_workspaces: Number(row.workflow_workspaces),
        },
      ]));
    },
  };

  const r2 = {
    async put(key, body, options) {
      const bodyPath = await privateFile(".attachment", Buffer.from(body));
      const contentType = options?.httpMetadata?.contentType;
      await run([
        "r2",
        "object",
        "put",
        `${bucket}/${key}`,
        ...modeArguments,
        "--file",
        bodyPath,
        "--force",
        ...(contentType ? ["--content-type", contentType] : []),
        "--config",
        resolvedConfig,
      ]);
    },
    async head(key) {
      const directory = await temporaryDirectory;
      const outputPath = path.join(directory, `${sequence += 1}.download`);
      try {
        await run([
          "r2",
          "object",
          "get",
          `${bucket}/${key}`,
          ...modeArguments,
          "--file",
          outputPath,
          "--config",
          resolvedConfig,
        ]);
      } catch (error) {
        if (missingR2Object(error)) return null;
        throw error;
      }
      await chmod(outputPath, 0o600);
      const body = await readFile(outputPath);
      return {
        size: body.byteLength,
        customMetadata: {
          sha256: createHash("sha256").update(body).digest("hex"),
        },
      };
    },
    async delete(key) {
      await run([
        "r2",
        "object",
        "delete",
        `${bucket}/${key}`,
        ...modeArguments,
        "--force",
        "--config",
        resolvedConfig,
      ]);
    },
  };

  return {
    d1,
    r2,
    async cleanup() {
      await rm(await temporaryDirectory, { recursive: true, force: true });
    },
  };
}

export function createCloudMigrationAdapters({ bundle, command } = {}) {
  const preparedImportSql = command === "import"
    ? createCloudD1ImportSql(bundle.tables)
    : undefined;
  return createWranglerCloudAdapters({ preparedImportSql });
}

export default createCloudMigrationAdapters;
