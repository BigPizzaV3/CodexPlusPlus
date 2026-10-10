#![cfg_attr(windows, windows_subsystem = "windows")]

/// Compatibility binary for the v1.7.x macOS updater.
///
/// The old updater requires a second manager bundle and opens it after replacing
/// both apps. The real Codex++ bundle is opened immediately afterward, so this
/// hidden shim only needs to be a valid small Mach-O process that exits cleanly.
fn main() {}
