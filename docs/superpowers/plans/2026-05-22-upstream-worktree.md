# Upstream Worktree Creation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a Codex++ enhancement that creates new Git worktrees from a fresh remote tracking ref such as `upstream/main` instead of stale local `HEAD`.

**Architecture:** Add a focused Rust `upstream_worktree` module in `codex-plus-core`, expose it through existing bridge routes, wire the launcher runtime to those routes, then add a guarded renderer menu workflow. The backend uses argument-array Git commands and an explicit fetch refspec so the created worktree is equivalent to `git worktree add -b <branch> <path> upstream/<base>` with a freshly updated `upstream/<base>` ref.

**Tech Stack:** Rust 2024, `std::process::Command`, existing `serde_json` bridge routing, existing injected `assets/inject/renderer-inject.js`, Cargo tests, `node --check` for renderer syntax validation.

---

## File Structure

- Create `crates/codex-plus-core/src/upstream_worktree.rs`: Git validation, defaults, worktree creation, JSON response shaping.
- Modify `crates/codex-plus-core/src/lib.rs`: export the new module.
- Create `crates/codex-plus-core/tests/upstream_worktree.rs`: integration-style tests using temporary Git repositories.
- Modify `crates/codex-plus-core/src/routes.rs`: add bridge trait methods and route dispatch for upstream worktree endpoints.
- Modify `apps/codex-plus-launcher/src/main.rs`: implement the new bridge runtime methods for the launcher runtime.
- Modify `crates/codex-plus-core/tests/bridge_routes.rs`: cover all new routes and fake runtime behavior.
- Modify `assets/inject/renderer-inject.js`: add setting, menu row, modal, backend calls, and guarded native adapter stub.
- Modify `README.md` and `README_EN.md`: document the user-facing enhancement after tests pass.

The backend and bridge work is independent from the renderer work. Implement tasks in order so every commit remains testable.

---

### Task 1: Add upstream worktree core API and validation

**Files:**
- Create: `crates/codex-plus-core/src/upstream_worktree.rs`
- Modify: `crates/codex-plus-core/src/lib.rs`
- Test: `crates/codex-plus-core/tests/upstream_worktree.rs`

- [ ] **Step 1: Write failing validation tests**

Create `crates/codex-plus-core/tests/upstream_worktree.rs` with this initial content:

```rust
use codex_plus_core::upstream_worktree::{
    UpstreamWorktreeCode, default_remote_name, source_ref, validate_branch_name,
};

#[test]
fn branch_validation_accepts_normal_branch_names() {
    validate_branch_name("feature/upstream-worktree").expect("branch should be valid");
    validate_branch_name("carson/test-123").expect("branch should be valid");
}

#[test]
fn branch_validation_rejects_invalid_branch_names() {
    let error = validate_branch_name("bad branch").expect_err("spaces are invalid");
    assert_eq!(error.code, UpstreamWorktreeCode::BranchInvalid);

    let error = validate_branch_name("-bad").expect_err("dash prefix is invalid");
    assert_eq!(error.code, UpstreamWorktreeCode::BranchInvalid);
}

#[test]
fn source_ref_joins_remote_and_base_branch() {
    assert_eq!(source_ref("upstream", "main"), "upstream/main");
    assert_eq!(source_ref("origin", "feature/x"), "origin/feature/x");
}

#[test]
fn default_remote_prefers_upstream_then_origin_then_first_remote() {
    assert_eq!(default_remote_name(&["origin".into(), "upstream".into()]), "upstream");
    assert_eq!(default_remote_name(&["origin".into()]), "origin");
    assert_eq!(default_remote_name(&["mirror".into()]), "mirror");
    assert_eq!(default_remote_name(&[]), "upstream");
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p codex-plus-core --test upstream_worktree
```

Expected: FAIL because `codex_plus_core::upstream_worktree` does not exist.

- [ ] **Step 3: Export the module**

Add this line to `crates/codex-plus-core/src/lib.rs` near the other `pub mod` declarations:

```rust
pub mod upstream_worktree;
```

- [ ] **Step 4: Implement core types and validation**

Create `crates/codex-plus-core/src/upstream_worktree.rs` with:

```rust
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpstreamWorktreeCode {
    GitMissing,
    NotGitRepo,
    RemoteMissing,
    BaseBranchMissing,
    FetchFailed,
    BranchInvalid,
    BranchExists,
    PathExists,
    WorktreeCreateFailed,
    AmbiguousNativeFlow,
}

impl UpstreamWorktreeCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GitMissing => "git-missing",
            Self::NotGitRepo => "not-git-repo",
            Self::RemoteMissing => "remote-missing",
            Self::BaseBranchMissing => "base-branch-missing",
            Self::FetchFailed => "fetch-failed",
            Self::BranchInvalid => "branch-invalid",
            Self::BranchExists => "branch-exists",
            Self::PathExists => "path-exists",
            Self::WorktreeCreateFailed => "worktree-create-failed",
            Self::AmbiguousNativeFlow => "ambiguous-native-flow",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamWorktreeError {
    pub code: UpstreamWorktreeCode,
    pub message: String,
}

impl UpstreamWorktreeError {
    pub fn new(code: UpstreamWorktreeCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn to_value(&self) -> Value {
        json!({
            "status": "failed",
            "code": self.code.as_str(),
            "message": self.message,
        })
    }
}

pub type UpstreamWorktreeResult<T> = Result<T, UpstreamWorktreeError>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpstreamWorktreeRequest {
    pub repo_path: PathBuf,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub remote: String,
    pub base_branch: String,
    pub fetch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutput {
    pub status_success: bool,
    pub stdout: String,
    pub stderr: String,
}

pub fn validate_branch_name(branch: &str) -> UpstreamWorktreeResult<()> {
    let branch = branch.trim();
    if branch.is_empty() || branch.starts_with('-') || branch.contains('\\') {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::BranchInvalid,
            format!("Invalid branch name: {branch}"),
        ));
    }
    let output = Command::new("git")
        .args(["check-ref-format", "--branch", branch])
        .output()
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::BranchInvalid,
            format!("Invalid branch name: {branch}"),
        ))
    }
}

pub fn validate_base_branch(base_branch: &str) -> UpstreamWorktreeResult<()> {
    validate_branch_name(base_branch).map_err(|_| {
        UpstreamWorktreeError::new(
            UpstreamWorktreeCode::BaseBranchMissing,
            format!("Invalid base branch: {base_branch}"),
        )
    })
}

pub fn default_remote_name(remotes: &[String]) -> String {
    if remotes.iter().any(|remote| remote == "upstream") {
        "upstream".to_string()
    } else if remotes.iter().any(|remote| remote == "origin") {
        "origin".to_string()
    } else {
        remotes.first().cloned().unwrap_or_else(|| "upstream".to_string())
    }
}

pub fn source_ref(remote: &str, base_branch: &str) -> String {
    format!("{}/{}", remote.trim(), base_branch.trim())
}

fn string_field(payload: &Value, key: &str) -> String {
    payload
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string()
}

pub fn request_from_payload(payload: &Value) -> UpstreamWorktreeResult<UpstreamWorktreeRequest> {
    let repo_path = string_field(payload, "repoPath");
    let branch_name = string_field(payload, "branchName");
    let worktree_path = string_field(payload, "worktreePath");
    let remote = string_field(payload, "remote");
    let base_branch = string_field(payload, "baseBranch");
    let fetch = payload.get("fetch").and_then(Value::as_bool).unwrap_or(true);

    if repo_path.is_empty() {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::NotGitRepo,
            "Repository path is required",
        ));
    }
    if worktree_path.is_empty() {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::PathExists,
            "Worktree path is required",
        ));
    }
    validate_branch_name(&branch_name)?;
    validate_base_branch(&base_branch)?;
    if remote.is_empty() || remote.starts_with('-') || remote.contains('/') || remote.contains('\\') {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::RemoteMissing,
            "Remote is required",
        ));
    }

    Ok(UpstreamWorktreeRequest {
        repo_path: PathBuf::from(repo_path),
        branch_name,
        worktree_path: PathBuf::from(worktree_path),
        remote,
        base_branch,
        fetch,
    })
}

fn git_output(args: Vec<OsString>) -> Result<GitOutput, std::io::Error> {
    let output = Command::new("git").args(args).output()?;
    Ok(GitOutput {
        status_success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).trim().to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).trim().to_string(),
    })
}

fn git_in_repo(repo: &Path, args: &[&str]) -> Result<GitOutput, std::io::Error> {
    let mut command_args = vec![OsString::from("-C"), repo.as_os_str().to_os_string()];
    command_args.extend(args.iter().map(OsString::from));
    git_output(command_args)
}
```

- [ ] **Step 5: Run validation tests**

Run:

```bash
cargo test -p codex-plus-core --test upstream_worktree branch_validation source_ref default_remote
```

Expected: PASS for the four tests in this task.

- [ ] **Step 6: Commit validation core**

Run:

```bash
git add "crates/codex-plus-core/src/lib.rs" "crates/codex-plus-core/src/upstream_worktree.rs" "crates/codex-plus-core/tests/upstream_worktree.rs"
git commit -m "feat(worktree): add upstream worktree validation core"
```

---

### Task 2: Add Git defaults and upstream worktree creation

**Files:**
- Modify: `crates/codex-plus-core/src/upstream_worktree.rs`
- Modify: `crates/codex-plus-core/tests/upstream_worktree.rs`

- [ ] **Step 1: Add failing Git integration tests**

Append this code to `crates/codex-plus-core/tests/upstream_worktree.rs`:

```rust
use std::path::Path;
use std::process::Command;

use serde_json::json;

use codex_plus_core::upstream_worktree::{
    create_response, defaults_response, status_response,
};

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn git_no_repo(args: &[&str]) {
    let output = Command::new("git").args(args).output().expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout: {}\nstderr: {}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn write_file(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().expect("file should have parent")).unwrap();
    std::fs::write(path, text).unwrap();
}

fn commit_file(repo: &Path, name: &str, text: &str, message: &str) -> String {
    write_file(&repo.join(name), text);
    git(repo, &["add", name]);
    git(repo, &["commit", "-m", message]);
    git(repo, &["rev-parse", "HEAD"])
}

fn prepare_remote_repo(temp: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let remote = temp.join("remote.git");
    let seed = temp.join("seed");
    git_no_repo(&["init", "--bare", remote.to_str().unwrap()]);
    git_no_repo(&["init", seed.to_str().unwrap()]);
    git(&seed, &["config", "user.email", "test@example.com"]);
    git(&seed, &["config", "user.name", "Test User"]);
    commit_file(&seed, "README.md", "v1\n", "initial");
    git(&seed, &["branch", "-M", "main"]);
    git(&seed, &["remote", "add", "upstream", remote.to_str().unwrap()]);
    git(&seed, &["push", "-u", "upstream", "main"]);
    (remote, seed)
}

#[test]
fn status_response_reports_git_available() {
    let result = status_response();

    assert_eq!(result["status"], "ok");
    assert_eq!(result["feature"], "upstream-worktree");
    assert_eq!(result["gitAvailable"], true);
}

#[test]
fn defaults_response_detects_repo_branch_and_upstream_remote() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    git_no_repo(&["init", repo.to_str().unwrap()]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    commit_file(&repo, "README.md", "v1\n", "initial");
    git(&repo, &["checkout", "-b", "feature/local"]);
    git(&repo, &["remote", "add", "origin", "https://example.invalid/origin.git"]);
    git(&repo, &["remote", "add", "upstream", "https://example.invalid/upstream.git"]);

    let result = defaults_response(&json!({"repoPath": repo}));

    assert_eq!(result["status"], "ok");
    assert_eq!(result["currentBranch"], "feature/local");
    assert_eq!(result["defaultBaseBranch"], "feature/local");
    assert_eq!(result["defaultRemote"], "upstream");
    assert_eq!(result["remotes"].as_array().unwrap().len(), 2);
}

#[test]
fn create_response_creates_new_worktree_from_fetched_upstream_ref() {
    let temp = tempfile::tempdir().unwrap();
    let (remote, seed) = prepare_remote_repo(temp.path());
    let repo = temp.path().join("repo");
    git_no_repo(&["clone", remote.to_str().unwrap(), repo.to_str().unwrap()]);
    git(&repo, &["remote", "rename", "origin", "upstream"]);
    git(&repo, &["checkout", "-b", "local-stale"]);
    let remote_head = commit_file(&seed, "README.md", "v2\n", "remote update");
    git(&seed, &["push", "upstream", "main"]);
    let worktree_path = temp.path().join("created worktree");

    let result = create_response(&json!({
        "repoPath": repo,
        "branchName": "feature/from-upstream",
        "worktreePath": worktree_path,
        "remote": "upstream",
        "baseBranch": "main",
        "fetch": true
    }));

    assert_eq!(result["status"], "ok");
    assert_eq!(result["sourceRef"], "upstream/main");
    let created_head = git(Path::new(result["worktreePath"].as_str().unwrap()), &["rev-parse", "HEAD"]);
    assert_eq!(created_head, remote_head);
}

#[test]
fn create_response_does_not_create_worktree_when_fetch_fails() {
    let temp = tempfile::tempdir().unwrap();
    let repo = temp.path().join("repo");
    git_no_repo(&["init", repo.to_str().unwrap()]);
    git(&repo, &["config", "user.email", "test@example.com"]);
    git(&repo, &["config", "user.name", "Test User"]);
    commit_file(&repo, "README.md", "v1\n", "initial");
    git(&repo, &["remote", "add", "upstream", temp.path().join("missing.git").to_str().unwrap()]);
    let worktree_path = temp.path().join("should-not-exist");

    let result = create_response(&json!({
        "repoPath": repo,
        "branchName": "feature/no-fetch",
        "worktreePath": worktree_path,
        "remote": "upstream",
        "baseBranch": "main",
        "fetch": true
    }));

    assert_eq!(result["status"], "failed");
    assert_eq!(result["code"], "fetch-failed");
    assert!(!temp.path().join("should-not-exist").exists());
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
cargo test -p codex-plus-core --test upstream_worktree
```

Expected: FAIL because `status_response`, `defaults_response`, and `create_response` are not implemented.

- [ ] **Step 3: Implement Git discovery and response helpers**

Append these functions to `crates/codex-plus-core/src/upstream_worktree.rs`:

```rust
fn git_available() -> bool {
    Command::new("git")
        .arg("--version")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

pub fn status_response() -> Value {
    let git_available = git_available();
    json!({
        "status": if git_available { "ok" } else { "failed" },
        "feature": "upstream-worktree",
        "gitAvailable": git_available,
        "platformSupported": true,
    })
}

fn repo_root(repo_path: &Path) -> UpstreamWorktreeResult<PathBuf> {
    let output = git_in_repo(repo_path, &["rev-parse", "--show-toplevel"])
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if !output.status_success || output.stdout.is_empty() {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::NotGitRepo,
            "Path is not inside a Git repository",
        ));
    }
    Ok(PathBuf::from(output.stdout))
}

fn current_branch(repo_root: &Path) -> String {
    git_in_repo(repo_root, &["branch", "--show-current"])
        .ok()
        .filter(|output| output.status_success)
        .map(|output| output.stdout)
        .filter(|branch| !branch.is_empty())
        .unwrap_or_default()
}

fn remote_names(repo_root: &Path) -> UpstreamWorktreeResult<Vec<String>> {
    let output = git_in_repo(repo_root, &["remote"])
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if !output.status_success {
        return Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::RemoteMissing,
            "Cannot read Git remotes",
        ));
    }
    Ok(output
        .stdout
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn failed_response(error: UpstreamWorktreeError) -> Value {
    error.to_value()
}

pub fn defaults_response(payload: &Value) -> Value {
    let repo_path = string_field(payload, "repoPath");
    if repo_path.is_empty() {
        return failed_response(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::NotGitRepo,
            "Repository path is required",
        ));
    }
    match defaults_for_repo(Path::new(&repo_path)) {
        Ok(value) => value,
        Err(error) => failed_response(error),
    }
}

fn defaults_for_repo(repo_path: &Path) -> UpstreamWorktreeResult<Value> {
    let root = repo_root(repo_path)?;
    let branch = current_branch(&root);
    let remotes = remote_names(&root)?;
    let default_base_branch = if branch.is_empty() { "main".to_string() } else { branch.clone() };
    Ok(json!({
        "status": "ok",
        "repoRoot": root.to_string_lossy(),
        "currentBranch": branch,
        "defaultBaseBranch": default_base_branch,
        "remotes": remotes,
        "defaultRemote": default_remote_name(&remotes),
    }))
}
```

- [ ] **Step 4: Implement create flow**

Append these functions to `crates/codex-plus-core/src/upstream_worktree.rs`:

```rust
fn normalize_worktree_path(repo_root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    }
}

fn ensure_remote_exists(remotes: &[String], remote: &str) -> UpstreamWorktreeResult<()> {
    if remotes.iter().any(|candidate| candidate == remote) {
        Ok(())
    } else {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::RemoteMissing,
            format!("Remote does not exist: {remote}"),
        ))
    }
}

fn ensure_branch_is_available(repo_root: &Path, branch_name: &str) -> UpstreamWorktreeResult<()> {
    let output = git_in_repo(repo_root, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{branch_name}")])
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if output.status_success {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::BranchExists,
            format!("Branch already exists: {branch_name}"),
        ))
    } else {
        Ok(())
    }
}

fn ensure_worktree_path_available(path: &Path) -> UpstreamWorktreeResult<()> {
    if path.exists() {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::PathExists,
            format!("Worktree path already exists: {}", path.display()),
        ))
    } else {
        Ok(())
    }
}

fn fetch_remote_branch(repo_root: &Path, remote: &str, base_branch: &str) -> UpstreamWorktreeResult<()> {
    let refspec = format!("+refs/heads/{base_branch}:refs/remotes/{remote}/{base_branch}");
    let output = git_in_repo(repo_root, &["fetch", remote, &refspec])
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if output.status_success {
        Ok(())
    } else {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::FetchFailed,
            if output.stderr.is_empty() {
                format!("Failed to fetch {remote}/{base_branch}")
            } else {
                output.stderr
            },
        ))
    }
}

fn ensure_source_ref_exists(repo_root: &Path, source_ref: &str) -> UpstreamWorktreeResult<String> {
    let output = git_in_repo(repo_root, &["rev-parse", "--verify", source_ref])
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if output.status_success && !output.stdout.is_empty() {
        Ok(output.stdout)
    } else {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::BaseBranchMissing,
            format!("Base branch does not exist: {source_ref}"),
        ))
    }
}

fn add_worktree(repo_root: &Path, branch_name: &str, worktree_path: &Path, source_ref: &str) -> UpstreamWorktreeResult<()> {
    let mut args = vec![
        OsString::from("-C"),
        repo_root.as_os_str().to_os_string(),
        OsString::from("worktree"),
        OsString::from("add"),
        OsString::from("-b"),
        OsString::from(branch_name),
        worktree_path.as_os_str().to_os_string(),
        OsString::from(source_ref),
    ];
    let output = git_output(std::mem::take(&mut args))
        .map_err(|_| UpstreamWorktreeError::new(UpstreamWorktreeCode::GitMissing, "Git is not available"))?;
    if output.status_success {
        Ok(())
    } else {
        Err(UpstreamWorktreeError::new(
            UpstreamWorktreeCode::WorktreeCreateFailed,
            if output.stderr.is_empty() {
                "Failed to create worktree".to_string()
            } else {
                output.stderr
            },
        ))
    }
}

pub fn create_response(payload: &Value) -> Value {
    match create_worktree(payload) {
        Ok(value) => value,
        Err(error) => failed_response(error),
    }
}

fn create_worktree(payload: &Value) -> UpstreamWorktreeResult<Value> {
    let request = request_from_payload(payload)?;
    let root = repo_root(&request.repo_path)?;
    let remotes = remote_names(&root)?;
    ensure_remote_exists(&remotes, &request.remote)?;
    ensure_branch_is_available(&root, &request.branch_name)?;
    let worktree_path = normalize_worktree_path(&root, &request.worktree_path);
    ensure_worktree_path_available(&worktree_path)?;
    if request.fetch {
        fetch_remote_branch(&root, &request.remote, &request.base_branch)?;
    }
    let source_ref = source_ref(&request.remote, &request.base_branch);
    let source_head = ensure_source_ref_exists(&root, &source_ref)?;
    add_worktree(&root, &request.branch_name, &worktree_path, &source_ref)?;
    Ok(json!({
        "status": "ok",
        "repoRoot": root.to_string_lossy(),
        "worktreePath": worktree_path.to_string_lossy(),
        "branchName": request.branch_name,
        "sourceRef": source_ref,
        "sourceHead": source_head,
    }))
}
```

- [ ] **Step 5: Run upstream worktree tests**

Run:

```bash
cargo test -p codex-plus-core --test upstream_worktree
```

Expected: PASS for all upstream worktree tests.

- [ ] **Step 6: Commit Git create flow**

Run:

```bash
git add "crates/codex-plus-core/src/upstream_worktree.rs" "crates/codex-plus-core/tests/upstream_worktree.rs"
git commit -m "feat(worktree): create worktrees from upstream refs"
```

---

### Task 3: Expose upstream worktree bridge routes

**Files:**
- Modify: `crates/codex-plus-core/src/routes.rs`
- Modify: `apps/codex-plus-launcher/src/main.rs`
- Modify: `crates/codex-plus-core/tests/bridge_routes.rs`

- [ ] **Step 1: Write failing bridge route expectations**

In `crates/codex-plus-core/tests/bridge_routes.rs`, add these route cases to `bridge_routes_cover_all_current_paths()` after the Zed remote cases:

```rust
        ("/upstream-worktree/status", json!({})),
        (
            "/upstream-worktree/defaults",
            json!({"repoPath": "/repo"}),
        ),
        (
            "/upstream-worktree/create",
            json!({
                "repoPath": "/repo",
                "branchName": "feature/demo",
                "worktreePath": "/worktrees/demo",
                "remote": "upstream",
                "baseBranch": "main",
                "fetch": true
            }),
        ),
```

Add this test near the existing runtime route tests:

```rust
#[tokio::test]
async fn upstream_worktree_routes_are_dispatched_to_runtime() {
    let ctx = test_context();

    assert_eq!(
        handle_bridge_request(ctx.clone(), "/upstream-worktree/status", json!({})).await,
        json!({"status": "ok", "feature": "upstream-worktree", "gitAvailable": true, "platformSupported": true})
    );
    assert_eq!(
        handle_bridge_request(ctx.clone(), "/upstream-worktree/defaults", json!({"repoPath": "/repo"})).await,
        json!({
            "status": "ok",
            "repoRoot": "/repo",
            "currentBranch": "main",
            "defaultBaseBranch": "main",
            "remotes": ["origin", "upstream"],
            "defaultRemote": "upstream"
        })
    );
    assert_eq!(
        handle_bridge_request(
            ctx,
            "/upstream-worktree/create",
            json!({
                "repoPath": "/repo",
                "branchName": "feature/demo",
                "worktreePath": "/worktrees/demo",
                "remote": "upstream",
                "baseBranch": "main",
                "fetch": true
            }),
        )
        .await,
        json!({
            "status": "ok",
            "repoRoot": "/repo",
            "worktreePath": "/worktrees/demo",
            "branchName": "feature/demo",
            "sourceRef": "upstream/main",
            "sourceHead": "abc123"
        })
    );
}
```

Add these methods to the `impl BridgeRuntimeService for FakeRuntime` block:

```rust
    async fn upstream_worktree_status(&self) -> anyhow::Result<Value> {
        Ok(json!({
            "status": "ok",
            "feature": "upstream-worktree",
            "gitAvailable": true,
            "platformSupported": true
        }))
    }

    async fn upstream_worktree_defaults(&self, payload: Value) -> anyhow::Result<Value> {
        assert_eq!(payload["repoPath"], json!("/repo"));
        Ok(json!({
            "status": "ok",
            "repoRoot": "/repo",
            "currentBranch": "main",
            "defaultBaseBranch": "main",
            "remotes": ["origin", "upstream"],
            "defaultRemote": "upstream"
        }))
    }

    async fn upstream_worktree_create(&self, payload: Value) -> anyhow::Result<Value> {
        assert_eq!(payload["repoPath"], json!("/repo"));
        assert_eq!(payload["branchName"], json!("feature/demo"));
        assert_eq!(payload["worktreePath"], json!("/worktrees/demo"));
        assert_eq!(payload["remote"], json!("upstream"));
        assert_eq!(payload["baseBranch"], json!("main"));
        Ok(json!({
            "status": "ok",
            "repoRoot": "/repo",
            "worktreePath": "/worktrees/demo",
            "branchName": "feature/demo",
            "sourceRef": "upstream/main",
            "sourceHead": "abc123"
        }))
    }
```

- [ ] **Step 2: Run route tests to verify they fail**

Run:

```bash
cargo test -p codex-plus-core --test bridge_routes upstream_worktree_routes_are_dispatched_to_runtime
```

Expected: FAIL because the trait and dispatch routes do not exist.

- [ ] **Step 3: Add bridge trait methods and route dispatch**

In `crates/codex-plus-core/src/routes.rs`, add these methods to `BridgeRuntimeService` after the Zed methods:

```rust
    async fn upstream_worktree_status(&self) -> anyhow::Result<Value>;
    async fn upstream_worktree_defaults(&self, payload: Value) -> anyhow::Result<Value>;
    async fn upstream_worktree_create(&self, payload: Value) -> anyhow::Result<Value>;
```

In `handle_bridge_request()`, add these match arms after the Zed remote arms:

```rust
        "/upstream-worktree/status" => ctx.runtime.upstream_worktree_status().await,
        "/upstream-worktree/defaults" => {
            ctx.runtime.upstream_worktree_defaults(payload.clone()).await
        }
        "/upstream-worktree/create" => ctx.runtime.upstream_worktree_create(payload.clone()).await,
```

In `impl BridgeRuntimeService for CoreRuntimeService`, add:

```rust
    async fn upstream_worktree_status(&self) -> anyhow::Result<Value> {
        Ok(crate::upstream_worktree::status_response())
    }

    async fn upstream_worktree_defaults(&self, payload: Value) -> anyhow::Result<Value> {
        Ok(crate::upstream_worktree::defaults_response(&payload))
    }

    async fn upstream_worktree_create(&self, payload: Value) -> anyhow::Result<Value> {
        Ok(crate::upstream_worktree::create_response(&payload))
    }
```

- [ ] **Step 4: Wire launcher runtime methods**

In `apps/codex-plus-launcher/src/main.rs`, add these methods to the `impl BridgeRuntimeService for LauncherRuntimeService` block after `open_zed_remote`:

```rust
    async fn upstream_worktree_status(&self) -> anyhow::Result<Value> {
        Ok(codex_plus_core::upstream_worktree::status_response())
    }

    async fn upstream_worktree_defaults(&self, payload: Value) -> anyhow::Result<Value> {
        Ok(codex_plus_core::upstream_worktree::defaults_response(&payload))
    }

    async fn upstream_worktree_create(&self, payload: Value) -> anyhow::Result<Value> {
        Ok(codex_plus_core::upstream_worktree::create_response(&payload))
    }
```

- [ ] **Step 5: Run bridge tests**

Run:

```bash
cargo test -p codex-plus-core --test bridge_routes
```

Expected: PASS.

- [ ] **Step 6: Run full core tests**

Run:

```bash
cargo test -p codex-plus-core
```

Expected: PASS.

- [ ] **Step 7: Commit bridge routes**

Run:

```bash
git add "crates/codex-plus-core/src/routes.rs" "apps/codex-plus-launcher/src/main.rs" "crates/codex-plus-core/tests/bridge_routes.rs"
git commit -m "feat(worktree): expose upstream worktree bridge routes"
```

---

### Task 4: Add Codex++ menu workflow

**Files:**
- Modify: `assets/inject/renderer-inject.js`

- [ ] **Step 1: Add renderer constants and settings**

In `assets/inject/renderer-inject.js`, add this constant near the other top-level class constants:

```javascript
  const upstreamWorktreeDialogClass = "codex-upstream-worktree-dialog";
```

Update `defaultCodexPlusSettings()` so the returned object includes `upstreamWorktreeCreate: true`:

```javascript
  function defaultCodexPlusSettings() {
    return { pluginEntryUnlock: true, forcePluginInstall: true, modelWhitelistUnlock: true, sessionDelete: true, markdownExport: true, projectMove: true, conversationTimeline: true, conversationView: false, conversationViewMaxWidth: conversationViewDefaultWidth, threadScrollRestore: true, zedRemoteOpen: true, upstreamWorktreeCreate: true, nativeMenuPlacement: true };
  }
```

Update the disabled-enhancements return object in `codexPlusSettings()` so it includes:

```javascript
        upstreamWorktreeCreate: false,
```

- [ ] **Step 2: Add menu row and action button**

In the home panel HTML in `openCodexPlusModal()`, insert this row immediately after the `Zed Remote open` row:

```html
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">Upstream worktree</div><div class="codex-plus-row-description">Create a Git worktree from a fresh upstream branch, equivalent to git worktree add -b branch path upstream/base.</div></div>
              <div class="codex-plus-worktree-actions">
                <button type="button" class="codex-plus-action-button" data-codex-upstream-worktree-open="true">创建</button>
                <button type="button" class="codex-plus-toggle" data-codex-plus-setting="upstreamWorktreeCreate"><span></span></button>
              </div>
            </div>
```

- [ ] **Step 3: Add modal helpers**

Add these functions near `showToast()` and `escapeHtml()`:

```javascript
  function upstreamWorktreeField(dialog, name) {
    return dialog.querySelector(`[data-codex-upstream-worktree-field="${name}"]`);
  }

  function upstreamWorktreePayload(dialog) {
    return {
      repoPath: upstreamWorktreeField(dialog, "repoPath")?.value || "",
      branchName: upstreamWorktreeField(dialog, "branchName")?.value || "",
      worktreePath: upstreamWorktreeField(dialog, "worktreePath")?.value || "",
      remote: upstreamWorktreeField(dialog, "remote")?.value || "upstream",
      baseBranch: upstreamWorktreeField(dialog, "baseBranch")?.value || "main",
      fetch: true,
    };
  }

  function setUpstreamWorktreeMessage(dialog, message, status = "idle") {
    const messageNode = dialog.querySelector("[data-codex-upstream-worktree-message]");
    if (!messageNode) return;
    messageNode.dataset.status = status;
    messageNode.textContent = message || "";
  }

  async function loadUpstreamWorktreeDefaults(dialog) {
    const repoPath = upstreamWorktreeField(dialog, "repoPath")?.value?.trim() || "";
    if (!repoPath) {
      setUpstreamWorktreeMessage(dialog, "填写仓库路径后会自动读取 remote 和当前分支。", "idle");
      return;
    }
    setUpstreamWorktreeMessage(dialog, "正在读取仓库默认值…", "loading");
    const result = await postJson("/upstream-worktree/defaults", { repoPath });
    if (result?.status !== "ok") {
      setUpstreamWorktreeMessage(dialog, result?.message || "读取仓库默认值失败", "failed");
      return;
    }
    const remote = upstreamWorktreeField(dialog, "remote");
    const baseBranch = upstreamWorktreeField(dialog, "baseBranch");
    if (remote && !remote.value) remote.value = result.defaultRemote || "upstream";
    if (baseBranch && (!baseBranch.value || baseBranch.value === "main")) baseBranch.value = result.defaultBaseBranch || "main";
    setUpstreamWorktreeMessage(dialog, `将从 ${remote?.value || "upstream"}/${baseBranch?.value || "main"} 创建 worktree。`, "ok");
  }

  async function submitUpstreamWorktree(dialog) {
    const payload = upstreamWorktreePayload(dialog);
    if (!payload.repoPath || !payload.branchName || !payload.worktreePath || !payload.remote || !payload.baseBranch) {
      setUpstreamWorktreeMessage(dialog, "仓库路径、分支名、worktree 路径、remote 和 base branch 都必须填写。", "failed");
      return;
    }
    setUpstreamWorktreeMessage(dialog, "正在 fetch 并创建 worktree…", "loading");
    const result = await postJson("/upstream-worktree/create", payload);
    if (result?.status === "ok") {
      setUpstreamWorktreeMessage(dialog, `已从 ${result.sourceRef} 创建：${result.worktreePath}`, "ok");
      showToast(`已创建 upstream worktree：${result.branchName}`, null);
    } else {
      setUpstreamWorktreeMessage(dialog, result?.message || "创建 upstream worktree 失败", "failed");
    }
  }

  function openUpstreamWorktreeDialog() {
    document.querySelectorAll(`.${upstreamWorktreeDialogClass}`).forEach((node) => node.remove());
    const overlay = document.createElement("div");
    overlay.className = `codex-delete-confirm-overlay ${upstreamWorktreeDialogClass}`;
    overlay.innerHTML = `
      <div class="codex-delete-confirm-content" role="dialog" aria-modal="true" aria-label="Create upstream worktree">
        <div class="codex-delete-confirm-title">Create from upstream</div>
        <div class="codex-delete-confirm-message">等价于 git worktree add -b branch path upstream/base。创建前会先 fetch 远端分支。</div>
        <label class="codex-plus-form-field">仓库路径<input data-codex-upstream-worktree-field="repoPath" type="text" placeholder="/path/to/repo"></label>
        <label class="codex-plus-form-field">新分支名<input data-codex-upstream-worktree-field="branchName" type="text" placeholder="feature/my-task"></label>
        <label class="codex-plus-form-field">Worktree 路径<input data-codex-upstream-worktree-field="worktreePath" type="text" placeholder="/path/to/worktrees/my-task"></label>
        <label class="codex-plus-form-field">Remote<input data-codex-upstream-worktree-field="remote" type="text" value="upstream"></label>
        <label class="codex-plus-form-field">Base branch<input data-codex-upstream-worktree-field="baseBranch" type="text" value="main"></label>
        <div class="codex-plus-form-message" data-codex-upstream-worktree-message>填写仓库路径后会自动读取 remote 和当前分支。</div>
        <div class="codex-delete-confirm-actions">
          <button type="button" data-codex-upstream-worktree-cancel="true">取消</button>
          <button type="button" data-codex-upstream-worktree-defaults="true">读取默认值</button>
          <button type="button" data-codex-upstream-worktree-submit="true">Create from upstream</button>
        </div>
      </div>
    `;
    overlay.addEventListener("click", (event) => {
      const target = event.target instanceof Element ? event.target : event.target?.parentElement;
      if (event.target === overlay || target?.closest("[data-codex-upstream-worktree-cancel]")) {
        overlay.remove();
        return;
      }
      if (target?.closest("[data-codex-upstream-worktree-defaults]")) {
        loadUpstreamWorktreeDefaults(overlay);
        return;
      }
      if (target?.closest("[data-codex-upstream-worktree-submit]")) {
        submitUpstreamWorktree(overlay);
      }
    }, true);
    upstreamWorktreeField(overlay, "repoPath")?.addEventListener("change", () => loadUpstreamWorktreeDefaults(overlay));
    document.body.appendChild(overlay);
    upstreamWorktreeField(overlay, "repoPath")?.focus();
  }
```

- [ ] **Step 4: Wire click handling**

In the `overlay.addEventListener("click", ...)` handler inside `openCodexPlusModal()`, add this block before the generic `[data-codex-plus-setting]` toggle block:

```javascript
      if (target?.closest("[data-codex-upstream-worktree-open]")) {
        if (!codexPlusSettings().upstreamWorktreeCreate) {
          showToast("Upstream worktree enhancement is disabled", null);
          return;
        }
        openUpstreamWorktreeDialog();
        return;
      }
```

- [ ] **Step 5: Add minimal form styles**

In `installStyle()`, add these CSS rules inside `style.textContent`:

```css
      .codex-plus-worktree-actions {
        display: inline-flex;
        align-items: center;
        gap: 8px;
      }
      .codex-plus-form-field {
        display: grid;
        gap: 4px;
        margin-top: 10px;
        color: #d4d4d8;
        font: 12px system-ui, sans-serif;
        text-align: left;
      }
      .codex-plus-form-field input {
        width: min(520px, 72vw);
        border: 1px solid rgba(255,255,255,.18);
        border-radius: 8px;
        background: #18181b;
        color: #f4f4f5;
        padding: 8px 10px;
        font: 13px ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas, monospace;
      }
      .codex-plus-form-message {
        min-height: 18px;
        margin-top: 10px;
        color: #a1a1aa;
        font: 12px system-ui, sans-serif;
        text-align: left;
      }
      .codex-plus-form-message[data-status="ok"] { color: #34d399; }
      .codex-plus-form-message[data-status="failed"] { color: #f87171; }
      .codex-plus-form-message[data-status="loading"] { color: #fbbf24; }
```

- [ ] **Step 6: Validate renderer syntax**

Run:

```bash
node --check "assets/inject/renderer-inject.js"
```

Expected: PASS with no syntax errors.

- [ ] **Step 7: Commit menu workflow**

Run:

```bash
git add "assets/inject/renderer-inject.js"
git commit -m "feat(worktree): add upstream worktree menu workflow"
```

---

### Task 5: Add guarded native Codex worktree adapter

**Files:**
- Modify: `assets/inject/renderer-inject.js`

- [ ] **Step 1: Add guarded adapter helpers**

Add these functions near the upstream worktree modal helpers:

```javascript
  function upstreamWorktreeNativePayloadFromElement(element) {
    const root = element?.closest?.("[data-codex-worktree-create], [data-worktree-create], form, [role='dialog']") || element;
    if (!root?.querySelector) return null;
    const valueFrom = (selectors) => {
      for (const selector of selectors) {
        const node = root.querySelector(selector);
        const value = node?.value || node?.getAttribute?.("data-value") || node?.textContent || "";
        if (String(value).trim()) return String(value).trim();
      }
      return "";
    };
    const repoPath = root.getAttribute?.("data-repo-path") || valueFrom(["[data-repo-path]", "[name='repoPath']", "[name='repo']"]);
    const branchName = root.getAttribute?.("data-branch-name") || valueFrom(["[data-branch-name]", "[name='branchName']", "[name='branch']"]);
    const worktreePath = root.getAttribute?.("data-worktree-path") || valueFrom(["[data-worktree-path]", "[name='worktreePath']", "[name='path']"]);
    const remote = root.getAttribute?.("data-remote") || valueFrom(["[data-remote]", "[name='remote']"]) || "upstream";
    const baseBranch = root.getAttribute?.("data-base-branch") || valueFrom(["[data-base-branch]", "[name='baseBranch']", "[name='base']"]) || "main";
    if (!repoPath || !branchName || !worktreePath || !remote || !baseBranch) return null;
    return { repoPath, branchName, worktreePath, remote, baseBranch, fetch: true };
  }

  async function handleUpstreamWorktreeNativeCreate(event) {
    if (!codexPlusSettings().upstreamWorktreeCreate) return false;
    const trigger = event.target?.closest?.("[data-codex-worktree-create], [data-worktree-create]");
    if (!trigger) return false;
    const payload = upstreamWorktreeNativePayloadFromElement(trigger);
    if (!payload) {
      showToast("无法安全识别 Codex 原生 worktree 表单，请使用 Codex++ 菜单创建。", null);
      return false;
    }
    event.preventDefault();
    event.stopPropagation();
    const result = await postJson("/upstream-worktree/create", payload);
    if (result?.status === "ok") {
      showToast(`已从 ${result.sourceRef} 创建 worktree`, null);
    } else {
      showToast(result?.message || "创建 upstream worktree 失败", null);
    }
    return true;
  }

  function installUpstreamWorktreeNativeAdapter() {
    if (window.__codexUpstreamWorktreeNativeAdapterInstalled === "1") return;
    window.__codexUpstreamWorktreeNativeAdapterInstalled = "1";
    document.addEventListener("click", (event) => {
      handleUpstreamWorktreeNativeCreate(event);
    }, true);
  }
```

- [ ] **Step 2: Install adapter in main loop**

Find the main enhancement installation section near the bottom of `renderer-inject.js`. Add this call next to the other installer calls:

```javascript
  installUpstreamWorktreeNativeAdapter();
```

If the bottom section has a repeated timer or observer callback, place the call where it runs once per injected script revision, not on every DOM mutation.

- [ ] **Step 3: Validate renderer syntax**

Run:

```bash
node --check "assets/inject/renderer-inject.js"
```

Expected: PASS with no syntax errors.

- [ ] **Step 4: Commit native adapter**

Run:

```bash
git add "assets/inject/renderer-inject.js"
git commit -m "feat(worktree): guard native Codex worktree creation"
```

---

### Task 6: Document and verify the feature

**Files:**
- Modify: `README.md`
- Modify: `README_EN.md`

- [ ] **Step 1: Update Chinese README feature bullet**

In `README.md`, add this bullet under `主要功能` near the existing Zed Remote bullet:

```markdown
- Upstream worktree 创建：可从 `upstream/<base-branch>` 创建新 worktree，创建前自动 fetch 远端分支，降低从陈旧本地 HEAD 派生导致的冲突风险。
```

Add this FAQ section near the Git/Zed or enhancement FAQ sections:

```markdown
### Upstream worktree 和 Codex 原生创建有什么区别

Codex++ 的 Upstream worktree 功能等价于先更新远端分支，再执行：

```bash
git worktree add -b <new-branch> <worktree-path> upstream/<base-branch>
```

这样新 worktree 从最新的远端跟踪分支开始，而不是从当前会话所在的本地 HEAD 开始。如果 Codex++ 无法安全识别当前 Codex 版本的原生 worktree 创建表单，请从 Codex++ 菜单中手动填写仓库路径、分支名、worktree 路径、remote 和 base branch。
```

- [ ] **Step 2: Update English README feature bullet**

In `README_EN.md`, add this bullet under the main feature list near the Zed Remote bullet:

```markdown
- Upstream worktree creation: create new worktrees from `upstream/<base-branch>` after fetching the remote branch, reducing conflicts caused by stale local HEAD state.
```

Add this FAQ section near the enhancement FAQ sections:

```markdown
### How is Upstream worktree different from Codex native creation?

Codex++ updates the remote branch first, then creates the worktree as if you ran:

```bash
git worktree add -b <new-branch> <worktree-path> upstream/<base-branch>
```

The new worktree starts from the fresh remote tracking branch instead of the local HEAD used by the current session. If Codex++ cannot safely recognize the current Codex version's native worktree form, use the Codex++ menu entry and enter the repository path, branch name, worktree path, remote, and base branch manually.
```

- [ ] **Step 3: Run deterministic validation**

Run:

```bash
cargo fmt --check
cargo test -p codex-plus-core --test upstream_worktree
cargo test -p codex-plus-core --test bridge_routes
cargo test -p codex-plus-core
node --check "assets/inject/renderer-inject.js"
```

Expected:

```text
cargo fmt --check: PASS
upstream_worktree tests: PASS
bridge_routes tests: PASS
codex-plus-core tests: PASS
node --check: PASS
```

- [ ] **Step 4: Manual verification with a temporary repo**

Run this manual command sequence from any scratch directory:

```bash
TMPDIR="$(mktemp -d)"
git init --bare "$TMPDIR/upstream.git"
git init "$TMPDIR/seed"
git -C "$TMPDIR/seed" config user.email "test@example.com"
git -C "$TMPDIR/seed" config user.name "Test User"
printf 'v1\n' > "$TMPDIR/seed/README.md"
git -C "$TMPDIR/seed" add README.md
git -C "$TMPDIR/seed" commit -m "initial"
git -C "$TMPDIR/seed" branch -M main
git -C "$TMPDIR/seed" remote add upstream "$TMPDIR/upstream.git"
git -C "$TMPDIR/seed" push -u upstream main
git clone "$TMPDIR/upstream.git" "$TMPDIR/repo"
git -C "$TMPDIR/repo" remote rename origin upstream
printf 'v2\n' > "$TMPDIR/seed/README.md"
git -C "$TMPDIR/seed" add README.md
git -C "$TMPDIR/seed" commit -m "remote update"
git -C "$TMPDIR/seed" push upstream main
```

Then launch Codex++ from the built app or development launcher, open the Codex++ menu, create:

```text
Repository path: $TMPDIR/repo
New branch name: feature/from-upstream
Worktree path: $TMPDIR/worktree
Remote: upstream
Base branch: main
```

Verify:

```bash
git -C "$TMPDIR/repo" rev-parse upstream/main
git -C "$TMPDIR/worktree" rev-parse HEAD
```

Expected: both commands print the same commit hash.

- [ ] **Step 5: Commit docs and final validation evidence**

Run:

```bash
git add "README.md" "README_EN.md"
git commit -m "docs(worktree): document upstream worktree creation"
```

---

## Self-Review Checklist

- [ ] Spec coverage: backend module, bridge routes, menu entry, guarded native adapter, errors, tests, README updates are each covered by tasks.
- [ ] Placeholder scan: the plan contains no deferred implementation markers.
- [ ] Type consistency: route names, setting key `upstreamWorktreeCreate`, Rust module name `upstream_worktree`, and JSON fields match across tasks.
- [ ] TDD sequence: each backend task starts with failing tests, then implementation, then passing tests.
- [ ] Safety: Git commands use argument arrays; no shell interpolation is introduced in Rust.
- [ ] Verification: final task includes Cargo, renderer syntax, and manual upstream-head equivalence checks.
