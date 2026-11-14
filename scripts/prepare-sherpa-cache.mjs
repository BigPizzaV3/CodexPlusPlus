import {
  existsSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, renameSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";

// Static libraries linked by sherpa-onnx-sys 1.13.8's build.rs.
const libraries = [
  "sherpa-onnx-c-api", "sherpa-onnx-core", "kaldi-decoder-core",
  "sherpa-onnx-kaldifst-core", "sherpa-onnx-fstfar", "sherpa-onnx-fst",
  "kaldi-native-fbank-core", "kissfft-float", "piper_phonemize", "espeak-ng",
  "ucd", "onnxruntime", "ssentencepiece_core",
];
const staticPackage = /^sherpa-onnx-v[\d.]+-(win-(?:x64|arm64)-static-MT-Release|linux-(?:x64|aarch64)-static|osx-(?:x64|arm64)-static)-lib$/;

export function prepareSherpaCache(targetDirectory, temporaryDirectory = tmpdir()) {
  const target = resolve(targetDirectory);
  const cache = join(target, "sherpa-onnx-prebuilt");
  if (!existsSync(cache)) return [];
  if (lstatSync(target).isSymbolicLink() || lstatSync(cache).isSymbolicLink()) {
    throw new Error("Refusing a symlinked sherpa cache");
  }
  const root = realpathSync(cache);
  const moved = [];
  let quarantine;
  for (const entry of readdirSync(root, { withFileTypes: true })) {
    if (!staticPackage.test(entry.name)) continue;
    const extracted = join(root, entry.name);
    if (entry.isSymbolicLink() || realpathSync(extracted) !== extracted) {
      throw new Error(`Refusing a symlinked sherpa package: ${extracted}`);
    }
    if (!entry.isDirectory()) continue;
    const windows = entry.name.includes("-win-");
    const complete = libraries.every(name => {
      const library = join(extracted, "lib", windows ? `${name}.lib` : `lib${name}.a`);
      if (!existsSync(library)) return false;
      const stat = lstatSync(library);
      return stat.isFile() && stat.size > 0;
    });
    if (complete) continue;

    // Keep incomplete entries recoverable outside Cargo's cached target directory.
    if (!quarantine) {
      mkdirSync(temporaryDirectory, { recursive: true });
      quarantine = realpathSync(mkdtempSync(join(temporaryDirectory, "codex-sherpa-cache-")));
      if (quarantine === root || quarantine.startsWith(`${root}/`) || quarantine.startsWith(`${root}\\`)) {
        throw new Error("Sherpa quarantine must be outside the prebuilt cache");
      }
    }
    if (dirname(extracted) !== root) throw new Error("Sherpa package escaped the cache");
    renameSync(extracted, join(quarantine, entry.name));
    const archive = `${extracted}.tar.bz2`;
    if (existsSync(archive)) renameSync(archive, join(quarantine, `${entry.name}.tar.bz2`));
    moved.push(entry.name);
    console.log(`Quarantined incomplete sherpa package: ${entry.name}`);
  }
  return moved;
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  prepareSherpaCache(process.env.CARGO_TARGET_DIR || "target", process.env.RUNNER_TEMP || tmpdir());
}
