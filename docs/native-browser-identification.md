# Native Edge / Chrome Request Identification Compatibility

This experimental Windows option adapts a pinned native browser service so a
local user can require request identification without using the service's
cloud rollout decision for that requirement. Browser execution still uses the
original Edge or Chrome extension and bundled native runtime. It does not install another
browser engine, impersonate a ChatGPT login, or provide access to other
authenticated services.

## Enablement

In Codex enhancements, enable **原生 Edge / Chrome 请求标识兼容（实验）**, review the
confirmation, and save. The setting defaults to off when absent; existing saved
values are retained. The master enhancements switch also controls activation.

Edge and Chrome share the existing
`codexAppNativeBrowserRequireIdentification` setting. If it was enabled in the
Edge-only build, starting this build also permits the supported Chrome pair;
it does not create a separate Chrome opt-in. The persistent identification
disclosure below applies to either browser.

Saving does not patch a running service or restart an application. The next
Codex++ launcher takes a settings snapshot and applies compatibility before
launching Codex when a supported runtime already exists. Restarting an old
launcher is necessary when installing this version; activating an existing
instance does not start another runtime owner.

**Persistent effect:** native controlled-tab requests can carry an
`x-browser-agent: ChatGPT/<session-id>` header to destination websites. The native
extension retains identification enablement. Disabling this Codex++ option or
restoring the service does **not** turn off identification already retained by
the extension. Site restrictions, enterprise policy, native operation approvals,
and user-stop handling remain in the original execution path.

## Compatibility And Status

The adapter accepts only these Windows stable extension pairs and the following
bundled runtime fingerprints. Cross-paired IDs, beta extensions, other browsers
and unknown runtimes fall back to the original decision or fail compatibility
checks. Version labels alone are not accepted.

| Browser family | Extension ID |
| --- | --- |
| `edge` | `odlomjlbamekndcpllcnffbgeohgkmjh` |
| `chrome` | `hehggadaopoacecdllhhajmbjkdcmajg` |

Both IDs are listed in the pinned service's production extension registry.
Locally inspected Edge and Chrome extension packages at version
`1.26.901.11451` have byte-identical background scripts. This is static protocol
evidence, not proof that a connected client loaded that particular disk copy.
The helper checks the actual client's family, ID, instance and Boolean
identification state, and rejects client or browser-pair changes across I/O.

| Component | SHA-256 |
| --- | --- |
| Browser service | `3e6fd4a8cf09f57549d63f2c9cbfa2abf42f0a6b0c09c3d6605fe07c8ba09e4a` |
| Native worker | `ef53f8f0d957b7cf437020499b6b9d880dee381214788930107b549237f7949c` |
| Node executable | `be14417b6c4b4a5af06be7c16bda58730f26b912c3e8c6489d12392ef08f35bf` |
| Runtime manifest | `ba3691b0717b6df8064c3841a75c784e8af9633c7b47f2fdb56d8de099efe6fc` |
| CUA entry point | `992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278` |

The generated `unified-computer-use/.mcp.json` descriptors must agree on the
native Node path and original browser service. Ambiguous descriptors, unknown
hashes, linked paths, and external file changes prevent enablement. No remote
runtime is downloaded or redistributed.

The launcher checks for late-created or rebuilt caches, initially at bounded
500 ms intervals while waiting for a descriptor or incomplete runtime, then every
15 seconds. At idle it compares file identities, sizes and timestamps rather
than repeatedly hashing executable contents. Actual service writes still require
the full fingerprint checks. This
cannot guarantee interception before the first worker loads a newly generated
cache. It does not force a worker reload, alter Desktop's generated descriptor,
or attach to a worker's debugger.

- `waiting_for_runtime`: no native runtime descriptor is available yet.
- `prepared`: the service is prepared for a **new** native worker. This is not a
  worker-loading acknowledgement or successful browser acceptance.
- `blocked`: a compatibility or recovery check failed; the specific reason is
  available in the manager.
- `restored`: the service is restored, but the extension's retained identification
  state is unchanged.
- `stale`: the launcher has not provided a recent status.

For a late-created cache, wait for `prepared`, then use a fresh native tool
context or restart Codex before acceptance. The manager never restarts it
automatically. A request that already passed a compatibility decision cannot be
revoked by changing the setting; native operation approvals remain independent.

## Recovery

Original service bytes, exact candidate bytes, original modification time and
journals are retained in
`~/.codex-session-delete/native-browser-identification`, outside Desktop's
runtime-cache cleanup scope. Writes use an exclusive transaction lock, synced
backups, and atomic replacement. Windows directory handles prevent parent
renaming during transactions. Restored bytes and modification time are prepared
on the same temporary-file handle before replacement, so recovery does not
reopen the target to update metadata after publishing the restored content.
All known caches are preflighted before recovery;
conflicting user changes are never deliberately overwritten.

To disable the adapter, save the option as off and restart Codex++ and Codex.
The new launcher disables the helper control file and restores known candidates.
Already-original files keep their modification times; deleted caches are not
recreated. Backups are retained. If recovery reports a conflict, preserve the
backup and affected runtime for diagnosis instead of deleting the journal.

An orderly launcher exit now disables the helper and restores service bytes and
their original modification time before releasing its monitor ownership lock.
This waits for an already-started filesystem transaction. A second compatible
monitor cannot take ownership until cleanup finishes. A generation-specific
completion receipt is synced through the locked file; a released lock with an
active, failed or unreadable receipt is not accepted as successful cleanup.
The manager's Windows restart path records existing launcher process identities,
stops Codex first, and waits up to ten seconds for native cleanup and then up to
ten seconds for those same launcher processes to exit. It does not terminate
launchers by name after cleanup. If either wait fails, restart stops without
forcibly terminating the launcher or starting another instance.

Forced process termination, crashes and older managers can bypass this exit
path. They are not evidence of completed restoration. Recovery journals remain
available for reconciliation by the next compatible launcher. A new manager
cannot retrofit orderly cleanup into an older launcher that does not implement
the ownership protocol. A missing ownership receipt is accepted only when the
existing control and recovery records show no remaining enabled adapter or
candidate service.

This is not a security boundary against another process with the same user's
write access. In particular, a malicious process able to replace both recovery
records and files can compromise local integrity. Future adapter versions must
retain support for restoring previously supported original-service fingerprints.

## Page Preparation Failures

`Unable to prepare popup request headers. Retry the browser command.` comes
from the original extension's popup script preparation, after browser discovery.
It is not the earlier API-key identity failure and does not, by itself, identify
a network outage or missing header-rule permission. The original extension
collapses several injection failures and timeouts into this message.

In local testing, both browsers read newly created ordinary pages and Edge read
a newly created Bilibili homepage. An existing Bilibili homepage and the Chrome
Web Store page still failed popup preparation. This does not establish that all
pages on either browser work, or that the failed existing page has recovered.
The specific underlying injection failure remains unresolved. This integration
does not suppress that check, disable request identification, navigate an
existing tab automatically, or replace native browser execution to hide it.

## Validation

Normal Rust tests use synthetic fixtures and never execute bundled proprietary
code. Windows regression tests cover independently spelled path separators and
reject genuinely conflicting paths. The explicit ignored fixture test reads a
locally supplied pinned runtime and its actual generated descriptor, validates
the original selection read-only, then relocates the descriptor and runtime to
a temporary directory for transformation and recovery:

```powershell
$env:CPP_NATIVE_BROWSER_FIXTURE = 'C:\path\to\pinned\cua_node\runtime'
$env:CPP_NATIVE_BROWSER_DESCRIPTOR = 'C:\path\to\unified-computer-use\version\.mcp.json'
cargo test -p codex-plus-core native_browser::tests::pinned_fixture_transaction_recovery_and_external_change --lib -- --ignored --exact
```

`CPP_NATIVE_BROWSER_FIXTURE` must be the actual 16-character runtime directory
selected by that descriptor, not an independently relocated offline copy.
The test itself performs the relocation; it never writes to the supplied runtime.

Node tests execute only the first-party identification helper with isolated
control files and stubbed metadata/fallbacks. They do not execute cloud identity,
site-policy or native approval implementations.

Release acceptance still requires human tests after restarting, separately for Edge and Chrome:
page creation, existing-tab access, input/click/reload, actual identification
headers, explicit site/approval denial, physical stop, turn cleanup, and
disable/restart recovery. Testing an already-enabled Edge profile alone cannot
prove first-time enablement, because the extension retains identification.
Earlier Edge success on an already-enabled profile did not establish that the
launcher had deployed compatibility. The corrected combined build requires
fresh native end-to-end acceptance and launcher status verification.
No macOS or other cross-platform acceptance is implied.
