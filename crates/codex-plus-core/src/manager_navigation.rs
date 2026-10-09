use anyhow::Context;
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagerNavigationIntent {
    pub page: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
}

pub fn save_pending_manager_navigation_from_payload(
    payload: &Value,
) -> anyhow::Result<Option<ManagerNavigationIntent>> {
    let raw_navigation = payload.as_object().context("管理工具导航参数必须是对象")?;
    if raw_navigation.is_empty() {
        return Ok(None);
    }
    let navigation: ManagerNavigationIntent =
        serde_json::from_value(payload.clone()).context("管理工具导航参数无效")?;
    let navigation = normalize_navigation(navigation)?;
    save_pending_manager_navigation(&navigation)?;
    Ok(Some(navigation))
}

pub fn save_pending_manager_navigation(navigation: &ManagerNavigationIntent) -> anyhow::Result<()> {
    save_pending_manager_navigation_at(
        &crate::paths::default_pending_manager_navigation_path(),
        navigation,
    )
}

pub fn consume_pending_manager_navigation() -> anyhow::Result<Option<ManagerNavigationIntent>> {
    consume_pending_manager_navigation_at(&crate::paths::default_pending_manager_navigation_path())
}

pub fn rollback_pending_manager_navigation_after_launch_failure(
    navigation: Option<&ManagerNavigationIntent>,
    launch_error: anyhow::Error,
) -> anyhow::Error {
    let Some(navigation) = navigation else {
        return launch_error;
    };
    match remove_pending_manager_navigation_if_matches(navigation) {
        Ok(_) => launch_error,
        Err(cleanup_error) => {
            launch_error.context(format!("清理未完成的管理工具导航失败：{cleanup_error}"))
        }
    }
}

pub fn save_pending_manager_navigation_at(
    path: &Path,
    navigation: &ManagerNavigationIntent,
) -> anyhow::Result<()> {
    let navigation = normalize_navigation(navigation.clone())?;
    let contents = format!("{}\n", serde_json::to_string_pretty(&navigation)?);
    crate::settings::atomic_write(path, contents.as_bytes())
        .with_context(|| format!("保存管理工具导航失败：{}", path.to_string_lossy()))
}

pub fn consume_pending_manager_navigation_at(
    path: &Path,
) -> anyhow::Result<Option<ManagerNavigationIntent>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("读取管理工具导航失败：{}", path.to_string_lossy()));
        }
    };
    let navigation = serde_json::from_str(&contents).context("管理工具导航内容无效")?;
    let navigation = normalize_navigation(navigation)?;
    match std::fs::remove_file(path) {
        Ok(()) => Ok(Some(navigation)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Some(navigation)),
        Err(error) => {
            Err(error).with_context(|| format!("清理管理工具导航失败：{}", path.to_string_lossy()))
        }
    }
}

fn remove_pending_manager_navigation_if_matches(
    navigation: &ManagerNavigationIntent,
) -> anyhow::Result<bool> {
    remove_pending_manager_navigation_if_matches_at(
        &crate::paths::default_pending_manager_navigation_path(),
        navigation,
    )
}

fn remove_pending_manager_navigation_if_matches_at(
    path: &Path,
    navigation: &ManagerNavigationIntent,
) -> anyhow::Result<bool> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("读取管理工具导航失败：{}", path.to_string_lossy()));
        }
    };
    let pending: ManagerNavigationIntent =
        serde_json::from_str(&contents).context("管理工具导航内容无效")?;
    if normalize_navigation(pending)? != normalize_navigation(navigation.clone())? {
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("清理管理工具导航失败：{}", path.to_string_lossy()))
        }
    }
}

fn normalize_navigation(
    mut navigation: ManagerNavigationIntent,
) -> anyhow::Result<ManagerNavigationIntent> {
    if navigation.page == "settings"
        && matches!(
            navigation.section.as_deref(),
            Some("stepwise" | "dictation")
        )
    {
        navigation.page = "enhance".to_string();
    }
    match (navigation.page.as_str(), navigation.section.as_deref()) {
        ("settings" | "pluginMarket", None) | ("enhance", None | Some("stepwise" | "dictation")) => Ok(navigation),
        _ => anyhow::bail!(
            "不支持的管理工具导航：{}/{}",
            navigation.page,
            navigation.section.as_deref().unwrap_or("")
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_consumes_stepwise_navigation_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-manager-navigation.json");
        let navigation = ManagerNavigationIntent {
            page: "enhance".to_string(),
            section: Some("stepwise".to_string()),
        };

        save_pending_manager_navigation_at(&path, &navigation).unwrap();

        assert_eq!(
            consume_pending_manager_navigation_at(&path).unwrap(),
            Some(navigation)
        );
        assert_eq!(consume_pending_manager_navigation_at(&path).unwrap(), None);
    }

    #[test]
    fn launch_failure_removes_only_matching_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-manager-navigation.json");
        let failed_navigation = ManagerNavigationIntent {
            page: "enhance".to_string(),
            section: Some("stepwise".to_string()),
        };
        let replacement_navigation = ManagerNavigationIntent {
            page: "settings".to_string(),
            section: None,
        };

        save_pending_manager_navigation_at(&path, &failed_navigation).unwrap();
        assert!(
            remove_pending_manager_navigation_if_matches_at(&path, &failed_navigation).unwrap()
        );
        assert!(!path.exists());

        save_pending_manager_navigation_at(&path, &replacement_navigation).unwrap();
        assert!(
            !remove_pending_manager_navigation_if_matches_at(&path, &failed_navigation).unwrap()
        );
        assert_eq!(
            consume_pending_manager_navigation_at(&path).unwrap(),
            Some(replacement_navigation)
        );
    }

    #[test]
    fn rejects_unknown_navigation_targets() {
        let payload = serde_json::json!({
            "page": "settings",
            "section": "unknown"
        });

        let error = save_pending_manager_navigation_from_payload(&payload).unwrap_err();

        assert!(error.to_string().contains("不支持的管理工具导航"));
    }

    #[test]
    fn empty_payload_does_not_create_navigation() {
        assert_eq!(
            save_pending_manager_navigation_from_payload(&serde_json::json!({})).unwrap(),
            None
        );
    }

    #[test]
    fn rejects_non_object_navigation_payloads() {
        for payload in [
            serde_json::json!(null),
            serde_json::json!([]),
            serde_json::json!("settings"),
        ] {
            let error = save_pending_manager_navigation_from_payload(&payload).unwrap_err();
            assert!(error.to_string().contains("必须是对象"));
        }
    }
    #[test]
    fn legacy_enhancement_navigation_is_normalized_on_save_and_consume() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-manager-navigation.json");
        for section in ["stepwise", "dictation"] {
            let legacy = ManagerNavigationIntent {
                page: "settings".to_string(),
                section: Some(section.to_string()),
            };
            let expected = ManagerNavigationIntent {
                page: "enhance".to_string(),
                section: Some(section.to_string()),
            };
            save_pending_manager_navigation_at(&path, &legacy).unwrap();
            let saved: ManagerNavigationIntent =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            assert_eq!(saved, expected);
            assert!(remove_pending_manager_navigation_if_matches_at(&path, &legacy).unwrap());
            std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
            assert_eq!(
                consume_pending_manager_navigation_at(&path).unwrap(),
                Some(expected)
            );
        }
    }

    #[test]
    fn supports_enhance_dictation_and_plain_settings_navigation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("pending-manager-navigation.json");
        for navigation in [
            ManagerNavigationIntent {
                page: "enhance".to_string(),
                section: Some("dictation".to_string()),
            },
            ManagerNavigationIntent {
                page: "settings".to_string(),
                section: None,
            },
        ] {
            save_pending_manager_navigation_at(&path, &navigation).unwrap();
            assert_eq!(
                consume_pending_manager_navigation_at(&path).unwrap(),
                Some(navigation)
            );
        }
    }
}
