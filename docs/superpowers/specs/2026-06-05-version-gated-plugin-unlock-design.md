# Version-Gated Plugin Unlock Strategy Design

**Goal:** Automatically choose the old or new plugin unlock strategy based on the installed Codex App version, while preserving manual switches for plugin marketplace unlock, force entry unlock, and force install/load.

**Architecture:** Reuse backend Codex App version detection and expose the detected version to the injected runtime settings payload. The injection script compares the Codex App version against a fixed cutoff (`26.601.21317`) and chooses a legacy or modern plugin unlock strategy at scan time.

**Tech Stack:** Rust backend settings/launcher helper routes, Codex++ runtime JavaScript injection, static injection tests in `codex-plus-core`, React/Tauri manager settings UI only if a status hint is added.

---

## Requirements

1. Use Codex App version to select plugin unlock strategy.
   - If Codex App version is lower than `26.601.21317`, prefer the legacy 1.1.9 plugin entry unlock path.
   - If Codex App version is greater than or equal to `26.601.21317`, prefer the current modern marketplace unlock path.
   - If Codex App version cannot be read or parsed, preserve current manual-switch behavior.

2. Preserve existing manual switches.
   - `插件市场解锁` controls the modern marketplace request/filter patch.
   - `强制解锁入口` controls the restored legacy plugin entry unlock path.
   - `特殊插件强制安装` controls disabled install button unblocking.
   - All three remain visible and independently configurable.

3. Preserve existing safety gates.
   - If `enhancementsEnabled` is false, plugin unlock work does not run.
   - If launch mode is `relay`, plugin unlock work stays disabled.
   - No `app.asar` patching.

4. Do not persist runtime-only Codex App version into settings JSON.
   - Version is read dynamically from the configured/discovered Codex App path.
   - The injected settings response may include `codexAppVersion`, but saving settings must not write that field.

## Version Cutoff

Use this fixed cutoff:

```text
PLUGIN_LEGACY_ENTRY_UNLOCK_BEFORE = 26.601.21317
```

Rules:

| Codex App version | Strategy | Main behavior |
| --- | --- | --- |
| `< 26.601.21317` | `legacy` | Run legacy `enablePluginEntry()` when entry unlock is enabled. |
| `>= 26.601.21317` | `modern` | Run modern marketplace request/filter patches when marketplace unlock is enabled. |
| unknown/unparseable | `unknown` | Keep current manual-switch behavior: run enabled marketplace and entry paths independently. |

## Backend Data Flow

The launcher helper currently serves `/settings/get` to the injected script. Extend that response with a runtime-only field:

```json
{
  "codexAppVersion": "26.601.21317"
}
```

If the version cannot be read:

```json
{
  "codexAppVersion": ""
}
```

Implementation options for locating the app version:

1. Use the configured Codex App path from settings if present.
2. Fall back to the same discovery path used by `load_overview`.
3. Use `codex_plus_core::app_paths::codex_app_version(path)` to parse the version.

The field is merged into the helper response only. `BackendSettings` does not need to persist `codexAppVersion` unless the existing route implementation requires serializing the full settings struct; in that case, use a separate response object instead of adding a serializable persisted field.

## Injection Script Strategy

Add constants and helpers to `assets/inject/renderer-inject.js`:

```js
const codexPluginLegacyEntryUnlockBeforeVersion = "26.601.21317";

function parseCodexVersionParts(version) { ... }
function compareCodexVersions(left, right) { ... }
function codexPluginUnlockStrategy() { ... }
```

Expected behavior:

```js
function codexPluginUnlockStrategy() {
  const version = String(codexPlusBackendSettings.codexAppVersion || "").trim();
  const comparison = compareCodexVersions(version, codexPluginLegacyEntryUnlockBeforeVersion);
  if (comparison == null) return "unknown";
  return comparison < 0 ? "legacy" : "modern";
}
```

Scan behavior in the non-relay branch:

```js
const strategy = codexPluginUnlockStrategy();
const settings = codexPlusSettings();

if ((strategy === "legacy" || strategy === "unknown") && settings.pluginEntryUnlock) {
  enablePluginEntry();
}

if ((strategy === "modern" || strategy === "unknown") && settings.pluginMarketplaceUnlock) {
  installPluginBuildFlavorFilterPatch();
  installPluginMarketplaceRequestPatch();
}

unblockPluginInstallButtons();
refreshForcePluginInstallUnlockLoop();
```

This means:

- Low versions do not automatically run the modern marketplace patch just because the marketplace switch is on.
- New versions do not automatically run the legacy entry spoof just because the entry switch defaults to on.
- Unknown versions keep the current behavior: both enabled plugin layers can run.

## UI/Diagnostics

Add lightweight diagnostics so runtime behavior is debuggable:

- `plugin_unlock_strategy_selected`
  - `strategy`
  - `codexAppVersion`
  - `cutoff`

Avoid logging this on every scan if it becomes noisy; either log only when the strategy changes or store the last logged strategy in `window.__codexPluginUnlockStrategyLogged`.

Optional UI hint in the injected Codex++ menu:

- Legacy: `检测到旧版 Codex App，自动优先使用旧入口解锁。`
- Modern: `检测到新版 Codex App，自动优先使用插件市场解锁。`
- Unknown: `未读取到 Codex App 版本，按手动开关执行。`

The hint is helpful but not required for the first implementation if tests cover the behavior.

## Testing Plan

1. Backend route tests:
   - `/settings/get` response contains `codexAppVersion` when a Codex App version is discoverable.
   - Saving settings does not persist `codexAppVersion`.
   - If version discovery fails, response contains an empty string rather than failing the route.

2. Injection static tests:
   - Script contains cutoff `26.601.21317`.
   - Script contains `compareCodexVersions` and `codexPluginUnlockStrategy`.
   - Legacy strategy gates `enablePluginEntry()`.
   - Modern strategy gates `installPluginBuildFlavorFilterPatch()` and `installPluginMarketplaceRequestPatch()`.
   - Unknown strategy allows both enabled paths.

3. Existing regression tests:
   - Three plugin switches default to true.
   - Legacy entry unlock code remains present.
   - Modern marketplace unlock code remains present.
   - Relay mode still skips plugin patch work.

## Out of Scope

- Do not remove any of the three plugin switches.
- Do not change `forcePluginInstall` behavior by version.
- Do not patch `app.asar`.
- Do not change provider/session sync behavior.
- Do not create a new release tag unless explicitly requested after implementation.
