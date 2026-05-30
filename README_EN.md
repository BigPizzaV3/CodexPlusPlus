# Codex++

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ icon" width="160">
</p>

<p align="center">
  <a href="README.md">中文</a> | English
</p>

<p align="center">
  <img alt="Release" src="https://img.shields.io/github/v/release/BigPizzaV3/CodexPlusPlus">
  <img alt="Stars" src="https://img.shields.io/github/stars/BigPizzaV3/CodexPlusPlus">
  <img alt="License" src="https://img.shields.io/github/license/BigPizzaV3/CodexPlusPlus">
  <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
  <img alt="Tauri" src="https://img.shields.io/badge/tauri-2.x-24C8DB">
</p>

Codex++ is an external enhancement launcher and manager for the Codex App. It does not modify the original Codex installation. Instead, it starts Codex externally and injects enhancements through the Chromium DevTools Protocol.

## Quick Start

Download the latest installer from [GitHub Releases](https://github.com/BigPizzaV3/CodexPlusPlus/releases):

- Windows: `CodexPlusPlus-*-windows-x64-setup.exe`
- macOS Intel: `CodexPlusPlus-*-macos-x64.dmg`
- macOS Apple Silicon: `CodexPlusPlus-*-macos-arm64.dmg`

After installation, two entry points are available:

- `Codex++`: a silent launcher. It does not show the manager UI and only starts Codex with Codex++ injection.
- `Codex++ Manager`: a Tauri control panel for launch, diagnostics, repair, updates, relay injection, enhancements, and user scripts.

The Windows installer creates desktop and Start Menu shortcuts. The macOS DMG installs `/Applications/Codex++.app` and `/Applications/Codex++ 管理工具.app`.

## Sponsors

<p align="center">
  <a href="mailto:1727532@qq.com">Want to be shown below?</a>
</p>
<table>
  <tr>
    <th width="180">🏆 Sponsor 🏆</th>
    <th>Introduction</th>
  </tr>
  <tr>
    <td align="center">
      <a href="https://jojocode.com/">
        <img src="docs/images/sponsor-jojocode.svg" alt="JOJO Code" width="150">
      </a>
    </td>
    <td><a href="https://jojocode.com/"><strong>JOJO Code | Official Codex++ Relay</strong></a><br>Thanks to JOJO Code for sponsoring this project! JOJO Code is the official Codex++ relay service. It is built for daily development and team collaboration, providing stable Codex API access for quick onboarding, long-term use, and project workflows.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://aigocode.com/invite/CodexPlusPlus">
        <img src="docs/images/sponsor-aigocode.png" alt="AIGoCode" width="150">
      </a>
    </td>
    <td><a href="https://aigocode.com/invite/CodexPlusPlus"><strong>AIGoCode</strong></a><br>Thanks to AIGoCode for sponsoring this project! AIGoCode is an all-in-one platform integrating the latest Claude Code, Codex, and Gemini models, providing stable, efficient, and cost-effective AI programming services. It offers flexible subscription plans, direct access in China, no extra network setup, and fast responses. AIGoCode provides a special benefit for CodexPlusPlus users: users who <a href="https://aigocode.com/invite/CodexPlusPlus">register through this link</a> can receive an extra 10% bonus credit on their first recharge.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://www.packyapi.com/">
        <img src="docs/images/sponsor-packycode.png" alt="PackyCode" width="150">
      </a>
    </td>
    <td><a href="https://www.packyapi.com/"><strong>PackyCode</strong></a><br>Thanks to PackyCode for sponsoring this project! PackyCode is a stable and efficient API relay service provider, offering relay services for Claude Code, Codex, Gemini, and more. PackyCode provides a special discount for users of this software: register through this link and enter the "CodexPlusPlus" coupon code when recharging to get 10% off your first recharge.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://apikey.fun/register?aff=CODEX">
        <img src="docs/images/sponsor-apikey-fun.png" alt="APIKEY.FUN" width="150">
      </a>
    </td>
    <td><a href="https://apikey.fun/register?aff=CODEX"><strong>APIKEY.FUN</strong></a><br>Thanks to APIKEY.FUN for sponsoring this project! APIKEY.FUN is an AI relay platform focused on open, stable, and cost-effective access to mainstream global models. It supports API relay services for Claude, OpenAI, Gemini, and other popular models, with prices as low as 7% of the official rate. Register through the dedicated link to receive up to a permanent 5% recharge discount.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://runapi.co/register?aff=AWJq">
        <img src="docs/images/sponsor-runapi.png" alt="RunAPI" width="150">
      </a>
    </td>
    <td><a href="https://runapi.co/register?aff=AWJq"><strong>RunAPI</strong></a><br>Thanks to RunAPI for sponsoring this project! RunAPI is an efficient and stable OpenRouter alternative API platform. One API key can access OpenAI, Claude, Gemini, DeepSeek, Grok, and 150+ mainstream models at prices as low as 10% of the original rate, with seamless compatibility for tools such as Claude Code and OpenClaw.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://www.0029.org/?promo=AFF11F">
        <img src="docs/images/sponsor-0029.svg" alt="0029 Cloud Bridge" width="150">
      </a>
    </td>
    <td><a href="https://www.0029.org/?promo=AFF11F"><strong>0029 Cloud Bridge | Codex API Relay Station (gpt5.5 gpt-image-2)</strong></a><br>Supports individual and enterprise access. Monthly plans and pay-as-you-go billing are available, with Pro/Plus account pools, stable site-wide APIs, and 24/7 technical support.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://rawchat.cn">
        <img src="docs/images/sponsor-rawchat.svg" alt="RawChat" width="150">
      </a>
    </td>
    <td><a href="https://rawchat.cn"><strong>RawChat | Codex Relay Station</strong></a><br>A long-running relay station with monthly plans, low-rate usage, high cache hit rates, Pro/Plus account pools, and dedicated all-day maintenance.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://coder.visioncoder.cn">
        <img src="https://coder.visioncoder.cn/logo.png" alt="VisionCoder" width="110">
      </a>
    </td>
    <td><a href="https://coder.visioncoder.cn"><strong>VisionCoder Developer Platform</strong></a><br>Thanks to VisionCoder for supporting this project. VisionCoder Developer Platform is a reliable and efficient API relay service provider, offering access to mainstream AI models such as Claude Code, Codex, and Gemini. It helps developers and teams integrate AI capabilities more easily and improve productivity. VisionCoder is also offering our users a limited-time <a href="https://coder.visioncoder.cn">Token Plan</a> promotion: buy 1 month and get 1 month free.</td>
  </tr>
</table>


## Highlights

- Rust backend and silent launcher with no extra runtime requirement.
- Tauri + React manager with dark/light theme support.
- External CDP injection. No `app.asar` patching and no DLL writes into the Codex installation.
- Relay injection mode with multiple relay profiles, `CodexPlusPlus` provider configuration, and a one-click switch back to official ChatGPT login mode.
- Traditional enhancement mode with plugin entry unlock, forced plugin install, session delete, Markdown export, project move, Timeline, and more.
- Independent user script management with startup injection.
- Provider Sync to keep historical sessions visible after switching providers.
- Zed open entry detects remote SSH context and opens the matching remote file in Zed Remote Development from Codex.
- Upstream worktree creation: create new worktrees from `upstream/<base-branch>` after fetching the remote branch, reducing conflicts caused by stale local HEAD state.
- GitHub Release updates. Both the manager and silent launcher can detect available updates.
- Windows single instance, no console window, administrator manifest, and system Desktop path detection.
- Separate macOS x64 and arm64 DMGs. The silent launcher hides its Dock icon.

## Relay Injection

Relay injection is for users who are already logged in with an official ChatGPT account in Codex/ChatGPT and want model requests to go through a custom compatible API.

The boundary of this hybrid mode is:

- The official ChatGPT/Codex login state still owns Codex App account features and the plugin entry.
- The relay profile only controls the Base URL, key, and model names used for model requests.
- The compatible API provider is not tied to any specific vendor; it only needs to match the selected upstream protocol and Codex configuration.
- Clearing API mode should return Codex to the official login mode so the official account and plugins keep working.

Before applying relay injection, run a minimal preflight:

1. Make sure Codex has detected the ChatGPT login state and the plugin entry is available.
2. Confirm the custom Base URL is reachable and supports the selected upstream protocol, such as a Responses-compatible endpoint.
3. Test the target key with the smallest useful auth probe, such as a model-list request or a short message request.
4. Only record whether the key exists and whether auth passed. Do not paste real keys into logs, screenshots, or issues.
5. Make sure `~/.codex/config.toml` has a backup so clearing API mode can safely roll back.

In the manager's Relay Injection page:

1. Make sure ChatGPT login status is detected.
2. Add one or more relay profiles with Base URL and Key.
3. Select the active profile and apply relay injection.
4. Launch `Codex++`.

Codex++ writes configuration similar to this into `~/.codex/config.toml`:

```toml
model_provider = "CodexPlusPlus"

[model_providers.CodexPlusPlus]
name = "CodexPlusPlus"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://example.com/v1"
experimental_bearer_token = "sk-..."
```

To return to the official login mode, use the clear API mode button in the Relay Injection page. This removes `OPENAI_API_KEY` related configuration and switches Codex back to official ChatGPT authentication.

## Enhancements

Enhancements are controlled in the manager. Enhancement injection is enabled by default. When disabled, Codex++ will not inject its menu or scripts.

When relay injection mode is active, plugin entry unlock and forced plugin install are unnecessary, and the UI will say so. Other enhancements, including session delete, export, move, Timeline, recommendations, and user scripts, can still be used.

## Recommendations

Recommended content is loaded from:

```text
https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json
https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json
```

Requests automatically append a `?v=timestamp` cache buster to avoid stale CDN content. Slow recommendation loading does not mark the backend connection as failed.

## Updates and Packages

Codex++ publishes installers through GitHub Releases. Windows builds an NSIS installer, while macOS builds separate Intel x64 and Apple Silicon arm64 DMGs.

The manager's About page can check and start updates. When the silent launcher finds a new version, it opens the manager directly on the update prompt.

## Data Locations

- Codex config: `~/.codex/config.toml`
- Codex auth state: `~/.codex/auth.json`
- Codex local database: `~/.codex/state_5.sqlite`
- Codex++ state and logs: `~/.codex-session-delete/`
- Provider Sync backups: `~/.codex/backups_state/provider-sync`

## FAQ

### The Codex++ menu does not appear

Make sure Codex was launched from the `Codex++` entry instead of the original Codex entry. You can also inspect the Diagnostics and Logs pages in the manager.

### The plugin says the backend is disconnected

First test the helper endpoint:

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:57321/backend/status -Body "{}" -ContentType "application/json"
```

If the endpoint works but the plugin still times out, it is usually a Codex page CDP bridge or script cache issue. Restart Codex++, or check manager logs for `renderer.script_loaded`, `bridge.request`, and `bridge.response`.

### How is Upstream worktree different from Codex native creation?

Codex++ updates the remote branch first, then creates the worktree as if you ran:

```bash
git worktree add -b <new-branch> <worktree-path> upstream/<base-branch>
```

The new worktree starts from the fresh remote tracking branch instead of the local HEAD used by the current session. If Codex++ cannot safely recognize the current Codex version's native worktree form, use the Codex++ menu entry and enter the repository path, branch name, worktree path, remote, and base branch manually.

### macOS says the app cannot be opened or is damaged

Unsigned and unnotarized builds may be blocked by Gatekeeper. Allow the app in System Settings -> Privacy & Security. For formal distribution, configure Apple Developer ID signing and notarization.

### Does it support Intel Macs?

Yes. Releases provide both `macos-x64.dmg` and `macos-arm64.dmg`. Intel Macs should use the x64 package, while Apple Silicon Macs should use the arm64 package.

## Development

```bash
# Frontend checks
cd apps/codex-plus-manager
npm install
npm run check
npm run vite:build

# Rust checks
cd ../..
cargo fmt --check
cargo test
cargo build --release
```

Project structure:

```text
apps/
  codex-plus-launcher/          Silent launcher
  codex-plus-manager/           Tauri manager
assets/inject/
  renderer-inject.js            Enhancement script injected into Codex
crates/
  codex-plus-core/              Launch, injection, config, update, install, bridge
  codex-plus-data/              Session data, export, Provider Sync
scripts/installer/
  windows/CodexPlusPlus.nsi     Windows NSIS installer
  macos/package-dmg.sh          macOS DMG packager
```

## Community and Support

Scan the QR code to join the Codex++ discussion group, report issues, share usage notes, or suggest features:

<img src="docs/images/discussion-group-qr.jpg" alt="Codex++ discussion group QR code" width="260">

If Codex++ has helped you, you can buy me a coffee or send a small tip to support continued maintenance.

<p align="center">
  <img src="docs/images/sponsor-alipay.jpg" alt="Alipay sponsor QR code" width="220">
  <img src="docs/images/sponsor-wechat.jpg" alt="WeChat sponsor QR code" width="220">
</p>

## Friendly Links

- [LINUX DO](https://linux.do)

## Notes

Codex++ is an external enhancement tool and does not modify original Codex App files. If a future Codex App update changes page structure, the injection script may need updates.
