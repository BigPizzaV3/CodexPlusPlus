# Plugin Entry Unlock Restoration Design

**Goal:** Restore the Codex++ 1.1.9 plugin entry unlock path while keeping the current marketplace unlock path, and expose separate switches for entry unlock, marketplace unlock, and force install/load.

**Architecture:** Split plugin-related injection into three independently controlled layers: current marketplace request/filter patching, restored legacy plugin entry unlocking, and force install button unlocking. Backend settings persist each layer independently, while the injected Codex++ menu and manager settings expose clear toggles.

**Tech Stack:** Rust settings persistence (`codex-plus-core`), Tauri manager commands/UI, React/TypeScript manager UI, and runtime JavaScript injection in `assets/inject/renderer-inject.js`.

---

## Requirements

1. Keep the current Codex++ marketplace unlock behavior.
   - The current `list-plugins` request patch must stay available.
   - The current marketplace/filter patch and marketplace display-name handling must stay available.
   - This layer is controlled by a dedicated `插件市场解锁` switch.

2. Restore the Codex++ 1.1.9 plugin entry unlock behavior.
   - Find the left navigation Plugins/插件 button.
   - Use the React fiber auth context path from 1.1.9 to temporarily set `authMethod` to `chatgpt`.
   - Force the entry button visible and enabled.
   - Patch React props `disabled` state when present.
   - Spoof auth again on capture-phase click.
   - Label the entry as unlocked, and clear that label when disabled.
   - This layer is controlled by a new `强制解锁入口` switch.

3. Restore/keep the force install/load switch as an independent plugin layer.
   - Unblock disabled plugin install buttons.
   - Keep the existing refresh loop that re-unblocks buttons after React rerenders.
   - Keep the “强制安装” label behavior.
   - This layer is controlled by the `特殊插件强制安装` / `强制加载` switch.

4. Do not lose current safeguards.
   - If global `enhancementsEnabled` is false, all three plugin layers are disabled.
   - In relay/compatibility mode, plugin-related patches remain disabled as they are today.
   - No `app.asar` patching; all behavior remains runtime injection plus backend settings.

5. Preserve upgrade compatibility.
   - Existing settings files without the new marketplace field default it to `true`.
   - Existing `codexAppPluginEntryUnlock` returns to meaning “legacy entry unlock.”
   - A new `codexAppPluginMarketplaceUnlock` field controls the current marketplace unlock layer.
   - `codexAppForcePluginInstall` remains the force install/load field.

## Current Context

Current code has these relevant pieces:

- `assets/inject/renderer-inject.js`
  - Uses `pluginMarketplaceUnlock` as the local setting name mapped to backend `codexAppPluginEntryUnlock`.
  - Contains the current marketplace request/filter patch:
    - `patchPluginMarketplaceRequestParams`
    - `installPluginBuildFlavorFilterPatch`
    - `installPluginMarketplaceRequestPatch`
  - Contains force install button code:
    - `pluginInstallCandidates`
    - `unblockPluginInstallButtons`
    - `refreshForcePluginInstallUnlockLoop`
  - No longer contains the 1.1.9 `enablePluginEntry` path.

- `v1.1.9:assets/inject/renderer-inject.js`
  - Used local setting `pluginEntryUnlock` mapped to backend `codexAppPluginEntryUnlock`.
  - Restored functions should include:
    - `reactFiberFrom`
    - `authContextValueFrom`
    - `spoofChatGPTAuthMethod`
    - `pluginEntryButton`
    - `labelUnlockedPluginEntry`
    - `clearPluginEntryUnlockLabel`
    - `enablePluginEntry`

- `crates/codex-plus-core/src/settings.rs`
  - Already has `codex_app_plugin_entry_unlock` and `codex_app_force_plugin_install`.
  - Needs `codex_app_plugin_marketplace_unlock`.

## Proposed Data Model

Add a backend setting field:

```rust
#[serde(rename = "codexAppPluginMarketplaceUnlock", default = "default_true")]
pub codex_app_plugin_marketplace_unlock: bool,
```

Defaults:

```rust
codex_app_plugin_entry_unlock: true,
codex_app_plugin_marketplace_unlock: true,
codex_app_force_plugin_install: true,
```

JavaScript local settings:

```js
{
  pluginEntryUnlock: true,
  pluginMarketplaceUnlock: true,
  forcePluginInstall: true,
}
```

Backend map:

```js
{
  pluginEntryUnlock: "codexAppPluginEntryUnlock",
  pluginMarketplaceUnlock: "codexAppPluginMarketplaceUnlock",
  forcePluginInstall: "codexAppForcePluginInstall",
}
```

## Runtime Injection Flow

During `scanDeferred()`:

1. If plugin patches are disabled by relay mode:
   - call `clearPluginPatchArtifacts()`;
   - refresh/stop the force install loop;
   - skip all plugin patch installs.

2. Otherwise:
   - `enablePluginEntry()` runs only when `pluginEntryUnlock` is enabled.
   - `installPluginBuildFlavorFilterPatch()` and `installPluginMarketplaceRequestPatch()` run only when `pluginMarketplaceUnlock` is enabled.
   - `unblockPluginInstallButtons()` and `refreshForcePluginInstallUnlockLoop()` run only when `forcePluginInstall` is enabled.

`clearPluginPatchArtifacts()` must clear both legacy entry labels and force install labels.

## UI Design

Codex++ injected menu shows three plugin rows:

1. **插件市场解锁**
   - Description: `API Key 模式下扩展插件市场请求，尽量显示完整插件列表。`
   - Setting: `pluginMarketplaceUnlock`

2. **强制解锁入口**
   - Description: `恢复 1.1.9 的入口解锁方式，强制显示并启用插件入口。`
   - Setting: `pluginEntryUnlock`

3. **特殊插件强制安装** or **强制加载**
   - Description: `解除 App unavailable / 应用不可用导致的前端安装禁用。`
   - Setting: `forcePluginInstall`

The manager settings UI should expose matching independent switches if it already surfaces these Codex App enhancement settings.

## Error Handling and Diagnostics

Add/keep diagnostics for:

- legacy entry unlock installed/succeeded;
- legacy entry button not found, but avoid noisy repeated logging;
- auth context spoof success/failure if available;
- marketplace request patch installed/not found/failed;
- force install buttons unblocked count if useful.

Diagnostics must not include tokens or sensitive config values.

## Testing Plan

1. Rust settings tests:
   - defaults include all three plugin booleans as `true`;
   - JSON missing `codexAppPluginMarketplaceUnlock` loads as `true`;
   - settings update preserves independent values for entry, marketplace, and force install.

2. Injection static/behavior tests:
   - backend setting map contains both `pluginEntryUnlock` and `pluginMarketplaceUnlock` with different backend keys;
   - `enablePluginEntry()` is gated by `pluginEntryUnlock`;
   - marketplace request/filter patch functions are gated by `pluginMarketplaceUnlock`;
   - force install functions are gated by `forcePluginInstall`;
   - `scanDeferred()` invokes all three independent paths.

3. Verification commands:
   - `cargo test -p codex-plus-core settings`
   - `cargo test -p codex-plus-core cdp_bridge -- --nocapture` if injection static tests live there
   - `cargo check -p codex-plus-manager`
   - `npm --prefix apps/codex-plus-manager run check`

## Out of Scope

- Do not patch `app.asar`.
- Do not remove the current marketplace unlock implementation.
- Do not change relay/proxy launch behavior.
- Do not change plugin marketplace grouping or display labels beyond what is needed for the switches.
- Do not touch `.tmp/cc-switch-src` or test cc-switch code.
