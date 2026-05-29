use std::fs;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::{Connection, params};
use serde_json::{Value, json};

use crate::settings::{RelayMode, RelayProfile, RelayProtocol};

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CcsProviderImport {
    pub source_id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub protocol: RelayProtocol,
    pub config_contents: String,
    pub auth_contents: String,
}

pub fn default_ccs_db_path() -> PathBuf {
    home_dir()
        .join(format!(".{}-{}", "cc", "switch"))
        .join(format!("{}-{}.db", "cc", "switch"))
}

pub fn default_ccs_settings_path() -> PathBuf {
    home_dir()
        .join(format!(".{}-{}", "cc", "switch"))
        .join("settings.json")
}

pub fn list_codex_providers_from_default_db() -> anyhow::Result<Vec<CcsProviderImport>> {
    list_codex_providers_from_db(&default_ccs_db_path())
}

pub fn sync_linked_profiles_from_default_db(
    profiles: &mut Vec<RelayProfile>,
) -> anyhow::Result<usize> {
    sync_linked_profiles_from_db(&default_ccs_db_path(), profiles)
}

pub fn sync_linked_profiles_from_db(
    path: &Path,
    profiles: &mut Vec<RelayProfile>,
) -> anyhow::Result<usize> {
    let providers = list_codex_providers_from_db(path)?;
    let mut existing_ids = profiles
        .iter()
        .map(|profile| profile.id.clone())
        .collect::<Vec<_>>();
    let mut changed = 0usize;

    for provider in providers {
        if let Some(profile) = profiles
            .iter_mut()
            .find(|profile| profile.linked_ccs_provider_id == provider.source_id)
        {
            apply_ccs_provider_to_profile(profile, &provider);
            changed += 1;
            continue;
        }

        let mut profile = relay_profile_from_ccs(&provider, &existing_ids);
        existing_ids.push(profile.id.clone());
        apply_ccs_provider_to_profile(&mut profile, &provider);
        profiles.push(profile);
        changed += 1;
    }

    Ok(changed)
}

pub fn write_linked_profiles_to_default_db(profiles: &[RelayProfile]) -> anyhow::Result<usize> {
    write_linked_profiles_to_db(&default_ccs_db_path(), profiles)
}

pub fn set_current_codex_provider_in_default_db(source_id: &str) -> anyhow::Result<bool> {
    set_current_codex_provider(
        &default_ccs_db_path(),
        &default_ccs_settings_path(),
        source_id,
    )
}

pub fn set_current_codex_provider(
    db_path: &Path,
    settings_path: &Path,
    source_id: &str,
) -> anyhow::Result<bool> {
    let source_id = source_id.trim();
    if source_id.is_empty() || !db_path.exists() {
        return Ok(false);
    }

    let mut conn = Connection::open(db_path)
        .with_context(|| format!("failed to open provider database {}", db_path.display()))?;
    let tx = conn.transaction()?;
    tx.execute(
        "UPDATE providers SET is_current = 0 WHERE app_type = 'codex'",
        [],
    )?;
    let affected = tx.execute(
        "UPDATE providers SET is_current = 1 WHERE id = ?1 AND app_type = 'codex'",
        params![source_id],
    )?;
    tx.commit()?;

    if affected == 0 {
        return Ok(false);
    }
    set_current_codex_provider_in_settings(settings_path, source_id)?;
    Ok(true)
}

fn set_current_codex_provider_in_settings(path: &Path, source_id: &str) -> anyhow::Result<()> {
    let mut settings = if path.exists() {
        let text = fs::read_to_string(path)
            .with_context(|| format!("failed to read cc-switch settings {}", path.display()))?;
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| json!({}))
    } else {
        json!({})
    };
    if !settings.is_object() {
        settings = json!({});
    }
    settings["currentProviderCodex"] = Value::String(source_id.to_string());
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, format!("{}\n", serde_json::to_string_pretty(&settings)?))
        .with_context(|| format!("failed to write cc-switch settings {}", path.display()))?;
    Ok(())
}

pub fn write_linked_profiles_to_db(
    path: &Path,
    profiles: &[RelayProfile],
) -> anyhow::Result<usize> {
    if !path.exists() {
        return Ok(0);
    }
    let linked_profiles = profiles
        .iter()
        .filter(|profile| !profile.linked_ccs_provider_id.trim().is_empty())
        .collect::<Vec<_>>();
    if linked_profiles.is_empty() {
        return Ok(0);
    }

    let conn = Connection::open(path)
        .with_context(|| format!("failed to open provider database {}", path.display()))?;
    let mut written = 0usize;
    for profile in linked_profiles {
        let source_id = profile.linked_ccs_provider_id.trim();
        let settings_config = profile_to_ccs_settings_config(profile)?;
        let affected = conn.execute(
            "UPDATE providers
             SET name = ?1, settings_config = ?2
             WHERE id = ?3 AND app_type = 'codex'",
            params![profile.name.trim(), settings_config.to_string(), source_id],
        )?;
        if affected > 0 {
            written += 1;
        }
    }
    Ok(written)
}

pub fn list_codex_providers_from_db(path: &Path) -> anyhow::Result<Vec<CcsProviderImport>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("failed to open provider database {}", path.display()))?;
    let mut stmt = conn.prepare(
        "SELECT id, name, settings_config
         FROM providers
         WHERE app_type = 'codex'
         ORDER BY COALESCE(sort_index, 999999), created_at ASC, id ASC",
    )?;
    let rows = stmt.query_map([], |row| {
        let source_id: String = row.get(0)?;
        let name: String = row.get(1)?;
        let settings_config: String = row.get(2)?;
        Ok((source_id, name, settings_config))
    })?;

    let mut providers = Vec::new();
    for row in rows {
        let (source_id, name, settings_config) = row?;
        let Ok(config) = serde_json::from_str::<Value>(&settings_config) else {
            continue;
        };
        if let Some(provider) = import_from_ccs_value(&source_id, &name, &config) {
            providers.push(provider);
        }
    }
    Ok(providers)
}

pub fn relay_profile_from_ccs(
    provider: &CcsProviderImport,
    existing_ids: &[String],
) -> RelayProfile {
    let id = unique_profile_id(
        &format!("ccs-{}", sanitize_id(&provider.source_id)),
        existing_ids,
    );
    RelayProfile {
        id,
        linked_ccs_provider_id: provider.source_id.clone(),
        name: provider.name.clone(),
        model: String::new(),
        base_url: provider.base_url.clone(),
        upstream_base_url: provider.base_url.clone(),
        api_key: provider.api_key.clone(),
        protocol: provider.protocol,
        relay_mode: relay_mode_from_ccs_provider(provider),
        official_mix_api_key: false,
        test_model: String::new(),
        config_contents: provider.config_contents.clone(),
        auth_contents: provider.auth_contents.clone(),
        use_common_config: true,
        context_selection: crate::settings::RelayContextSelection::default(),
        context_selection_initialized: false,
        context_window: String::new(),
        auto_compact_limit: String::new(),
        model_insert_mode: Default::default(),
        model_list: String::new(),
    }
}

fn apply_ccs_provider_to_profile(profile: &mut RelayProfile, provider: &CcsProviderImport) {
    profile.linked_ccs_provider_id = provider.source_id.clone();
    profile.name = provider.name.clone();
    profile.base_url = provider.base_url.clone();
    profile.upstream_base_url = provider.base_url.clone();
    profile.api_key = provider.api_key.clone();
    profile.protocol = provider.protocol;
    profile.relay_mode = relay_mode_from_ccs_provider(provider);
    profile.config_contents = provider.config_contents.clone();
    profile.auth_contents = provider.auth_contents.clone();
}

fn relay_mode_from_ccs_provider(provider: &CcsProviderImport) -> RelayMode {
    if provider.base_url.trim().is_empty() && provider.api_key.trim().is_empty() {
        RelayMode::Official
    } else {
        RelayMode::PureApi
    }
}

fn profile_to_ccs_settings_config(profile: &RelayProfile) -> anyhow::Result<Value> {
    let auth = if profile.auth_contents.trim().is_empty() {
        json!({})
    } else {
        serde_json::from_str::<Value>(&profile.auth_contents)
            .with_context(|| format!("{} 的 auth.json JSON 解析失败", profile.name))?
    };
    Ok(json!({
        "auth": auth,
        "config": profile.config_contents,
    }))
}

fn import_from_ccs_value(source_id: &str, name: &str, config: &Value) -> Option<CcsProviderImport> {
    let base_url = extract_base_url(config).unwrap_or_default();
    let api_key = extract_api_key(config).unwrap_or_default();
    let protocol = extract_protocol(config);
    let config_from_ccs = extract_config_contents(config).filter(|value| !value.trim().is_empty());
    if config_from_ccs.is_none() && base_url.trim().is_empty() && api_key.trim().is_empty() {
        return None;
    }
    let config_contents =
        config_from_ccs.unwrap_or_else(|| build_config_toml(&base_url, &api_key, protocol));
    let auth_contents = extract_auth_contents(config)
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| {
            if api_key.trim().is_empty() {
                "{}\n".to_string()
            } else {
                build_auth_json(&api_key)
            }
        });
    if config_contents.trim().is_empty() && auth_contents.trim().is_empty() {
        return None;
    }
    Some(CcsProviderImport {
        source_id: source_id.to_string(),
        name: name.to_string(),
        base_url,
        api_key,
        protocol,
        config_contents,
        auth_contents,
    })
}

fn extract_base_url(config: &Value) -> Option<String> {
    string_at(config, &["base_url", "baseURL"])
        .or_else(|| {
            config
                .get("config")
                .and_then(|value| string_at(value, &["base_url", "baseURL"]))
        })
        .or_else(|| {
            config
                .get("config")
                .and_then(Value::as_str)
                .and_then(extract_toml_base_url)
        })
        .map(trim_trailing_slash)
        .filter(|value| !value.is_empty())
}

fn extract_api_key(config: &Value) -> Option<String> {
    if let Some(key) = config
        .pointer("/env/OPENAI_API_KEY")
        .and_then(Value::as_str)
    {
        return Some(key.to_string());
    }
    if let Some(key) = config
        .pointer("/auth/OPENAI_API_KEY")
        .and_then(Value::as_str)
    {
        return Some(key.to_string());
    }
    string_at(config, &["apiKey", "api_key"]).or_else(|| {
        config
            .get("config")
            .and_then(|value| string_at(value, &["apiKey", "api_key"]))
    })
}

fn extract_protocol(config: &Value) -> RelayProtocol {
    if let Some(api_format) = string_at(config, &["api_format", "apiFormat"]) {
        if is_chat_protocol(&api_format) {
            return RelayProtocol::ChatCompletions;
        }
    }
    if let Some(wire_api) = config
        .get("config")
        .and_then(Value::as_str)
        .and_then(extract_toml_wire_api)
    {
        if is_chat_protocol(&wire_api) {
            return RelayProtocol::ChatCompletions;
        }
    }
    if extract_base_url(config)
        .map(|value| value.to_ascii_lowercase().ends_with("/chat/completions"))
        .unwrap_or(false)
    {
        return RelayProtocol::ChatCompletions;
    }
    RelayProtocol::Responses
}

fn extract_config_contents(config: &Value) -> Option<String> {
    config
        .get("config")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn extract_auth_contents(config: &Value) -> Option<String> {
    config.get("auth").and_then(|auth| {
        if auth.is_object() {
            serde_json::to_string_pretty(auth)
                .ok()
                .map(|value| format!("{value}\n"))
        } else {
            auth.as_str().map(str::to_string)
        }
    })
}

fn string_at(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(str::to_string)
}

fn trim_trailing_slash(value: String) -> String {
    value.trim().trim_end_matches('/').to_string()
}

fn is_chat_protocol(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "chat" | "chat_completions" | "chat-completions" | "openai_chat" | "openai-chat"
    )
}

fn extract_toml_base_url(text: &str) -> Option<String> {
    extract_toml_string_value(text, "base_url")
}

fn extract_toml_wire_api(text: &str) -> Option<String> {
    extract_toml_string_value(text, "wire_api")
}

fn extract_toml_string_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix(key) else {
            continue;
        };
        let rest = rest.trim_start();
        let Some(rest) = rest.strip_prefix('=') else {
            continue;
        };
        let rest = rest.trim_start();
        let quote = rest.chars().next()?;
        if quote != '"' && quote != '\'' {
            continue;
        }
        let rest = &rest[quote.len_utf8()..];
        let end = rest.find(quote)?;
        return Some(rest[..end].to_string());
    }
    None
}

fn build_config_toml(base_url: &str, api_key: &str, protocol: RelayProtocol) -> String {
    let wire_api = match protocol {
        RelayProtocol::Responses => "responses",
        RelayProtocol::ChatCompletions => "chat",
    };
    [
        "model_provider = \"custom\"".to_string(),
        String::new(),
        "[model_providers.custom]".to_string(),
        "name = \"custom\"".to_string(),
        format!("wire_api = \"{wire_api}\""),
        "requires_openai_auth = true".to_string(),
        format!("base_url = \"{}\"", toml_string(base_url)),
        format!("experimental_bearer_token = \"{}\"", toml_string(api_key)),
        String::new(),
    ]
    .join("\n")
}

fn build_auth_json(api_key: &str) -> String {
    format!(
        "{}\n",
        serde_json::to_string_pretty(&serde_json::json!({ "OPENAI_API_KEY": api_key }))
            .unwrap_or_else(|_| "{\"OPENAI_API_KEY\":\"\"}".to_string())
    )
}

fn toml_string(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn sanitize_id(value: &str) -> String {
    let mut result = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            result.push(ch.to_ascii_lowercase());
        } else if !result.ends_with('-') {
            result.push('-');
        }
    }
    let result = result.trim_matches('-').to_string();
    if result.is_empty() {
        "provider".to_string()
    } else {
        result
    }
}

fn unique_profile_id(base: &str, existing_ids: &[String]) -> String {
    if !existing_ids.iter().any(|id| id == base) {
        return base.to_string();
    }
    let mut index = 2;
    loop {
        let candidate = format!("{base}-{index}");
        if !existing_ids.iter().any(|id| id == &candidate) {
            return candidate;
        }
        index += 1;
    }
}

fn home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().to_path_buf())
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::params;
    use serde_json::json;

    fn create_ccs_db(path: &Path) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "CREATE TABLE providers (
                id TEXT NOT NULL,
                app_type TEXT NOT NULL,
                name TEXT NOT NULL,
                settings_config TEXT NOT NULL,
                created_at INTEGER,
                sort_index INTEGER,
                is_current BOOLEAN NOT NULL DEFAULT 0,
                PRIMARY KEY (id, app_type)
            )",
            [],
        )
        .unwrap();
    }

    fn insert_provider(path: &Path, id: &str, name: &str, config: Value, sort_index: i64) {
        let conn = Connection::open(path).unwrap();
        conn.execute(
            "INSERT INTO providers (id, app_type, name, settings_config, created_at, sort_index)
             VALUES (?1, 'codex', ?2, ?3, ?4, ?5)",
            params![id, name, config.to_string(), 1000 + sort_index, sort_index],
        )
        .unwrap();
    }

    #[test]
    fn imports_direct_base_url_and_api_key_provider() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        insert_provider(
            &db,
            "openai",
            "OpenAI",
            json!({
                "base_url": "https://api.openai.com/v1/",
                "api_key": "sk-openai"
            }),
            0,
        );

        let providers = list_codex_providers_from_db(&db).unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].source_id, "openai");
        assert_eq!(providers[0].name, "OpenAI");
        assert_eq!(providers[0].base_url, "https://api.openai.com/v1");
        assert_eq!(providers[0].api_key, "sk-openai");
        assert_eq!(providers[0].protocol, RelayProtocol::Responses);
        assert!(
            providers[0]
                .config_contents
                .contains("wire_api = \"responses\"")
        );
    }

    #[test]
    fn imports_auth_and_config_object_provider_as_chat_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        insert_provider(
            &db,
            "chat",
            "Chat Provider",
            json!({
                "auth": { "OPENAI_API_KEY": "sk-chat" },
                "config": { "base_url": "https://relay.example/v1/chat/completions" }
            }),
            0,
        );

        let providers = list_codex_providers_from_db(&db).unwrap();

        assert_eq!(
            providers[0].base_url,
            "https://relay.example/v1/chat/completions"
        );
        assert_eq!(providers[0].api_key, "sk-chat");
        assert_eq!(providers[0].protocol, RelayProtocol::ChatCompletions);
        assert_eq!(
            serde_json::from_str::<Value>(&providers[0].auth_contents).unwrap()["OPENAI_API_KEY"],
            json!("sk-chat")
        );
    }

    #[test]
    fn imports_toml_config_provider_and_preserves_config_text() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        let toml = r#"
model_provider = "Foo"

[model_providers.Foo]
wire_api = "chat"
base_url = "https://toml.example/v1"
"#;
        insert_provider(
            &db,
            "toml/provider",
            "TOML Provider",
            json!({
                "auth": { "OPENAI_API_KEY": "sk-toml" },
                "config": toml
            }),
            0,
        );

        let providers = list_codex_providers_from_db(&db).unwrap();
        let profile = relay_profile_from_ccs(&providers[0], &["ccs-toml-provider".to_string()]);

        assert_eq!(providers[0].base_url, "https://toml.example/v1");
        assert_eq!(providers[0].protocol, RelayProtocol::ChatCompletions);
        assert_eq!(providers[0].config_contents, toml);
        assert_eq!(profile.id, "ccs-toml-provider-2");
        assert_eq!(profile.relay_mode, RelayMode::PureApi);
    }

    #[test]
    fn imports_codex_config_snapshot_without_base_url_as_official_profile() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        insert_provider(
            &db,
            "blue-eagle",
            "蓝鹰AI",
            json!({
                "auth": {},
                "config": "model = \"gpt-image-2\"\n\n[features]\ngoals = true\n"
            }),
            0,
        );

        let providers = list_codex_providers_from_db(&db).unwrap();

        assert_eq!(providers.len(), 1);
        assert_eq!(providers[0].name, "蓝鹰AI");
        assert_eq!(providers[0].base_url, "");
        let profile = relay_profile_from_ccs(&providers[0], &[]);
        assert_eq!(profile.relay_mode, RelayMode::Official);
        assert!(profile.config_contents.contains("gpt-image-2"));
    }

    #[test]
    fn sync_linked_profiles_updates_existing_and_adds_new_profiles() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        insert_provider(
            &db,
            "linked-one",
            "Linked One",
            json!({
                "auth": { "OPENAI_API_KEY": "sk-linked" },
                "config": "model_provider = \"linked\"\n\n[model_providers.linked]\nbase_url = \"https://linked.example/v1\"\n"
            }),
            0,
        );
        insert_provider(
            &db,
            "linked-two",
            "Linked Two",
            json!({
                "base_url": "https://two.example/v1",
                "api_key": "sk-two"
            }),
            1,
        );

        let mut profiles = vec![RelayProfile {
            id: "local-linked".to_string(),
            linked_ccs_provider_id: "linked-one".to_string(),
            name: "Old".to_string(),
            ..RelayProfile::default()
        }];

        let synced = sync_linked_profiles_from_db(&db, &mut profiles).unwrap();

        assert_eq!(synced, 2);
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].name, "Linked One");
        assert_eq!(profiles[0].api_key, "sk-linked");
        assert_eq!(profiles[1].linked_ccs_provider_id, "linked-two");
        assert_eq!(profiles[1].base_url, "https://two.example/v1");
    }

    #[test]
    fn write_linked_profiles_updates_cc_switch_provider_config() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        create_ccs_db(&db);
        insert_provider(
            &db,
            "linked-one",
            "Before",
            json!({
                "auth": { "OPENAI_API_KEY": "old" },
                "config": "old"
            }),
            0,
        );
        let profiles = vec![RelayProfile {
            linked_ccs_provider_id: "linked-one".to_string(),
            name: "After".to_string(),
            config_contents: "model_provider = \"custom\"\n".to_string(),
            auth_contents: "{\"OPENAI_API_KEY\":\"sk-after\"}\n".to_string(),
            ..RelayProfile::default()
        }];

        let written = write_linked_profiles_to_db(&db, &profiles).unwrap();

        assert_eq!(written, 1);
        let conn = Connection::open(&db).unwrap();
        let (name, settings_config): (String, String) = conn
            .query_row(
                "SELECT name, settings_config FROM providers WHERE id = 'linked-one' AND app_type = 'codex'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let settings_config: Value = serde_json::from_str(&settings_config).unwrap();
        assert_eq!(name, "After");
        assert_eq!(settings_config["auth"]["OPENAI_API_KEY"], "sk-after");
        assert_eq!(settings_config["config"], "model_provider = \"custom\"\n");
    }

    #[test]
    fn set_current_codex_provider_updates_db_and_settings_file() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join(format!("{}-{}.db", "cc", "switch"));
        let settings = dir.path().join("settings.json");
        create_ccs_db(&db);
        insert_provider(&db, "old", "Old", json!({ "config": "old" }), 0);
        insert_provider(&db, "new", "New", json!({ "config": "new" }), 1);
        Connection::open(&db)
            .unwrap()
            .execute(
                "UPDATE providers SET is_current = 1 WHERE id = 'old' AND app_type = 'codex'",
                [],
            )
            .unwrap();

        let updated = set_current_codex_provider(&db, &settings, "new").unwrap();

        assert!(updated);
        let conn = Connection::open(&db).unwrap();
        let old_current: i64 = conn
            .query_row(
                "SELECT is_current FROM providers WHERE id = 'old' AND app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let new_current: i64 = conn
            .query_row(
                "SELECT is_current FROM providers WHERE id = 'new' AND app_type = 'codex'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(old_current, 0);
        assert_eq!(new_current, 1);
        let settings_value: Value =
            serde_json::from_str(&std::fs::read_to_string(settings).unwrap()).unwrap();
        assert_eq!(settings_value["currentProviderCodex"], json!("new"));
    }
}
