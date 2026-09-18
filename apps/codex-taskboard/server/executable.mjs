import { execFile, spawn } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { promisify } from "node:util";

const execFileAsync = promisify(execFile);

function usesNodeRuntime(executable) {
  const ext = path.extname(executable).toLowerCase();
  if (ext === ".js" || ext === ".mjs" || ext === ".cjs") return true;
  try {
    const firstLine = readFileSync(executable, "utf8").split(/\r?\n/, 1)[0];
    return /^#!.*\bnode(?:\.exe)?\b/i.test(firstLine);
  } catch {
    return false;
  }
}

function normalizeExecutable(executable, args = []) {
  if (usesNodeRuntime(executable)) {
    return { executable: process.execPath, args: [executable, ...args] };
  }
  return { executable, args };
}

function childProcessOptions(options) {
  return {
    ...options,
    windowsHide: process.platform === "win32",
  };
}

export function spawnExecutable(executable, args, options) {
  const command = normalizeExecutable(executable, args);
  return spawn(command.executable, command.args, childProcessOptions(options));
}

export function execFileExecutable(executable, args, options) {
  const command = normalizeExecutable(executable, args);
  return execFileAsync(command.executable, command.args, childProcessOptions(options));
}
