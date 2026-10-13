pub use codex_plus_core::install::{
    EntryPointState, InstallActionResult, InstallOptions, ShortcutState, inspect_entrypoints,
};

pub fn repair_shortcuts() -> InstallActionResult {
    codex_plus_core::install::repair_entrypoints(&InstallOptions::default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compatibility_fields_report_the_same_application_entrypoint() {
        let state = inspect_entrypoints();

        assert_eq!(
            state.silent_shortcut.installed,
            state.management_shortcut.installed
        );
        assert_eq!(state.silent_shortcut.path, state.management_shortcut.path);
    }
}
