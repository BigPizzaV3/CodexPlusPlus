import fs from "node:fs";
import path from "node:path";

const MAX_ARCHIVE_BYTES = 2 * 1024 * 1024 * 1024;
const MAX_HEADER_BYTES = 64 * 1024 * 1024;
const MAX_RESOURCE_BYTES = 64 * 1024 * 1024;
const PUBLIC_ROOTS = ["assets/", "audio/", "i18n/", "images/"];
const PUBLIC_FILES = new Set([
  "client-shell-v1.html",
  "favicon-internal.ico",
  "favicon-staging.ico",
  "favicon.ico",
  "frame-connect.html",
  "frame-manage.html",
  "frame-shell.html",
  "index.html",
  "robots.txt",
  "scc.json",
  "third-party-notices.txt",
]);
const CONTENT_TYPES = new Map([
  [".css", "text/css; charset=utf-8"],
  [".gif", "image/gif"],
  [".html", "text/html; charset=utf-8"],
  [".ico", "image/x-icon"],
  [".jpeg", "image/jpeg"],
  [".jpg", "image/jpeg"],
  [".js", "text/javascript; charset=utf-8"],
  [".json", "application/json; charset=utf-8"],
  [".m4a", "audio/mp4"],
  [".mp3", "audio/mpeg"],
  [".mp4", "video/mp4"],
  [".ogg", "audio/ogg"],
  [".png", "image/png"],
  [".svg", "image/svg+xml"],
  [".txt", "text/plain; charset=utf-8"],
  [".wasm", "application/wasm"],
  [".wav", "audio/wav"],
  [".webp", "image/webp"],
  [".woff", "font/woff"],
  [".woff2", "font/woff2"],
]);

function readExactly(fd, length, position) {
  const buffer = Buffer.alloc(length);
  let read = 0;
  while (read < length) {
    const count = fs.readSync(fd, buffer, read, length - read, position + read);
    if (count === 0) throw new Error("Unexpected end of Claude Desktop ASAR");
    read += count;
  }
  return buffer;
}

function readAsar(resourcesDir) {
  const realResources = fs.realpathSync(resourcesDir);
  if (!fs.statSync(realResources).isDirectory()) throw new Error("Claude Desktop resources path is not a directory");
  const archivePath = path.join(realResources, "ion-dist.asar");
  const metadata = fs.lstatSync(archivePath);
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size < 24 || metadata.size > MAX_ARCHIVE_BYTES) {
    throw new Error("Claude Desktop ion-dist.asar is not a supported regular file");
  }

  const fd = fs.openSync(archivePath, fs.constants.O_RDONLY | (fs.constants.O_NOFOLLOW ?? 0));
  try {
    const stat = fs.fstatSync(fd);
    if (!stat.isFile() || stat.size !== metadata.size) throw new Error("Claude Desktop ion-dist.asar changed while opening");
    const prefix = readExactly(fd, 16, 0);
    const headerSize = prefix.readUInt32LE(4);
    const jsonSize = prefix.readUInt32LE(12);
    const payloadStart = 8 + headerSize;
    if (
      headerSize < 8 ||
      headerSize > MAX_HEADER_BYTES ||
      payloadStart > stat.size ||
      jsonSize < 2 ||
      jsonSize > headerSize - 8 ||
      16 + jsonSize > payloadStart
    ) {
      throw new Error("Claude Desktop ion-dist.asar has an invalid header");
    }

    const header = JSON.parse(readExactly(fd, jsonSize, 16).toString("utf8"));
    if (!header || typeof header !== "object" || !header.files || typeof header.files !== "object") {
      throw new Error("Claude Desktop ion-dist.asar has no file table");
    }

    const entries = new Map();
    const walk = (directory, parent = "") => {
      for (const [name, entry] of Object.entries(directory)) {
        if (!name || name === "." || name === ".." || /[/\\\0]/.test(name) ||
          !entry || typeof entry !== "object" || Array.isArray(entry)) {
          throw new Error("Claude Desktop ion-dist.asar has an invalid path entry");
        }
        const member = parent ? `${parent}/${name}` : name;
        if (entry.files !== undefined) {
          if (!entry.files || typeof entry.files !== "object" || Array.isArray(entry.files)) {
            throw new Error("Claude Desktop ion-dist.asar has an invalid directory entry");
          }
          walk(entry.files, member);
          continue;
        }
        if (entry.link !== undefined || entry.unpacked === true ||
          !Number.isSafeInteger(entry.size) || entry.size < 0 ||
          typeof entry.offset !== "string" || !/^(0|[1-9][0-9]*)$/.test(entry.offset)) {
          throw new Error("Claude Desktop ion-dist.asar has an unsupported file entry");
        }
        const offset = Number(entry.offset);
        if (!Number.isSafeInteger(offset) || offset + entry.size > stat.size - payloadStart) {
          throw new Error("Claude Desktop ion-dist.asar has an out-of-bounds file entry");
        }
        entries.set(member, { offset: payloadStart + offset, size: entry.size });
      }
    };
    walk(header.files);

    const index = entries.get("index.html");
    if (!index || index.size > MAX_RESOURCE_BYTES) throw new Error("Claude Desktop ion-dist.asar has no usable web entry");
    const indexHtml = readExactly(fd, index.size, index.offset).toString("utf8");
    const buildId = indexHtml.match(/<html\b[^>]*\bdata-build-id="([^"]+)"/i)?.[1];
    if (!buildId || buildId.length > 128) throw new Error("Claude Desktop web entry has no supported build identifier");
    const i18nLocales = [...entries.keys()]
      .map((member) => member.match(/^i18n\/([a-zA-Z]{2,3}(?:-[a-zA-Z0-9]{2,8})*)\.json$/)?.[1])
      .filter(Boolean)
      .sort();
    return { fd, entries, indexHtml, buildId, archivePath, i18nLocales };
  } catch (error) {
    fs.closeSync(fd);
    throw error;
  }
}

function normalizeMember(pathname) {
  if (typeof pathname !== "string" || !pathname.startsWith("/") || pathname.includes("\0")) return null;
  let decoded;
  try {
    decoded = decodeURIComponent(pathname);
  } catch {
    return null;
  }
  if (decoded !== pathname || decoded.includes("\\") || decoded.includes("\0")) return null;
  const parts = decoded.split("/");
  if (parts[0] !== "" || parts.slice(1).some((part) => !part || part === "." || part === "..")) return null;
  const member = parts.slice(1).join("/");
  if (!PUBLIC_FILES.has(member) && !PUBLIC_ROOTS.some((root) => member.startsWith(root))) return null;
  return member;
}

export function createOfficialAssets(resourcesDir) {
  if (!resourcesDir) return null;
  const archive = readAsar(resourcesDir);
  let closed = false;
  return {
    archivePath: archive.archivePath,
    buildId: archive.buildId,
    indexHtml: archive.indexHtml,
    i18nLocales: archive.i18nLocales,
    read(pathname) {
      if (closed) return null;
      const member = normalizeMember(pathname);
      if (!member) return null;
      const entry = archive.entries.get(member);
      if (!entry || entry.size > MAX_RESOURCE_BYTES) return null;
      return {
        body: readExactly(archive.fd, entry.size, entry.offset),
        contentType: CONTENT_TYPES.get(path.extname(member).toLowerCase()) ?? "application/octet-stream",
        isHtml: member.endsWith(".html"),
        member,
      };
    },
    close() {
      if (closed) return;
      closed = true;
      fs.closeSync(archive.fd);
    },
  };
}
