use super::*;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::Storage::EnhancedStorage::PKEY_AppUserModel_RelaunchIconResource;
use windows::Win32::System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize};
use windows::Win32::UI::Shell::PropertiesSystem::PROPERTYKEY;
use windows::Win32::UI::WindowsAndMessaging::{
    CopyIcon, CreateWindowExW, DestroyIcon, DestroyWindow, HICON, ICON_BIG, ICON_SMALL,
    IDI_APPLICATION, LoadIconW, SW_SHOWMINNOACTIVE, SendMessageW, ShowWindow, WM_GETICON,
    WM_SETICON, WS_EX_TOOLWINDOW, WS_OVERLAPPEDWINDOW,
};
use windows::core::{GUID, w};

struct TestApartment;

impl TestApartment {
    fn new() -> Self {
        unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok().unwrap() };
        Self
    }
}

impl Drop for TestApartment {
    fn drop(&mut self) {
        unsafe { CoUninitialize() };
    }
}

struct TestWindow(HWND);

impl Drop for TestWindow {
    fn drop(&mut self) {
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

fn read(store: &IPropertyStore, key: &PROPERTYKEY) -> String {
    unsafe { store.GetValue(key).unwrap() }.to_string()
}

#[test]
fn both_taskbar_apis_preserve_native_icons_and_unrelated_properties() {
    let _apartment = TestApartment::new();
    let window = TestWindow(unsafe {
        CreateWindowExW(
            WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("Codex taskbar regression"),
            WS_OVERLAPPEDWINDOW,
            -32000,
            -32000,
            320,
            200,
            None,
            None,
            None,
            None,
        )
        .unwrap()
    });
    // 工具窗口不创建正式任务栏按钮，仍可走真实进程窗口查找入口。
    unsafe {
        let _ = ShowWindow(window.0, SW_SHOWMINNOACTIVE);
    }
    let original = unsafe { LoadIconW(None, IDI_APPLICATION).unwrap() };
    for kind in [ICON_BIG, ICON_SMALL] {
        unsafe {
            SendMessageW(
                window.0,
                WM_SETICON,
                WPARAM(kind as usize),
                LPARAM(original.0 as isize),
            );
        }
    }
    let store: IPropertyStore = unsafe { SHGetPropertyStoreForWindow(window.0).unwrap() };
    let unrelated = PROPERTYKEY {
        fmtid: GUID::from_u128(0x2fb8896b_1d19_42c5_a6a8_fca622a47738),
        pid: 1,
    };
    unsafe {
        store
            .SetValue(&unrelated, &PROPVARIANT::from("preserve-me"))
            .unwrap()
    };

    for seed in [None, Some("native.application.icon")] {
        if let Some(value) = seed {
            unsafe {
                store
                    .SetValue(
                        &PKEY_AppUserModel_RelaunchIconResource,
                        &PROPVARIANT::from(value),
                    )
                    .unwrap();
                store.Commit().unwrap();
            }
        }
        let icon_before = read(&store, &PKEY_AppUserModel_RelaunchIconResource);
        for legacy in [false, true] {
            for key in [
                PKEY_AppUserModel_ID,
                PKEY_AppUserModel_RelaunchDisplayNameResource,
                PKEY_AppUserModel_RelaunchCommand,
            ] {
                unsafe {
                    store
                        .SetValue(&key, &PROPVARIANT::from("before-test"))
                        .unwrap()
                };
            }
            unsafe { store.Commit().unwrap() };
            assert!(if legacy {
                crate::windows_apply_codexplusplus_icon_to_process_window(
                    std::process::id(),
                    std::path::PathBuf::from("ignored-icon.exe"),
                )
            } else {
                crate::windows_apply_codexplusplus_taskbar_to_process_window(std::process::id())
            });
            assert_eq!(
                read(&store, &PKEY_AppUserModel_RelaunchIconResource),
                icon_before
            );
            assert_eq!(read(&store, &unrelated), "preserve-me");
            for (key, expected) in [
                (
                    PKEY_AppUserModel_ID,
                    "com.bigpizzav3.codexplusplus.codex".to_string(),
                ),
                (
                    PKEY_AppUserModel_RelaunchDisplayNameResource,
                    "Codex++".to_string(),
                ),
                (
                    PKEY_AppUserModel_RelaunchCommand,
                    std::env::current_exe()
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                ),
            ] {
                assert_eq!(read(&store, &key), expected);
            }
            for kind in [ICON_BIG, ICON_SMALL] {
                let actual =
                    unsafe { SendMessageW(window.0, WM_GETICON, WPARAM(kind as usize), LPARAM(0)) };
                assert_eq!(actual.0, original.0 as isize);
                let copy = unsafe { CopyIcon(HICON(actual.0 as *mut core::ffi::c_void)).unwrap() };
                unsafe { DestroyIcon(copy).unwrap() };
            }
        }
    }
}

#[test]
fn both_taskbar_apis_report_missing_process_window() {
    assert!(!crate::windows_apply_codexplusplus_taskbar_to_process_window(0));
    assert!(!crate::windows_apply_codexplusplus_icon_to_process_window(
        0,
        std::path::PathBuf::from("ignored-icon.exe"),
    ));
}
