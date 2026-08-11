import os from "node:os";
import { pathToFileURL } from "node:url";

import { createTaskboardServer, resolveHost, resolvePort } from "./app.mjs";

export { createTaskboardServer, resolveHost, resolvePort, resolveServerOptions } from "./app.mjs";

async function main() {
  const app = createTaskboardServer();
  const host = resolveHost();
  const address = await app.listen({ host, port: resolvePort() });
  console.log(`Codex Taskboard listening on http://127.0.0.1:${address.port}`);
  if (host === "0.0.0.0") {
    const addresses = Object.values(os.networkInterfaces())
      .flat()
      .filter((entry) => entry?.family === "IPv4" && !entry.internal)
      .map((entry) => entry.address);
    for (const lanAddress of [...new Set(addresses)]) {
      console.log(`Codex Taskboard available on LAN at http://${lanAddress}:${address.port}`);
    }
  }

  let closing = false;
  const close = async () => {
    if (closing) return;
    closing = true;
    await app.close();
  };
  process.once("SIGINT", () => close().then(() => process.exit(0)));
  process.once("SIGTERM", () => close().then(() => process.exit(0)));
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error) => {
    console.error(error);
    process.exitCode = 1;
  });
}
