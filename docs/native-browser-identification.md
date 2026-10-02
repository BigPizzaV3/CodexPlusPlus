# Native Edge / Chrome Request Identification Compatibility

This experimental Windows option adapts a verified native browser service so a
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

The adapter accepts only these Windows stable extension pairs. Cross-paired IDs,
beta extensions and other browsers fall back to the original decision or fail
compatibility checks, and version labels alone are never accepted.

For runtimes there are two tiers, described in full below: the two bundled
fingerprints are known-good fast paths verified by exact SHA-256, and a runtime
whose manifest is not one of those is accepted only after the structural
invariants hold. A runtime that fails either tier is not adapted.

| Browser family | Extension ID |
| --- | --- |
| `edge` | `odlomjlbamekndcpllcnffbgeohgkmjh` |
| `chrome` | `hehggadaopoacecdllhhajmbjkdcmajg` |

Both IDs are listed in the supported services' production extension registries.
Locally inspected Edge and Chrome extension packages at version
`1.26.901.11451` have byte-identical background scripts. This is static protocol
evidence, not proof that a connected client loaded that particular disk copy.
The helper checks the actual client's family, ID, instance and Boolean
identification state, and rejects client or browser-pair changes across I/O.

**Known-good fingerprints (fast path).** Two runtime generations are registered by
content. They are accepted without re-deriving anything, and their files are
verified by exact SHA-256 as listed below.

| Component | 0.0.11 runtime SHA-256 | 0.0.24 runtime SHA-256 |
| --- | --- | --- |
| Browser service | `3e6fd4a8cf09f57549d63f2c9cbfa2abf42f0a6b0c09c3d6605fe07c8ba09e4a` | `fc0660ba45e6c10b532d8faa0c1bac704d987dad3d4b74478f49fdd82bf90086` |
| Native worker | `ef53f8f0d957b7cf437020499b6b9d880dee381214788930107b549237f7949c` | `e42e0d846b9c1e5da3ec7b5e069fdae3643df590f4e304f433cfaa7fbd8732a7` |
| Node executable | `be14417b6c4b4a5af06be7c16bda58730f26b912c3e8c6489d12392ef08f35bf` | `d3c3c290b11d55ef747e63f5a63538e0d8ca95f3f9668bb6a8081a25ba2befab` |
| Runtime manifest | `ba3691b0717b6df8064c3841a75c784e8af9633c7b47f2fdb56d8de099efe6fc` | `2c8ea57bfab596fb3b9cf78673b62a763f8d484aa8d380e341324354ce9e90e8` |
| CUA entry point | `992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278` | `992174a5e637645aeb444adfdb1bae688e997bb84d7db07532f68e358e60f278` |

**Structural acceptance for other versions.** This table is a fast path, not the
whitelist it used to be. A manifest whose hash is not listed above is parsed and
accepted when its structure holds:

- the manifest is a JSON object carrying either package `name` / `version`,
  or Desktop's generated `runtime_archive_version` / `runtime_archive_name`;
  the generated archive name must agree with its version and build identity;
- the package version, or the version before `/` in an archive identity,
  parses as `major.minor.patch` (optionally `v`-prefixed, optional
  pre-release/build suffix) and is not below `0.0.11`;
- the runtime directory contains `bin/node_repl.exe`, `bin/node.exe`,
  `manifest.json` and
  `bin/node_modules/@oai/cua-repl/bin/cua-repl.mjs`, each a regular file within
  size limits.

Only then is the runtime treated as *adapted-unknown*: the browser service is
still bound by the SHA-256 computed from the bytes on disk, the status detail says
the runtime is unverified, and the launcher records
`native_browser.runtime_adapted_unknown` with both hashes. If any structural
invariant fails, compatibility still fails closed and `control.json` is written
with `requireIdentification:false`.

This exists because a hash table is invalidated by a single upstream release:
pinning 0.0.11 and 0.0.24 meant that a newer runtime failed the manifest check,
which wrote `requireIdentification:false`, which fell back to the cloud rollout
decision, which reports API-key sessions as unusable
(`unsupported Codex auth method: apikey`). Version labels alone are still not
accepted; the structural invariants are.

The browser constructor and metadata/policy callback carry no fixed names in the
adapter. Known profiles `nf/ze/cD` (0.0.11) and `eh/je/sv` (0.0.24) are
matched by shape — `new <ctor>(r,this.clientApi,()=><getter>(this.runtime),this.turnEndedTracker,<policy>)`
— with the policy callback name carried over from the source. Exactly one such
binding must be present; ambiguity is refused rather than guessed. After
rewriting, the adapter re-checks that the original binding is gone and that no
compressed identifier changed its occurrence count, accounting for inserted
control-path literals. Unknown profiles use the AST binding checks below.

The adaptation binds the same first-party, turn-scoped identification reader at
the callback, only for the verified Chrome/Edge extension pair and while the
opt-in control is enabled. It does not change the native worker's
authentication, impersonate account credentials, or disable browser site checks
and operation approvals. A new extension with identification off can otherwise
ask the original policy callback for caller identity and encounter
`unsupported Codex auth method: apikey`; an already enabled extension can avoid
that path even without the adapter. A successful `getInfo` probe alone does not
test this request path.

**Connection reporting.** Read-only discovery over the native pipe reports the
connected extension's family, its reported identification state, and whether its
extension ID is one of the two registered pairs above. An extension that
connects from the same family with an unregistered ID (for example a newer
store build) is shown as connected but unrecognized rather than as
disconnected, so the manager can distinguish "the extension never connected"
from "it connected and this build does not recognize it". Both readings refuse
compatibility; only the registered pairs are accepted by the adapter.

### Semantic Compatibility

A runtime update does not require a new complete-file fingerprint when its
browser callback contract is unchanged. For builds outside the two known
profiles, the adapter:

1. Checks the generated descriptor, Windows x64 manifest paths, native executable
   format, browser package export and the CUA worker entry's bound, awaited
   `@oai/cua-repl.launch()` call. The manifest version must be at least 0.0.11.
   Entry comments, formatting and added diagnostics do not require new hashes.
2. Uses an embedded Babel AST inspector to resolve the actual constructor,
   metadata reader and policy callback in their lexical scopes. The constructor,
   metadata reader, policy callback, client handshake and session-request method
   retain their callback relationships. Known session-request AST shapes are
   fast paths; other shapes qualify through bound session parameters, awaited
   policy invocation, header-decision data flow and request forwarding.
   Instrumentation, local renaming and unrelated command exclusions are not
   grounds for rejection. Metadata and policy semantics remain checked;
   ambiguity, rebinding and changed callback relationships are rejected.
3. Replaces only the selected policy argument with the existing local
   identification reader. Every other service byte is retained; the helper is
   appended. It does not rewrite worker authentication or execute the service
   during inspection.

The inspector runs in a separate, hidden instance of the selected runtime's Node,
with inherited environment removed, a bounded heap and deadline. Input comes
from a temporary regular file, not a potentially blocked pipe writer; the child
is killed and reaped on timeout and the input file removed. Its source and
vendored dependency licenses are embedded in the application. It needs no
additional Node installation, runtime download or separately installed parser
at runtime. Building the embedded bundle uses explicitly pinned Babel and
esbuild devDependencies; CI verifies the complete generated bundle.
The current contracts cover the service structures observed in CUA 0.0.11,
0.0.24 and 0.0.27; a real protocol change, ambiguous constructor, changed callback,
changed worker entry signature, malformed manifest or incompatible platform still blocks
adaptation. This is not an assurance of compatibility with arbitrary future
runtime versions.

All inspected component hashes become exact transaction guards: an intervening
file change still prevents deployment. The full original and candidate service
hashes remain mandatory for integrity and restoration. A passing AST check or
`prepared` status is not proof of browser connectivity or first-use success.
Acceptance should use a clean extension profile with identification initially
off; an already-enabled extension can skip the original failing policy callback
even when no service patch is active.

The generated `unified-computer-use/.mcp.json` descriptors must agree on the
native Node path and original browser service. Ambiguous descriptors, manifests
that fail the structural invariants above, linked paths, and external file
changes prevent enablement. No remote runtime is downloaded or redistributed.

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

Known fingerprint profiles retain schema-1 recovery records. Semantically
qualified builds use schema-2 records containing the checked callback position,
binding names and the normalized request-method digest for audit (not a
fixed admission list). Restoration recomputes the
minimal transformation and requires it to equal the recorded candidate; hashes
alone are insufficient. Both formats remain recoverable without starting Node,
with a missing descriptor, or after other runtime components have changed.
Deleted service caches are not recreated. Older Codex++ builds and the standalone
browser unlocker v0.1.0 do not understand schema 2: restore with this or a newer
compatible Codex++ before downgrading or switching recovery tools. Future
helper or insertion-format changes must retain or version the schema-2
reconstruction algorithm so existing journals remain recoverable. Do not delete
the journal to suppress a conflict.

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
write access. The selected runtime's `node.exe` is a trust root: PE x64 validation
does not authenticate its publisher or detect a malicious replacement before
inspection. Component snapshots prevent subsequent drift, not pre-existing
compromise. In particular, a malicious process able to replace both recovery
records and files can compromise local integrity. What recovery does enforce is
that a journal's recorded original hash matches the bytes of the actual backup
it points at: records and files must be replaced together to forge one, and
backups from a runtime that is no longer registered still restore.

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

Normal Rust tests use public synthetic runtimes and never execute bundled
proprietary code. CI's Node 22 runs only our embedded inspector on the public
service specimen; synthetic PE files are not executed. Detection, version and
layout rejection, entry signatures, component drift, schema-2 transactions and
recovery without Node are covered without private fixtures. Windows regression
tests cover independently spelled path separators and
reject genuinely conflicting paths. The explicit ignored fixture test reads a
locally supplied pinned runtime and its actual generated descriptor, validates
the original selection read-only, then relocates the descriptor and runtime to
a temporary directory for transformation and recovery:

```powershell
$env:CPP_NATIVE_BROWSER_FIXTURE = 'C:\path\to\pinned\cua_node\runtime'
$env:CPP_NATIVE_BROWSER_DESCRIPTOR = 'C:\path\to\unified-computer-use\version\.mcp.json'
cargo test -p codex-plus-core native_browser::tests::pinned_fixture_transaction_recovery_and_external_change --lib -- --ignored --exact
```

`CPP_NATIVE_BROWSER_FIXTURE` must be an unmodified, supported 16-character runtime directory
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
