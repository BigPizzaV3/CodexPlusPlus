import { createRequire } from "node:module";
import { createHash } from "node:crypto";
import { readFile, writeFile } from "node:fs/promises";
import path from "node:path";

const root = path.resolve(import.meta.dirname, "..");
const require = createRequire(path.join(root, "apps/codex-plus-manager/package.json"));
// Exact Babel/esbuild versions keep the shipped AST audit digests and bundle reproducible.
const { build } = require("esbuild");
const result = await build({
  entryPoints: [path.join(root, "assets/native-browser/inspect-service.mjs")],
  bundle: true,
  platform: "node",
  format: "cjs",
  target: "node22",
  minify: true,
  legalComments: "eof",
  nodePaths: [path.join(root, "apps/codex-plus-manager/node_modules")],
  write: false,
  metafile: true,
});
const notice = await readFile(path.join(root, "assets/native-browser/inspector-NOTICE.txt"), "utf8");
const dependencies = new Set();
for (const input of Object.keys(result.metafile.inputs)) {
  const matches = [...input.replaceAll("\\", "/").matchAll(/node_modules\/((?:@[^/]+\/)?[^/]+)/g)];
  if (matches.length) dependencies.add(matches.at(-1)[1]);
}
let licenses = notice;
for (const dependency of [...dependencies].sort()) {
  const dir = path.join(root, "apps/codex-plus-manager/node_modules", dependency);
  const pkg = JSON.parse(await readFile(path.join(dir, "package.json"), "utf8"));
  let license;
  for (const name of ["LICENSE", "LICENSE.md", "LICENSE.txt", "LICENSE-MIT.txt"]) {
    license = await readFile(path.join(dir, name), "utf8").catch(() => null);
    if (license) break;
  }
  if (!license) throw new Error(`Missing license for ${dependency}`);
  licenses += `\n${dependency} ${pkg.version}\n${license}\n`;
}
const source = await readFile(path.join(root, "assets/native-browser/inspect-service.mjs"));
const sourceSha = createHash("sha256").update(source).digest("hex");
const output = `/* Inspector source SHA256: ${sourceSha}\n${licenses.replaceAll("*/", "* /")}*/\n${result.outputFiles[0].text}`;
const file = path.join(root, "assets/native-browser/inspect-service.cjs");
if (process.argv.includes("--check")) {
  if (await readFile(file, "utf8") !== output) throw new Error("Inspector bundle drift");
} else {
  await writeFile(file, output, "utf8");
}
