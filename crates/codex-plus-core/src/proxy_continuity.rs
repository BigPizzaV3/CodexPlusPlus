//! 已打开的 Codex 客户端缓存 base_url；管理器换端口后仍需接住这些入口。

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

const HISTORY_FILE: &str = "proxy-endpoints.json";

#[derive(Default, Serialize, Deserialize)]
struct History {
    version: u32,
    homes: Vec<HomeEndpoints>,
}

#[derive(Serialize, Deserialize)]
struct HomeEndpoints {
    codex_home: PathBuf,
    ports: BTreeSet<u16>,
}

pub(crate) struct ProxyEndpoints {
    path: PathBuf,
    home: PathBuf,
    legacy_default: bool,
}

impl ProxyEndpoints {
    pub(crate) fn current() -> Self {
        let home = crate::codex_home::default_codex_home_dir();
        let user_home = directories::BaseDirs::new().map(|dirs| dirs.home_dir().join(".codex"));
        // 旧版默认端口迁移只属于生产主实例，不能让隔离实例接走主实例流量。
        let legacy_default = crate::paths::state_dir_override().is_none()
            && std::env::var_os("CODEX_PLUS_MULTI_INSTANCE").as_deref()
                != Some(std::ffi::OsStr::new("1"))
            && user_home.is_some_and(|expected| same_home(&home, &expected));
        Self::new(crate::paths::default_app_state_dir(), home, legacy_default)
    }

    pub(crate) fn new(state_dir: PathBuf, home: PathBuf, legacy_default: bool) -> Self {
        Self {
            path: state_dir.join(HISTORY_FILE),
            home,
            legacy_default,
        }
    }

    fn read_history(path: &Path) -> anyhow::Result<History> {
        let contents = match std::fs::read(path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(History {
                    version: 1,
                    ..History::default()
                });
            }
            Err(error) => return Err(error.into()),
        };
        let history: History = serde_json::from_slice(&contents)?;
        anyhow::ensure!(
            history.version == 1,
            "unsupported proxy endpoint history version"
        );
        Ok(history)
    }

    fn backup_path(&self) -> PathBuf {
        self.path.with_file_name("proxy-endpoints.backup.json")
    }

    fn history(&self) -> anyhow::Result<History> {
        match Self::read_history(&self.path) {
            Ok(history) if self.path.exists() || !self.backup_path().exists() => Ok(history),
            result => {
                if let Err(error) = &result {
                    log_failure("history_read", error);
                }
                if self.backup_path().exists() {
                    Self::read_history(&self.backup_path())
                } else {
                    result
                }
            }
        }
    }

    pub(crate) fn ports(&self) -> anyhow::Result<BTreeSet<u16>> {
        let history = self.history()?;
        let mut ports = history
            .homes
            .iter()
            .filter(|entry| same_home(&entry.codex_home, &self.home))
            .flat_map(|entry| entry.ports.iter().copied())
            .filter(|port| *port > 0)
            .collect::<BTreeSet<_>>();
        if self.legacy_default {
            ports.insert(crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT);
        }
        Ok(ports)
    }

    pub(crate) fn restore_required(&self) -> bool {
        !self.ports_for_restore().is_empty()
    }

    pub(crate) fn ports_for_restore(&self) -> BTreeSet<u16> {
        self.ports().unwrap_or_else(|error| {
            log_failure("history_read", &error);
            if self.legacy_default {
                BTreeSet::from([crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT])
            } else {
                BTreeSet::new()
            }
        })
    }

    pub(crate) fn remember_live_proxy(&self, settings: &crate::settings::BackendSettings) {
        if !settings.relay_profiles_enabled || !settings.active_relay_uses_protocol_proxy() {
            return;
        }
        let Ok(contents) = std::fs::read_to_string(self.home.join("config.toml")) else {
            return;
        };
        let Ok(config) = contents.parse::<toml::Value>() else {
            return;
        };
        let provider = config
            .get("model_provider")
            .and_then(toml::Value::as_str)
            .unwrap_or("openai");
        let base = if provider == "openai" {
            config.get("openai_base_url")
        } else {
            config
                .get("model_providers")
                .and_then(|providers| providers.get(provider))
                .and_then(|provider| provider.get("base_url"))
        }
        .and_then(toml::Value::as_str);
        let Some(base) = base else {
            return;
        };
        // 当 live 指向供应商本身的本地代理（例如 CC Switch）时，不能认领其端口。
        let normalized = |value: &str| {
            reqwest::Url::parse(value.trim()).ok().map(|mut url| {
                let path = url.path().trim_end_matches('/').to_string();
                url.set_path(&path);
                url
            })
        };
        let mut profiles = settings.relay_profiles.clone();
        profiles.push(settings.active_relay_profile());
        if profiles.iter().any(|profile| {
            let upstream = if profile.upstream_base_url.trim().is_empty() {
                crate::relay_config::relay_profile_base_url(profile)
            } else {
                profile.upstream_base_url.clone()
            };
            normalized(base).is_some() && normalized(base) == normalized(&upstream)
        }) {
            return;
        }
        let Ok(url) = reqwest::Url::parse(base) else {
            return;
        };
        if url.scheme() == "http"
            && url.host_str() == Some("127.0.0.1")
            && url.path().trim_end_matches('/') == "/v1"
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && let Some(port) = url.port()
            && (self.ports_for_restore().contains(&port) || self.receipt_owns_port(port))
            && let Err(error) = self.remember(port)
        {
            log_failure("live_endpoint_migration", &error);
        }
    }

    fn receipt_owns_port(&self, port: u16) -> bool {
        let read = |name| {
            std::fs::read(self.path.with_file_name(name))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        };
        let (Some(owner), Some(status)) = (read("resume-owner.json"), read("latest-status.json"))
        else {
            return false;
        };
        owner["version"].as_u64() == Some(1)
            && owner["codex_home"]
                .as_str()
                .is_some_and(|home| same_home(Path::new(home), &self.home))
            && owner["debug_port"]
                .as_u64()
                .is_some_and(|debug| debug > 0 && status["debug_port"].as_u64() == Some(debug))
            && status["helper_port"].as_u64() == Some(u64::from(port))
    }

    pub(crate) fn remember(&self, port: u16) -> anyhow::Result<()> {
        anyhow::ensure!(port > 0, "cannot remember an unbound proxy port");
        if let Ok(contents) = std::fs::read(&self.path) {
            // 新版本格式不能被旧程序覆盖；损坏文件则保留现场后从副本恢复。
            if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&contents)
                && let Some(version) = value["version"].as_u64()
            {
                anyhow::ensure!(
                    version <= 1,
                    "unsupported proxy endpoint history version"
                );
            }
            if !serde_json::from_slice::<History>(&contents)
                .is_ok_and(|history| history.version == 1)
            {
                let stamp = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_nanos();
                let invalid = self
                    .path
                    .with_file_name(format!("proxy-endpoints.invalid-{stamp}.json"));
                std::fs::copy(&self.path, invalid)?;
            }
        }
        let mut history = self.history()?;
        if let Some(entry) = history
            .homes
            .iter_mut()
            .find(|entry| same_home(&entry.codex_home, &self.home))
        {
            entry.ports.insert(port);
        } else {
            history.homes.push(HomeEndpoints {
                codex_home: self.home.clone(),
                ports: BTreeSet::from([port]),
            });
        }
        if self.legacy_default {
            history
                .homes
                .iter_mut()
                .filter(|entry| same_home(&entry.codex_home, &self.home))
                .for_each(|entry| {
                    entry
                        .ports
                        .insert(crate::protocol_proxy::DEFAULT_PROTOCOL_PROXY_PORT);
                });
        }
        let parent = self.path.parent().context("proxy history has no parent")?;
        std::fs::create_dir_all(parent)?;
        // 由状态目录的单实例锁串行化写入；持久记录不会随 helper 关闭而删除。
        crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&history)?)?;
        crate::settings::atomic_write(&self.backup_path(), &serde_json::to_vec_pretty(&history)?)?;
        Ok(())
    }
}

fn same_home(left: &Path, right: &Path) -> bool {
    let left = std::fs::canonicalize(left).unwrap_or_else(|_| left.to_path_buf());
    let right = std::fs::canonicalize(right).unwrap_or_else(|_| right.to_path_buf());
    if cfg!(windows) {
        left.to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
    } else {
        left == right
    }
}

pub(crate) fn log_failure(operation: &str, error: &anyhow::Error) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "helper.proxy_continuity_failed",
        serde_json::json!({ "operation": operation, "message": error.to_string() }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_upgrade_migrates_live_proxy_before_rewrite_without_claiming_a_supplier_port() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let endpoints = ProxyEndpoints::new(temp.path().join("state"), home.clone(), false);
        let mut settings: crate::settings::BackendSettings =
            serde_json::from_value(serde_json::json!({
                "relayProfilesEnabled": true,
                "relayProfiles": [{
                    "id": "local", "name": "Local", "relayMode": "pureApi", "protocol": "chatCompletions",
                    "upstreamBaseUrl": "http://127.0.0.1:15721/v1"
                }],
                "activeRelayId": "local"
            }))
            .unwrap();
        for (provider, key) in [
            ("custom", "[model_providers.custom]\nbase_url"),
            ("openai", "openai_base_url"),
        ] {
            std::fs::write(
                home.join("config.toml"),
                format!("model_provider = \"{provider}\"\n{key} = \"http://127.0.0.1:15721/v1\"\n"),
            )
            .unwrap();
            endpoints.remember_live_proxy(&settings);
            assert!(endpoints.ports().unwrap().is_empty());
        }
        std::fs::write(home.join("config.toml"),
            "model_provider = \"custom\"\n[model_providers.custom]\nbase_url = \"http://127.0.0.1:41004/v1\"\n").unwrap();
        // 单凭不同于当前供应商的 localhost URL 不能证明它属于本工具。
        endpoints.remember_live_proxy(&settings);
        assert!(endpoints.ports().unwrap().is_empty());
        std::fs::create_dir_all(endpoints.path.parent().unwrap()).unwrap();
        std::fs::write(
            endpoints.path.with_file_name("resume-owner.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "debug_port": 19401, "codex_home": home,
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            endpoints.path.with_file_name("latest-status.json"),
            br#"{"debug_port":19401,"helper_port":41004}"#,
        )
        .unwrap();
        endpoints.remember_live_proxy(&settings);
        assert_eq!(endpoints.ports().unwrap(), BTreeSet::from([41004]));
        settings.relay_profiles_enabled = false;
        std::fs::write(
            home.join("config.toml"),
            "openai_base_url = \"http://127.0.0.1:41005/v1\"\n",
        )
        .unwrap();
        endpoints.remember_live_proxy(&settings);
        assert_eq!(endpoints.ports().unwrap(), BTreeSet::from([41004]));
    }

    #[test]
    fn continuity_history_recovers_from_corruption_without_losing_cached_ports() {
        let temp = tempfile::tempdir().unwrap();
        let endpoints = ProxyEndpoints::new(temp.path().into(), temp.path().join("home"), false);
        endpoints.remember(41001).unwrap();
        endpoints.remember(41002).unwrap();
        std::fs::write(&endpoints.path, b"interrupted write").unwrap();
        assert_eq!(endpoints.ports().unwrap(), BTreeSet::from([41001, 41002]));
        endpoints.remember(41003).unwrap();
        assert_eq!(
            endpoints.ports().unwrap(),
            BTreeSet::from([41001, 41002, 41003])
        );
        assert!(std::fs::read_dir(temp.path()).unwrap().any(|entry| {
            let path = entry.unwrap().path();
            path.file_name()
                .unwrap()
                .to_string_lossy()
                .contains("invalid")
                && std::fs::read(path).unwrap() == b"interrupted write"
        }));
    }

    #[test]
    fn continuity_repairs_valid_json_corruption_from_backup_and_remembers_new_ports() {
        for contents in [
            "{}",
            "null",
            r#"{"version":null}"#,
            r#"{"version":"damaged","homes":[]}"#,
            r#"{"version":1,"homes":"damaged"}"#,
            r#"{"version":0,"homes":[]}"#,
        ] {
            let temp = tempfile::tempdir().unwrap();
            let home = temp.path().join("home");
            let endpoints = ProxyEndpoints::new(temp.path().into(), home.clone(), false);
            endpoints.remember(41001).unwrap();
            std::fs::write(&endpoints.path, contents).unwrap();
            assert_eq!(endpoints.ports().unwrap(), BTreeSet::from([41001]));
            endpoints.remember(41002).unwrap();
            let restarted = ProxyEndpoints::new(temp.path().into(), home, false);
            assert_eq!(restarted.ports().unwrap(), BTreeSet::from([41001, 41002]));
            assert_eq!(
                ProxyEndpoints::read_history(&restarted.backup_path()).unwrap().version,
                1
            );
            assert!(std::fs::read_dir(temp.path()).unwrap().any(|entry| {
                let path = entry.unwrap().path();
                path.file_name().unwrap().to_string_lossy().contains("invalid-")
                    && std::fs::read(path).unwrap() == contents.as_bytes()
            }));
        }
    }

    #[test]
    fn endpoint_history_survives_restarts_and_is_scoped_to_state_and_home() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        let home = temp.path().join("home");
        let first = ProxyEndpoints::new(state.clone(), home.clone(), false);
        first.remember(41001).unwrap();
        let second = ProxyEndpoints::new(state.clone(), home.clone(), false);
        second.remember(41002).unwrap();
        second.remember(41001).unwrap();
        assert_eq!(second.ports().unwrap(), BTreeSet::from([41001, 41002]));
        let other_home = ProxyEndpoints::new(state.clone(), temp.path().join("other"), false);
        assert!(!other_home.restore_required());
        other_home.remember(41003).unwrap();
        assert_eq!(first.ports().unwrap(), BTreeSet::from([41001, 41002]));
        let other_state = ProxyEndpoints::new(temp.path().join("isolated"), home, false);
        assert!(other_state.ports().unwrap().is_empty());
        let legacy = ProxyEndpoints::new(state, temp.path().join("legacy"), true);
        assert_eq!(legacy.ports().unwrap(), BTreeSet::from([57321]));
    }

    #[test]
    fn corrupt_or_newer_history_is_reported_and_preserved() {
        let temp = tempfile::tempdir().unwrap();
        let endpoints = ProxyEndpoints::new(temp.path().into(), temp.path().join("home"), false);
        for contents in [b"invalid".as_slice(), br#"{"version":2,"homes":[]}"#] {
            std::fs::write(&endpoints.path, contents).unwrap();
            assert!(endpoints.ports().is_err());
            assert!(endpoints.remember(41001).is_err());
            assert_eq!(std::fs::read(&endpoints.path).unwrap(), contents);
        }
    }

    #[test]
    fn continuity_preserves_future_format_but_restores_backup_and_legacy_fallback() {
        let temp = tempfile::tempdir().unwrap();
        let endpoints = ProxyEndpoints::new(temp.path().into(), temp.path().join("home"), true);
        std::fs::write(&endpoints.path, b"invalid").unwrap();
        assert_eq!(endpoints.ports_for_restore(), BTreeSet::from([57321]));
        std::fs::write(&endpoints.path, br#"{"version":1,"homes":[]}"#).unwrap();
        endpoints.remember(41001).unwrap();
        let future = br#"{"version":2,"homes":[]}"#;
        std::fs::write(&endpoints.path, future).unwrap();
        assert_eq!(
            endpoints.ports_for_restore(),
            BTreeSet::from([41001, 57321])
        );
        assert!(endpoints.remember(41002).is_err());
        assert_eq!(std::fs::read(&endpoints.path).unwrap(), future);
        assert_eq!(endpoints.ports().unwrap(), BTreeSet::from([41001, 57321]));
    }

    #[test]
    fn continuity_migration_requires_matching_home_and_excludes_inactive_supplier() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        std::fs::create_dir(&home).unwrap();
        let endpoints = ProxyEndpoints::new(temp.path().into(), home.clone(), false);
        let settings = serde_json::from_value(serde_json::json!({
            "relayProfilesEnabled": true, "activeRelayId": "new",
            "relayProfiles": [
                { "id": "new", "name": "New", "relayMode": "mixedApi", "protocol": "chatCompletions",
                  "upstreamBaseUrl": "https://example.invalid/v1" },
                { "id": "old", "name": "Old", "upstreamBaseUrl": "http://127.0.0.1:41001/v1/" }
            ]
        }))
        .unwrap();
        std::fs::write(home.join("config.toml"),
            "model_provider=\"custom\"\n[model_providers.custom]\nbase_url=\"http://127.0.0.1:41001/v1\"\n").unwrap();
        std::fs::write(
            temp.path().join("latest-status.json"),
            br#"{"debug_port":19401,"helper_port":41001}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("resume-owner.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "debug_port": 19401, "codex_home": home,
            }))
            .unwrap(),
        )
        .unwrap();
        endpoints.remember_live_proxy(&settings);
        assert!(endpoints.ports().unwrap().is_empty());
        std::fs::write(home.join("config.toml"),
            "model_provider=\"custom\"\n[model_providers.custom]\nbase_url=\"http://127.0.0.1:41002/v1\"\n").unwrap();
        std::fs::write(
            temp.path().join("latest-status.json"),
            br#"{"debug_port":19401,"helper_port":41002}"#,
        )
        .unwrap();
        std::fs::write(
            temp.path().join("resume-owner.json"),
            serde_json::to_vec(&serde_json::json!({
                "version": 1, "debug_port": 19401, "codex_home": temp.path().join("other-home"),
            }))
            .unwrap(),
        )
        .unwrap();
        endpoints.remember_live_proxy(&settings);
        assert!(endpoints.ports().unwrap().is_empty());
    }
}
