# Token Usage Reconciliation

Codex++ exposes one local bridge route for the companion user script:

```text
/token-usage/events
```

The bridge does not provide prices and does not contact a billing provider. Model prices,
long-context thresholds, and multipliers remain optional local settings in the user script. An
empty price table and multiplier `1` are the defaults.

## Data flow

1. `MessagePort` events update the current page immediately.
2. At script startup, the bridge scans recent rollout JSONL files once and builds per-file tail
   cursors.
3. Every 60 seconds, the script asks for incremental reconciliation. Codex++ checks rollout file
   metadata and reads only new bytes from changed or newly created files.
4. Responses routed through the local protocol proxy append terminal usage and retry attempts to a
   bounded JSONL ledger.
5. The script matches MessagePort, rollout, and proxy events one-to-one using Input, Cached Input,
   Output, and a bounded time window. A model-name mismatch is accepted only when those fields
   identify one unique candidate; proxy-only Cache Write and Reasoning values replace the live
   request instead of creating a second request.

This captures subagent rollout files without repeatedly reading all session history.

## Script installation

Install `Codex Reconciled Token Usage` from the Codex++ Script Market after using a Codex++ build
that contains this bridge. The market script is:

```text
scripts/codex-reconciled-token-usage.js
```

Release metadata:

```text
Version: 1.0.0
Author: QingJunXue
```

After installation, reload user scripts or restart Codex++ once. Existing data from
`__myCodexTokenStatsV1` and existing local price settings are migrated once to the new storage
namespace. No relay CSV or provider-specific API is required.

## Bridge request

```json
{
  "since": "2026-07-28T00:00:00.000Z",
  "days": 7,
  "limit": 10000,
  "includeRollout": true,
  "rolloutIncremental": true,
  "proxySinceMs": 1785196800000,
  "proxyOffset": 12345,
  "proxyGeneration": "ledger-generation"
}
```

- Initial reconciliation uses `rolloutIncremental: false` and empty cursors.
- Periodic reconciliation uses `rolloutIncremental: true` and the returned cursors.
- A changed proxy ledger generation sets `proxyReset: true`; the script then performs one full
  rebuild.

## Bridge response

```json
{
  "events": [],
  "warnings": [],
  "nextSince": "2026-07-28T00:00:00.000Z",
  "proxyNextSinceMs": 1785196800000,
  "proxyNextOffset": 12345,
  "proxyGeneration": "ledger-generation",
  "proxyReset": false,
  "missingUsage": 0
}
```

Each event contains model, timestamp, status, source, response ID when available, and Token counts
for input, cached input, cache write, output, reasoning, and total.

Request status meanings:

- `completed`: terminal model response.
- `failed` or `incomplete`: final request without a successful terminal response.
- `retry`: an upstream attempt failed and Codex++ continued with another relay candidate.

## Disk behavior

- The proxy usage ledger is append-only during normal requests, rotates at 20 MB, and retains three
  archives.
- Incomplete JSONL tail records do not advance the read cursor.
- Rollout files are read fully once at startup, then tailed incrementally.
- Diagnostic logs rotate at 20 MB and retain three archives.
- High-frequency successful `bridge.request`, `bridge.response`, `bridge.resolve_start`, and
  `bridge.resolve_ok` events are not persisted.
- No explicit `fsync` is performed per Token event.

## Privacy and limits

- The usage ledger does not store prompts, response text, API keys, or upstream URLs.
- Rollout files currently do not expose cache-write Token fields in every Codex build. Missing values
  remain zero rather than being inferred from input Tokens.
- Official-login traffic that does not pass through the protocol proxy is reconciled from rollout
  tails. API relay traffic additionally receives request-attempt and terminal usage events from the
  proxy.
- Cost estimates remain local and depend on user-entered request-level prices and thresholds.
