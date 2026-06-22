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
const SERVICE_TIER_SETTINGS_ASSET_RE = /^use-service-tier-settings-.*\.js$/;
const READ_SERVICE_TIER_ASSET_RE = /^read-service-tier-for-request-.*\.js$/;
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
  patched = replaceFirstOf(
    patched,
    [
      [
        "s=o?.authMethod===`chatgpt`",
        "s=o?.authMethod===`chatgpt`||o?.authMethod===`apikey`",
      ],
      [
        "c=o?.authMethod===`chatgpt`",
        "c=o?.authMethod===`chatgpt`||o?.authMethod===`apikey`",
      ],
    ],
    "service tier settings auth gate"
  );
  patched = replaceFirstOf(
    patched,
    [
      ["s&&!f&&u!=null", "s&&!f"],
      ["c&&!p&&d!=null", "c&&!p"],
    ],
    "service tier settings API key config requirement"
  );
  return patched;
}

function patchReadServiceTierAsset(source) {
  let patched = source;
  patched = replaceFirstOf(
    patched,
    [
      [
        "return n===`chatgpt`?(await e.query.fetch(c,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:!1",
        "return n===`chatgpt`?(await e.query.fetch(c,{authMethod:n,hostId:t})).requirements?.featureRequirements?.fast_mode!==!1:n===`apikey`",
      ],
    ],
    "read service tier auth gate"
  );
  patched = replaceFirstOf(
    patched,
    [
      [
        "return d.service_tier==null?t(await m(o,c??d.model),d.service_tier,s):t(null,d.service_tier,s)",
        "return d.service_tier==null?t(await m(o,c??d.model),d.service_tier,s):t(await m(o,c??d.model),d.service_tier,s)",
      ],
      [
        "return d.service_tier==null?i(await m(o,c??d.model),d.service_tier,s):i(null,d.service_tier,s)",
        "return d.service_tier==null?i(await m(o,c??d.model),d.service_tier,s):i(await m(o,c??d.model),d.service_tier,s)",
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

function replaceFirstOf(source, replacements, label) {
  for (const [from, to] of replacements) {
    if (source.includes(from)) return source.replace(from, to);
  }
  for (const [, to] of replacements) {
    if (source.includes(to)) return source;
  }
  throw new Error(`${label} pattern not found`);
}

function tryPatchServiceTierAsset(kind, source, url) {
  try {
    return {
      ok: true,
      source: patchServiceTierAsset(kind, source),
    };
  } catch (error) {
    log("service_tier_preload_asset_patch_failed", {
      kind,
      url,
      message: String(error),
      version: PATCH_VERSION,
    });
    return {
      ok: false,
      source,
    };
  }
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

function serviceTierAssetKindFromUrl(url) {
  const name = appProtocolAssetName(url);
  if (SERVICE_TIER_SETTINGS_ASSET_RE.test(name)) return "native-service-tier-settings";
  if (READ_SERVICE_TIER_ASSET_RE.test(name)) return "native-read-service-tier";
  return "";
}

function patchServiceTierAsset(kind, source) {
  if (kind === "native-service-tier-settings") return patchServiceTierSettingsAsset(source);
  if (kind === "native-read-service-tier") return patchReadServiceTierAsset(source);
  return source;
}

function responseHeadersWithPatchMark(response, source) {
  const headers = new Headers(response && response.headers ? response.headers : undefined);
  headers.set("Content-Length", String(Buffer.byteLength(source, "utf8")));
  headers.set("Content-Type", "text/javascript; charset=utf-8");
  headers.set("X-Codex-Plus-Patch", PATCH_VERSION);
  return headers;
}

async function patchedAssetResponse(kind, request, handler, self) {
  const response = await handler.call(self, request);
  const source = await response.text();
  const result = tryPatchServiceTierAsset(kind, source, request && request.url);
  if (result.ok) {
    log("service_tier_preload_asset_patched", { kind, url: request && request.url, version: PATCH_VERSION });
  }
  return new Response(Buffer.from(result.source, "utf8"), {
    status: response.status,
    statusText: response.statusText,
    headers: result.ok ? responseHeadersWithPatchMark(response, result.source) : response.headers,
  });
}

function shouldInstallProtocolPatch() {
  if (!serviceTierControlsEnabled()) {
    log("service_tier_preload_disabled_by_settings", {});
    return false;
  }
  return true;
}

function installProtocolHandlePatch(electron) {
  const protocol = electron && electron.protocol;
  if (!protocol || typeof protocol.handle !== "function") {
    log("service_tier_preload_protocol_unavailable", {});
    return;
  }
  if (protocol.handle[PATCH_MARK] === PATCH_VERSION) return;
  if (!shouldInstallProtocolPatch()) return;

  const originalHandle = protocol.handle;
  const wrappedHandle = function codexPlusServiceTierProtocolHandle(scheme, handler) {
    if (String(scheme) !== "app" || typeof handler !== "function") {
      return originalHandle.apply(this, arguments);
    }
    const wrappedHandler = async function codexPlusServiceTierAppProtocolHandler(request) {
      const kind = serviceTierAssetKindFromUrl(request && request.url);
      if (!kind) return handler.call(this, request);
      return patchedAssetResponse(kind, request, handler, this);
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
        assert!(script.contains("patchedAssetResponse"));
        assert!(script.contains("tryPatchServiceTierAsset"));
        assert!(script.contains("service_tier_preload_asset_patch_failed"));
        assert!(script.contains("handler.call(self, request)"));
        assert!(!script.contains("app.asar\", \"webview\", \"assets"));
        assert!(script.contains("use-service-tier-settings-"));
        assert!(script.contains("read-service-tier-for-request-"));
        assert!(script.contains("s=o?.authMethod===`chatgpt`||o?.authMethod===`apikey`"));
        assert!(script.contains("n===`apikey`"));
    }
}
