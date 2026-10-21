use anyhow::Context;
use codex_plus_core::global_dictation::{GlobalDictationConfig, GlobalHotkeyHandle};
use serde_json::json;
use std::sync::{Mutex, OnceLock};

struct RegisteredHotkey<H> {
    hotkey: String,
    _handle: H,
}

static REGISTRATION: OnceLock<Mutex<Option<RegisteredHotkey<GlobalHotkeyHandle>>>> = OnceLock::new();

pub fn apply_config() -> anyhow::Result<()> {
    let config = codex_plus_core::global_dictation::load_config();
    update_config(&config, |_| Ok(()))
}

pub fn save_and_apply_config(config: &GlobalDictationConfig) -> anyhow::Result<()> {
    update_config(config, codex_plus_core::global_dictation::save_config)
}

fn update_config(
    config: &GlobalDictationConfig,
    persist: impl FnOnce(&GlobalDictationConfig) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let mut registration = REGISTRATION.get_or_init(|| Mutex::new(None))
        .lock().map_err(|_| anyhow::anyhow!("全局听写热键状态不可用"))?;
    update_registration(&mut registration, config, |hotkey| {
        codex_plus_core::global_dictation::start(hotkey, || {
            let _ = tauri::async_runtime::block_on(trigger_global_toggle());
        })
    }, persist)
}

fn update_registration<H>(
    registration: &mut Option<RegisteredHotkey<H>>,
    config: &GlobalDictationConfig,
    register: impl FnOnce(&str) -> anyhow::Result<H>,
    persist: impl FnOnce(&GlobalDictationConfig) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    codex_plus_core::global_dictation::validate_hotkey(&config.hotkey)?;
    // 新键注册和配置保存均成功后才释放旧键；同一组合键重存时复用注册，避免占用自身。
    let same_hotkey = registration.as_ref().is_some_and(|current| {
        codex_plus_core::global_dictation::hotkeys_match(&current.hotkey, &config.hotkey)
    });
    let replacement = if config.enabled && !same_hotkey {
        Some(RegisteredHotkey {
            hotkey: config.hotkey.clone(),
            _handle: register(&config.hotkey).context("注册全局听写热键失败")?,
        })
    } else {
        None
    };
    persist(config).context("保存全局听写热键失败")?;
    if !config.enabled {
        *registration = None;
    } else if let Some(replacement) = replacement {
        *registration = Some(replacement);
    }
    Ok(())
}

async fn trigger_global_toggle() -> anyhow::Result<()> {
    let debug_port = codex_plus_core::status::StatusStore::default()
        .load_latest()
        .ok()
        .flatten()
        .and_then(|status| status.debug_port)
        .unwrap_or(9229);
    let targets = codex_plus_core::cdp::list_targets(debug_port).await?;
    let target = codex_plus_core::cdp::pick_injectable_codex_page_target(&targets)?;
    let websocket = target.web_socket_debugger_url.as_deref().context("Codex 页面没有 websocket")?;
    codex_plus_core::bridge::evaluate_script(
        websocket,
        "window.__codexPlusDictationGlobalToggle?.()",
    ).await?;
    let _ = codex_plus_core::diagnostic_log::append_diagnostic_log(
        "dictation.global_hotkey_toggle",
        json!({ "debugPort": debug_port }),
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    struct TestHandle(Arc<AtomicBool>);

    impl Drop for TestHandle {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn config(enabled: bool, hotkey: &str) -> GlobalDictationConfig {
        GlobalDictationConfig { enabled, hotkey: hotkey.into() }
    }

    fn active(hotkey: &str) -> (Option<RegisteredHotkey<TestHandle>>, Arc<AtomicBool>) {
        let dropped = Arc::new(AtomicBool::new(false));
        (Some(RegisteredHotkey { hotkey: hotkey.into(), _handle: TestHandle(dropped.clone()) }), dropped)
    }

    #[test]
    fn occupied_replacement_keeps_old_registration_and_saved_config() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("global-dictation.json");
        let old_config = config(true, "Ctrl+Shift+Space");
        std::fs::write(&path, serde_json::to_vec(&old_config).unwrap()).unwrap();
        let (mut registration, old_dropped) = active(&old_config.hotkey);
        let result = update_registration(&mut registration, &config(true, "Ctrl+Alt+Space"),
            |_| anyhow::bail!("occupied"),
            |_| panic!("注册失败时不应保存配置"));

        assert!(result.is_err());
        assert!(!old_dropped.load(Ordering::SeqCst));
        assert_eq!(registration.as_ref().unwrap().hotkey, old_config.hotkey);
        assert_eq!(serde_json::from_slice::<GlobalDictationConfig>(&std::fs::read(path).unwrap()).unwrap(), old_config);
    }

    #[test]
    fn failed_save_releases_new_registration_and_keeps_old_one() {
        let (mut registration, old_dropped) = active("Ctrl+Shift+Space");
        let new_dropped = Arc::new(AtomicBool::new(false));
        let result = update_registration(&mut registration, &config(true, "Ctrl+Alt+Space"),
            |_| Ok(TestHandle(new_dropped.clone())),
            |_| {
                assert!(!old_dropped.load(Ordering::SeqCst));
                assert!(!new_dropped.load(Ordering::SeqCst));
                anyhow::bail!("disk full")
            });

        assert!(result.is_err());
        assert!(!old_dropped.load(Ordering::SeqCst));
        assert!(new_dropped.load(Ordering::SeqCst));
        assert_eq!(registration.as_ref().unwrap().hotkey, "Ctrl+Shift+Space");
    }

    #[test]
    fn successful_save_replaces_registration_only_after_persisting() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("global-dictation.json");
        let new_config = config(true, "Ctrl+Alt+Space");
        let (mut registration, old_dropped) = active("Ctrl+Shift+Space");
        let new_dropped = Arc::new(AtomicBool::new(false));
        update_registration(&mut registration, &new_config,
            |_| Ok(TestHandle(new_dropped.clone())),
            |value| {
                assert!(!old_dropped.load(Ordering::SeqCst));
                std::fs::write(&path, serde_json::to_vec(value)?)?;
                Ok(())
            }).unwrap();

        assert!(old_dropped.load(Ordering::SeqCst));
        assert!(!new_dropped.load(Ordering::SeqCst));
        assert_eq!(registration.as_ref().unwrap().hotkey, new_config.hotkey);
        assert_eq!(serde_json::from_slice::<GlobalDictationConfig>(&std::fs::read(path).unwrap()).unwrap(), new_config);
    }

    #[test]
    fn saving_same_hotkey_reuses_current_registration() {
        let (mut registration, old_dropped) = active("Ctrl+Shift+Space");
        let saved = AtomicBool::new(false);
        update_registration(&mut registration, &config(true, "Ctrl+Shift+Space"),
            |_| panic!("同键重存不应重复注册"),
            |_| { saved.store(true, Ordering::SeqCst); Ok(()) }).unwrap();
        assert!(saved.load(Ordering::SeqCst));
        assert!(!old_dropped.load(Ordering::SeqCst));
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn saving_equivalent_hotkey_spelling_reuses_current_registration() {
        let (mut registration, old_dropped) = active("Ctrl+Shift+Space");
        update_registration(&mut registration, &config(true, "shift + CONTROL + space"),
            |_| panic!("同义组合键不应重复注册"), |_| Ok(())).unwrap();
        assert!(!old_dropped.load(Ordering::SeqCst));
    }

    #[test]
    fn disabling_keeps_registration_until_config_is_saved() {
        let (mut registration, old_dropped) = active("Ctrl+Shift+Space");
        let disabled = config(false, "Ctrl+Shift+Space");
        assert!(update_registration(&mut registration, &disabled,
            |_| panic!("关闭时不应注册"), |_| anyhow::bail!("disk full")).is_err());
        assert!(!old_dropped.load(Ordering::SeqCst));
        assert!(registration.is_some());

        update_registration(&mut registration, &disabled,
            |_| panic!("关闭时不应注册"),
            |_| { assert!(!old_dropped.load(Ordering::SeqCst)); Ok(()) }).unwrap();
        assert!(old_dropped.load(Ordering::SeqCst));
        assert!(registration.is_none());
    }
}
