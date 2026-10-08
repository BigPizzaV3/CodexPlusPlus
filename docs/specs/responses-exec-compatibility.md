# Spec: Responses Exec Compatibility

Status: proposed design; no runtime implementation is included.

Source-impact audit: [Responses exec compatibility audit](../reports/2026-10-08-responses-exec-compatibility-audit.md).

## Objective

Allow a mixed-model Responses supplier to use Code Mode without a global
disable flag, a mandatory model-list setting, or a restart when switching models.
Preserve native behavior unless the current upstream explicitly rejects the
top-level custom `exec` tool. Protocol compatibility is not a guarantee of a
model's ability to write correct Code Mode programs.

## Assumptions and Scope

- The request already passes through the Codex++ local protocol proxy.
- The first version supports full-history replay requests, not server-managed
  state using `previous_response_id` or `conversation`.
- The client remains responsible for approvals and tool execution. The adapter
  translates protocol data only and never executes code.
- A model name or `/models` listing is not capability evidence.
- Existing direct Responses transport and the official DeepSeek configuration
  workaround remain unchanged. Those paths are not covered by this adapter.

## Behavioral Contract

1. Add no compatibility transformations to the native request by default.
   Existing proxy normalization and configured image handling still apply.
2. Negotiate only on a structured HTTP 400 validation response explicitly
   rejecting custom `exec`, before any response has been delivered to the client.
   Require the `invalid_request_error` type and an exact rejected tool name in a
   recognized message/code fixture. A supplied tool index must identify that
   declaration; generic "unsupported tools" text is insufficient.
   Generic errors, authentication failures, throttling, timeouts, and HTTP 200
   stream errors do not trigger negotiation. Error inspection must preserve
   the original response for existing error handling when no match is found.
   Bound negotiation error-body inspection to 64 KiB and five seconds. Exceeding
   either bound bypasses negotiation without losing buffered bytes or changing
   the original error's status/body.
3. Allow at most one negotiation retry across the entire client request,
   including failover and cooldown loops. Retry the same upstream with a flat Responses function schema
   wrapping `exec` input in a required string property named `input`.
   Preserve unrelated tools, including custom `apply_patch`. A conflicting
   function name or ambiguous tool declaration disables adaptation. Translate
   the simple named custom `exec` tool choice to its function equivalent; bypass
   adaptation for an unrecognized tool-choice shape referencing `exec`.
4. Convert only matching historical custom `exec` calls and their outputs.
   Preserve `call_id`, ordering, and consistent item-ID references. Leave the
   canonical client history and unrelated reasoning or compaction data intact.
   Match results through the original call identity, not output type alone.
   Orphan results, duplicate call IDs, item-ID collisions, and unresolved
   references disable adaptation rather than risking a misassociated result.
5. Translate matching upstream function calls back into custom `exec` calls.
   Both complete JSON responses and SSE responses must work. Buffer arguments
   until they can be parsed and validated as a single object with exactly one
   string `input`; malformed, incomplete, or oversized input must never become
   executable raw text. This includes items in `output_item.done` and terminal
   response output, not only argument-delta events. Reject duplicate `input` keys, not just extra distinct
   keys. Preserve unrelated response content and events. Bound each argument
   buffer to 1 MiB, total pending stream state to 4 MiB, and in-flight bridged
   calls to 64; exceeding a bound fails the adapted response explicitly.
   On parsing failure, truncation, or an unsuccessful terminal event, suppress
   unvalidated executable input, emit at most one client `response.failed`
   terminal event, and close the stream without retry.
6. All transformation context is per request. Never infer tool identity from a
   global name map, and never substitute a shell tool for the Code Mode executor.
7. Cache adaptation only after a complete, validated adapted response, not merely
   successful HTTP headers. For SSE, require the terminal successful response
   event and validated completion of all bridged calls. Cache keys include
   relay identity, final normalized endpoint, final upstream model after routing,
   tool fingerprint, and a non-logged effective-authentication/header identity
   digest. Fingerprint the final Authorization/custom tenant and organization
   headers, not just `RelayProfile.api_key`, since custom headers may override
   authentication. Distinguish no-auth requests and relevant beta headers.
   Do not log either credential values or their digests. Bound the cache
   to 256 entries with a 30-minute TTL. Never store or log plaintext credentials.
   An adapted HTTP success proves acceptance, not correct tool execution.
8. Stateful and compaction requests bypass both negotiation and cached adaptation. Preserve
   their existing request and error behavior; do not attempt to rewrite upstream
   server history.
9. The physical retry participates in channel queueing and rate accounting
   without reacquiring a lock while holding it. Recoverable validation rejection
   does not count as a supplier failure. Adapted rotation success is recorded
   only on a complete validated response; parsing/truncation or upstream failure
   records one failure for that attempt and evicts its compatibility entry.
   Client cancellation is neutral, creates no cache evidence, and releases the
   permit. Cooldown remains based on configured HTTP statuses, not synthetic
   parser errors. Native paths retain their existing accounting behavior.
   Other candidate attempts retain ordinary failover accounting.
10. Do not replay a request after output delivery. Diagnostics describe protocol
    adaptation without logging request bodies, generated programs, or secrets.

## Tech Stack

Use existing Rust, Tokio, reqwest, serde_json, and SHA-256 dependencies.
Do not add packages or change frontend assets.

## Commands

Run from the repository root:

```powershell
cargo test --offline --locked -p codex-plus-core --test protocol_proxy
cargo test --offline --locked -p codex-plus-core --test responses_catalog_identity --test relay_config
cargo test --offline --locked -p codex-plus-core
cargo check --offline --locked -p codex-plus-core
cargo fmt --all -- --check
git diff --check
```

## Project Structure

- `crates/codex-plus-core/src/responses_exec.rs`: focused protocol translation,
  bounded cache, and SSE adaptation; final module name may follow local patterns.
- `crates/codex-plus-core/src/protocol_proxy.rs`: request negotiation before
  failure accounting, routing-aware context, and whole-body response handling.
- `crates/codex-plus-core/src/launcher.rs`: actual served JSON and SSE paths.
- `crates/codex-plus-core/tests/protocol_proxy.rs`: local upstream integration
  tests; internal unit tests cover strict parsing and event assembly.
- This document: acceptance contract and scope exclusions.

## Code Style

Follow existing Rust formatting and error handling. Prefer a focused typed
context over new global configuration or a generic capability framework.
Strict input validation should have explicit failure semantics. The example
below illustrates the shape/type check only; an implementation must additionally
reject duplicate JSON keys and enforce the size limits before allocating:

```rust
fn decode_exec_input(arguments: &str) -> anyhow::Result<String> {
    let value: serde_json::Value = serde_json::from_str(arguments)?;
    let object = value.as_object().ok_or_else(|| anyhow::anyhow!("expected object"))?;
    anyhow::ensure!(object.len() == 1, "unexpected exec arguments");
    let input = object.get("input").and_then(serde_json::Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("expected string input"))?;
    Ok(input.to_owned())
}
```

## Testing Strategy

Use existing Rust tests, tempfile, and local mock HTTP servers. Begin with a
failing reproducer and confirm it fails for the rejected-tool behavior.

- Native GPT requests and responses remain unchanged; test GPT to adapted model
  to GPT switching with the same supplier.
- Explicit rejection produces exactly one adapted retry. Generic 400, 401,
  429, timeout, and stream errors do not.
- Verify final-model identity after model routes and aggregation overrides;
  unrelated models, endpoints, credentials, and tool declarations stay isolated.
- Cover two tool-call rounds, historical results, call IDs, and item references,
  including orphan results, collisions, and named/unsupported tool choices.
- Cover JSON and SSE, fragmented UTF-8, escapes, interleaved calls, empty input,
  malformed arguments, duplicate completion signals, and truncated streams.
- Verify retained custom `apply_patch`, unrelated tools, opaque reasoning,
  compaction bypass, and pass-through stateful requests.
- Verify effective custom authentication/tenant-header cache isolation,
  queue release, retry rate accounting, final-only adapted failure statistics,
  failover behavior, cache capacity and expiry, and cancellation.
- Prove the reproducer detects a deliberate regression to the retry guard or
  strict parser. Do not weaken existing assertions to obtain passing results.

## Non-Goals

This proposal does not broaden tool coverage, migrate direct transport, rewrite
server-managed history, introduce configuration/UI changes, or install a test
build. It does not globally disable Code Mode, classify support by model brand,
execute code in the proxy, accept malformed arguments, or probe paid models.

## Success Criteria

- The explicit rejection reproducer passes in JSON and SSE paths, including
  subsequent history replay, with no restart or global feature change.
- All affected baseline tests pass, along with new targeted tests and core build
  checks. Report pre-existing warnings separately.
- An independent implementation review covers execution-input validation, SSE lifecycle,
  history identity, cache isolation, and channel/rotation effects. Resolve all
  blocking findings and rerun the affected checks.
- Implementation PR notes contain sanitized reproduction, verified coverage,
  exclusions, and residual risks. Local mock tests are not real-provider
  certification.

## Open Questions

Maintainer feedback is requested on the intentionally limited first-version
coverage and proposed buffer/cache limits. Direct transport, server-managed
conversation state, and compaction remain unchanged. No live supplier validation
is required or claimed by this specification.
