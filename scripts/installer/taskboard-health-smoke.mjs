#!/usr/bin/env node

import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { spawn } from "node:child_process";

const runtimeArgument = process.argv[2];
const runtimeRoot = runtimeArgument ? path.resolve(runtimeArgument) : "";
const port = Number(process.env.CODEX_TASKBOARD_SMOKE_PORT || "47824");

if (!runtimeRoot || !Number.isInteger(port) || port < 1 || port > 65535) {
  throw new Error("usage: taskboard-health-smoke.mjs RUNTIME_ROOT");
}

const serverPath = path.join(runtimeRoot, "server", "index.mjs");
for (const required of [
  path.join(runtimeRoot, "dist", "web", "index.html"),
  path.join(runtimeRoot, "scripts", "codex-injector.mjs"),
  path.join(runtimeRoot, "server", "index.mjs"),
]) {
  if (!existsSync(required)) throw new Error(`Missing Taskboard runtime file: ${required}`);
}
const dataRoot = await mkdtemp(path.join(tmpdir(), "codex-taskboard-smoke-"));
const origin = `http://127.0.0.1:${port}`;
let output = "";

const child = spawn(process.execPath, [serverPath], {
  cwd: runtimeRoot,
  env: {
    ...process.env,
    CODEX_HOME: path.join(dataRoot, "codex-home"),
    CODEX_TASKBOARD_DATA_DIR: path.join(dataRoot, "taskboard"),
    CODEX_TASKBOARD_HOST: "127.0.0.1",
    CODEX_TASKBOARD_PORT: String(port),
  },
  stdio: ["ignore", "pipe", "pipe"],
});

child.stdout.on("data", (chunk) => { output += chunk.toString(); });
child.stderr.on("data", (chunk) => { output += chunk.toString(); });

function request(pathname) {
  return fetch(`${origin}${pathname}`, {
    signal: AbortSignal.timeout(1_000),
  });
}

async function waitForHealth() {
  const deadline = Date.now() + 10_000;
  while (Date.now() < deadline) {
    if (child.exitCode !== null) {
      throw new Error(`Taskboard server exited with code ${child.exitCode}\n${output}`);
    }
    try {
      const response = await request("/health");
      if (response.ok) return response;
    } catch {
      // The server may still be binding its port.
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  throw new Error(`Taskboard /health did not become ready\n${output}`);
}

try {
  const health = await waitForHealth();
  const healthBody = await health.json();
  if (healthBody?.status !== "ok") {
    throw new Error(`Unexpected Taskboard health response: ${JSON.stringify(healthBody)}`);
  }
  if (health.headers.get("x-taskboard-bind-host") !== "127.0.0.1") {
    throw new Error("Taskboard health response did not verify loopback binding");
  }

  const page = await request("/?host=codex");
  const pageBody = await page.text();
  if (!page.ok || !pageBody.includes('<div id="root"')) {
    throw new Error(`Taskboard page entry is unavailable: HTTP ${page.status}`);
  }
  console.log(`Taskboard runtime health OK: ${runtimeRoot}`);
} finally {
  if (child.exitCode === null) {
    await new Promise((resolve) => {
      child.once("close", resolve);
      child.kill();
    });
  }
  await rm(dataRoot, { recursive: true, force: true });
}
