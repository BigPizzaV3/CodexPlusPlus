# Codex++

<p align="center">
  <img src="docs/images/codex-plus-plus.png" alt="Codex++ Icon" width="160">
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

Codex++ is an external enhancement launcher and management tool for the Codex App. It does not modify Codex App's original installation files. Instead, it launches Codex through an external launcher and injects enhancement scripts using the Chromium DevTools Protocol.

## Quick Start

Download the latest installer from [GitHub Releases](https://github.com/BigPizzaV3/CodexPlusPlus/releases):

- Windows: `CodexPlusPlus-*-windows-x64-setup.exe`
- macOS Intel: `CodexPlusPlus-*-macos-x64.dmg`
- macOS Apple Silicon: `CodexPlusPlus-*-macos-arm64.dmg`

After installation, two entry points are available:

- `Codex++`: Silent launcher — no UI, only launches Codex and injects enhancements.
- `Codex++ Manager`: Tauri control panel for launching, inspecting, repairing, updating, configuring relay injection, and managing enhancements and user scripts.

The Windows installer creates desktop and start menu shortcuts. The macOS DMG installs `/Applications/Codex++.app` and `/Applications/Codex++ Manager.app`.

## Sponsors

<a href="mailto:1727532@qq.com">Want to be featured here?</a>
<p align="center">
</p>
<table>
  <tr>
    <th width="180">🏆 Sponsor 🏆</th>
    <th>Description</th>
  </tr>
  <tr>
    <td align="center">
      <a href="https://jojocode.com/">
        <img src="docs/images/sponsor-jojocode.svg" alt="JOJO Code" height="80">
      </a>
    </td>
    <td><a href="https://jojocode.com/"><strong>JOJO Code｜Official Codex++ Relay</strong></a><br>Thank you to JOJO Code for sponsoring this project! JOJO Code is the official Codex++ relay service, providing stable and reliable Codex API access for daily development and team collaboration. Suitable for quick integration, long-term use, and project-level workflows.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://aigocode.com/invite/CodexPlusPlus">
        <img src="docs/images/sponsor-aigocode.png" alt="AIGoCode" height="80">
      </a>
    </td>
    <td><a href="https://aigocode.com/invite/CodexPlusPlus"><strong>AIGoCode</strong></a><br>Thank you to AIGoCode for sponsoring this project! AIGoCode is a one-stop platform integrating Claude Code, Codex, and the latest Gemini models, offering stable, efficient, and cost-effective AI coding services. Flexible subscription plans, domestic direct connection (China), no VPN needed, blazing fast response. AIGoCode offers a special benefit for CodexPlusPlus users: <a href="https://aigocode.com/invite/CodexPlusPlus">registering via this link</a> grants an extra 10% bonus on first top-up!</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://www.packyapi.com/">
        <img src="docs/images/sponsor-packycode.png" alt="PackyCode" height="80">
      </a>
    </td>
    <td><a href="https://www.packyapi.com/"><strong>PackyCode</strong></a><br>Thank you to PackyCode for sponsoring this project! PackyCode is a stable and efficient API relay service provider, offering Claude Code, Codex, Gemini and more. PackyCode provides a special discount for users of this software — register via this link and enter the promo code <code>CodexPlusPlus</code> when topping up to get 10% off your first purchase!</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://apikey.fun/register?aff=CODEX">
        <img src="docs/images/sponsor-apikey-fun.png" alt="APIKEY.FUN" height="80">
      </a>
    </td>
    <td><a href="https://apikey.fun/register?aff=CODEX"><strong>APIKEY.FUN</strong></a><br>Thank you to APIKEY.FUN for sponsoring this project! APIKEY.FUN is an AI relay service dedicated to providing open, stable, and cost-effective access to major global LLMs. The platform supports API relay services for Claude, OpenAI, Gemini, and other popular models, with prices as low as 7% of official pricing. <a href="https://apikey.fun/register?aff=CODEX">Register via this link</a> to enjoy a permanent 5% discount on top-ups.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://runapi.co/register?aff=AWJq">
        <img src="docs/images/sponsor-runapi.png" alt="RunAPI" height="80">
      </a>
    </td>
    <td><a href="https://runapi.co/register?aff=AWJq"><strong>RunAPI</strong></a><br>Thank you to RunAPI for sponsoring this project! RunAPI is an efficient and stable OpenRouter alternative platform. One API key grants access to 150+ mainstream models including OpenAI, Claude, Gemini, DeepSeek, Grok, etc., priced as low as 10% of official rates. Extremely stable and seamlessly compatible with Claude Code, OpenClaw, and other tools.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://www.0029.org/?promo=AFF11F">
        <img src="docs/images/sponsor-0029.svg" alt="0029 Cloud Bridge" height="80">
      </a>
    </td>
    <td><a href="https://www.0029.org/?promo=AFF11F"><strong>0029 Cloud Bridge｜Codex API Relay (GPT-5.5, GPT-Image-2)</strong></a><br>Supports personal and enterprise access. Monthly subscription / pay-as-you-go, Pro/Plus account pool, stable and reliable interfaces across the board, 24/7 technical support!</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://rawchat.cn">
        <img src="docs/images/sponsor-rawchat.svg" alt="RawChat" height="80">
      </a>
    </td>
    <td><a href="https://rawchat.cn"><strong>RawChat｜Codex Relay</strong></a><br>Established relay service with monthly subscription plans. Low multiplier calls, high cache hit rate, Pro/Plus account pool, around-the-clock dedicated maintenance.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://coder.visioncoder.cn">
        <img src="https://coder.visioncoder.cn/logo.png" alt="VisionCoder" height="80">
      </a>
    </td>
    <td><a href="https://coder.visioncoder.cn"><strong>VisionCoder Development Platform</strong></a><br>Thank you to VisionCoder for supporting this project. VisionCoder Development Platform is a reliable and efficient API relay service provider offering major AI models including Claude Code, Codex, and Gemini, helping developers and teams integrate AI capabilities more easily and boost productivity. VisionCoder also offers a limited-time <a href="https://coder.visioncoder.cn">Token Plan</a> promotion: buy 1 month, get 1 month free.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://aihub2api.cloud/register?promo=CODEXPLUSPLUS">
        <img src="docs/images/sponsor-aihub2api.png" alt="AIHub2API" height="80">
      </a>
    </td>
    <td><a href="https://aihub2api.cloud/register?promo=CODEXPLUSPLUS"><strong>AIHub2API</strong></a><br>Thank you to AIHub2API for sponsoring this project! AIHub2API is a stable and efficient API relay service provider specializing in Codex relay services, offering high cache hit rates and low multiplier relay services. Network-optimized with no VPN required, blazing fast response, prices as low as 1% of official pricing. <a href="https://aihub2api.cloud/register?promo=CODEXPLUSPLUS">Register via this exclusive link</a> to receive a $10 trial credit.</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://www.compshare.cn/?ytag=GPU_YY_git_codex++">
        <img src="docs/images/sponsor-ucloud-compshare.png" alt="UCloud CompShare" height="80">
      </a>
    </td>
    <td><a href="https://www.compshare.cn/?ytag=GPU_YY_git_codex++"><strong>UCloud CompShare</strong></a><br>Thank you to UCloud CompShare for sponsoring this project! CompShare is UCloud's AI cloud platform, offering cost-effective monthly/per-use domestic model Agent Plan packages starting from just ¥49/month. Also provides officially relayed stable overseas models supporting Claude Code, Codex, and API access — with enterprise-grade high concurrency, 24/7 technical support, and self-service invoicing. Users registering through this link receive ¥5 free platform trial credit!</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://cubence.com?source=codexplusplus">
        <img src="docs/images/sponsor-cubence.png" alt="Cubence" height="80">
      </a>
    </td>
    <td><a href="https://cubence.com?source=codexplusplus"><strong>Cubence</strong></a><br>Thank you to Cubence for supporting this project. Cubence is an API relay service provider committed to delivering stable and efficient service. Since September 2025, they have been providing support for Claude Code, Codex, Gemini, and various other models. Cubence offers a special exclusive discount for open source project users: the promo code <code>CODEXPLUSPLUS</code> gives you 12% off your first purchase!</td>
  </tr>
  <tr>
    <td align="center">
      <a href="https://maolaoapi.com">
        <img src="docs/images/sponsor-maolao-api.jpg" alt="MaoLao API" height="80">
      </a>
    </td>
    <td><a href="https://maolaoapi.com"><strong>MaoLao API</strong></a><br>MaoLao API is an API relay service focused on mainstream VibeCoding models, with its own pure Pro20X/Plus account pool — enabling low-cost plans even at low multipliers. All models and groups are unrestricted in every plan! MaoLao API: maolaoapi.com</td>
  </tr>
</table>

## Community & Support

Join the Codex++ QQ group (QQ Group: 1103050832) to report issues, share experiences, or suggest new features.

WeChat Group: <a href="https://docs.qq.com/doc/DQ2VOanZTTFZJcUpZ#">Click here for the latest WeChat group QR code</a>.

<img src="docs/images/discussion-group-qr.jpg" alt="Codex++ WeChat Group QR Code" width="260">

Telegram Channel: <https://t.me/CodexPlusPlus>

If Codex++ has been helpful to you, feel free to buy me a coffee or support the project via donation.

<p align="center">
  <img src="docs/images/sponsor-alipay.jpg" alt="Alipay Donation QR" width="220">
  <img src="docs/images/sponsor-wechat.jpg" alt="WeChat Donation QR" width="220">
</p>

## Main Features

- Rust backend and silent launcher — no additional runtime required at startup.
- Tauri + React management tool with dark/light theme switching.
- External CDP injection — does not modify `app.asar` or write DLLs to Codex installation directory.
- Relay injection mode: supports multiple relay configurations, writes to `CodexPlusPlus` provider, and can switch back to official ChatGPT login.
- Traditional enhancement mode: plugin entry unlock, forced plugin installation, session deletion, Markdown export, project move, Timeline, and more.
- Independent user script management — inject custom scripts at startup.
- Provider sync: synchronizes local session metadata before startup, keeping old sessions visible after switching providers.
- Zed open entry: detects remote SSH context and opens the corresponding file directly in Zed Remote Development from Codex.
- Upstream worktree creation: creates new worktrees from `upstream/<base-branch>`, automatically fetches remote branches beforehand to reduce merge conflicts from stale local HEAD.
- GitHub Release auto-update — both the manager and silent launcher check for available updates.
- Windows single instance, no-black-window launch, admin privilege manifest, system desktop path detection.
- macOS x64/arm64 per-architecture DMG, silent launcher hides Dock icon.

## Pain Points & Solutions

In API Key login mode, Codex's native plugin entry prompts "Please log in to ChatGPT," making plugins unusable:

![Plugin entry unavailable in API Key mode](docs/images/pain-plugin-disabled.png)

Codex's native session list only has an archive option — no real delete button:

![Native session list lacks delete functionality](docs/images/pain-no-delete-button.png)

After launching Codex++, the plugin entry is unlocked, and a delete button appears on session hover:

![Codex++ unlocks plugin entry and adds delete button](docs/images/solution-plugin-and-delete.png)

A `Codex++` menu bar appears at the top, showing backend status and providing access to settings:

![Codex++ backend status indicator](docs/images/backend-status-indicator.png)
![Codex++ settings panel](docs/images/settings-panel.png)

## Relay Injection

Relay injection is for users who have already logged in with an official Codex/ChatGPT account but want to route model requests to a custom compatible API.

In the "Relay Injection" page of the manager tool:

1. Confirm that ChatGPT login status has been detected.
2. Add one or more relay configurations, providing Base URL and Key.
3. Select the current configuration and apply relay injection.
4. Launch `Codex++`.

Codex++ writes a configuration similar to the following into `~/.codex/config.toml`:

```toml
model_provider = "CodexPlusPlus"

[model_providers.CodexPlusPlus]
name = "CodexPlusPlus"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://example.com/v1"
experimental_bearer_token = "sk-..."
```

To return to official login mode, click "Clear API Mode" on the Relay Injection page to remove `OPENAI_API_KEY`-related config and switch back to official ChatGPT login.

## Enhancements

Enhancements are toggled on/off from within the manager tool. Enhance injection is enabled by default; disabling it prevents the Codex++ menu and scripts from being injected.

If relay injection mode is enabled, plugin entry unlock and forced installation are no longer needed — the UI will indicate "Not required in relay injection mode." Session deletion, export, move, Timeline, recommendations, and user scripts remain available.

## Recommendations

Recommendations are loaded from a remote ad list:

```text
https://raw.githubusercontent.com/BigPizzaV3/Ad-List/main/ads.json
https://cdn.jsdelivr.net/gh/BigPizzaV3/Ad-List@main/ads.json
```

Requests append `?v=timestamp` to bypass CDN cache. Slow recommendation loading does not affect backend connection status.

## Auto-Update & Installers

Codex++ distributes installers via GitHub Release. Windows generates an NSIS installer; macOS generates two DMG files for Intel x64 and Apple Silicon arm64.

The "About" page in the manager tool can check for and initiate updates. When the silent launcher detects a new version, it launches the manager tool and shows an update prompt.

## Data Locations

- Codex config: `~/.codex/config.toml`
- Codex login state: `~/.codex/auth.json`
- Codex local database: `~/.codex/state_5.sqlite`
- Codex++ state & logs: `~/.codex-session-delete/`
- Provider sync backups: `~/.codex/backups_state/provider-sync`

## FAQ

### Codex++ menu not appearing

Make sure you launched Codex via the `Codex++` entry point, not the original Codex. You can also open the "Diagnostics" and "Logs" pages in the manager tool to check the injection status.

### Plugin says backend is unreachable

First test it in your browser or PowerShell:

```powershell
Invoke-RestMethod -Method Post -Uri http://127.0.0.1:57321/backend/status -Body "{}" -ContentType "application/json"
```

If the endpoint responds but the plugin still times out, it's usually a CDP bridge or script cache issue in the Codex page. Restart Codex++, or check `renderer.script_loaded`, `bridge.request`, `bridge.response` in the manager tool's logs.

### How is Upstream worktree different from Codex's native creation?

Codex++'s Upstream worktree feature is equivalent to first updating the remote branch, then running:

```bash
git worktree add -b <new-branch> <worktree-path> upstream/<base-branch>
```

This creates the new worktree from the latest remote tracking branch rather than from the local HEAD of the current session. If Codex++ cannot safely detect the current Codex version's native worktree creation form, manually fill in the repo path, branch name, worktree path, remote, and base branch from the Codex++ menu.

### macOS says "cannot be opened" or "damaged"

Since the current installer is unsigned/not notarized, macOS Gatekeeper may block it with a "damaged" warning:

![macOS shows Codex++ Manager is damaged](docs/images/macos-damaged-warning.png)

If you see this, run the following two commands in the terminal to remove Apple's security quarantine:

```bash
sudo xattr -rd com.apple.quarantine /Applications/Codex++\ 管理工具.app
sudo xattr -rd com.apple.quarantine /Applications/Codex++.app
```

After running these, reopen `Codex++` or `Codex++ 管理工具`.

### Can I use Codex++ on macOS Intel?

Yes. Releases provide both `macos-x64.dmg` and `macos-arm64.dmg`. Intel Macs download the x64 package; Apple Silicon Macs download the arm64 package.

## Development

```bash
# Frontend check
cd apps/codex-plus-manager
npm install
npm run check
npm run vite:build

# Rust check
cd ../..
cargo fmt --check
cargo test
cargo build --release
```

Main structure:

```text
apps/
  codex-plus-launcher/          Silent launcher entry point
  codex-plus-manager/           Tauri manager tool
assets/inject/
  renderer-inject.js            Enhancement scripts injected into Codex renderer
crates/
  codex-plus-core/              Core logic: launch, injection, config, update, install, bridge
  codex-plus-data/              Session data, export, provider sync
scripts/installer/
  windows/CodexPlusPlus.nsi     Windows NSIS installer
  macos/package-dmg.sh          macOS DMG packaging
```

## Friends

- [LINUX DO](https://linux.do)

## Disclaimer

Codex++ is an external enhancement tool and does not modify Codex App's original files. If the Codex App updates and its page structure changes, the injection scripts may need updating accordingly.
