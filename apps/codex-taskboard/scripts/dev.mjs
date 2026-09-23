import { spawn } from "node:child_process";

const children = [
  spawn(process.execPath, ["--watch", "server/index.mjs", "--dev"], {
    stdio: "inherit",
  }),
  spawn(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "dev:web"], {
    stdio: "inherit",
  }),
];

let shuttingDown = false;

function stop(exitCode = 0) {
  if (shuttingDown) return;
  shuttingDown = true;
  for (const child of children) child.kill("SIGTERM");
  process.exitCode = exitCode;
}

for (const child of children) {
  child.on("exit", (code, signal) => {
    if (!shuttingDown && code !== 0 && signal !== "SIGTERM") stop(code ?? 1);
  });
}

process.on("SIGINT", () => stop());
process.on("SIGTERM", () => stop());
