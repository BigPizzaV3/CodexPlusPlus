#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const assetUrl = "https://assets-proxy.anthropic.com/claude-ai/v2/assets/v1/c342ae4ef-ROR3Yba4.js";
const assetKey = assetUrl.replace(/^https:\/\//, "");
const cacheName = crypto.createHash("sha1").update(assetKey).digest("hex");
const oldTimeout = "D=2e3";
const previousMockTimeout = "D=3e4";
const mockTimeout = "D=125e3";
const backupDirName = ".dictation-drain-backup";
const electronCacheDefault = path.join(
  os.homedir(),
  "Library",
  "Application Support",
  "Claude",
  "Cache",
  "Cache_Data",
);

function occurrences(text, value) {
  return text.split(value).length - 1;
}

export function patchDictationBundle(input) {
  const source = Buffer.isBuffer(input) ? input.toString("utf8") : String(input);
  validateTargetBundle(source);

  const oldCount = occurrences(source, oldTimeout);
  const previousCount = occurrences(source, previousMockTimeout);
  const patchedCount = occurrences(source, mockTimeout);
  if (oldCount === 1 && previousCount === 0 && patchedCount === 0) {
    return Buffer.from(source.replace(oldTimeout, mockTimeout), "utf8");
  }
  if (oldCount === 0 && previousCount === 1 && patchedCount === 0) {
    return Buffer.from(source.replace(previousMockTimeout, mockTimeout), "utf8");
  }
  if (oldCount === 0 && previousCount === 0 && patchedCount === 1) return Buffer.from(source, "utf8");
  throw new Error("Cached dictation asset has an unexpected drain timeout");
}

function validateTargetBundle(source) {
  const isTarget = source.startsWith('import{ph as e}from"./preload-helper-')
    && source.includes('JSON.stringify({type:"CloseStream"})')
    && source.includes("c?A:D+l")
    && source.includes("A=6e4")
    && source.includes('i==="TranscriptEndpoint"');
  if (!isTarget) throw new Error("Cached dictation asset does not match the expected client contract");
}

export function extractElectronCacheAsset(entry, expectedUrl = assetUrl) {
  const bytes = Buffer.isBuffer(entry) ? entry : Buffer.from(entry);
  const url = Buffer.from(expectedUrl, "utf8");
  const urlOffset = bytes.indexOf(url);
  if (urlOffset < 0 || bytes.indexOf(url, urlOffset + url.length) >= 0) {
    throw new Error("Expected exactly one matching Electron cache URL");
  }

  const bodyStart = urlOffset + url.length;
  const responseHeaderStart = bytes.indexOf(Buffer.from("HTTP/1.1 200 OK\0", "ascii"), bodyStart);
  const contentLengthStart = responseHeaderStart < 0
    ? -1
    : bytes.indexOf(Buffer.from("content-length: ", "ascii"), responseHeaderStart);
  if (responseHeaderStart < 0 || contentLengthStart < 0) {
    throw new Error("Electron cache entry is missing an HTTP 200 response header");
  }

  const contentLengthEnd = bytes.indexOf(0, contentLengthStart);
  const lengthText = bytes.subarray(contentLengthStart + 16, contentLengthEnd).toString("ascii");
  if (contentLengthEnd < 0 || !/^\d+$/.test(lengthText)) {
    throw new Error("Electron cache entry has an invalid content length");
  }

  const contentLength = Number(lengthText);
  const bodyEnd = bodyStart + contentLength;
  if (!Number.isSafeInteger(contentLength) || bodyEnd > responseHeaderStart) {
    throw new Error("Electron cache body bounds are invalid");
  }
  const body = bytes.subarray(bodyStart, bodyEnd);
  patchDictationBundle(body);
  return Buffer.from(body);
}

function cachePaths(cacheDir) {
  const bodyPath = path.join(cacheDir, cacheName);
  return {
    bodyPath,
    metaPath: `${bodyPath}.json`,
    backupDir: path.join(cacheDir, backupDirName),
    backupBodyPath: path.join(cacheDir, backupDirName, cacheName),
    backupMetaPath: path.join(cacheDir, backupDirName, `${cacheName}.json`),
  };
}

function readMeta(metaPath) {
  try {
    return JSON.parse(fs.readFileSync(metaPath, "utf8"));
  } catch {
    return null;
  }
}

function validateMeta(meta) {
  return meta?.status === 200
    && meta.url === assetKey
    && typeof meta.headers?.["content-type"] === "string"
    && !meta.headers?.["content-encoding"];
}

function locateElectronAsset(cacheDir, expectedUrl = assetUrl) {
  if (!fs.existsSync(cacheDir)) throw new Error("Electron HTTP cache directory was not found");
  const matches = [];
  for (const entry of fs.readdirSync(cacheDir, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith("_0")) continue;
    const filePath = path.join(cacheDir, entry.name);
    const bytes = fs.readFileSync(filePath);
    if (!bytes.includes(Buffer.from(expectedUrl, "utf8"))) continue;
    matches.push(extractElectronCacheAsset(bytes, expectedUrl));
  }
  if (matches.length === 0) throw new Error("The expected dictation asset is not present in Electron's cache");
  const first = matches[0];
  if (matches.some((body) => !body.equals(first))) {
    throw new Error("Multiple different versions of the dictation asset are cached");
  }
  return first;
}

function cacheMetadata(body) {
  return {
    status: 200,
    headers: {
      "content-type": "text/javascript; charset=utf-8",
      "content-length": String(body.length),
      "cache-control": "no-store",
    },
    url: assetKey,
  };
}

function writeMetadata(filePath, metadata) {
  fs.writeFileSync(filePath, `${JSON.stringify(metadata)}\n`, { mode: 0o600 });
}

export function applyPatch({ cacheDir = path.join(here, ".cache"), electronCacheDir = electronCacheDefault } = {}) {
  const paths = cachePaths(cacheDir);
  fs.mkdirSync(cacheDir, { recursive: true });

  let originalBody;
  let originalMeta;
  let upgradingPreviousPatch = false;
  const existingMeta = readMeta(paths.metaPath);
  if (fs.existsSync(paths.bodyPath) && existingMeta) {
    if (!validateMeta(existingMeta)) throw new Error("Local asset cache metadata does not match the target URL");
    const currentBody = fs.readFileSync(paths.bodyPath);
    const currentText = currentBody.toString("utf8");
    if (occurrences(currentText, mockTimeout) === 1 && occurrences(currentText, oldTimeout) === 0) {
      if (!fs.existsSync(paths.backupBodyPath) || !fs.existsSync(paths.backupMetaPath)) {
        throw new Error("Patched asset has no reversible original backup");
      }
      const savedBody = fs.readFileSync(paths.backupBodyPath);
      const savedMeta = readMeta(paths.backupMetaPath);
      if (!validateMeta(savedMeta) || occurrences(savedBody.toString("utf8"), oldTimeout) !== 1) {
        throw new Error("Existing asset backup is invalid");
      }
      patchDictationBundle(savedBody);
      return "already-patched";
    }
    if (occurrences(currentText, previousMockTimeout) === 1 && occurrences(currentText, oldTimeout) === 0) {
      validateTargetBundle(currentText);
      upgradingPreviousPatch = true;
      if (fs.existsSync(paths.backupBodyPath) && fs.existsSync(paths.backupMetaPath)) {
        originalBody = fs.readFileSync(paths.backupBodyPath);
        originalMeta = readMeta(paths.backupMetaPath);
        if (!validateMeta(originalMeta) || occurrences(originalBody.toString("utf8"), oldTimeout) !== 1) {
          throw new Error("Existing asset backup is invalid");
        }
        patchDictationBundle(originalBody);
      } else {
        originalBody = Buffer.from(currentText.replace(previousMockTimeout, oldTimeout), "utf8");
        originalMeta = existingMeta;
      }
    } else {
      originalBody = currentBody;
      originalMeta = existingMeta;
    }
  } else {
    if (fs.existsSync(paths.bodyPath) || fs.existsSync(paths.metaPath)) {
      throw new Error("Local asset cache is incomplete");
    }
    originalBody = locateElectronAsset(electronCacheDir);
    originalMeta = cacheMetadata(originalBody);
  }

  const patchedBody = patchDictationBundle(originalBody);
  fs.mkdirSync(paths.backupDir, { recursive: true, mode: 0o700 });
  if (fs.existsSync(paths.backupBodyPath) || fs.existsSync(paths.backupMetaPath)) {
    if (!fs.existsSync(paths.backupBodyPath) || !fs.existsSync(paths.backupMetaPath)) {
      throw new Error("Local asset backup is incomplete");
    }
    const savedBody = fs.readFileSync(paths.backupBodyPath);
    const savedMeta = readMeta(paths.backupMetaPath);
    if (!savedBody.equals(originalBody) || !validateMeta(savedMeta)) {
      throw new Error("Existing asset backup does not match the original cache entry");
    }
  } else {
    fs.writeFileSync(paths.backupBodyPath, originalBody, { mode: 0o600 });
    writeMetadata(paths.backupMetaPath, originalMeta);
  }

  fs.writeFileSync(paths.bodyPath, patchedBody, { mode: 0o600 });
  writeMetadata(paths.metaPath, originalMeta);
  return upgradingPreviousPatch ? "updated" : "patched";
}

export function restorePatch({ cacheDir = path.join(here, ".cache") } = {}) {
  const paths = cachePaths(cacheDir);
  if (!fs.existsSync(paths.backupBodyPath) || !fs.existsSync(paths.backupMetaPath)) {
    throw new Error("No reversible dictation asset backup exists");
  }
  const body = fs.readFileSync(paths.backupBodyPath);
  const meta = readMeta(paths.backupMetaPath);
  if (!validateMeta(meta)) throw new Error("Saved asset backup metadata does not match the target URL");
  patchDictationBundle(body);
  if (occurrences(body.toString("utf8"), oldTimeout) !== 1) {
    throw new Error("Saved asset backup is not the expected unpatched version");
  }
  fs.writeFileSync(paths.bodyPath, body, { mode: 0o600 });
  writeMetadata(paths.metaPath, meta);
  return "restored";
}

function parseArgs(argv) {
  const options = { action: null, cacheDir: path.join(here, ".cache"), electronCacheDir: electronCacheDefault };
  for (let i = 0; i < argv.length; i += 1) {
    const arg = argv[i];
    if (arg === "--apply") options.action = "apply";
    else if (arg === "--restore") options.action = "restore";
    else if (arg === "--cache-dir") options.cacheDir = path.resolve(argv[++i] ?? "");
    else if (arg === "--electron-cache-dir") options.electronCacheDir = path.resolve(argv[++i] ?? "");
    else throw new Error(`Unknown argument: ${arg}`);
  }
  if (!options.action) throw new Error("Choose --apply or --restore");
  return options;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const options = parseArgs(process.argv.slice(2));
    if (options.action === "apply" && process.env.CLAUDE_LOCAL_MODE !== "mock") {
      throw new Error("Applying the dictation asset patch requires CLAUDE_LOCAL_MODE=mock");
    }
    const result = options.action === "apply"
      ? applyPatch(options)
      : restorePatch(options);
    console.log(`Claude local dictation asset: ${result}`);
  } catch (error) {
    console.error(`Claude local dictation asset patch refused: ${error.message}`);
    process.exitCode = 1;
  }
}
