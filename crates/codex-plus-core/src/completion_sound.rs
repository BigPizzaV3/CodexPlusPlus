use std::path::{Path, PathBuf};

use anyhow::Context;
use toml_edit::{Array, DocumentMut};

use crate::settings::BackendSettings;

const NOTIFY_SCRIPT_NAME: &str = "completion-sound-notify.sh";
const DEFAULT_SOUND_NAME: &str = "finish.mp3";

pub fn ensure_completion_sound_notify(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<()> {
    if !settings.codex_app_completion_sound_enabled {
        return Ok(());
    }
    let sound_path = ensure_completion_sound_file(home, settings)?;
    let previous_notify = current_notify_command(home)?;
    let script_path = home.join("codex-plus-plus").join(NOTIFY_SCRIPT_NAME);
    let previous_notify = previous_notify
        .filter(|notify| {
            notify
                .first()
                .map(|item| item != &script_path.to_string_lossy())
                .unwrap_or(true)
        })
        .unwrap_or_default();
    ensure_completion_sound_script(home, &sound_path, &previous_notify)?;
    ensure_notify_config(home, &script_path)?;
    Ok(())
}

fn ensure_completion_sound_file(
    home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<PathBuf> {
    let configured = settings.codex_app_completion_sound_path.trim();
    if !configured.is_empty() {
        return Ok(PathBuf::from(configured));
    }
    let sound_dir = home.join("codex-plus-plus");
    std::fs::create_dir_all(&sound_dir)
        .with_context(|| format!("failed to create {}", sound_dir.display()))?;
    let sound_path = sound_dir.join(DEFAULT_SOUND_NAME);
    if !sound_path.exists() {
        std::fs::write(&sound_path, crate::assets::default_completion_sound_bytes())
            .with_context(|| format!("failed to write {}", sound_path.display()))?;
    }
    Ok(sound_path)
}

fn ensure_completion_sound_script(
    home: &Path,
    sound_path: &Path,
    previous_notify: &[String],
) -> anyhow::Result<PathBuf> {
    let script_dir = home.join("codex-plus-plus");
    std::fs::create_dir_all(&script_dir)
        .with_context(|| format!("failed to create {}", script_dir.display()))?;
    let script_path = script_dir.join(NOTIFY_SCRIPT_NAME);
    let forward = previous_notify_script(previous_notify);
    let script = format!(
        "#!/bin/sh\n{forward}if [ \"$1\" = \"turn-ended\" ] || [ \"$CODEX_NOTIFY_EVENT\" = \"turn-ended\" ]; then\n  /usr/bin/afplay {} >/dev/null 2>&1 &\nfi\n",
        shell_quote(&sound_path.to_string_lossy())
    );
    std::fs::write(&script_path, script.as_bytes())
        .with_context(|| format!("failed to write {}", script_path.display()))?;
    make_executable(&script_path)?;
    Ok(script_path)
}

fn current_notify_command(home: &Path) -> anyhow::Result<Option<Vec<String>>> {
    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let doc = parse_toml_document(existing.trim_start_matches('\u{feff}'))?;
    Ok(doc
        .get("notify")
        .and_then(|item| item.as_array())
        .map(|array| {
            array
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect::<Vec<_>>()
        })
        .filter(|items| !items.is_empty()))
}

fn ensure_notify_config(home: &Path, script_path: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(home)?;
    let config_path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&config_path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", config_path.display()));
        }
    };
    let updated = completion_sound_notify_config_text(&existing, script_path)?;
    if updated.as_bytes() != existing.as_bytes() {
        crate::settings::atomic_write(&config_path, updated.as_bytes())
            .with_context(|| format!("failed to write {}", config_path.display()))?;
    }
    Ok(())
}

pub fn completion_sound_notify_config_text(
    config_text: &str,
    script_path: &Path,
) -> anyhow::Result<String> {
    let without_bom = config_text.trim_start_matches('\u{feff}');
    let mut doc = parse_toml_document(without_bom)?;
    let mut notify = Array::default();
    notify.push(script_path.to_string_lossy().as_ref());
    notify.push("turn-ended");
    doc["notify"] = toml_edit::value(notify);
    Ok(ensure_trailing_newline(doc.to_string()))
}

fn previous_notify_script(previous_notify: &[String]) -> String {
    let Some(command) = previous_notify.first() else {
        return String::new();
    };
    let args = previous_notify
        .iter()
        .skip(1)
        .map(|arg| shell_quote(arg))
        .collect::<Vec<_>>()
        .join(" ");
    let command = shell_quote(command);
    if args.is_empty() {
        format!("{command} \"$@\" >/dev/null 2>&1 &\n")
    } else {
        format!("{command} {args} \"$@\" >/dev/null 2>&1 &\n")
    }
}

fn parse_toml_document(contents: &str) -> anyhow::Result<DocumentMut> {
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .parse::<DocumentMut>()
            .with_context(|| "config.toml TOML parse failed")
    }
}

fn ensure_trailing_newline(mut text: String) -> String {
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(unix)]
fn make_executable(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let mut permissions = std::fs::metadata(path)?.permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> anyhow::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completion_sound_notify_config_sets_wrapper_command() {
        let updated = completion_sound_notify_config_text(
            "notify = [\"existing\", \"other-event\"]\n",
            Path::new("/tmp/codex-plus-plus/completion-sound-notify.sh"),
        )
        .unwrap();

        let doc = updated.parse::<DocumentMut>().unwrap();
        let notify = doc["notify"].as_array().unwrap();
        assert_eq!(
            notify.get(0).and_then(|value| value.as_str()),
            Some("/tmp/codex-plus-plus/completion-sound-notify.sh")
        );
        assert_eq!(
            notify.get(1).and_then(|value| value.as_str()),
            Some("turn-ended")
        );
    }

    #[test]
    fn previous_notify_script_forwards_existing_command() {
        let script =
            previous_notify_script(&["/tmp/notify tool".to_string(), "turn-ended".to_string()]);

        assert!(script.contains("'/tmp/notify tool' 'turn-ended' \"$@\""));
    }
}
