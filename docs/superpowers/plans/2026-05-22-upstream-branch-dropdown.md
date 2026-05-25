# Upstream Branch Dropdown Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add upstream branch choices to Codex App's native start-new-chat branch dropdown so `New worktree` can start from `upstream/<base>` without a separate Codex++ dialog.

**Architecture:** Extend the existing upstream worktree backend defaults with structured upstream refs, then add a conservative renderer adapter that injects `Upstream` options into native branch menus and records the selected upstream ref for the existing native-create interception path. Keep the manual Codex++ dialog as fallback for now, but make the native dropdown path the primary UX.

**Tech Stack:** Rust `codex-plus-core`, JSON bridge routes, plain JavaScript renderer injection, `cargo test`, `node --check`.

---

### Task 1: Backend defaults expose upstream refs

**Files:**
- Modify: `crates/codex-plus-core/src/upstream_worktree/defaults.rs`
- Test: `crates/codex-plus-core/tests/upstream_worktree.rs`

- [ ] **Step 1: Write failing test**

Add this test to `crates/codex-plus-core/tests/upstream_worktree.rs`:

```rust
#[test]
fn defaults_response_lists_preferred_upstream_ref_for_current_branch() {
    let temp = tempfile::tempdir().unwrap();
    let remote = temp.path().join("remote.git");
    git(temp.path(), &["init", "--bare", remote.to_str().unwrap()]);
    let repo = temp.path().join("repo");
    git(temp.path(), &["clone", remote.to_str().unwrap(), repo.to_str().unwrap()]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    std::fs::write(repo.join("README.md"), "hello").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-m", "initial"]);
    git(&repo, &["branch", "-M", "main"]);
    git(&repo, &["push", "origin", "main"]);
    git(&repo, &["remote", "rename", "origin", "upstream"]);
    git(&repo, &["fetch", "upstream", "main"]);

    let result = defaults_response(&json!({ "repoPath": repo }));

    assert_eq!(result["status"], "ok");
    assert_eq!(result["defaultRemote"], "upstream");
    assert_eq!(result["upstreamRefs"][0]["remote"], "upstream");
    assert_eq!(result["upstreamRefs"][0]["branch"], "main");
    assert_eq!(result["upstreamRefs"][0]["label"], "upstream/main");
    assert_eq!(result["upstreamRefs"][0]["sourceRef"], "refs/remotes/upstream/main");
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo test -p codex-plus-core --test upstream_worktree defaults_response_lists_preferred_upstream_ref_for_current_branch
```

Expected: fails because `upstreamRefs` is missing.

- [ ] **Step 3: Implement minimal backend defaults**

In `defaults.rs`, add a small helper that emits one structured upstream ref using `default_remote_name(&remotes)` and `default_base_branch`:

```rust
fn upstream_refs(remote: &str, base_branch: &str) -> Vec<Value> {
    if remote.trim().is_empty() || base_branch.trim().is_empty() {
        return Vec::new();
    }
    vec![json!({
        "remote": remote,
        "branch": base_branch,
        "label": format!("{remote}/{base_branch}"),
        "sourceRef": format!("refs/remotes/{remote}/{base_branch}"),
    })]
}
```

Then include it in `defaults_for_repo`:

```rust
let default_remote = default_remote_name(&remotes);
Ok(json!({
    "status": "ok",
    "repoRoot": root.to_string_lossy(),
    "currentBranch": branch,
    "defaultBaseBranch": default_base_branch,
    "remotes": remotes,
    "defaultRemote": default_remote,
    "upstreamRefs": upstream_refs(&default_remote, &default_base_branch),
}))
```

- [ ] **Step 4: Verify GREEN**

Run the same single test. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/codex-plus-core/src/upstream_worktree/defaults.rs crates/codex-plus-core/tests/upstream_worktree.rs
git commit -m "feat(worktree): expose upstream refs in defaults"
```

### Task 2: Renderer script exposes branch dropdown adapter markers

**Files:**
- Modify: `assets/inject/renderer-inject.js`
- Test: `crates/codex-plus-core/tests/cdp_bridge.rs`

- [ ] **Step 1: Write failing script-marker test**

Add this test to `crates/codex-plus-core/tests/cdp_bridge.rs`:

```rust
#[test]
fn injection_script_installs_upstream_branch_dropdown_adapter() {
    let script = assets::injection_script(57321);

    assert!(script.contains("installUpstreamBranchDropdownAdapter"));
    assert!(script.contains("data-codex-upstream-branch-option"));
    assert!(script.contains("codexUpstreamBranchSelection"));
    assert!(script.contains("/upstream-worktree/defaults"));
}
```

- [ ] **Step 2: Verify RED**

Run:

```bash
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo test -p codex-plus-core --test cdp_bridge injection_script_installs_upstream_branch_dropdown_adapter
```

Expected: fails because adapter names and markers are not present.

- [ ] **Step 3: Implement adapter skeleton**

In `renderer-inject.js`, add constants near other upstream constants:

```js
const upstreamBranchOptionAttribute = "data-codex-upstream-branch-option";
const upstreamBranchSelectionKey = "codexUpstreamBranchSelection";
```

Add functions near the upstream worktree helpers:

```js
function readUpstreamBranchSelection() {
  try {
    return JSON.parse(sessionStorage.getItem(upstreamBranchSelectionKey) || "null");
  } catch {
    return null;
  }
}

function writeUpstreamBranchSelection(selection) {
  if (!selection) {
    sessionStorage.removeItem(upstreamBranchSelectionKey);
    return;
  }
  sessionStorage.setItem(upstreamBranchSelectionKey, JSON.stringify(selection));
}

function nativeBranchMenuCandidates() {
  return [...document.querySelectorAll('[role="menu"], [data-radix-menu-content], [cmdk-list]')]
    .filter((node) => !node.querySelector(`[${upstreamBranchOptionAttribute}]`));
}

function looksLikeBranchMenu(menu) {
  const text = (menu.innerText || menu.textContent || "").toLowerCase();
  return /branch|main|worktree|create branch/.test(text);
}

function installUpstreamBranchDropdownAdapter() {
  if (window.__codexUpstreamBranchDropdownAdapterInstalled === "1") return;
  window.__codexUpstreamBranchDropdownAdapterInstalled = "1";
  document.addEventListener("click", (event) => {
    const target = event.target instanceof Element ? event.target : event.target?.parentElement;
    const option = target?.closest?.(`[${upstreamBranchOptionAttribute}]`);
    if (!option) return;
    event.preventDefault();
    event.stopPropagation();
    writeUpstreamBranchSelection({
      repoPath: option.getAttribute("data-repo-path") || "",
      remote: option.getAttribute("data-remote") || "upstream",
      baseBranch: option.getAttribute("data-base-branch") || "main",
      label: option.textContent?.trim() || "upstream/main",
    });
    showToast(`将从 ${option.textContent?.trim() || "upstream/main"} 创建新 worktree`, null);
  }, true);
}
```

Call `installUpstreamBranchDropdownAdapter();` during boot next to `installUpstreamWorktreeNativeAdapter();`.

This is only the skeleton; Task 3 adds menu injection.

- [ ] **Step 4: Verify GREEN**

Run the single cdp bridge marker test. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add assets/inject/renderer-inject.js crates/codex-plus-core/tests/cdp_bridge.rs
git commit -m "feat(worktree): install upstream branch dropdown adapter"
```

### Task 3: Inject upstream options into native branch menus

**Files:**
- Modify: `assets/inject/renderer-inject.js`
- Test: `crates/codex-plus-core/tests/cdp_bridge.rs`

- [ ] **Step 1: Write failing marker test**

Extend `injection_script_installs_upstream_branch_dropdown_adapter` with:

```rust
assert!(script.contains("injectUpstreamBranchOptions"));
assert!(script.contains("Upstream"));
assert!(script.contains("data-base-branch"));
assert!(script.contains("MutationObserver"));
```

- [ ] **Step 2: Verify RED**

Run the same cdp bridge test. Expected: fails because injection function is missing.

- [ ] **Step 3: Implement menu injection**

Add a runtime cache:

```js
let upstreamBranchDefaultsCache = null;
```

Add helpers:

```js
function currentProjectRepoPath() {
  const expandedProject = document.querySelector('[data-app-action-sidebar-project-collapsed="false"][data-app-action-sidebar-project-id]');
  return expandedProject?.getAttribute("data-app-action-sidebar-project-id") || "";
}

async function loadUpstreamBranchDefaults(repoPath) {
  if (!repoPath) return null;
  if (upstreamBranchDefaultsCache?.repoPath === repoPath) return upstreamBranchDefaultsCache;
  const result = await postJson("/upstream-worktree/defaults", { repoPath });
  if (result?.status !== "ok") return null;
  upstreamBranchDefaultsCache = { repoPath, result };
  return upstreamBranchDefaultsCache;
}

function renderUpstreamBranchOption(menu, repoPath, ref) {
  const item = document.createElement("div");
  item.setAttribute("role", "menuitem");
  item.setAttribute(upstreamBranchOptionAttribute, "true");
  item.setAttribute("data-repo-path", repoPath);
  item.setAttribute("data-remote", ref.remote || "upstream");
  item.setAttribute("data-base-branch", ref.branch || "main");
  item.className = "codex-upstream-branch-option cursor-interaction flex items-center gap-2 rounded-sm px-2 py-1.5 text-sm text-token-foreground hover:bg-token-list-hover-background";
  item.textContent = ref.label || `${ref.remote || "upstream"}/${ref.branch || "main"}`;
  menu.appendChild(item);
}

async function injectUpstreamBranchOptions() {
  if (!codexPlusSettings().upstreamWorktreeCreate) return;
  const repoPath = currentProjectRepoPath();
  if (!repoPath) return;
  const defaults = await loadUpstreamBranchDefaults(repoPath);
  const refs = defaults?.result?.upstreamRefs || [];
  if (!refs.length) return;
  for (const menu of nativeBranchMenuCandidates()) {
    if (!looksLikeBranchMenu(menu)) continue;
    if (menu.querySelector(`[${upstreamBranchOptionAttribute}]`)) continue;
    const group = document.createElement("div");
    group.className = "codex-upstream-branch-group px-2 py-1 text-xs text-token-text-tertiary";
    group.textContent = "Upstream";
    menu.appendChild(group);
    refs.forEach((ref) => renderUpstreamBranchOption(menu, repoPath, ref));
  }
}
```

In the adapter install function, add a mutation observer that schedules injection:

```js
let upstreamBranchInjectTimer = null;
const schedule = () => {
  clearTimeout(upstreamBranchInjectTimer);
  upstreamBranchInjectTimer = setTimeout(() => {
    injectUpstreamBranchOptions().catch((error) => reportDiagnostic("upstream_branch_inject_failed", { error: error?.message || String(error) }));
  }, 80);
};
new MutationObserver(schedule).observe(document.body || document.documentElement, { childList: true, subtree: true });
schedule();
```

- [ ] **Step 4: Verify GREEN**

Run the cdp bridge test. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add assets/inject/renderer-inject.js crates/codex-plus-core/tests/cdp_bridge.rs
git commit -m "feat(worktree): inject upstream branch menu options"
```

### Task 4: Route native create through selected upstream ref

**Files:**
- Modify: `assets/inject/renderer-inject.js`
- Test: `crates/codex-plus-core/tests/cdp_bridge.rs`

- [ ] **Step 1: Write failing marker test**

Extend the cdp bridge test with:

```rust
assert!(script.contains("upstreamWorktreePayloadFromSelection"));
assert!(script.contains("readUpstreamBranchSelection"));
assert!(script.contains("writeUpstreamBranchSelection(null)"));
```

- [ ] **Step 2: Verify RED**

Run the cdp bridge test. Expected: fails because payload-from-selection is missing.

- [ ] **Step 3: Implement selected upstream create path**

Add helper:

```js
function upstreamWorktreePayloadFromSelection(trigger) {
  const selection = readUpstreamBranchSelection();
  if (!selection?.repoPath || !selection?.remote || !selection?.baseBranch) return null;
  const nativePayload = upstreamWorktreeNativePayloadFromElement(trigger);
  if (!nativePayload?.branchName || !nativePayload?.worktreePath) return null;
  return {
    ...nativePayload,
    repoPath: selection.repoPath,
    remote: selection.remote,
    baseBranch: selection.baseBranch,
    fetch: true,
  };
}
```

Update `handleUpstreamWorktreeNativeCreate` so it first tries selected upstream payload:

```js
const payload = upstreamWorktreePayloadFromSelection(trigger) || upstreamWorktreeNativePayloadFromElement(trigger);
```

After success, clear selection:

```js
writeUpstreamBranchSelection(null);
```

- [ ] **Step 4: Verify GREEN**

Run the cdp bridge test. Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add assets/inject/renderer-inject.js crates/codex-plus-core/tests/cdp_bridge.rs
git commit -m "feat(worktree): route native create from upstream selection"
```

### Task 5: Validate and install locally

**Files:**
- No source edits expected.

- [ ] **Step 1: Run focused checks**

```bash
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo fmt --check
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo test -p codex-plus-core --test upstream_worktree
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo test -p codex-plus-core --test cdp_bridge
node --check "assets/inject/renderer-inject.js"
```

Expected: all PASS.

- [ ] **Step 2: Run full core tests**

```bash
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo test -p codex-plus-core
```

Expected: PASS.

- [ ] **Step 3: Build and package**

```bash
npm install --prefix "apps/codex-plus-manager"
npm run --prefix "apps/codex-plus-manager" check
npm run --prefix "apps/codex-plus-manager" vite:build
env -u CFLAGS -u CPPFLAGS -u LDFLAGS rustup run "1.95.0" cargo build --release
bash "scripts/installer/macos/package-dmg.sh" "1.1.5-local-upstream-dropdown" "$(uname -m)"
```

Expected: DMG created under `dist/macos/`.

- [ ] **Step 4: Install and sign local app**

```bash
hdiutil attach "dist/macos/CodexPlusPlus-1.1.5-local-upstream-dropdown-macos-$(uname -m).dmg" -nobrowse -readonly
ditto "/Volumes/Codex++/Codex++.app" "/Applications/Codex++.app"
ditto "/Volumes/Codex++/Codex++ 管理工具.app" "/Applications/Codex++ 管理工具.app"
codesign --force --deep --sign - "/Applications/Codex++.app"
codesign --force --deep --sign - "/Applications/Codex++ 管理工具.app"
codesign --verify --deep --strict --verbose=2 "/Applications/Codex++.app"
codesign --verify --deep --strict --verbose=2 "/Applications/Codex++ 管理工具.app"
hdiutil detach "/Volumes/Codex++"
```

Expected: both apps verify.

- [ ] **Step 5: Clean temporary package manager files**

If `npm install` created untracked files, remove only untracked generated dependency files:

```bash
python3 - <<'PY'
from pathlib import Path
import shutil
root = Path('/Users/carson/.codex/worktrees/b4e7/CodexPlusPlus')
for rel in ['apps/codex-plus-manager/node_modules', 'apps/codex-plus-manager/package-lock.json']:
    path = root / rel
    if path.is_dir():
        shutil.rmtree(path)
    elif path.exists():
        path.unlink()
PY
```

Then verify `git status --short` is clean except ignored build outputs.
