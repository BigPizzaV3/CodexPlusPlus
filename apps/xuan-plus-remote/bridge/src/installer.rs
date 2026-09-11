use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

const USER_SCRIPT: &str = include_str!("../user-scripts/xuan-plus-remote-command.user.js");
const USER_SCRIPT_NAME: &str = "xuan-plus-remote-command.user.js";

pub fn install_user_script(destination: Option<&Path>) -> anyhow::Result<PathBuf> {
    let directory = destination
        .map(Path::to_path_buf)
        .unwrap_or_else(default_user_script_dir);
    std::fs::create_dir_all(&directory)
        .with_context(|| format!("无法创建用户脚本目录 {}", directory.display()))?;
    let path = directory.join(USER_SCRIPT_NAME);
    if path.exists() {
        let current = std::fs::read_to_string(&path)
            .with_context(|| format!("无法读取现有用户脚本 {}", path.display()))?;
        if current == USER_SCRIPT {
            return Ok(path);
        }
        bail!("用户脚本已存在且内容不同，拒绝覆盖：{}", path.display());
    }
    std::fs::write(&path, USER_SCRIPT)
        .with_context(|| format!("无法写入用户脚本 {}", path.display()))?;
    Ok(path)
}

fn default_user_script_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA") {
            return PathBuf::from(roaming).join("Codex++").join("user_scripts");
        }
    }
    directories::BaseDirs::new()
        .map(|dirs| dirs.config_dir().join("Codex++").join("user_scripts"))
        .unwrap_or_else(|| PathBuf::from("user_scripts"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_never_overwrites_different_content() {
        let directory = tempfile::tempdir().unwrap();
        let path = install_user_script(Some(directory.path())).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), USER_SCRIPT);
        assert_eq!(install_user_script(Some(directory.path())).unwrap(), path);
        std::fs::write(&path, "custom").unwrap();
        assert!(install_user_script(Some(directory.path())).is_err());
    }
}
