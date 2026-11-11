#!/usr/bin/env node
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const assetRoot = "https://assets-proxy.anthropic.com/claude-ai/v2/assets/v1/";
const patchMarker = "/* codex-local-dictation-start-v1 */";
const resumeOriginal = 'r.state==="suspended"&&await r.resume()';
const resumeReplacement = 'r.state==="suspended"&&await __codexLocalDictationResume(r)';
const pendingOriginal = "XS(t);let a=++sk;try{WM();";
const pendingReplacement = 'XS(t);__codexLocalDictationNote("pending");let a=++sk;try{WM();';
const loadOriginal = "c=await Tl()";
const loadReplacement = "c=await __codexLocalDictationLoad(()=>Tl())";
const startOriginal = "await c.dictationSession.start(...e);let u=fD.getState()";
const startReplacement = "if(!await __codexLocalDictationStart(c.dictationSession,e,()=>a===sk&&eu))return;let u=fD.getState()";

const resumeRuntime = `
${patchMarker}
async function __codexLocalDictationResume(context){
  let timer;
  try{
    await Promise.race([
      Promise.resolve().then(()=>context.resume()),
      new Promise((_,reject)=>{timer=setTimeout(()=>{
        const error=new Error("Dictation audio context did not start in time");
        error.name="DictationAudioContextTimeout";
        reject(error);
      },5000);})
    ]);
  }catch(error){
    try{Promise.resolve(context.close()).catch(()=>{});}catch{}
    throw error;
  }finally{clearTimeout(timer);}
}
`;

const facadeRuntime = `
${patchMarker}
let __codexLocalDictationLastPhase=null;
function __codexLocalDictationNote(phase){
  const state=fD.getState();
  const snapshot={
    phase,status:state.status,waiting:state.startIsWaiting===true,
    draining:state.drainPending===true,finishing:state.finishingOwnerId!==null,
    connected:state.connectedAt!==null,at:Math.round(performance.now())
  };
  globalThis.__codexPlusLocalDictationStatus=snapshot;
  if(phase!==__codexLocalDictationLastPhase){
    __codexLocalDictationLastPhase=phase;
    console.info("[Claude Local Dictation]",snapshot);
  }
}
fD.subscribe(state=>{
  __codexLocalDictationNote(
    state.status==="error"?"error":
    state.status==="connecting"?(state.startIsWaiting?"pending":"connecting"):
    state.status==="recording"?"recording":
    state.finishingOwnerId!==null||state.drainPending?"finishing":"idle"
  );
});
async function __codexLocalDictationLoad(load){
  let timer;
  try{
    return await Promise.race([
      Promise.resolve().then(load),
      new Promise((_,reject)=>{timer=setTimeout(()=>{
        const error=new Error("Dictation machinery did not load in time");
        error.name="DictationLoadTimeout";
        reject(error);
      },15000);})
    ]);
  }finally{clearTimeout(timer);}
}
function __codexLocalDictationFailStart(session,reason){
  try{session.cancel(reason);}catch{}
  fD.getState().setError("无法启动听写，请重试。");
}
async function __codexLocalDictationStart(session,args,isCurrent){
  let timer;
  try{
    return await Promise.race([
      Promise.resolve().then(()=>session.start(...args)).then(()=>true),
      new Promise(resolve=>{timer=setTimeout(()=>{
        if(isCurrent()){
          __codexLocalDictationFailStart(session,"local_start_timeout");
        }
        resolve(false);
      },20000);})
    ]);
  }catch{
    if(isCurrent()){
      __codexLocalDictationFailStart(session,"local_start_error");
    }
    return false;
  }finally{clearTimeout(timer);}
}
`;

function count(source, value) {
  return source.split(value).length - 1;
}

function patchBundle(input, { anchors, replacements, runtime }) {
  const source = Buffer.isBuffer(input) ? input.toString("utf8") : String(input);
  if (anchors.some((anchor) => count(source, anchor) !== 1)) {
    throw new Error("Dictation startup asset does not match the expected client contract");
  }
  if (source.includes(patchMarker)) {
    if (
      count(source, patchMarker) !== 1 ||
      replacements.some(([original, replacement]) => count(source, original) !== 0 || count(source, replacement) !== 1) ||
      !source.endsWith(runtime)
    ) throw new Error("Dictation startup asset contains an unexpected existing patch");
    return Buffer.from(source);
  }
  let patched = source;
  for (const [original, replacement] of replacements) {
    if (count(patched, original) !== 1) throw new Error("Dictation startup asset has an ambiguous patch location");
    patched = patched.replace(original, replacement);
  }
  return Buffer.from(patched + runtime);
}

export function patchRecorderBundle(input) {
  return patchBundle(input, {
    anchors: ['async startWithStream(', "setupPcmCapture(){", 'this.scriptProcessorNode.onaudioprocess=e=>{'],
    replacements: [[resumeOriginal, resumeReplacement]],
    runtime: resumeRuntime,
  });
}

export function patchFacadeBundle(input) {
  return patchBundle(input, {
    anchors: [
      "async start(...e){let t=e[2],n=fD.getState()",
      'context:"voice machinery chunk load failed on dictation start()"',
      "a===sk&&(XS(null),fD.getState().startIsWaiting&&Vf.stop())",
    ],
    replacements: [
      [pendingOriginal, pendingReplacement],
      [loadOriginal, loadReplacement],
      [startOriginal, startReplacement],
    ],
    runtime: facadeRuntime,
  });
}

export const startupAssets = [
  { name: "cca745a7a-BZDG7lE7.js", patch: patchRecorderBundle },
  { name: "shared-33-DE0321kK.js", patch: patchFacadeBundle },
].map((asset) => ({
  ...asset,
  url: `${assetRoot}${asset.name}`,
  pathname: `/claude-ai/v2/assets/v1/${asset.name}`,
}));

export function isPatchedStartupAsset(pathname, input) {
  const asset = startupAssets.find((candidate) => candidate.pathname === pathname);
  if (!asset) return false;
  const body = Buffer.isBuffer(input) ? input : Buffer.from(input);
  if (!body.includes(Buffer.from(patchMarker))) return false;
  try {
    return asset.patch(body).equals(body);
  } catch {
    return false;
  }
}

function pathsFor(cacheDir, asset) {
  const key = asset.url.replace(/^https:\/\//, "");
  const name = crypto.createHash("sha1").update(key).digest("hex");
  const backupDir = path.join(cacheDir, ".dictation-start-backup");
  return {
    key, body: path.join(cacheDir, name), meta: path.join(cacheDir, `${name}.json`),
    backupDir, backupBody: path.join(backupDir, name), backupMeta: path.join(backupDir, `${name}.json`),
  };
}

function validMeta(meta, key) {
  return meta?.status === 200 && meta.url === key &&
    typeof meta.headers?.["content-type"] === "string" && !meta.headers?.["content-encoding"];
}

function readAssetCache(bodyPath, metaPath, key) {
  if (!fs.existsSync(bodyPath) || !fs.existsSync(metaPath)) throw new Error("Dictation startup asset cache is incomplete");
  const meta = JSON.parse(fs.readFileSync(metaPath, "utf8"));
  if (!validMeta(meta, key)) throw new Error("Dictation startup asset cache metadata does not match the target URL");
  return { body: fs.readFileSync(bodyPath), meta };
}

const simpleCacheMagic = Buffer.from("305c72a71b6dfbfc", "hex");
const simpleCacheEofMagic = Buffer.from("d8410d97456ffaf4", "hex");
const simpleCacheHeaderBytes = 24;
const simpleCacheEofBytes = 24;

export function extractElectronStartupAsset(bytes, asset) {
  if (bytes.length < simpleCacheHeaderBytes || !bytes.subarray(0, 8).equals(simpleCacheMagic)) return null;
  const keyLength = bytes.readUInt32LE(12);
  const start = simpleCacheHeaderBytes + keyLength;
  if (start > bytes.length) return null;
  // URL 也会出现在其他页面正文里，只认 Simple Cache 文件头声明的完整键。
  if (!bytes.subarray(simpleCacheHeaderBytes, start).equals(Buffer.from(`1/0/${asset.url}`))) return null;
  if (bytes.readUInt32LE(8) !== 5) throw new Error("Unsupported dictation startup Electron cache version");
  const footer = bytes.indexOf(simpleCacheEofMagic, start);
  const header = footer < 0 ? -1 : bytes.indexOf(Buffer.from("HTTP/1.1 200 OK\0"), footer + simpleCacheEofBytes);
  const length = header < 0 ? null : bytes.subarray(header).toString("latin1").match(/\0content-length: (\d+)\0/);
  if (!length || !Number.isSafeInteger(Number(length[1])) || start + Number(length[1]) !== footer) {
    throw new Error("Dictation startup asset has invalid Electron cache body bounds");
  }
  return bytes.subarray(start, footer);
}

function readElectronAsset(cacheDir, asset) {
  const matches = [];
  for (const entry of fs.readdirSync(cacheDir, { withFileTypes: true })) {
    if (!entry.isFile() || !entry.name.endsWith("_0")) continue;
    const bytes = fs.readFileSync(path.join(cacheDir, entry.name));
    const body = extractElectronStartupAsset(bytes, asset);
    if (!body) continue;
    asset.patch(body);
    matches.push(body);
  }
  if (!matches.length) throw new Error(`Expected dictation startup asset is missing: ${asset.name}`);
  if (matches.some((body) => !body.equals(matches[0]))) throw new Error("Multiple different dictation startup assets are cached");
  return {
    body: matches[0],
    meta: { status: 200, url: asset.url.replace(/^https:\/\//, ""), headers: { "content-type": "text/javascript; charset=utf-8" } },
  };
}

function writeAsset(bodyPath, metaPath, { body, meta }) {
  fs.writeFileSync(bodyPath, body, { mode: 0o600 });
  fs.writeFileSync(metaPath, `${JSON.stringify(meta)}\n`, { mode: 0o600 });
}

const defaultElectronCache = path.join(os.homedir(), "Library", "Application Support", "Claude", "Cache", "Cache_Data");

export function applyStartupPatches({ cacheDir = path.join(here, ".cache"), electronCacheDir = defaultElectronCache } = {}) {
  // 先验证全部资源与备份，再写入，避免版本不匹配时留下部分补丁。
  const prepared = startupAssets.map((asset) => {
    const paths = pathsFor(cacheDir, asset);
    const current = fs.existsSync(paths.body) || fs.existsSync(paths.meta)
      ? readAssetCache(paths.body, paths.meta, paths.key)
      : readElectronAsset(electronCacheDir, asset);
    const patched = asset.patch(current.body);
    const hasBackup = fs.existsSync(paths.backupBody) || fs.existsSync(paths.backupMeta);
    const original = hasBackup ? readAssetCache(paths.backupBody, paths.backupMeta, paths.key) : current;
    if (original.body.includes(Buffer.from(patchMarker))) throw new Error("Dictation startup asset has no original reversible backup");
    if (!asset.patch(original.body).equals(patched)) throw new Error("Dictation startup asset backup does not match the local asset");
    return { asset, paths, original, patched, hasBackup, unchanged: current.body.equals(patched) };
  });
  for (const { paths, original, patched, hasBackup } of prepared) {
    fs.mkdirSync(paths.backupDir, { recursive: true, mode: 0o700 });
    if (!hasBackup) writeAsset(paths.backupBody, paths.backupMeta, original);
    writeAsset(paths.body, paths.meta, { body: patched, meta: original.meta });
  }
  return prepared.map(({ asset, unchanged }) => ({ asset: asset.name, status: unchanged ? "already-patched" : "patched" }));
}

export function restoreStartupPatches({ cacheDir = path.join(here, ".cache") } = {}) {
  const prepared = startupAssets.map((asset) => {
    const paths = pathsFor(cacheDir, asset);
    const original = readAssetCache(paths.backupBody, paths.backupMeta, paths.key);
    if (original.body.includes(Buffer.from(patchMarker))) throw new Error("Saved dictation startup asset is not the original version");
    asset.patch(original.body);
    return { paths, original };
  });
  for (const { paths, original } of prepared) writeAsset(paths.body, paths.meta, original);
  return "restored";
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const args = process.argv.slice(2);
    if (args.length !== 1 || !["--apply", "--restore"].includes(args[0])) throw new Error("Choose --apply or --restore");
    if (args[0] === "--apply" && process.env.CLAUDE_LOCAL_MODE !== "mock") {
      throw new Error("Applying dictation startup patches requires CLAUDE_LOCAL_MODE=mock");
    }
    const result = args[0] === "--apply" ? applyStartupPatches() : restoreStartupPatches();
    console.log(`Claude local dictation startup assets: ${JSON.stringify(result)}`);
  } catch (error) {
    console.error(`Claude local dictation startup patch refused: ${error.message}`);
    process.exitCode = 1;
  }
}
