# Contributing to CodexPlusPlus

Thank you for your interest in contributing to CodexPlusPlus!

## Development Setup

1. **Clone the repository**
   ```bash
   git clone https://github.com/BigPizzaV3/CodexPlusPlus.git
   cd CodexPlusPlus
   ```

2. **Install Rust toolchain**
   Ensure you have Rust 1.85+ installed:
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   rustc --version  # Should be 1.85+
   ```

3. **Build the project**
   ```bash
   cargo build --release
   ```

4. **Run tests**
   ```bash
   cargo test
   ```
   Integration tests of `codex-plus-core` and `codex-plus-data` are compiled into a single
   test binary under `tests/all/` (one `mod` per file in `main.rs`). Add new test files there,
   and run one file with e.g. `cargo test -p codex-plus-core --test all relay_config::`.

## Project Structure

```
CodexPlusPlus/
├── apps/
│   ├── codex-plus-launcher/  # Silent launcher
│   └── codex-plus-manager/   # Tauri manager
├── crates/
│   ├── codex-plus-data/    # Data handling and provider sync
│   └── codex-plus-core/    # Core Codex++ logic
└── README.md               # Project documentation
```

## Making Changes

1. **Create a feature branch**
   ```bash
   git checkout -b feat/your-feature-name
   ```

2. **Make your changes**
   - Write idiomatic Rust code
   - Add tests for new functionality
   - Update documentation as needed

3. **Run the test suite**
   ```bash
   cargo test --all-features
   cargo clippy  # Linting
   ```

4. **Commit your changes**
   ```bash
   git commit -m "feat: add your feature description"
   ```

## Code Style

- Follow Rust standard formatting (`cargo fmt`)
- Use `clippy` for linting recommendations
- Write self-documenting code with clear variable/function names
- Add doc comments (`///`) for public APIs

## UI Injected into Codex

UI that Codex++ injects into the Codex app (the `assets/inject/renderer-inject/` fragments: extensions page,
providers page, task board, …) should look and behave like Codex's own components, so it doesn't feel bolted on.

- **Measure the native component first.** Start an isolated Codex instance (separate `--user-data-dir` plus a
  remote debugging port), open the equivalent native component, and read its computed styles and the CSS
  variables its classes use (size, radius, padding, colors, shadow, blur) over CDP. Match those values; don't
  invent a new look.
- **Don't use native HTML form controls** such as `<select>`: their popups ignore the Codex theme. For dropdowns
  and menus reuse `.codex-plus-menu-trigger` / `.codex-plus-menu-popover` / `.codex-plus-menu-item` from
  `10-style.js` (see the sort menu in `41-extension-sources.js`).
- **Reference Codex's CSS variables with a fallback** instead of hard-coded colors, so light and dark themes both
  follow Codex, e.g. `--color-surface-elevated-secondary`, `--shadow-dropdown-overlay`,
  `--color-token-list-hover-background`, `--color-text-tertiary`.
- **Handle keys in the window capture phase.** Codex grabs key events in the document capture phase and sends
  them to its composer.
- Put a screenshot next to the native component you matched in the pull request.

## Pull Request Process

1. Fork the repository
2. Create your feature branch
3. Make your changes with adequate tests
4. Ensure all tests pass and clippy is clean
5. Submit a pull request with a clear description

## Reporting Issues

- Use GitHub Issues for bug reports and feature requests
- Include Rust version (`rustc --version`) and OS information
- For bugs, provide minimal reproduction steps

## License

By contributing, you agree that your contributions will be licensed under the project's [GNU Affero General Public License v3.0](LICENSE), using the SPDX identifier `AGPL-3.0-only`.
