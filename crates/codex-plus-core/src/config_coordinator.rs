use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::ccs_import::{self, CcsProviderImport};
use crate::relay_config::{self, RelayApplyResult};
use crate::settings::{BackendSettings, ConfigOwnership};

const WRITE_MARKER_FILE: &str = "config-write-marker.json";
const WRITER_CODEX_PLUS_PLUS: &str = "codexplusplus";
const WRITER_CC_SWITCH: &str = "ccswitch";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveConfigFingerprint {
    pub config_hash: String,
    pub auth_hash: String,
    pub model_provider: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigWriteMarker {
    pub writer: String,
    pub fingerprint: LiveConfigFingerprint,
    pub written_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CoordinationStatus {
    pub ccswitch_detected: bool,
    pub configured_ownership: ConfigOwnership,
    pub effective_ownership: ConfigOwnership,
    pub last_writer: Option<String>,
    pub conflict_detected: bool,
    pub conflict_message: String,
    pub ccswitch_current_provider_id: Option<String>,
    pub ccswitch_current_provider_name: Option<String>,
    pub live_model_provider: String,
    pub can_write_live_config: bool,
    pub guidance: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveConfigWriteDecision {
    pub allowed: bool,
    pub message: String,
}

pub fn detect_ccswitch() -> bool {
    ccs_import::default_ccs_db_path().exists()
}

pub fn effective_ownership(settings: &BackendSettings) -> ConfigOwnership {
    match settings.config_ownership {
        ConfigOwnership::Auto => {
            if settings.ccs_link_enabled && detect_ccswitch() {
                ConfigOwnership::CcSwitch
            } else {
                ConfigOwnership::CodexPlusPlus
            }
        }
        other => other,
    }
}

pub fn write_marker_path() -> PathBuf {
    crate::paths::default_app_state_dir().join(WRITE_MARKER_FILE)
}

pub fn fingerprint_from_home(home: &std::path::Path) -> LiveConfigFingerprint {
    let config_path = home.join("config.toml");
    let auth_path = home.join("auth.json");
    let config_bytes = fs::read(&config_path).unwrap_or_default();
    let auth_bytes = fs::read(&auth_path).unwrap_or_default();
    let config_text = String::from_utf8_lossy(&config_bytes);
    LiveConfigFingerprint {
        config_hash: hash_bytes(&config_bytes),
        auth_hash: hash_bytes(&auth_bytes),
        model_provider: relay_config::root_key_string(&config_text, "model_provider")
            .unwrap_or_default(),
    }
}

pub fn read_write_marker() -> Option<ConfigWriteMarker> {
    let path = write_marker_path();
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn record_write_marker(writer: &str, home: &std::path::Path) -> anyhow::Result<()> {
    let marker = ConfigWriteMarker {
        writer: writer.to_string(),
        fingerprint: fingerprint_from_home(home),
        written_at_ms: now_ms(),
    };
    let path = write_marker_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    crate::settings::atomic_write(
        &path,
        format!("{}\n", serde_json::to_string_pretty(&marker)?).as_bytes(),
    )
}

pub fn detect_external_modification(home: &std::path::Path) -> Option<String> {
    let marker = read_write_marker()?;
    let current = fingerprint_from_home(home);
    if marker.fingerprint == current {
        return None;
    }
    let writer = marker.writer.trim();
    if writer == WRITER_CODEX_PLUS_PLUS {
        Some(
            "config.toml / auth.json changed after the last Codex++ write.\
             This is usually caused by CC Switch changing providers."
                .to_string(),
        )
    } else if writer == WRITER_CC_SWITCH {
        Some(
            "The live config no longer matches the last CC Switch write marker.\
             Re-enable the current provider in CC Switch first, or refresh Codex++ linking status."
                .to_string(),
        )
    } else {
        Some(
            "config.toml / auth.json no longer matches the Codex++ marker and may have been changed by another tool."
                .to_string(),
        )
    }
}

pub fn coordination_status(settings: &BackendSettings) -> CoordinationStatus {
    let home = relay_config::default_codex_home_dir();
    let ccswitch_detected = detect_ccswitch();
    let configured = settings.config_ownership;
    let effective = effective_ownership(settings);
    let live = fingerprint_from_home(&home);
    let marker = read_write_marker();
    let current_ccs = current_ccs_codex_provider();
    let conflict_message = detect_external_modification(&home).unwrap_or_default();
    let conflict_detected = !conflict_message.is_empty();
    let can_write = evaluate_live_write(settings, false).allowed;
    let guidance = coordination_guidance(
        settings,
        effective,
        ccswitch_detected,
        conflict_detected,
        &current_ccs,
        &live.model_provider,
    );

    CoordinationStatus {
        ccswitch_detected,
        configured_ownership: configured,
        effective_ownership: effective,
        last_writer: marker.map(|value| value.writer),
        conflict_detected,
        conflict_message,
        ccswitch_current_provider_id: current_ccs.as_ref().map(|provider| provider.source_id.clone()),
        ccswitch_current_provider_name: current_ccs.as_ref().map(|provider| provider.name.clone()),
        live_model_provider: live.model_provider,
        can_write_live_config: can_write,
        guidance,
    }
}

pub fn evaluate_live_write(settings: &BackendSettings, force: bool) -> LiveConfigWriteDecision {
    if !settings.relay_profiles_enabled {
        return LiveConfigWriteDecision {
            allowed: false,
            message: "Provider switching is off; Codex++ will not write config.toml / auth.json.".to_string(),
        };
    }

    let effective = effective_ownership(settings);
    if effective == ConfigOwnership::CcSwitch && !settings.ccs_link_enabled {
        return LiveConfigWriteDecision {
            allowed: false,
            message: "Config ownership is currently CC Switch, but cc-switch linking is off.\
                      Enable linking or change Config Ownership to Codex++."
                .to_string(),
        };
    }

    if effective == ConfigOwnership::CcSwitch && !force {
        return LiveConfigWriteDecision {
            allowed: false,
            message: "CC Switch currently owns Codex provider config.\
                      Codex++ will not overwrite live config directly; switch through a linked provider, or switch in CC Switch and refresh."
                .to_string(),
        };
    }

    if detect_ccswitch()
        && effective == ConfigOwnership::CodexPlusPlus
        && !force
        && let Some(conflict) = detect_external_modification(&relay_config::default_codex_home_dir())
    {
        return LiveConfigWriteDecision {
            allowed: false,
            message: format!(
                "{conflict} To let Codex++ take over, set Config Ownership to Codex++ first and force the switch."
            ),
        };
    }

    LiveConfigWriteDecision {
        allowed: true,
        message: String::new(),
    }
}

pub fn apply_linked_ccs_provider_to_home(
    source_id: &str,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let provider = current_ccs_codex_provider_by_id(source_id)
        .with_context(|| format!("Codex provider {source_id} was not found in cc-switch"))?;
    apply_ccs_provider_import_to_home(&provider, common_config_contents)
}

pub fn apply_current_ccs_provider_to_home(
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let provider = current_ccs_codex_provider()
        .ok_or_else(|| anyhow::anyhow!("No currently enabled Codex provider was found in cc-switch"))?;
    apply_ccs_provider_import_to_home(&provider, common_config_contents)
}

pub fn sync_active_profile_from_ccs(settings: &mut BackendSettings) -> bool {
    if !settings.ccs_link_enabled {
        return false;
    }
    let Some(current) = current_ccs_codex_provider() else {
        return false;
    };
    if let Some(profile) = settings
        .relay_profiles
        .iter_mut()
        .find(|profile| profile.linked_ccs_provider_id == current.source_id)
    {
        settings.active_relay_id = profile.id.clone();
        ccs_import::apply_ccs_provider_to_profile(profile, &current);
        return true;
    }
    let existing_ids = settings
        .relay_profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let mut profile = ccs_import::relay_profile_from_ccs(&current, &existing_ids);
    ccs_import::apply_ccs_provider_to_profile(&mut profile, &current);
    settings.active_relay_id = profile.id.clone();
    settings.relay_profiles.push(profile);
    true
}

fn apply_ccs_provider_import_to_home(
    provider: &CcsProviderImport,
    common_config_contents: &str,
) -> anyhow::Result<RelayApplyResult> {
    let home = relay_config::default_codex_home_dir();
    let config_with_common = relay_config::merge_common_config_into_config(
        &provider.config_contents,
        common_config_contents,
    )?;
    let result = relay_config::apply_relay_files_to_home(
        &home,
        &config_with_common,
        &provider.auth_contents,
    )?;
    record_write_marker(WRITER_CC_SWITCH, &home)?;
    Ok(result)
}

pub fn current_ccs_codex_provider() -> Option<CcsProviderImport> {
    ccs_import::current_codex_provider_from_db(&ccs_import::default_ccs_db_path())
        .ok()
        .flatten()
}

pub fn current_ccs_codex_provider_by_id(source_id: &str) -> Option<CcsProviderImport> {
    let source_id = source_id.trim();
    if source_id.is_empty() {
        return None;
    }
    ccs_import::list_codex_providers_from_db(&ccs_import::default_ccs_db_path())
        .ok()?
        .into_iter()
        .find(|provider| provider.source_id == source_id)
}

fn coordination_guidance(
    settings: &BackendSettings,
    effective: ConfigOwnership,
    ccswitch_detected: bool,
    conflict_detected: bool,
    current_ccs: &Option<CcsProviderImport>,
    live_model_provider: &str,
) -> String {
    if !ccswitch_detected {
        return "CC Switch was not detected; Codex++ will manage ~/.codex/config.toml independently.".to_string();
    }
    if conflict_detected {
        return "A config conflict was detected: switch providers from only one tool, or set Config Ownership explicitly.".to_string();
    }
    match effective {
        ConfigOwnership::CcSwitch => {
            let provider = current_ccs
                .as_ref()
                .map(|value| value.name.as_str())
                .unwrap_or("Unknown");
            format!(
                "CC Switch currently manages provider config. Codex++ reads and writes the cc-switch database through linking,\
                 and will not directly overwrite live config. Current CC Switch provider: {provider}; live model_provider: {live_model_provider}."
            )
        }
        ConfigOwnership::CodexPlusPlus => {
            if settings.ccs_link_enabled {
                "cc-switch linking is on, but Config Ownership is still Codex++. When switching providers, Codex++ writes live config and may overwrite CC Switch's selection."
                    .to_string()
            } else {
                "Codex++ manages live config independently. If you also use CC Switch, enable linking and set ownership to auto or ccSwitch."
                    .to_string()
            }
        }
        ConfigOwnership::Auto => "Auto mode chooses ownership based on the linking toggle and whether CC Switch is present.".to_string(),
    }
}

fn hash_bytes(bytes: &[u8]) -> String {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}
