# Responses Exec Compatibility: Source-Impact Audit

Date: 2026-10-08

Audited upstream revision: `1db62e9239ce564e670219922584f66f35508ef8`.

Status: runtime adapter implemented for existing local-proxy traffic.
The source-evidence table and 331-test baseline below describe the original
upstream revision. Implementation verification is recorded separately.
No production configuration changes or real-provider certification are included.

## Problem

A Responses-compatible endpoint can expose both models accepting native custom
tools and models rejecting the custom Code Mode `exec` tool. An observed HTTP
validation error was:

```text
Unsupported custom tool: 'exec'. Only 'apply_patch' is supported.
```

This proves rejection somewhere on that request's upstream path, not which
backend layer produced it. It does not prove that every third-party model is
incompatible, or that wrapping the tool will make the model generate correct
JavaScript.

Disabling Code Mode globally would affect compatible models on the same supplier.
Inferring capability from model names or a supplier-wide flag has the same
granularity problem. The proposed alternative keeps native behavior by default,
then negotiates only the explicitly rejected tool for the actual upstream model.
This implementation addresses that observed custom `exec` rejection, not every
native-tool validation error.

See [the implemented behavioral contract](../specs/responses-exec-compatibility.md).

## Source Findings

Paths are relative to `crates/codex-plus-core/`; line references describe the
audited revision, not a promise that future revisions retain those line numbers.

| Concern | Source evidence | Consequence for an implementation |
| --- | --- | --- |
| Native request path | `src/protocol_proxy.rs:1804`, `:1824` select Responses without the Chat conversion; existing ID/image handling still applies. | Add a narrow Responses adapter, not a whole Chat conversion. Preserve opaque reasoning and unrelated fields. |
| Actual served response path | `src/launcher.rs:1699`, `:1799`, `:1883` pass native Responses SSE/JSON through; `src/protocol_proxy.rs:1970` is a separate whole-body path. | Both paths need request-specific reverse conversion. Tests of the Chat converter alone cannot prove this behavior. |
| Retry and failover | `src/protocol_proxy.rs:1353` records rotation success/failure before returning or failing over. | Perform bounded negotiation before that accounting. Do not penalize a supplier for a recovered validation rejection. |
| Queue and rate accounting | `src/channel_protection.rs:56` acquires an owned queue guard and reserves request capacity; the response retains its permit at `src/protocol_proxy.rs:1399`. | A retry must count as a physical request without waiting on its own held queue lock. Cancellation must release permits. |
| Routing and aggregation | `src/protocol_proxy.rs:1429`, `:1475`, `:1820` route/override model identity. | A capability key must use the final endpoint, relay, and actual upstream model, not the UI-selected model alone. |
| Effective credentials | `src/relay_headers.rs:94`, `:117` apply custom headers, including Authorization overrides; `src/protocol_proxy.rs:1938` uses this helper. | Cache identity must cover effective auth and tenant/organization headers, not only the profile API key. Neither values nor their digests belong in diagnostics. |
| Existing custom-input reconstruction | `src/protocol_proxy.rs:6002` returns raw arguments when JSON parsing fails. | Do not reuse that permissive behavior for executable Code Mode input. Require a bounded, duplicate-key-aware string wrapper parser. |
| Proxy coverage | `src/settings.rs:892` distinguishes transport proxying from session-provider proxying; `src/relay_config.rs:965` preserves direct Responses transport. | The first version covers existing proxy traffic only. A direct-transport migration needs a separate design and regression review. |
| Existing DeepSeek behavior | `src/relay_config.rs:3035` contains an official-endpoint workaround; `tests/relay_config.rs:5545` preserves third-party Code Mode/catalog behavior. | Leave those configuration rules unchanged in this proposal. Do not globally disable features for mixed-model relays. |

## Sanitized Protocol Example

This example is a contract illustration, not a live test or a complete API
request. Omitted request fields and unrelated tools stay intact.

Native tool declaration:

```json
{
  "type": "custom",
  "name": "exec",
  "description": "Execute a JavaScript Code Mode program."
}
```

Only after the explicit validation rejection, the retry uses the flat
Responses function shape, not the nested Chat Completions function shape:

```json
{
  "type": "function",
  "name": "exec",
  "description": "Execute a JavaScript Code Mode program supplied in input.",
  "parameters": {
    "type": "object",
    "properties": { "input": { "type": "string" } },
    "required": ["input"],
    "additionalProperties": false
  },
  "strict": true
}
```

A matching complete upstream `function_call` with arguments
`{"input":"text(\"compatibility\")"}` becomes a client
`custom_tool_call` containing the raw string `text("compatibility")`.
`call_id` remains unchanged; item IDs and references must be mapped consistently.
The proxy does not execute that string.

Historical matching calls and results are translated on each adapted
request. The client history remains canonical, so switching back to a native
model does not inherit the compatibility wrapper.

## Verification Already Performed

The following existing baseline suites passed at the audited revision:

```powershell
cargo test --offline --locked -p codex-plus-core --test protocol_proxy
cargo test --offline --locked -p codex-plus-core --test responses_catalog_identity --test relay_config
```

| Suite | Passed | Failed |
| --- | ---: | ---: |
| `protocol_proxy` | 140 | 0 |
| `relay_config` | 182 | 0 |
| `responses_catalog_identity` | 9 | 0 |
| Total | 331 | 0 |

These are unchanged-code baselines, not tests of the proposed adapter. Existing
warnings include unused imports/dead code and duplicate test attributes; they
were not altered for this proposal.

## Implementation Coverage

The adapter and regression tests now cover the following acceptance criteria:

- A failing explicit-rejection reproducer must turn green for served JSON and
  SSE, including a second tool round and history replay.
- Native GPT traffic and GPT-to-adapted-model-to-GPT switching must remain intact.
- Nonmatching errors, server-managed state, compaction, and direct transport must
  retain their existing behavior.
- Partial UTF-8, malformed/duplicate keys, ambiguous tool names, truncated
  streams, interleaved calls, and buffer limits require negative tests.
- Cache separation, TTL/capacity, queue release, retry request accounting,
  rotation, cooldown, and failover require focused tests.
- Core build/test checks and implementation review are recorded below.

## Independent Design Review

A separate read-only reviewer inspected the proposal against the audited source.
It found no Critical design finding, but required more precise contracts before
implementation. The proposal was amended to address each:

| Finding | Contract clarification |
| --- | --- |
| HTTP success is not a completed valid stream. | Cache and adapted rotation success require complete validated output; truncation and parser failures evict evidence, cancellation is neutral, and native accounting stays unchanged. |
| API key alone is not the effective identity. | Fingerprint custom authentication and tenant/organization headers, including no-auth and relevant beta-header distinctions; do not log digests. |
| Tool-choice and history references can still name the original custom tool. | Translate recognized named choices; guard unknown choices, orphan results, duplicate call IDs, and unresolved references. |
| A generic JSON value parser loses duplicate-key evidence. | Require duplicate-key-aware validation, explicit buffer/concurrency/deadline bounds, and suppression of unvalidated input in every executable item event. |

Optional comments were also incorporated: recognize conservative structured
rejection fixtures, limit negotiation retries across the whole client request,
and distinguish new transformations from existing proxy normalization.

This initial review covered the proposed contract. A subsequent independent
implementation review identified five Required findings: unwrapped custom exec
in adapted JSON, conflicting SSE identities, duplicate terminal delivery,
incomplete terminal snapshots, and retained metadata outside the state budget.
Each finding was corrected and guarded by negative tests. The final resource
budget correction was checked locally after the reviewer identified it.
No real-provider validation has been performed.

## Runtime Verification

The initial implementation introduced 15 strict translation/parser tests, 14 SSE tests,
5 transport/cache tests, 6 local-upstream integration tests, and one actual
helper JSON/SSE test: 41 tests in total. The helper fixture enables channel
queueing and bounds completion time to detect a self-lock during negotiation.

The initial HTTP rejection reproducer failed with `400 Bad Request` before
integration and now passes. The retained-metadata reproducer also failed before
its fix. A deliberate temporary removal of the duplicate-key guard makes the
strict-parser regression fail; the guard is restored in the submitted code.

Validation commands are the core test/check commands and scoped rustfmt command
in the specification. A clean detached worktree at runtime commit `de74c3a`
completed the entire default core test suite: **1,470 passed, 0 failed,
7 pre-existing opt-in tests ignored**. Scoped Rust formatting and the complete
submitted diff check also passed.

Existing warnings and pre-existing opt-in ignored tests
are retained, not disabled to obtain a passing result. Submitted files do not
include private configuration, session logs, generated binaries, or unrelated
worktree changes.

Mocks verify protocol behavior, not model intelligence or supplier conformance.
The adapter is not installed into the user's running application by this PR.

## Follow-up Corrections

Additional review corrected four runtime defects:

- Historical calls and tool results can omit item `id`. Preserve that absence
  while translating types and correlating results by `call_id`; requiring an
  output item ID prevented a normal second Code Mode round from negotiating.
- Adapted JSON output must reject duplicate call IDs across both function and
  custom tools, not just function calls.
- SSE argument completion is not item completion. Emit done only after the
  completed item or successful terminal snapshot, preserve final metadata, and
  require stable nonnegative integer output indices.
- Cache-key request-build failures must participate in normal aggregation
  failover. Construct the key inside the send error boundary and only for
  eligible exec requests; unrelated native requests must not gain a new early
  return path.

Regression coverage includes omitted historical IDs, shared JSON call IDs,
arguments-done preceding completed metadata, output-index conflicts at each
completion surface, and failover after an invalid endpoint with and without
exec. The two-round model-switch fixture now switches back using the same
canonical history, rather than a fresh request. A separate read-only review of
these corrections found no blocking issue; it did not perform live-provider
validation.

Scope wording now explicitly excludes ordinary direct Responses relays without
a proxy routing requirement, as well as regular follow-up requests carrying
compaction history. These paths still report the original upstream rejection;
the PR does not silently reroute them or disable Code Mode globally.

Current coverage is 17 strict translation/parser tests, 16 SSE tests, 5
transport/cache tests, 6 local-upstream compatibility tests, one actual helper
JSON/SSE test, and one request-build failover regression: **46 tests**.
The complete default core suite passed with **1,475 passed, 0 failed,
7 pre-existing opt-in tests ignored**. `cargo check --offline --locked -j 1 -p
codex-plus-core`, scoped formatting, and `git diff --check` passed.
Inverting the optional-history-ID guard made its focused regression
fail with `tool item lacks id`; the original guard was restored and all 38
adapter unit tests passed again.

The initial default parallel run and one focused suite run also encountered
HTTP 502 in the existing `aggregate_proxy_fails_over_to_next_member_in_same_request`
fixture, followed by shared-lock poisoning. Isolation and the serial suite
passed; after adding response-body diagnostics, both the default proxy suite
and complete default core suite passed. That diagnostic change does not prove
the intermittent failure is fixed. The fixture writes only 500 and 200, and
the proxy forwards the received status; external proxy/interception or fixture
timing remains an unconfirmed hypothesis. No production HTTP proxy policy was
changed, and this residual test instability is not hidden by disabling tests.

## Current-Main Recovery and HTTP-Date Review (2026-10-14)

This section supersedes the follow-up verification attached to `c588621`.
That submission used an older PR history and did not preserve the
unconditional `Retry-After` wait from the author's reviewed revision
`e5ebafb`. Parser-only unit coverage did not detect the behavioral regression.

The author's reviewed PR history was recovered and rebased onto upstream
`main` at `f6092f2`. `git range-diff` confirms all 15 PR patches were preserved
through the rebase. Upstream session-header forwarding, official-login
transport, aggregate routing, and deliberate removal of channel protection
remain intact.

The fix for [comment 6150422715](https://github.com/BigPizzaV3/CodexPlusPlus/pull/2411#issuecomment-6150422715)
now changes the actual exec-negotiation path in `protocol_proxy.rs`:

- Preserve delta-seconds, and parse HTTP-date with the already-locked
  `httpdate` dependency.
- Compute the remaining duration before retrying; past dates produce zero
  delay. Missing, invalid, and non-text header values retain the no-wait
  behavior.
- Wait independently of retired channel settings, after an exact rejection
  is confirmed and before the one permitted physical compatibility retry.
- Keep the terminal-SSE correction: complete adapted responses emit `[DONE]`
  and close downstream without waiting for an upstream socket to close.

Regression evidence:

- The future-date HTTP fixture records the server's advertised deadline and
  actual arrival of the adapted request. It failed before the fix: the retry
  arrived before the deadline, with the test completing in 0.21 seconds.
- A second HTTP fixture verifies successful negotiation with a past date.
- Four parser unit tests cover integer/zero values, a future date with a
  fractional remaining duration, a past date, and missing/invalid values.
- The held-open upstream SSE fixture failed before the terminal correction,
  then guards downstream `[DONE]` delivery and closure.

Current verification:

| Check | Result |
| --- | --- |
| Core library tests, default parallel execution | 831 passed, 0 failed, 3 existing ignored |
| `responses_exec_compatibility` | 9 passed, 0 failed |
| Manager `npm test` | 608 passed, 0 failed, 1 existing skipped |
| Manager `npm run check` | Passed |
| Manager `npm run vite:build` | Passed |
| Native-browser inspector assembly check | Passed |
| Submitted diff whitespace check | Passed |

An initial full-library run exposed an overly broad two-second timeout in
the new SSE fixture: request startup had consumed its closure deadline.
The fixture now waits for the terminal event before starting the same
two-second closure deadline, awaits aborted fixture tasks, and releases
shared state before assertions. The default parallel suite then passed;
no existing test was disabled.

The remote `c588621` history is retained as a merge parent without importing
its obsolete tree. Its two functional fixes are represented by the new
current-main commits; this recovery deliberately does not revive channel
protection or replace newer upstream code. This permits a normal fast-forward
push rather than force-pushing the PR branch.

### Final Verification (2026-10-14)

The implementation head for these results is
`87054ccf50837a60f63d7d3fd92e9f4e2b310b8b`. The verification below combines
the completed previous session with the resumed startup check. A fresh fetch
at 20:44 China Standard Time still found upstream `main` at `f6092f2`;
the implementation was behind by zero commits.

| Check | Result |
| --- | --- |
| Complete Core suite, single-threaded tests | 61 targets; 1857 passed, 0 failed, 8 existing ignored |
| `protocol_proxy` integration tests | 173 passed, 0 failed |
| `launcher` integration tests | 101 passed, 0 failed |
| `dream_skin_package` integration tests | 11 passed, 0 failed |
| Windows artifacts CI | Success: frontend checks, workspace Rust tests, release application, NSIS installer, artifact upload |
| macOS universal DMG CI | Success: universal build, bundle structure check, artifact upload |
| Local native release artifact | Produced at 20:36:17 China Standard Time; 72,192,000-byte executable |
| Resumed isolated startup | Responding process with a window handle, `manager.start` for version `1.8.0`, and loopback guard at `127.0.0.1:49319` |

The complete Core command was:

```powershell
cargo test --offline --locked --no-fail-fast -j 2 -p codex-plus-core -- --test-threads=1
```

Both CI jobs belong to [run 38556910278](https://github.com/BigPizzaV3/CodexPlusPlus/actions/runs/38556910278),
whose head is exactly `87054cc`. Earlier CI on `e5ebafb` is not used as
evidence for this implementation. Any subsequent documentation-only head
must be distinguished from this tested implementation head.

The local native build command was:

```powershell
$env:CARGO_INCREMENTAL = '0'
$env:SHERPA_ONNX_LIB_DIR = 'C:\Users\Rat\AppData\Local\Temp\codexpp-pr2411-current-main-20261014\target\sherpa-onnx-prebuilt\sherpa-onnx-v1.13.8-win-x64-static-MT-Release-lib\lib'
cargo build --offline --locked --release -j 2 -p codex-plus-manager --target-dir C:\Users\Rat\AppData\Local\Temp\codexpp-pr2411-native-target-20261014
```

The original terminal session was no longer attachable on resume, so its
final exit output is unavailable. The executable and dependency manifest
survived, and the resumed check directly launched that executable. Its SHA-256
is `fbf6226acb10b242184fe4b03ee344ac605d03c4b62f39d81135c7c4f9c24cf8`.
Existing compiler warnings were recorded in the original session, not
silenced or repaired as part of this PR.

Temporary source and smoke-script files were no longer present on resume.
The pushed implementation was restored into a persistent local checkout;
no missing local changes were inferred or recreated as source changes.
The restored checkout has no frontend dependencies: a resumed inspector
assembly check could not run because `esbuild` was absent. Dependencies were
not installed for this documentation-only change, and the complete suites
were not rerun on resume. The earlier local and implementation-head CI
results above remain separately identified.
The smoke script needed two local corrections: avoid PowerShell's read-only
`$HOME` variable, and explicitly set the exact guard-port override. The first
launch used the existing file-lock fallback; after restarting only that
test process, the final process listened on port 49319 without fallback.

The final smoke instance uses a dedicated `CODEX_HOME` and
`CODEX_PLUS_STATE_DIR`. Auto-takeover, provider sync, enhancements, and Weixin
connection are disabled in its settings. The saved launcher remains stopped;
neither helper port 49321 nor debug port 49329 was started. This checks manager
startup, not a real supplier request or the full interactive UI. The installed
application was not replaced.

Two complete parallel Core runs had timed out in the unchanged
`dictation_disconnect_stops_waiting_for_upstream` fixture while waiting for
the mock's first request, before its cancellation assertion. Isolation,
all 34 dictation-related tests, the complete single-threaded library, the
complete single-threaded Core suite, and the new-head Windows workspace CI
passed. The root cause remains unconfirmed; the fixture was not changed or
ignored. This local test-instability risk remains open.

Real-provider behavior and model-generated program correctness remain
uncertified.

## Residual Risks and Rollback

Accepting the wrapper proves protocol acceptance only. A particular model may
still produce invalid programs or fail to follow Code Mode instructions. A
supplier may also change its backend behind a stable model name; bounded cache
expiry limits stale evidence but cannot eliminate that uncertainty.

Streaming adaptation must never emit a partially validated program. It must
buffer only bounded tool state and fail explicitly on incomplete input without
replaying a request after output delivery. Server-state and compaction bypasses
avoid pretending that client-side rewrites can repair upstream opaque state.

Rollback is limited to the three adapter modules and their proxy integration
points. It does not require changing catalogs, supplier configuration, or user
feature flags.
