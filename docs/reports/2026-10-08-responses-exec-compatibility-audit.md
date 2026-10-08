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

See [the proposed behavioral contract](../specs/responses-exec-compatibility.md).

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

Only after the explicit validation rejection, the proposed retry uses the flat
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
`{"input":"text(\"compatibility\")"}` is proposed to become a client
`custom_tool_call` containing the raw string `text("compatibility")`.
`call_id` remains unchanged; item IDs and references must be mapped consistently.
The proxy does not execute that string.

Historical matching calls and results must be translated on each adapted
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

New coverage comprises 15 strict translation/parser tests, 14 SSE tests,
5 transport/cache tests, 6 local-upstream integration tests, and one actual
helper JSON/SSE test: 41 tests in total. The helper fixture enables channel
queueing and bounds completion time to detect a self-lock during negotiation.

The initial HTTP rejection reproducer failed with `400 Bad Request` before
integration and now passes. The retained-metadata reproducer also failed before
its fix. A deliberate temporary removal of the duplicate-key guard makes the
strict-parser regression fail; the guard is restored in the submitted code.

Validation commands are the core test/check commands and scoped rustfmt command
in the specification. Existing warnings and pre-existing opt-in ignored tests
are retained, not disabled to obtain a passing result. Submitted files do not
include private configuration, session logs, generated binaries, or unrelated
worktree changes.

Mocks verify protocol behavior, not model intelligence or supplier conformance.
The adapter is not installed into the user's running application by this PR.

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
