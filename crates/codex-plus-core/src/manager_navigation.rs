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
    if payload.as_object().is_some_and(|object| object.is_empty()) {
        return Ok(None);
    }
    let navigation: ManagerNavigationIntent =
        serde_json::from_value(payload.clone()).context("管理工具导航参数无效")?;
    validate(&navigation)?;
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
        Err(error) => launch_error.context(format!("清理未完成的管理工具导航失败：{error}")),
    }
}

pub fn save_pending_manager_navigation_at(
    path: &Path,
    navigation: &ManagerNavigationIntent,
) -> anyhow::Result<()> {
    validate(navigation)?;
    let contents = format!("{}\n", serde_json::to_string_pretty(navigation)?);
    crate::settings::atomic_write(path, contents.as_bytes())
        .with_context(|| format!("保存管理工具导航失败：{}", path.display()))
}

pub fn consume_pending_manager_navigation_at(
    path: &Path,
) -> anyhow::Result<Option<ManagerNavigationIntent>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("读取管理工具导航失败：{}", path.display())),
    };
    let navigation = serde_json::from_str(&contents).context("管理工具导航内容无效")?;
    validate(&navigation)?;
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).with_context(|| format!("清理管理工具导航失败：{}", path.display())),
    }
    Ok(Some(navigation))
}

fn remove_pending_manager_navigation_if_matches(
    navigation: &ManagerNavigationIntent,
) -> anyhow::Result<bool> {
    let path = crate::paths::default_pending_manager_navigation_path();
    let contents = match std::fs::read_to_string(&path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error).with_context(|| format!("读取管理工具导航失败：{}", path.display())),
    };
    let pending: ManagerNavigationIntent =
        serde_json::from_str(&contents).context("管理工具导航内容无效")?;
    if pending != *navigation {
        return Ok(false);
    }
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error).context("清理管理工具导航失败"),
    }
}

fn validate(navigation: &ManagerNavigationIntent) -> anyhow::Result<()> {
    match (navigation.page.as_str(), navigation.section.as_deref()) {
        ("settings", None | Some("stepwise")) => Ok(()),
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
    fn saves_and_consumes_navigation_once() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pending-manager-navigation.json");
        let navigation = ManagerNavigationIntent {
            page: "settings".to_string(),
            section: Some("stepwise".to_string()),
        };

        save_pending_manager_navigation_at(&path, &navigation).unwrap();
        assert_eq!(consume_pending_manager_navigation_at(&path).unwrap(), Some(navigation));
        assert_eq!(consume_pending_manager_navigation_at(&path).unwrap(), None);
    }

    #[test]
    fn rejects_unknown_navigation_targets() {
        let error = save_pending_manager_navigation_from_payload(&serde_json::json!({
            "page": "settings",
            "section": "unknown"
        }))
        .unwrap_err();
        assert!(error.to_string().contains("不支持的管理工具导航"));
    }

    #[test]
    fn launch_cleanup_does_not_remove_replacement_navigation() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("pending-manager-navigation.json");
        let failed = ManagerNavigationIntent {
            page: "settings".to_string(),
            section: Some("stepwise".to_string()),
        };
        let replacement = ManagerNavigationIntent {
            page: "settings".to_string(),
            section: None,
        };

        save_pending_manager_navigation_at(&path, &failed).unwrap();
        save_pending_manager_navigation_at(&path, &replacement).unwrap();
        let loaded = consume_pending_manager_navigation_at(&path).unwrap();
        assert_eq!(loaded, Some(replacement));
    }
}
