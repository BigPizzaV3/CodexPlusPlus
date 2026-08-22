# PR Build Actions Design

## Goal

Add a GitHub Actions workflow that builds downloadable Windows and macOS test artifacts for pull requests, pushes to `main`, and manual runs, without changing the existing release publishing workflow.

## Existing Context

The repository already has `.github/workflows/release-assets.yml`. That workflow runs only when a GitHub Release is published and uploads official Windows installer, macOS DMGs, and `latest.json` release metadata. The new workflow should not replace or broaden that release workflow. It should produce temporary CI artifacts only.

The project build entry points are:

- `apps/codex-plus-manager/package.json`
  - `npm run check` runs TypeScript checking.
  - `npm run vite:build` builds the frontend.
  - `npm run build` locally builds launcher and Tauri manager, but CI can use more explicit steps to stage artifacts.
- Workspace root `Cargo.toml`
  - `cargo test --workspace` validates Rust crates and apps.
  - `cargo build --release` builds Windows binaries on Windows.
  - `cargo build --release --target <mac target>` builds macOS binaries on macOS.
- Existing installer scripts:
  - Windows: `scripts/installer/windows/CodexPlusPlus.nsi`
  - macOS: `scripts/installer/macos/package-dmg.sh`

## Workflow

Create `.github/workflows/pr-build.yml` with these triggers:

- `pull_request`
- `push` to `main`
- `workflow_dispatch`

Use minimal permissions:

```yaml
permissions:
  contents: read
```

The workflow produces artifacts for testing a specific PR or commit. It does not upload GitHub Release assets and does not require write permissions.

## Windows Job

Run on `windows-latest`.

Steps:

1. Checkout the repository.
2. Set up Node 22.
3. Set up Rust stable.
4. Install NSIS with Chocolatey.
5. Install frontend dependencies in `apps/codex-plus-manager` using `npm install --package-lock=false`.
6. Run `npm run check` in `apps/codex-plus-manager`.
7. Run `cargo test --workspace` from the repository root.
8. Run `npm run vite:build` in `apps/codex-plus-manager`.
9. Run `cargo build --release` from the repository root.
10. Stage Windows binaries into `dist/windows/app`:
    - `target/release/codex-plus-plus.exe`
    - `target/release/codex-plus-plus-manager.exe`
11. Build the NSIS installer using `scripts/installer/windows/CodexPlusPlus.nsi`.
12. Upload two artifacts:
    - `codex-plus-plus-windows-binaries`
    - `codex-plus-plus-windows-installer`

The Windows job verifies TypeScript and Rust tests before uploading artifacts.

## macOS Job

Run a matrix with `fail-fast: false`:

- Intel:
  - runner: `macos-15-intel`
  - target: `x86_64-apple-darwin`
  - arch: `x64`
- Apple Silicon:
  - runner: `macos-14`
  - target: `aarch64-apple-darwin`
  - arch: `arm64`

Steps for each matrix entry:

1. Checkout the repository.
2. Set up Node 22.
3. Set up Rust stable with the matrix target.
4. Install frontend dependencies in `apps/codex-plus-manager` using `npm install --package-lock=false`.
5. Run `npm run vite:build` in `apps/codex-plus-manager`.
6. Run `cargo build --release --target <target>` from the repository root.
7. Build the DMG using `scripts/installer/macos/package-dmg.sh`, passing the package version and architecture.
8. Upload the generated DMG as an artifact named for the architecture:
   - `codex-plus-plus-macos-x64-dmg`
   - `codex-plus-plus-macos-arm64-dmg`

The macOS job focuses on producing downloadable DMGs. Rust unit tests remain covered by the Windows job to keep PR runtime reasonable while still validating the workspace.

## Artifact Policy

Artifacts are temporary GitHub Actions artifacts scoped to a workflow run. They are for PR/main/manual testing only.

Official release downloads remain controlled by `.github/workflows/release-assets.yml` and GitHub Releases.

## Error Handling

- Use independent Windows and macOS jobs so platform-specific failures are easy to diagnose.
- Use `fail-fast: false` for macOS matrix builds so one architecture does not cancel the other.
- If NSIS or packaging fails, the build should fail rather than silently skipping installer output.

## Testing and Verification

After implementation:

1. Validate the workflow file is present at `.github/workflows/pr-build.yml`.
2. Run a local repository build command that matches the existing project build path:
   - `npm --prefix /e/Desktop/CodexPlusPlus/apps/codex-plus-manager run build`
3. Push the workflow to GitHub and verify a PR or manual run uploads Windows and macOS artifacts.

Local execution cannot fully prove GitHub-hosted macOS runner behavior, so the first GitHub Actions run is the final integration verification.
