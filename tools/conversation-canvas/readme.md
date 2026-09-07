# Conversation Canvas / 对话脉络

An opt-in Codex++ userscript for following a long conversation as a task tree. Open **对话脉络** in the conversation header to view goals, alternative approaches, failed attempts and the current direction on a zoomable canvas. Click a node for its evidence and jump to the original message.

## Features

- Conversation-area tree with curved edges, pan, zoom, branch folding and current-node focus.
- User-triggered organization in a background Codex conversation (GPT-5.6 Luna, medium), or through an OpenAI-compatible Chat Completions API.
- Paginated history, complete text segmentation, durable batch checkpoints and incremental updates. Each batch must cover every supplied source fragment before it commits.
- External API streaming progress and up to two automatic retries for temporary failures. Official DeepSeek V4 models default to non-thinking mode, with a provider-default option in settings.
- Local tree storage in IndexedDB. API keys remain in memory unless the user explicitly chooses plaintext local persistence.

## Build and test

Requires Node.js 22 or newer. No npm dependencies are required.

```powershell
cd tools/conversation-canvas
npm test
npm run build
```

The generated script is `public/canvas.user.js`. Builds do not read local conversations or embed annotations. The release is reproducible from this directory alone.

## Install in Codex++

1. Build the script, or use the generated file included in this directory.
2. In Codex++ **用户脚本**, install `public/canvas.user.js` and enable it. If updating an existing installation, replace that script rather than enabling a duplicate.
3. Choose **重新加载用户脚本**, return to a task and open **对话脉络** in its header.
4. Use **整理设置** to choose Codex background organization or enter your API endpoint, model and key. Click **整理脉络** once to process the available history.

Drag the canvas to pan, use the wheel or +/- to zoom, and **适应视图** to fit the tree. New messages remain pending until the next incremental organization. Disabling and reloading the script removes its UI; it does not erase locally saved trees.

The model identifies the tree structure. A saved chain remains a chain; the renderer does not fabricate branches. Source coverage establishes traceability, not the accuracy of every model conclusion.

## Architecture and compatibility

- `model.mjs`, `tree-markup.mjs`, `tree-canvas.mjs`: grounded graph model and view helpers.
- `long-organizer.mjs`: segmentation, selected prior context, atomic batch merging and resume.
- `external-api.mjs`: provider options, streaming decoding, retries and cancellation.
- `native-history.mjs`, `src/native-sidechat.js`, `src/native-api.js`: desktop history, background task and HTTP adapters.
- `src/canvas-panel.js`: native header entry, canvas UI and settings; `src/canvas-store.js`: local persistence.
- `build-userscript.mjs`: dependency-free userscript bundler.

The desktop adapters currently target **Windows Codex Desktop 26.901.6511 with Codex++ 1.2.56**. They call private desktop module exports and contain versioned asset names; other desktop versions may need adapter changes. This is an optional experimental tool, not a general compatibility promise. A future built-in integration should expose stable host APIs for paginated history, background organization and message navigation.

The script covers user/assistant text, not tool output or image semantics. Sending a batch to an external API shares that text with the configured provider. An interrupted request may still incur provider charges; retries can be billed again. Completed batches are retained. No service keys, session files, local diagnostics, extracted desktop bundles or personal annotations are included here.

## Synthetic UI test

```powershell
npm run build
npm run dev
```

Open `http://127.0.0.1:47834/host?thread=11111111-1111-1111-1111-111111111111&tree&canvas` for the branching layout fixture. For API recovery, omit `&canvas` and add `&apiretry`; select external API, enter a test HTTPS endpoint/model and the synthetic key `fixture-key`. The mock returns 503 once, then streams a successful reply. This server binds only to loopback and mocks the native bridge. It never calls the configured provider.

## Validation

63 distributable Node tests cover history paging, long-message coverage, persistence, invalid sources, task isolation, paused requests, SSE boundaries, retries and 10,000-node tree layout. The original development workspace also has a legacy local-server test, which is intentionally excluded with that obsolete server.

Synthetic browser checks cover strict CSP (`frame-src 'none'`, `connect-src 'none'`), zoom/pan, folding, source navigation and one-click recovery after 503. Earlier desktop testing confirmed background organization and source navigation on the target version. The 0.3.1 API speed change has not been benchmarked against a live DeepSeek request. No claim is made here that the upstream Rust workspace tests or clippy have run for this addition.

## License

AGPL-3.0-only, consistent with CodexPlusPlus. See `license` and the upstream contribution policy.
