//! Codex++ 的任务栏集成仅写分组、名称和重启入口，图标由 Codex 管理。

use windows::Win32::Foundation::HWND;
use windows::Win32::Storage::EnhancedStorage::{
    PKEY_AppUserModel_ID, PKEY_AppUserModel_RelaunchCommand,
    PKEY_AppUserModel_RelaunchDisplayNameResource,
};
use windows::Win32::UI::Shell::PropertiesSystem::{IPropertyStore, SHGetPropertyStoreForWindow};
use windows::core::PROPVARIANT;

pub(crate) fn apply_to_window(hwnd: HWND) -> anyhow::Result<()> {
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(hwnd)? };
    let relaunch_command = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "codex-plus-plus.exe".to_string());

    for (key, value) in [
        (PKEY_AppUserModel_ID, "com.bigpizzav3.codexplusplus.codex"),
        (PKEY_AppUserModel_RelaunchDisplayNameResource, "Codex++"),
        (PKEY_AppUserModel_RelaunchCommand, relaunch_command.as_str()),
    ] {
        unsafe { store.SetValue(&key, &PROPVARIANT::from(value))? };
    }
    unsafe { store.Commit()? };
    Ok(())
}

#[cfg(test)]
mod tests;
