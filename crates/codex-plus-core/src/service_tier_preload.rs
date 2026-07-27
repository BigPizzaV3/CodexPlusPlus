use std::fs;
use std::path::PathBuf;

use anyhow::Context;

const PRELOAD_FILE: &str = "service-tier-preload.js";

pub fn ensure_service_tier_preload() -> anyhow::Result<PathBuf> {
    let dir = crate::paths::default_app_state_dir().join("preload");
    fs::create_dir_all(&dir).with_context(|| {
        format!(
            "failed to create service tier preload directory {}",
            dir.display()
        )
    })?;
    let path = dir.join(PRELOAD_FILE);
    fs::write(&path, service_tier_preload_script()).with_context(|| {
        format!(
            "failed to write service tier preload script {}",
            path.display()
        )
    })?;
    Ok(path)
}

pub fn node_options_with_service_tier_preload(
    existing: Option<&str>,
    preload_path: &str,
) -> String {
    let require_arg = format!("--require={preload_path}");
    match existing.map(str::trim).filter(|value| !value.is_empty()) {
        Some(existing) if existing.contains(&require_arg) => existing.to_string(),
        Some(existing) => format!("{require_arg} {existing}"),
        None => require_arg,
    }
}

pub fn service_tier_preload_script() -> &'static str {
    r#""use strict";

const fs = require("fs");
const path = require("path");
const Module = require("module");

const PATCH_MARK = Symbol.for("codex-plus.service-tier-protocol-handle-patched");
const PATCH_VERSION = "protocol-handle-3";
const LOG_PATH = path.join(process.env.HOME || process.cwd(), ".codex-session-delete", "codex-plus.log");
const SETTINGS_PATH = path.join(process.env.HOME || process.cwd(), ".codex-session-delete", "settings.json");

function log(event, detail) {
  try {
    fs.mkdirSync(path.dirname(LOG_PATH), { recursive: true });
    fs.appendFileSync(LOG_PATH, JSON.stringify({
      timestamp_ms: Date.now(),
      pid: process.pid,
      event,
      detail: detail || {},
    }) + "\n");
  } catch {}
}

function patchServiceTierSettingsAsset(source) {
  let patched = source;
  if (!isServiceTierSettingsCandidate(source)) {
    return source;
  }

  patched = replaceOneOf(
    patched,
    [
      [
        "s=o?.authMethod===`chatgpt`",
        "s=o?.authMethod===`chatgpt`||o?.authMethod===`apikey`",
      ],
      [
        "a=i?.authMethod===`chatgpt`",
        "a=i?.authMethod===`chatgpt`||i?.authMethod===`apikey`",
      ],
    ],
    "service tier settings auth gate"
  );
  patched = replaceOneOf(
    patched,
    [
      ["s&&!f&&u!=null", "s&&!f"],
      ["d=a&&!u&&c!=null&&c?.requirements?.featureRequirements?.fast_mode!==!1", "d=a&&!u&&c?.requirements?.featureRequirements?.fast_mode!==!1"],
    ],
    "service tier settings API key config requirement"
  );
  return patched;
}

function patchReadServiceTierAsset(source) {
  let patched = source;
  if (!isReadServiceTierCandidate(source)) {
    return source;
  }

  patched = replaceOneOf(
    patched,
    [
      [
        "return n===`chatgpt`?(await e.query.fetch(c,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(c,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
      [
        "return n===`chatgpt`?(await e.query.fetch(li,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(li,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
      [
        "return n===`chatgpt`?(await e.query.fetch(Bu,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(Bu,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
      [
        "return n===`chatgpt`?(await e.query.fetch(Gd,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(Gd,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
      [
        "return n===`chatgpt`?(await e.query.fetch(wu,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(wu,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
      [
        /return ([A-Za-z_$][\w$]*)===`chatgpt`\?\(await ([A-Za-z_$][\w$]*)\.query\.fetch\(([A-Za-z_$][\w$]*),\{authMethod:\1,hostId:([A-Za-z_$][\w$]*)\}\)\)\.requirements\?\.featureRequirements\?\.fast_mode!==!1:!1/,
        "return $1===`chatgpt`?(await $2.query.fetch($3,{authMethod:$1,hostId:$4})).requirements?.featureRequirements?.fast_mode!==!1:$1===`apikey`",
      ],
      [
        "if(n!==`chatgpt`)return!1;let r=await J8n(t,{priority:`critical`});return e.query.setData($E,{authMethod:n,hostId:t},r),r.requirements?.featureRequirements?.fast_mode!==!1",
        "if(n!==`chatgpt`)return n===`apikey`;let r=await J8n(t,{priority:`critical`});return e.query.setData($E,{authMethod:n,hostId:t},r),r.requirements?.featureRequirements?.fast_mode!==!1",
      ],
      [
        /if\(([A-Za-z_$][\w$]*)!==`chatgpt`\)return!1;let ([A-Za-z_$][\w$]*)=await ([A-Za-z_$][\w$]*)\(/,
        "if($1!==`chatgpt`)return $1===`apikey`;let $2=await $3(",
      ],
    ],
    "read service tier auth gate"
  );
  patched = replaceOneOf(
    patched,
    [
      [
        "return d.service_tier==null?t(await m(o,c??d.model),d.service_tier,s):t(null,d.service_tier,s)",
        "return d.service_tier==null?t(await m(o,c??d.model),d.service_tier,s):t(await m(o,c??d.model),d.service_tier,s)",
      ],
      [
        "return o.service_tier==null?Ia(await np(t,n??o.model),o.service_tier,r):Ia(null,o.service_tier,r)",
        "return o.service_tier==null?Ia(await np(t,n??o.model),o.service_tier,r):Ia(await np(t,n??o.model),o.service_tier,r)",
      ],
      [
        "return o.service_tier==null?Pc(await xAe(t,n??o.model),o.service_tier,r):Pc(null,o.service_tier,r)",
        "return o.service_tier==null?Pc(await xAe(t,n??o.model),o.service_tier,r):Pc(await xAe(t,n??o.model),o.service_tier,r)",
      ],
      [
        "return o.service_tier==null?jd(await fQe(t,n??o.model),o.service_tier,r):jd(null,o.service_tier,r)",
        "return o.service_tier==null?jd(await fQe(t,n??o.model),o.service_tier,r):jd(await fQe(t,n??o.model),o.service_tier,r)",
      ],
      [
        "return o.service_tier==null?gu(await EKe(t,n??o.model),o.service_tier,r):gu(null,o.service_tier,r)",
        "return o.service_tier==null?gu(await EKe(t,n??o.model),o.service_tier,r):gu(await EKe(t,n??o.model),o.service_tier,r)",
      ],
      [
        "return o.service_tier==null?Zer(await IWi(t,n??o.model),o.service_tier,r):Zer(null,o.service_tier,r)",
        "return o.service_tier==null?Zer(await IWi(t,n??o.model),o.service_tier,r):Zer(await IWi(t,n??o.model),o.service_tier,r)",
      ],
      [
        /return ([A-Za-z_$][\w$]*)\.service_tier==null\?([A-Za-z_$][\w$]*)\(await ([A-Za-z_$][\w$]*)\(([^)]*?\?\?\1\.model)\),\1\.service_tier,([A-Za-z_$][\w$]*)\):\2\(null,\1\.service_tier,\5\)/,
        "return $1.service_tier==null?$2(await $3($4),$1.service_tier,$5):$2(await $3($4),$1.service_tier,$5)",
      ],
    ],
    "read service tier explicit config model lookup"
  );
  return patched;
}

function replaceOnce(source, from, to, label) {
  if (source.includes(to)) return source;
  if (!source.includes(from)) throw new Error(`${label} pattern not found`);
  return source.replace(from, to);
}

function replaceOneOf(source, variants, label) {
  for (const [from, to] of variants) {
    if (typeof from === "string") {
      if (source.includes(to)) {
        return source;
      }
      if (source.includes(from)) {
        return source.replace(from, to);
      }
      continue;
    }
    if (from instanceof RegExp) {
      if (from.test(source)) {
        return source.replace(from, to);
      }
    }
  }
  throw new Error(`${label} pattern not found`);
}

function isServiceTierSettingsCandidate(source) {
  return (
    source.includes("isServiceTierAllowed") &&
    source.includes("featureRequirements?.fast_mode") &&
    source.includes("authMethod")
  );
}

function isReadServiceTierCandidate(source) {
  return (
    source.includes("Failed to read service tier for request") &&
    source.includes("featureRequirements?.fast_mode") &&
    source.includes("service_tier")
  );
}

function appProtocolAssetName(url) {
  if (typeof url !== "string") return "";
  try {
    const parsed = new URL(url);
    if (parsed.protocol !== "app:" || parsed.host !== "-") return "";
    const segments = decodeURIComponent(parsed.pathname).split("/").filter(Boolean);
    return segments.length >= 2 && segments[0] === "assets" ? segments[segments.length - 1] : "";
  } catch {
    return "";
  }
}

function serviceTierControlsEnabled() {
  try {
    const settings = JSON.parse(fs.readFileSync(SETTINGS_PATH, "utf8"));
    return settings && settings.enhancementsEnabled !== false;
  } catch {
    return true;
  }
}

function discoverPatchedAssets() {
  if (!serviceTierControlsEnabled()) {
    log("service_tier_preload_disabled_by_settings", {});
    return new Map();
  }
  const assetsDir = path.join(process.resourcesPath, "app.asar", "webview", "assets");
  const assets = new Map();
  for (const name of fs.readdirSync(assetsDir)) {
    const filePath = path.join(assetsDir, name);
    if (!name.endsWith(".js")) continue;
    const source = fs.readFileSync(filePath, "utf8");
    let patched = source;
    let kinds = [];

    const settingsPatched = patchServiceTierSettingsAsset(patched);
    if (settingsPatched !== patched) {
      patched = settingsPatched;
      kinds.push("native-service-tier-settings");
    }

    const readPatched = patchReadServiceTierAsset(patched);
    if (readPatched !== patched) {
      patched = readPatched;
      kinds.push("native-read-service-tier");
    }

    if (patched !== source) {
      assets.set(name, {
        kind: kinds.join("+"),
        patched,
      });
    }
  }
  if (assets.size === 0) throw new Error("target native speed UI assets were not found");
  return assets;
}

function installProtocolHandlePatch(electron) {
  const protocol = electron && electron.protocol;
  if (!protocol || typeof protocol.handle !== "function") {
    log("service_tier_preload_protocol_unavailable", {});
    return;
  }
  if (protocol.handle[PATCH_MARK] === PATCH_VERSION) return;

  const patchedAssets = discoverPatchedAssets();
  if (patchedAssets.size === 0) return;
  const originalHandle = protocol.handle;
  const wrappedHandle = function codexPlusServiceTierProtocolHandle(scheme, handler) {
    if (String(scheme) !== "app" || typeof handler !== "function") {
      return originalHandle.apply(this, arguments);
    }
    const wrappedHandler = async function codexPlusServiceTierAppProtocolHandler(request) {
      const asset = patchedAssets.get(appProtocolAssetName(request && request.url));
      if (!asset) return handler.call(this, request);
      log("service_tier_preload_asset_patched", { kind: asset.kind, url: request && request.url, version: PATCH_VERSION });
      return new Response(Buffer.from(asset.patched, "utf8"), {
        headers: {
          "Content-Length": String(Buffer.byteLength(asset.patched, "utf8")),
          "Content-Type": "text/javascript; charset=utf-8",
          "X-Codex-Plus-Patch": PATCH_VERSION,
        },
      });
    };
    return originalHandle.call(this, scheme, wrappedHandler);
  };

  Object.defineProperty(wrappedHandle, PATCH_MARK, {
    configurable: false,
    enumerable: false,
    value: PATCH_VERSION,
  });
  protocol.handle = wrappedHandle;
  log("service_tier_preload_protocol_patch_installed", {
    assets: Array.from(patchedAssets.keys()),
    version: PATCH_VERSION,
  });
}

const originalLoad = Module._load;
Module._load = function codexPlusServiceTierModuleLoad(request, parent, isMain) {
  const result = originalLoad.apply(this, arguments);
  if (request === "electron") {
    try {
      installProtocolHandlePatch(result);
    } catch (error) {
      log("service_tier_preload_protocol_patch_failed", { message: String(error) });
    }
  }
  return result;
};

log("service_tier_preload_loaded", { version: PATCH_VERSION });
"#
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_options_prepends_preload() {
        assert_eq!(
            node_options_with_service_tier_preload(Some("--trace-warnings"), "/tmp/preload.js"),
            "--require=/tmp/preload.js --trace-warnings"
        );
    }

    #[test]
    fn preload_script_wraps_electron_module_load_and_app_protocol() {
        let script = service_tier_preload_script();

        assert!(script.contains("Module._load"));
        assert!(script.contains("protocol.handle"));
        assert!(script.contains("serviceTierControlsEnabled"));
        assert!(script.contains("settings.enhancementsEnabled !== false"));
        assert!(script.contains("service_tier_preload_disabled_by_settings"));
        assert!(script.contains("isServiceTierSettingsCandidate"));
        assert!(script.contains("isReadServiceTierCandidate"));
        assert!(script.contains("replaceOneOf"));
        assert!(script.contains("a=i?.authMethod===`chatgpt`||i?.authMethod===`apikey`"));
        assert!(script.contains("e.query.fetch(Gd,{authMethod:n,hostId:t})"));
        assert!(script.contains("jd(await fQe(t,n??o.model),o.service_tier,r)"));
        assert!(script.contains("e.query.fetch(wu,{authMethod:n,hostId:t})"));
        assert!(script.contains("gu(await EKe(t,n??o.model),o.service_tier,r)"));
        assert!(script.contains("Zer(await IWi(t,n??o.model),o.service_tier,r)"));
        assert!(script.contains("from instanceof RegExp"));
        assert!(script.contains("n===`apikey`"));
    }
}
