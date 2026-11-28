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

Codex++ is a unified manager for AI coding agents, built on an everything-is-an-extension architecture: UI, commands, tools for the model, agent-loop hooks, and the requests and replies sent to models can all be taken over by extensions. No code required: say “turn the send button into a dog” in a Codex chat and the agent writes the extension, hot-reloads it, and confirms with a screenshot.

It also ships a local service (a local gateway): Codex, Claude Code, Claude Desktop, Gemini CLI, OpenCode, Zed, and 50+ other agents share one provider list, requests are routed by model name, and traffic moves to the next provider automatically when a key cools down or a provider trips. ChatGPT, GitHub Copilot, Cursor, and other subscriptions can be signed in as well, and one provider can hold several accounts or keys. At the top of each agent's provider page you pick **Aggregate** or **Direct**: Aggregate goes through the local service and uses the providers you switch on together; Direct works as before (click **Use** on one provider), with official login, official login + API, and pure API all still there.

There is also session management, Codex UI enhancements, Claude Desktop enhancements, Computer Use sharing across agents, and sub-agent collaboration. The top of the manager's sidebar holds Home, MCP & Skills, Session, Usage, and Local service; below are the three agents, Codex, Claude Code, and Claude Desktop, each with its pages listed under it; the gear opens Settings. Double-click to open its window and launch each app from there. It never modifies any official app's installation files.

<p align="center">
  <img src="docs/images/readme-preview/extension-chat.png" alt="Real session: in a Codex chat the agent turned the send button into a dog and the context ring into a rainbow progress bar, both live" width="720">
  <br><sub>A real session (in Chinese): two requests, a dog send button, and a rainbow context bar.</sub>
</p>

## Quick Start

Download the latest installer from [GitHub Releases](https://github.com/BigPizzaV3/CodexPlusPlus/releases):

- Windows: `CodexPlusPlus-*-windows-x64-setup.exe`
- macOS Intel and Apple Silicon: `CodexPlusPlus-*-macos-universal.dmg`

After installation, one `Codex++` entry is available. Double-click it to open the window, then:

The first launch shows a two-step setup (connect a model, launch Codex). You can skip it and do this yourself:

1. **Add providers** — open **Codex → Connection & providers** and click **Add provider**: paste an API key or sign in to a ChatGPT / GitHub Copilot / SuperGrok subscription; **Import from third party** adds several at once. Model lists and prices are filled in automatically.
2. **Pick a connection** — choose **Aggregate** or **Direct** at the top of the page. With Aggregate, switch on the providers you want and pick the default model (a model group works too); with Direct, click **Use** on one provider. Claude Code works the same way on its own **Provider** page; Grok and other agents connect under **Local service → Service & Agents** and keep working in your terminal as usual.
3. **Launch Codex** — click **Launch Codex** on the Codex page or the start button next to Codex in the sidebar. Settings that rely on injection (most enhancements, Fast unlock) take effect after restarting Codex from Codex++.

See the full [guide](https://codexpp.cc/guide.html) (Chinese) on the website.

The Windows installer creates Desktop and Start Menu shortcuts. On macOS, drag `Codex++.app` from the DMG into Applications.

**Upgrading an older macOS version:** The old two-app updater cannot install the new single-app package. Quit both old apps, then drag `Codex++.app` from the new DMG into Applications and replace the old app with the same name. Remove the old `Codex++ 管理工具.app`. Existing settings and session data are preserved.

## Community and Support

Join <a href="https://qm.qq.com/q/5h3pxpxg7S">Codex++ community group 4 (QQ group: 1127858981)</a> to report issues, share feedback, or suggest features.

Telegram: <https://t.me/CodexPlusPlus>

Friendly link: <a href="https://linux.do">LINUX DO</a>

## Current Features

| Agent | What Codex++ does |
| --- | --- |
| **Codex desktop** | Providers in Aggregate (through the local service) or Direct mode, protocol conversion, session management, UI enhancements, Codex plugins, multi-launch, extensions, and the Agent pet |
| **Claude Code** | In Aggregate mode, providers of any protocol through the local service (Anthropic requests are converted to Chat / Responses / Gemini); in Direct mode, Anthropic-protocol providers are written straight into its own settings; Computer Use connection |
| **Claude Desktop** | Always through the local service; full-feature mode (Chinese UI, dictation, Cowork), a patched copy, extensions, and Computer Use connection |
| **Grok CLI** | Connected under **Local service → Service & Agents**, including SuperGrok subscriptions |
| **Gemini CLI, OpenCode, Zed, VS Code, and 50+ more** | The shared provider list through the local service, with Anthropic / Chat / Responses / Gemini conversion |
| **DeepSeek Harness and more** | Computer Use connection; callable as a sub-agent from Codex chats; dsh plugins can be imported into Codex |

| Area | Capabilities |
| --- | --- |
| Everything is an extension | An extension is a folder with a `codexpp.json`: UI slots and pages, slash commands and composer buttons, message actions and side panels, tools for the model over MCP, Codex hooks, a tool pipeline, request / response / retry middleware, background processes and services, session queries, config forms, and i18n. Registrations are undone automatically and edits hot-reload. Claude Desktop has its own extensions page too (manifest `claudeDesktop`, same `codexPlus` API). See the [extension guide](EXTENSIONS.md) (Chinese) |
| Build by chatting | Click **Build by chat** under **Codex → Everything is an extension**, then describe what you want in a Codex chat; the agent inspects the page, writes the extension, hot-reloads, and takes a screenshot. Writes stay in your extensions folder and go through Codex tool approval |
| Extension ecosystem | Featured and official extensions reviewed by Codex++ and pinned to the reviewed commit; community extensions from GitHub repos tagged `codexpp-extension` (capabilities shown before install, disabled by default); one-click publish to GitHub; import DeepSeek Harness (dsh) plugins |
| Aggregate / Direct · new | Codex's **Connection & providers** and Claude Code's **Provider** page share one provider list with Aggregate / Direct at the top; in Aggregate each provider has a switch and model groups can be the default model |
| Local service | One local service for Codex, Claude Code, Claude Desktop, Grok, and more; routing by bare model name, `<provider>/<model>`, or `group/<id>`; models offered by several providers form automatic groups; **Service & Agents** lists connected agents and lets you edit their config files |
| Routing, accounts and keys | Groups run in order, rotate, by weight, quota first, speed first, or manually, with optional conversation stickiness, and you can see how each request was routed; several accounts or keys per provider (new) with their own assignment mode, 429 cooldown, quota cooldown, 401 disable, and 5xx circuit breaking; optional per-key RPM and concurrency limits |
| Subscriptions and relays | Sign in to ChatGPT, ChatGPT API, GitHub Copilot, and SuperGrok and see remaining quota, a quota timeline, and reset cards; sub2api / new-api relays are detected with their multiplier and balance; Gemini upstreams are supported |
| Usage and request log | Daily cost, tokens, cache hits, and calls broken down by agent, provider, or model, plus hourly usage, a heatmap, and subscription quota trends; every request with status, retries, time to first token, tokens, cache, estimated cost, and the failure reason |
| Direct mode | Official login, official login plus API, pure API, and Failover / Model aggregation providers; Responses / Chat Completions; model tests, Provider Doctor, third-party and deep-link imports. **Codex sign-in identity** picks a ChatGPT account, a virtual account (API key with ChatGPT-only entries), or no sign-in |
| Models and context | Per-model context windows, prices, and reasoning levels; `model_catalog_json`; per-provider proxies; image generation models per agent |
| Computer Use sharing | Expose the Computer Use and Chrome browser control built into the Codex desktop app (ChatGPT.app) over MCP to Claude Code, Claude Desktop, Codex CLI, DeepSeek Harness, ZCode, and WorkBuddy, with a selectable approval policy (macOS) |
| Claude Desktop | Full-feature mode launches the official Claude through the local service with Chinese UI, dictation, Cowork, and page extensions; or use a patched copy. The official app's files stay untouched |
| Session management | Codex, Claude Code, and Grok sessions in one list; bulk deletion with undo, Markdown export, project move, thread IDs, opt-in [turn rewind](docs/turn-rewind.md), and side chats saved as ordinary conversations |
| Codex multi-launch | Start an isolated Codex instance per provider identity, each with its own config, sessions, and user data directory; restart or stop each one individually |
| Sub-agent collaboration | Delegate in natural language from a Codex chat to Codex CLI, DeepSeek Harness (dsh), Claude Code, or Google Antigravity (`agy`) and receive results in the same chat; tasks for a busy agent run concurrently up to a limit, then queue |
| Codex enhancements | Built-in extensions switched on Codex's own **Codex++** settings page: plugin marketplace and model whitelist handling, session actions, paste fix, dictation with a global hotkey, native composer predictions, conversation width and scroll restore, native Fast unlock, Goals, next-step suggestions, skins, and image overlays |
| Usage status line | Output throughput, cache hit rate, and context usage below the composer, read from local Codex session records |
| Tray panel and WeChat | A tray quick panel to switch models and check usage and recent requests; QR login connects personal WeChat to local Codex sessions |
| Maintenance | MCP and Skills per agent (Skills can be exported to another machine), Codex plugins without a ChatGPT login, user scripts, cache cleanup (Codex, Claude, Codex++), app detection, launch at login, settings export / import, logs, diagnostics, and Release updates downloaded in the background |

Every UI enhancement is independently configurable. Turning off the **Enable built-in extensions** master switch still leaves Codex++ available as a provider and launch manager.

## Screenshots

**Home** — how Codex and Claude are connected and whether they run, today's usage, and the main entries on one screen.

<p align="center">
  <img src="docs/images/readme-preview/home-en.webp" alt="Home: status, connection, and auto-takeover for Codex and Claude, then today's usage, the local service, extensions, and sessions" width="100%">
</p>

**Extensions** — featured, community, local extensions, and dsh plugins on one page under **Codex → Everything is an extension**.

<p align="center">
  <img src="docs/images/readme-preview/ext-featured-en.webp" alt="Extensions page: search, the build-by-chat button, and reviewed official and featured extension packages" width="100%">
</p>

<table>
  <tr>
    <td width="50%"><img src="docs/images/readme-preview/ext-community-en.webp" alt="Community extensions: the unreviewed notice and extensions sorted by stars" width="100%"></td>
    <td width="50%"><img src="docs/images/readme-preview/ext-dsh-en.webp" alt="dsh plugins: an imported bundle with each item marked compatible or not" width="100%"></td>
  </tr>
</table>

**Providers** — one list, Aggregate or Direct. In Aggregate each provider has a switch, model groups sit below, and subscriptions show remaining quota while relays show their multiplier and balance.

<p align="center">
  <img src="docs/images/readme-preview/gateway-providers-en.webp" alt="Codex Connection and providers in Aggregate mode: ChatGPT subscription, DeepSeek, Kimi, Zhipu GLM, and two relays each with a switch, then model groups and the Codex sign-in identity" width="100%">
</p>

<p align="center">
  <img src="docs/images/readme-preview/claude-providers-en.webp" alt="Claude Code Provider page: the same provider list and model groups, with Claude Code's default model at the bottom" width="100%">
</p>

**Local service** — connected agents with their default models and config files; listening address, multi-key rotation, and restoring direct connections on exit. Agent config files are backed up first, and disconnecting restores only what changed.

<p align="center">
  <img src="docs/images/readme-preview/gateway-agents-en.webp" alt="Local service, Service and Agents: Codex and Claude Code connected with their default models, followed by local service options" width="100%">
</p>

**Routing** — groups decide which provider goes first, whether a conversation sticks to one, and who takes over on errors. Models offered by several providers are grouped automatically.

<p align="center">
  <img src="docs/images/readme-preview/gateway-routing-en.webp" alt="Routing: a Main coding group in order and a Budget rotation group, each member with quota, last use, and time to first token; automatic groups below" width="100%">
</p>

**Usage and request log**

<p align="center">
  <img src="docs/images/readme-preview/usage-stats-en.webp" alt="Usage statistics: estimated cost, tokens, cache hit rate, calls, a daily stacked bar chart, and a per-provider breakdown" width="100%">
</p>

<p align="center">
  <img src="docs/images/readme-preview/usage-requests-en.webp" alt="Request log: an in-flight request, an automatic retry, a 429 that moved to the next provider, and input, output, cached tokens, and cost" width="100%">
</p>

**Computer Use sharing** — under **Settings → Computer Use**, click **Connect** next to a client and restart it; tools appear as `mcp__codex-plus-cu__*` and `mcp__codex-plus-browser__*` (macOS).

<p align="center">
  <img src="docs/images/readme-preview/computer-use-en.webp" alt="Computer Use: Claude Code and DeepSeek Harness connected, reaching ChatGPT.app's native Computer Use and the Chrome extension through Codex++ MCP servers" width="100%">
</p>

**Codex plugins, Claude Desktop enhancements, sessions, and cache cleanup**

<table>
  <tr>
    <td width="50%"><img src="docs/images/readme-preview/codex-plugins-en.webp" alt="Codex plugins: official plugins grouped by category, installable without a ChatGPT login" width="100%"></td>
    <td width="50%"><img src="docs/images/readme-preview/claude-local-settings-en.webp" alt="Desktop enhancements: Claude Desktop in full-feature mode, with model, account and session, feature, and certificate settings" width="100%"></td>
  </tr>
  <tr>
    <td><img src="docs/images/readme-preview/sessions-en.webp" alt="Sessions: local Codex, Claude Code, and Grok sessions grouped by project" width="100%"></td>
    <td><img src="docs/images/readme-preview/agent-cache-en.webp" alt="Cache cleanup in Settings for Codex, Claude, and Codex++" width="100%"></td>
  </tr>
</table>

**Usage status line**

<p align="center">
  <img src="docs/images/usage-statusline.png" alt="Usage status line below the composer: output throughput, cache hit rate, and context usage" width="100%">
</p>

**Codex multi-launch**

<p align="center">
  <img src="docs/images/readme-preview/codex-multi-launch-en.webp" alt="Codex multi-launch: the main Codex and two provider instances running, each with restart and stop actions" width="100%">
</p>

**Sub-agent collaboration** — say “Have DeepSeek implement this feature” in a Codex chat; see the [guide](docs/agent-collaboration.md).

<p align="center">
  <img src="docs/images/agent-collaboration.png" alt="Verified DeepSeek example: ask what 1+1 equals and receive 2 through the Codex Plus Collaboration integration" width="100%">
</p>

**Agent pet:** an optional clickable desktop pet for Codex that squishes with a sound effect when clicked. Pick an image from the built-in gallery (Fluent Emoji, fetched at runtime) or use your own https image URL, and import your own sounds. You can also pick the “Big fat fish (DeepSeek whale)” pet type, the original usage widget with balance, price and usage statistics, bubble editor and sounds. Turn it on with **Settings** on its row under **Everything is an extension → Local extensions**; see the [Agent pet guide](docs/agent-pet.md) and the [whale widget guide](docs/whale-widget.md).

**Whale widget source and credits:** The original features, complete UI engine, character, built-in sounds and animations come from [MeteorNOX/DeepSeek-Balance-Whale-Widget](https://github.com/MeteorNOX/DeepSeek-Balance-Whale-Widget), adapted here as a built-in Codex++ feature. The Codex desktop architecture also references [Yang-huai406/Codex-Whale-Public](https://github.com/Yang-huai406/Codex-Whale-Public). The upstream [LICENSE](assets/inject/upstream/whale-widget/LICENSE) and [PROVENANCE.md](assets/inject/upstream/whale-widget/PROVENANCE.md) are preserved; the media assets are not claimed as original work by this project.

## Aggregate and Direct

**Aggregate (recommended)** serves Codex, Claude Code, Claude Desktop, Grok, and more from one provider list through the local service. It is available while Codex++ is running. Model names in the local service come in three forms:

| Form | Goes to |
| --- | --- |
| `deepseek-v4-pro` | Any switched-on provider that offers it; when several do, they form an automatic group and fail over in order |
| `deepseek/deepseek-v4-pro` | Only that provider |
| `group/coding` | A group you created, using the group's routing mode |

Context windows are set per model in the provider details (one click fills them from public model data); a request longer than a provider's window is tried there last. To fall back to direct connections after quitting Codex++, turn on **Restore direct connections when Codex++ exits** under **Local service → Service & Agents**. Codex++ always switches back to direct mode before installing an update.

Codex picks one or the other at the top of **Codex → Connection & providers**; clicking **Use** on a provider while in Aggregate disconnects the local service first. Claude Code has the same choice; Claude Desktop always goes through the local service.

**Direct mode** keeps the original Codex provider setup. Official login, mixed API, and pure API are stored and switched separately:

| Mode | Purpose | Authentication boundary |
| --- | --- | --- |
| Official login | Use only the official ChatGPT / Codex account | Removes custom providers and API keys while preserving official login state |
| Official login + API | Keep official account features and plugins while routing model requests to a compatible API | Stores the key as a provider bearer token, not in pure API `auth.json` |
| Pure API | Use a custom Base URL and key without an official account | Maintains independent `config.toml` and API-key auth without mixing official credentials |
| Failover / Model aggregation provider | Two types: Failover (fail over or rotate across providers) and Model aggregation (bind catalog models to specific members) | Members can be ordinary API providers or signed-in official accounts; Failover supports failover, conversation round-robin, request round-robin, and weighted round-robin |

Each provider can configure Responses or Chat Completions, model lists, a test model, User-Agent, context windows, auto-compact limits, and enabled MCP servers, Skills, and Plugins. Chat Completions can be converted locally into the Responses protocol used by Codex.

Per-model windows accept values such as `1M`, `200K`, or plain integers. Codex++ generates a dedicated `model_catalog_json` for Codex.

With several providers, the model picker gets three columns: favorites and providers, that provider's models, and reasoning effort.

<p align="center">
  <img src="docs/images/readme-preview/model-picker-groups.png" alt="Model picker in three columns: favorites and providers on the left, that provider's models in the middle, reasoning effort on the right" width="560">
</p>

Provider switching saves the current profile before applying the target profile. Real API keys remain local and should never be posted in logs, screenshots, or issues.

## Codex Enhancements

- Session delete, bulk delete, Markdown export, and project move actions.
- Plugin marketplace unlock, plugin auto-expand, and model whitelist handling.
- Plain-text paste and startup acceleration.
- Dictation: use an independent OpenAI-compatible service or the local SenseVoiceSmall INT8 model (downloaded manually from the voice settings and run on-device), then record and insert or send the text.
- Conversation width, scroll restoration, thread IDs, native Fast unlock, and Goals.
- Next-step suggestions: manage toggles, API, model, item count, and timeout together on the Codex++ settings page in Codex.
- Skin management: search, preview, install, and image replacement for Dream Skin community themes.
- Custom image overlays and user scripts.

These are built-in extensions, switched on Codex's own **Codex++** settings page; another extension can take one over. Settings that depend on renderer injection generally require saving and restarting Codex from the Codex++ window.

<p align="center">
  <img src="docs/images/readme-preview/codex-builtins.webp" alt="The Codex++ settings page inside Codex (in Chinese), Models and plugins group: switches for plugin marketplace unlock, model whitelist unlock, forced native Fast and more" width="100%">
</p>

### Use native Fast controls with an API key

**Force unlock native Fast** gives pure API-key setups access to Codex's native **Standard / ⚡ Fast** controls for supported models, using the existing model and reasoning-effort menus.

1. In Codex, open the **Codex++** settings page: make sure **Enable built-in extensions** under General is on, then turn on **Force unlock native Fast** under Models & plugins (disabled by default).
2. Make sure the model declares a Fast service tier. In Direct mode, open that model's settings in **Codex → Connection & providers**, set **Fast (priority) support**, and save the model and provider. In Aggregate mode, a model group supports Fast when every member does.
3. Save the enhancement settings and restart Codex from Codex++. Open the model or reasoning-effort menu and select **⚡ Fast**, or select **Standard** to switch back.

Availability, speed improvements, and pricing depend on the provider. Enabling this feature does not automatically speed up every model. Disable the option and restart Codex to restore its original authentication checks.

## Updates and Packages

Codex++ publishes installers through GitHub Releases. Windows builds an NSIS installer, while macOS builds one universal DMG for Intel and Apple Silicon Macs. Each package contains one application executable.

When a new version is found, Codex++ downloads the installer in the background and shows its progress in a card at the bottom of the sidebar; nothing is installed until you click **Install**. You can also check for updates under **Settings → Version & updates**. Older macOS two-app versions require the manual migration described above.

## Data Locations

- Codex config: `~/.codex/config.toml`
- Codex auth state: `~/.codex/auth.json`
- Codex local database: prefers `~/.codex/sqlite/*.db`, falls back to legacy `~/.codex/state_5.sqlite`
- Codex++ state and logs: `~/.codex-session-delete/`
- Provider Sync backups: `~/.codex/backups_state/provider-sync`

`~/.codex` above refers to the Codex home directory: it follows the `CODEX_HOME` environment variable when set, and defaults to `.codex` under the user profile otherwise.

## FAQ

### Is it safe to install other people's extensions?

Featured and official extensions are reviewed by Codex++ and install the exact reviewed commit. Community extensions are not reviewed and authors can add any code in a new version: before install you see every capability they declare (background processes, hooks, gateway middleware, model tools, routes, page scripts), and installs start disabled. Only install extensions from authors you trust.

### The Codex++ menu does not appear

Enhancements are only attached to an official app launched from `Codex++`. If you opened Codex yourself from the Dock or Start menu, the Codex row in the sidebar says it is not attached and its button becomes **Restart & attach**; click it. If the menu is still missing, check the detected app path, launch status, and diagnostic logs under **Settings → Codex** (which includes install & maintenance) and **Settings → Logs & Diagnostics**. You can also turn on **Auto-take over Codex** on Home so Codex++ takes over when you open the official app yourself.

### Requests fail after switching providers

In Aggregate mode, click **Test** in the provider row's **⋯** menu: it sends a short request with the model (and key) you pick and shows the status code, time to first token, and the upstream error text. Failed requests under **Usage → Requests** show their reason as well. In Direct mode, run the model test or Provider Doctor from the provider detail page. Verify that the protocol, Base URL, key, and test model match. Pure API and official-login-plus-API use different authentication locations; do not manually copy `auth.json` between them.

### Agents cannot connect after I quit Codex++

In Aggregate mode agents keep pointing at the local service, which only runs while Codex++ is open; reopening Codex++ restores them. To fall back to direct connections on exit, turn on **Restore direct connections when Codex++ exits** under **Local service → Service & Agents**.

### Does it support Intel Macs?

Yes. The `macos-universal.dmg` package runs on both Intel and Apple Silicon Macs.

### Computer Use tools do not appear in my client

Restart the client after connecting. Fully quit Claude Desktop before connecting, because it rewrites its own config on exit. If the tools are still missing, check the config directory the client actually reads under **Config paths and custom folders** below the diagram in **Settings → Computer Use**, and make sure ChatGPT has Accessibility and Screen Recording permissions.

## Development

```bash
cd apps/codex-plus-manager
npm ci
npm run check
npm run vite:build

cd ../..
cargo fmt --all -- --check
cargo test
cargo build --release
```

Project structure:

```text
apps/
  codex-plus-launcher/          Embedded launch and injection runtime library
  codex-plus-manager/           Unified Tauri application (codex-plus-plus)
assets/inject/
  renderer-inject.js            Enhancement script injected into Codex
crates/
  codex-plus-core/              Launch, injection, config, update, install, bridge
  codex-plus-data/              Session data, export, Provider Sync
scripts/installer/
  windows/CodexPlusPlus.nsi     Windows NSIS installer
  macos/package-dmg.sh          macOS DMG packager
```

## License

Copyright (C) 2026 BigPizzaV3

CodexPlusPlus is licensed under the [GNU Affero General Public License v3.0](LICENSE), SPDX identifier `AGPL-3.0-only`. Modified versions that are distributed or offered to users over a network must provide the corresponding source code as required by AGPLv3.

The license covers CodexPlusPlus code only. It does not grant rights to OpenAI, ChatGPT, Codex, Anthropic, Claude, xAI, or Grok trademarks, application assets, or other third-party content.

## Compatibility

Codex++ depends on each official app's page structure, CDP behavior, and local data formats. Official app updates may require injection updates. Keep backups before changing provider configuration or local session data.
