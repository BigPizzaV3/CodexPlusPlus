# Upstream Branch Dropdown Design

## Goal

Make upstream-based worktree creation feel native in Codex App by adding `upstream/*` choices to the start-new-chat branch dropdown instead of exposing a separate Codex++ worktree dialog.

## User Experience

On the start-new-chat screen, the bottom controls already separate two decisions:

- `New worktree` decides that the next chat should run in a new worktree.
- The branch dropdown decides which branch or ref the worktree should start from.

Codex++ should extend the branch dropdown with an `Upstream` group. For a repository on `main`, the user should be able to pick `upstream/main` from the same branch menu shown in the screenshot. After that, the existing `New worktree` flow should create the worktree from the selected upstream tracking ref.

The user should not need to open a Codex++ modal or manually enter repo path, branch name, worktree path, remote, or base branch.

## Design

Codex++ keeps the existing backend bridge and upstream worktree creation core, then adds a renderer-side adapter for the native branch selector.

The adapter has three responsibilities:

1. Discover native branch dropdown menus when they open.
2. Inject an `Upstream` section containing remote tracking refs such as `upstream/main`.
3. When an injected upstream item is selected, record the selected upstream ref in page state and mark the native worktree flow so the later create action uses the upstream route.

The first iteration will provide `upstream/<current-branch>` because it is reliable and matches the user's main use case. If backend defaults can read more branches later, the same UI section can list additional upstream refs without changing the user model.

## Backend Contract

`/upstream-worktree/defaults` should return enough data for the dropdown adapter:

- `repoRoot`
- `currentBranch`
- `defaultRemote`
- `defaultBaseBranch`
- `upstreamRefs`

`upstreamRefs` contains display refs like `upstream/main` plus structured fields:

```json
{
  "remote": "upstream",
  "branch": "main",
  "label": "upstream/main",
  "sourceRef": "refs/remotes/upstream/main"
}
```

The create route continues to receive `repoPath`, `branchName`, `worktreePath`, `remote`, `baseBranch`, and `fetch`.

## Renderer Behavior

The renderer adapter should be conservative:

- It only mutates open menus that look like Codex branch dropdown menus.
- It adds identifiable `data-codex-upstream-branch-option` nodes so repeated scans do not duplicate options.
- It does not remove or rename native local branch options.
- It does not block the native flow unless a complete upstream selection and create payload are available.
- If Codex DOM changes and the adapter cannot safely detect the native menu, it should leave Codex behavior unchanged.

Selecting an upstream item should visibly update the branch dropdown label to `upstream/main` when possible. If Codex re-renders and overwrites that label, the selected state still remains in Codex++ runtime until the user changes project/branch or the page reloads.

## Error Handling

If `upstream` is missing, show no injected upstream option or show a disabled explanatory option. Creating from an upstream selection should fail with the existing backend error message if fetch or worktree creation fails.

If Codex++ cannot derive the native new-branch/worktree payload, it should not create a worktree and should show a short toast explaining that the current Codex version's native worktree form could not be recognized.

## Tests

Add tests at the script level first:

- The injection script contains the branch dropdown adapter functions and data markers.
- The defaults response returns `upstreamRefs` for the current branch and preferred remote.
- Existing create tests still prove `refs/remotes/<remote>/<branch>` is used internally.

Run these checks:

- `cargo test -p codex-plus-core --test upstream_worktree`
- `cargo test -p codex-plus-core --test cdp_bridge`
- `cargo test -p codex-plus-core --test bridge_routes`
- `node --check assets/inject/renderer-inject.js`
- `cargo test -p codex-plus-core`

## Out of Scope

This change will not patch Codex App's bundled source, will not require users to type worktree paths manually, and will not replace the full native branch picker implementation. It only injects remote branch choices and routes upstream selections through the Codex++ bridge.
