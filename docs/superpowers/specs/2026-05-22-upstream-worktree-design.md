# Upstream-Based Worktree Creation Design

## Summary

Codex++ will add an optional enhancement for Codex App worktree creation. The feature makes new worktrees start from a remote tracking branch such as `upstream/main`, instead of inheriting a stale local `HEAD` or unsynced branch state.

The intended Git equivalent is:

```bash
git fetch upstream <base-branch>
git worktree add -b <new-branch> <worktree-path> upstream/<base-branch>
```

The implementation will first add a reliable Rust backend capability and a Codex++ menu entry. After that is tested, the renderer injection can enhance Codex App's native worktree creation UI when the current Codex version exposes a recognizable action point. If the native action point cannot be detected, Codex++ must show a clear fallback message and keep the menu entry usable.

## Goals

- Create new worktrees from a remote tracking ref, defaulting to `upstream/<base-branch>`.
- Reduce conflicts caused by Codex App creating worktrees from stale local state.
- Keep the Git operation explicit and testable in Rust, not hidden inside the renderer script.
- Provide a stable Codex++ menu workflow even when Codex App UI internals change.
- Enhance the native Codex App worktree flow when it can be detected safely.
- Report actionable errors for missing remotes, missing base branches, existing branches, existing paths, and invalid repositories.

## Non-Goals

- Replacing Git's own branch, worktree, or conflict behavior.
- Force-resetting local branches or deleting existing worktrees.
- Automatically pushing branches or changing upstream tracking for existing branches.
- Modifying `/Applications/Codex.app` bundle files directly.
- Supporting non-Git repositories.

## User Experience

Codex++ settings gains an `Upstream worktree` enhancement toggle. The toggle is enabled only when backend support is available.

When the user uses the Codex++ menu entry, Codex++ asks for:

- Repository path.
- New branch name.
- Worktree path.
- Base branch, defaulting to the current branch name when available, otherwise `main`.
- Remote name, defaulting to `upstream`, with `origin` available only when `upstream` is absent and the user explicitly chooses or confirms fallback behavior.

The primary action label should make the Git source clear, for example:

```text
Create from upstream/<base-branch>
```

On success, Codex++ shows the created worktree path and source ref. On failure, Codex++ shows a concise error and keeps the user's entered values so they can fix the issue.

When native Codex App worktree enhancement is active, the user can continue using Codex's normal worktree creation path. Codex++ intercepts or augments the action only when it can identify the repository path, new branch name, worktree path, and base branch. If any required value is ambiguous, Codex++ does not run Git. It shows a toast that says the native flow could not be safely enhanced and points the user to the Codex++ menu entry.

## Backend Architecture

Add a focused Rust module in `crates/codex-plus-core`, tentatively named `upstream_worktree`.

The module owns:

- Repository discovery with `git -C <repo> rev-parse --show-toplevel`.
- Current branch detection with `git -C <repo> branch --show-current`.
- Remote validation with `git -C <repo> remote`.
- Base ref validation against `<remote>/<base-branch>` after fetch.
- Safe command construction for `git fetch <remote> <base-branch>`.
- Safe command construction for `git worktree add -b <new-branch> <worktree-path> <remote>/<base-branch>`.
- Error classification and structured responses.

The module should avoid shell interpolation. Use `std::process::Command` or Tokio process APIs with each Git argument passed separately. This preserves paths with spaces and prevents branch names or paths from changing command structure.

## Bridge Routes

Extend the existing bridge route system with:

- `/upstream-worktree/status`
- `/upstream-worktree/defaults`
- `/upstream-worktree/create`

`/upstream-worktree/status` returns whether Git is available and whether the current platform/backend supports the feature.

`/upstream-worktree/defaults` accepts a repository path and returns detected values:

```json
{
  "repoRoot": "/path/to/repo",
  "currentBranch": "feature/demo",
  "defaultBaseBranch": "feature/demo",
  "remotes": ["origin", "upstream"],
  "defaultRemote": "upstream"
}
```

`/upstream-worktree/create` accepts:

```json
{
  "repoPath": "/path/to/repo",
  "branchName": "feature/demo-codex",
  "worktreePath": "/path/to/worktrees/demo-codex",
  "remote": "upstream",
  "baseBranch": "main",
  "fetch": true
}
```

It returns either:

```json
{
  "status": "ok",
  "repoRoot": "/path/to/repo",
  "worktreePath": "/path/to/worktrees/demo-codex",
  "branchName": "feature/demo-codex",
  "sourceRef": "upstream/main"
}
```

or:

```json
{
  "status": "failed",
  "code": "branch-exists",
  "message": "Branch already exists: feature/demo-codex"
}
```

## Validation Rules

Validate input before running Git:

- `repoPath` must resolve inside a Git worktree.
- `branchName` must be non-empty and must pass `git check-ref-format --branch <branchName>`.
- `worktreePath` must be absolute or resolvable to an absolute path; if it exists, it must be empty only if Git accepts it for `worktree add`.
- `remote` must be a configured remote.
- `baseBranch` must be non-empty and must not contain path traversal semantics.
- Source ref is always constructed as `<remote>/<baseBranch>` after validation.

Fetch should be enabled by default. If fetch fails because the network or remote is unavailable, the create step must not run, because the feature's purpose is to avoid stale refs.

## Error Handling

Return stable error codes for UI handling:

- `git-missing`
- `not-git-repo`
- `remote-missing`
- `base-branch-missing`
- `fetch-failed`
- `branch-invalid`
- `branch-exists`
- `path-exists`
- `worktree-create-failed`
- `ambiguous-native-flow`

Include stderr in diagnostic logs, but keep renderer-facing messages short and safe.

## Renderer Injection

Add the menu workflow first. It should reuse existing Codex++ injected menu patterns instead of creating a visually separate system.

The renderer script should:

1. Check `/upstream-worktree/status`.
2. Show an `Upstream worktree` setting row and action entry when supported.
3. Open a compact modal for repo path, branch, worktree path, remote, and base branch.
4. Call `/upstream-worktree/defaults` after the repo path changes.
5. Call `/upstream-worktree/create` on submit.
6. Display success or failure through existing toast patterns.

Native Codex App enhancement should be implemented as a guarded adapter:

- Detect known Codex worktree creation controls through stable attributes first, then text/structure as fallback.
- Extract repository path, branch name, target path, and base branch before intercepting.
- If extraction is incomplete, do not prevent the native action unless Codex++ can show a clear fallback.
- Keep the adapter versioned so future Codex App DOM changes can be diagnosed.

## Settings

Add a renderer setting key for the injected UI, for example `upstreamWorktreeCreate`.

If the setting only affects renderer behavior, it can remain in the existing local settings path. If a future version needs launcher-time behavior, move it to backend settings like provider sync.

## Testing Strategy

Rust tests should cover:

- Command argument construction uses separate arguments, not shell strings.
- Branch name validation accepts normal branch names and rejects invalid refs.
- Remote selection prefers `upstream` when present.
- `origin` fallback is not automatic when `upstream` is present.
- Create flow stops after fetch failure.
- Structured error codes map from common Git failures.
- Bridge route payloads call the upstream worktree runtime correctly.

Integration tests should use temporary Git repositories:

1. Create a bare remote named `upstream`.
2. Push a base branch to it.
3. Clone or initialize a local repository with a stale local branch.
4. Call the Rust create function.
5. Assert the new worktree's `HEAD` equals `upstream/<base-branch>` at creation time.

Renderer tests can be lightweight and focused on pure helpers:

- Default form state from `/defaults`.
- Create payload construction.
- Error message rendering.
- Native adapter refusing ambiguous DOM.

Manual verification should cover:

1. A repository with `upstream/main` newer than local `main`.
2. Creating `feature/test` at a new worktree path through Codex++ menu.
3. Confirming `git -C <worktree-path> rev-parse HEAD` matches `git -C <repo> rev-parse upstream/main`.
4. Confirming a duplicate branch gives a clear `branch-exists` error.
5. Confirming a missing `upstream` remote gives a clear `remote-missing` error.
6. Confirming native Codex worktree enhancement either creates correctly or falls back without corrupting the native flow.

## Rollout Plan

Phase 1 adds the Rust backend module, bridge routes, tests, and Codex++ menu entry.

Phase 2 adds native Codex App worktree UI detection and guarded interception.

Phase 3 updates README feature bullets and troubleshooting notes after the behavior is verified against the current Codex App version.

This staged rollout keeps the Git operation reliable before depending on Codex App's internal UI shape.
