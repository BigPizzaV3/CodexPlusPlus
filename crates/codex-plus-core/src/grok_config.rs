use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, Item, Table, TableLike};

const CONFIG_FILE: &str = "config.toml";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokModelConfig {
    pub alias: String,
    pub model: String,
    pub name: String,
    pub base_url: String,
    pub api_backend: String,
    pub context_window: Option<u64>,
    pub api_key_configured: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokConfigPayload {
    pub grok_home: String,
    pub config_path: String,
    pub config_exists: bool,
    pub cli_path: Option<String>,
    pub cli_installed: bool,
    pub revision: String,
    pub default_model: String,
    pub models_base_url: String,
    pub models: Vec<GrokModelConfig>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrokModelInput {
    #[serde(default)]
    pub source_alias: String,
    pub alias: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub base_url: String,
    #[serde(default = "default_api_backend")]
    pub api_backend: String,
    #[serde(default)]
    pub context_window: Option<u64>,
    #[serde(default)]
    pub api_key_update: String,
    #[serde(default)]
    pub remove_api_key: bool,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveGrokConfigRequest {
    pub revision: String,
    #[serde(default)]
    pub default_model: String,
    #[serde(default)]
    pub models_base_url: String,
    #[serde(default)]
    pub models: Vec<GrokModelInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveGrokConfigResult {
    #[serde(flatten)]
    pub config: GrokConfigPayload,
    pub backup_path: Option<String>,
}

fn default_api_backend() -> String {
    "chat_completions".to_string()
}

pub fn default_grok_home_dir() -> PathBuf {
    std::env::var_os("GROK_HOME")
        .map(PathBuf::from)
        .filter(|path| !path.as_os_str().is_empty() && !path.to_string_lossy().trim().is_empty())
        .unwrap_or_else(|| {
            directories::BaseDirs::new()
                .map(|dirs| dirs.home_dir().join(".grok"))
                .unwrap_or_else(|| PathBuf::from(".grok"))
        })
}

pub fn load_grok_config() -> anyhow::Result<GrokConfigPayload> {
    load_grok_config_from_home(&default_grok_home_dir())
}

pub fn load_grok_config_from_home(home: &Path) -> anyhow::Result<GrokConfigPayload> {
    let config_path = home.join(CONFIG_FILE);
    let bytes = read_optional_bytes(&config_path)?;
    let text = std::str::from_utf8(&bytes).context("Grok config.toml 不是有效的 UTF-8")?;
    let doc = parse_document(text, &config_path)?;
    let cli_path = find_grok_cli(home);

    Ok(GrokConfigPayload {
        grok_home: home.to_string_lossy().to_string(),
        config_path: config_path.to_string_lossy().to_string(),
        config_exists: config_path.is_file(),
        cli_installed: cli_path.is_some(),
        cli_path: cli_path.map(|path| path.to_string_lossy().to_string()),
        revision: revision_for(&bytes),
        default_model: table_string(&doc, "models", "default"),
        models_base_url: table_string(&doc, "endpoints", "models_base_url"),
        models: read_models(&doc),
    })
}

pub fn save_grok_config(
    request: &SaveGrokConfigRequest,
    backup_root: &Path,
) -> anyhow::Result<SaveGrokConfigResult> {
    save_grok_config_at(&default_grok_home_dir(), request, backup_root)
}

pub fn save_grok_config_at(
    home: &Path,
    request: &SaveGrokConfigRequest,
    backup_root: &Path,
) -> anyhow::Result<SaveGrokConfigResult> {
    validate_request(request)?;

    let config_path = home.join(CONFIG_FILE);
    let previous = read_optional_bytes(&config_path)?;
    let actual_revision = revision_for(&previous);
    if request.revision != actual_revision {
        bail!("Grok 配置已被其他程序修改，请刷新后再保存");
    }
    let text = std::str::from_utf8(&previous).context("Grok config.toml 不是有效的 UTF-8")?;
    let mut doc = parse_document(text, &config_path)?;

    update_root_string(&mut doc, "models", "default", &request.default_model)?;
    update_root_string(
        &mut doc,
        "endpoints",
        "models_base_url",
        &request.models_base_url,
    )?;
    update_models(&mut doc, &request.models)?;

    let mut updated = doc.to_string();
    if !updated.is_empty() && !updated.ends_with('\n') {
        updated.push('\n');
    }
    let backup_path = create_backup(&config_path, &previous, backup_root)?;
    secure_atomic_write(&config_path, updated.as_bytes())?;

    let mut config = load_grok_config_from_home(home)?;
    // CLI detection uses the same home, while the revision must reflect the bytes just written.
    config.revision = revision_for(updated.as_bytes());
    Ok(SaveGrokConfigResult {
        config,
        backup_path: backup_path.map(|path| path.to_string_lossy().to_string()),
    })
}

fn read_models(doc: &DocumentMut) -> Vec<GrokModelConfig> {
    let Some(models) = doc.get("model").and_then(Item::as_table_like) else {
        return Vec::new();
    };
    models
        .iter()
        .filter_map(|(alias, item)| {
            let table = item.as_table_like()?;
            Some(GrokModelConfig {
                alias: alias.to_string(),
                model: item_string(table, "model"),
                name: item_string(table, "name"),
                base_url: item_string(table, "base_url"),
                api_backend: match item_string(table, "api_backend").trim() {
                    "" => default_api_backend(),
                    value => value.to_string(),
                },
                context_window: table
                    .get("context_window")
                    .and_then(Item::as_integer)
                    .and_then(|value| u64::try_from(value).ok()),
                api_key_configured: table
                    .get("api_key")
                    .and_then(Item::as_str)
                    .is_some_and(|value| !value.trim().is_empty()),
            })
        })
        .collect()
}

fn update_models(doc: &mut DocumentMut, inputs: &[GrokModelInput]) -> anyhow::Result<()> {
    if doc.get("model").is_none() && inputs.is_empty() {
        return Ok(());
    }
    if doc.get("model").is_none() {
        doc["model"] = toml_edit::table();
    }
    let models = doc["model"]
        .as_table_mut()
        .context("Grok config.toml 中的 [model] 不是表")?;

    let existing_table_aliases = models
        .iter()
        .filter_map(|(alias, item)| item.as_table_like().map(|_| alias.to_string()))
        .collect::<HashSet<_>>();
    let source_aliases = inputs
        .iter()
        .filter_map(|input| {
            let source = input.source_alias.trim();
            (!source.is_empty()).then(|| source.to_string())
        })
        .collect::<HashSet<_>>();

    let mut preserved = HashMap::new();
    for source in &source_aliases {
        let item = models
            .remove(source)
            .with_context(|| format!("Grok 模型来源别名不存在：{source}"))?;
        preserved.insert(source.clone(), item);
    }
    for alias in existing_table_aliases.difference(&source_aliases) {
        models.remove(alias);
    }

    for input in inputs {
        let source = input.source_alias.trim();
        let alias = input.alias.trim();
        let mut item = if source.is_empty() {
            Item::Table(Table::new())
        } else {
            preserved
                .remove(source)
                .unwrap_or_else(|| Item::Table(Table::new()))
        };
        let table = item
            .as_table_like_mut()
            .context("Grok 模型配置不是有效的 TOML 表")?;
        update_table_string(table, "model", &input.model);
        update_table_string(table, "name", &input.name);
        update_table_string(table, "base_url", &input.base_url);
        update_table_string(table, "api_backend", &input.api_backend);
        match input.context_window {
            Some(value) => {
                table.insert("context_window", toml_edit::value(value as i64));
            }
            None => {
                table.remove("context_window");
            }
        }
        if input.remove_api_key {
            table.remove("api_key");
        } else if !input.api_key_update.trim().is_empty() {
            table.insert("api_key", toml_edit::value(input.api_key_update.trim()));
        }
        if models.contains_key(alias) {
            bail!("Grok 模型别名与未管理配置冲突：{alias}");
        }
        models.insert(alias, item);
    }
    Ok(())
}

fn validate_request(request: &SaveGrokConfigRequest) -> anyhow::Result<()> {
    let mut aliases = HashSet::new();
    let mut sources = HashSet::new();
    for input in &request.models {
        let alias = input.alias.trim();
        if alias.is_empty() {
            bail!("Grok 模型别名不能为空");
        }
        if alias.chars().any(char::is_control) {
            bail!("Grok 模型别名不能包含控制字符");
        }
        if !aliases.insert(alias.to_string()) {
            bail!("Grok 模型别名重复：{alias}");
        }
        let source = input.source_alias.trim();
        if !source.is_empty() && !sources.insert(source.to_string()) {
            bail!("Grok 模型来源别名重复：{source}");
        }
        if !matches!(
            input.api_backend.trim(),
            "responses" | "chat_completions" | "messages"
        ) {
            bail!("Grok 模型 {alias} 的 API 协议无效");
        }
        if input.context_window == Some(0) {
            bail!("Grok 模型 {alias} 的上下文窗口必须大于 0");
        }
        if input
            .context_window
            .is_some_and(|value| value > i64::MAX as u64)
        {
            bail!("Grok 模型 {alias} 的上下文窗口过大");
        }
        if input.remove_api_key && !input.api_key_update.is_empty() {
            bail!("Grok 模型 {alias} 不能同时替换并移除 API Key");
        }
    }
    Ok(())
}

fn table_string(doc: &DocumentMut, table: &str, key: &str) -> String {
    doc.get(table)
        .and_then(Item::as_table_like)
        .and_then(|values| values.get(key))
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_string()
}

fn item_string<T: TableLike + ?Sized>(table: &T, key: &str) -> String {
    table
        .get(key)
        .and_then(Item::as_str)
        .unwrap_or_default()
        .to_string()
}

fn update_root_string(
    doc: &mut DocumentMut,
    table: &str,
    key: &str,
    value: &str,
) -> anyhow::Result<()> {
    if doc.get(table).is_none() && value.trim().is_empty() {
        return Ok(());
    }
    if doc.get(table).is_none() {
        doc[table] = toml_edit::table();
    }
    let values = doc[table]
        .as_table_like_mut()
        .with_context(|| format!("Grok config.toml 中的 [{table}] 不是表"))?;
    update_table_string(values, key, value);
    Ok(())
}

fn update_table_string(table: &mut dyn TableLike, key: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        table.remove(key);
    } else {
        table.insert(key, toml_edit::value(value));
    }
}

fn parse_document(contents: &str, path: &Path) -> anyhow::Result<DocumentMut> {
    if contents.trim().is_empty() {
        return Ok(DocumentMut::new());
    }
    contents
        .parse::<DocumentMut>()
        .with_context(|| format!("无法解析 Grok 配置 {}", path.display()))
}

fn read_optional_bytes(path: &Path) -> anyhow::Result<Vec<u8>> {
    match fs::read(path) {
        Ok(bytes) => Ok(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error).with_context(|| format!("无法读取 {}", path.display())),
    }
}

fn revision_for(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn find_grok_cli(home: &Path) -> Option<PathBuf> {
    let binary = if cfg!(windows) { "grok.exe" } else { "grok" };
    let bundled = home.join("bin").join(binary);
    if bundled.is_file() {
        return Some(bundled);
    }
    if let Some(user_local) = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".local").join("bin").join(binary))
        .filter(|path| path.is_file())
    {
        return Some(user_local);
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|path| path.join(binary))
        .find(|path| path.is_file())
}

fn create_backup(
    config_path: &Path,
    previous: &[u8],
    backup_root: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if previous.is_empty() && !config_path.is_file() {
        return Ok(None);
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let backup_dir = backup_root.join("grok").join(timestamp.to_string());
    create_private_dir_all(&backup_dir)?;
    let backup_path = backup_dir.join(CONFIG_FILE);
    write_private_file(&backup_path, previous)?;
    Ok(Some(backup_path))
}

#[cfg(unix)]
fn create_private_dir_all(path: &Path) -> anyhow::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let existed = path.is_dir();
    fs::create_dir_all(path)?;
    if !existed {
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn create_private_dir_all(path: &Path) -> anyhow::Result<()> {
    fs::create_dir_all(path)?;
    Ok(())
}

fn write_private_file(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;

        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        return Ok(());
    }
    #[cfg(not(unix))]
    {
        fs::write(path, bytes)?;
        Ok(())
    }
}

#[cfg(unix)]
fn secure_atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path.parent().context("Grok 配置路径缺少父目录")?;
    create_private_dir_all(parent)?;
    let mode = fs::metadata(path)
        .ok()
        .map(|metadata| metadata.permissions().mode() & 0o777)
        .unwrap_or(0o600);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temp_path = parent.join(format!(".{CONFIG_FILE}.codex-plus-{timestamp}.tmp"));
    let write_result = (|| -> anyhow::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(mode)
            .open(&temp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::set_permissions(&temp_path, fs::Permissions::from_mode(mode))?;
        fs::rename(&temp_path, path)?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    write_result.with_context(|| format!("无法写入 Grok 配置 {}", path.display()))
}

#[cfg(not(unix))]
fn secure_atomic_write(path: &Path, bytes: &[u8]) -> anyhow::Result<()> {
    crate::settings::atomic_write(path, bytes)
        .with_context(|| format!("无法写入 Grok 配置 {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(payload: &GrokConfigPayload) -> SaveGrokConfigRequest {
        SaveGrokConfigRequest {
            revision: payload.revision.clone(),
            default_model: "renamed".to_string(),
            models_base_url: "https://new.example/v1".to_string(),
            models: vec![GrokModelInput {
                source_alias: "custom".to_string(),
                alias: "renamed".to_string(),
                model: "grok-4.5".to_string(),
                name: "Renamed Grok".to_string(),
                base_url: String::new(),
                api_backend: "responses".to_string(),
                context_window: Some(1_000_000),
                api_key_update: String::new(),
                remove_api_key: false,
            }],
        }
    }

    #[test]
    fn load_redacts_api_keys_and_reads_native_fields() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            r#"[models]
default = "custom"

[endpoints]
models_base_url = "https://example.com/v1"

[model.custom]
model = "grok-4.5"
name = "Grok"
api_key = "secret"
api_backend = "responses"
context_window = 1000000
"#,
        )
        .unwrap();

        let payload = load_grok_config_from_home(temp.path()).unwrap();
        assert_eq!(payload.default_model, "custom");
        assert_eq!(payload.models_base_url, "https://example.com/v1");
        assert_eq!(payload.models.len(), 1);
        assert!(payload.models[0].api_key_configured);
        assert_eq!(payload.models[0].context_window, Some(1_000_000));
        assert!(!serde_json::to_string(&payload).unwrap().contains("secret"));
    }

    #[test]
    fn save_preserves_unknown_sections_fields_comments_and_api_key() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            r#"# keep this comment
[ui]
yolo = false

[models]
default = "custom"
web_search = "search-model"

[model.custom]
model = "old-model"
api_key = "keep-secret"
temperature = 0.4

[model.obsolete]
model = "old"
"#,
        )
        .unwrap();
        let payload = load_grok_config_from_home(temp.path()).unwrap();

        let result = save_grok_config_at(temp.path(), &request(&payload), backup.path()).unwrap();
        let updated = fs::read_to_string(temp.path().join(CONFIG_FILE)).unwrap();
        assert!(updated.contains("# keep this comment"));
        assert!(updated.contains("[ui]"));
        assert!(updated.contains("web_search = \"search-model\""));
        assert!(updated.contains("temperature = 0.4"));
        assert!(updated.contains("api_key = \"keep-secret\""));
        assert!(updated.contains("[model.renamed]"));
        assert!(!updated.contains("[model.custom]"));
        assert!(!updated.contains("[model.obsolete]"));
        assert!(result.backup_path.is_some());
    }

    #[test]
    fn save_can_replace_and_remove_api_key() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join(CONFIG_FILE),
            "[model.custom]\napi_key = \"old\"\n",
        )
        .unwrap();
        let payload = load_grok_config_from_home(temp.path()).unwrap();
        let mut next = request(&payload);
        next.models[0].api_key_update = "new-secret".to_string();
        save_grok_config_at(temp.path(), &next, backup.path()).unwrap();
        let updated = fs::read_to_string(temp.path().join(CONFIG_FILE)).unwrap();
        assert!(updated.contains("api_key = \"new-secret\""));

        let payload = load_grok_config_from_home(temp.path()).unwrap();
        let mut next = request(&payload);
        next.models[0].source_alias = "renamed".to_string();
        next.models[0].remove_api_key = true;
        save_grok_config_at(temp.path(), &next, backup.path()).unwrap();
        let updated = fs::read_to_string(temp.path().join(CONFIG_FILE)).unwrap();
        assert!(!updated.contains("api_key"));
    }

    #[test]
    fn save_rejects_stale_revision_without_writing() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let config_path = temp.path().join(CONFIG_FILE);
        fs::write(&config_path, "[models]\ndefault = \"one\"\n").unwrap();
        let payload = load_grok_config_from_home(temp.path()).unwrap();
        fs::write(&config_path, "[models]\ndefault = \"external\"\n").unwrap();

        let error =
            save_grok_config_at(temp.path(), &request(&payload), backup.path()).unwrap_err();
        assert!(error.to_string().contains("已被其他程序修改"));
        assert!(
            fs::read_to_string(config_path)
                .unwrap()
                .contains("external")
        );
        assert!(!backup.path().join("grok").exists());
    }

    #[test]
    fn save_rejects_invalid_managed_table_without_overwriting() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let config_path = temp.path().join(CONFIG_FILE);
        let original = "models = \"invalid\"\n";
        fs::write(&config_path, original).unwrap();
        let payload = load_grok_config_from_home(temp.path()).unwrap();

        let error =
            save_grok_config_at(temp.path(), &request(&payload), backup.path()).unwrap_err();
        assert!(error.to_string().contains("[models] 不是表"));
        assert_eq!(fs::read_to_string(config_path).unwrap(), original);
        assert!(!backup.path().join("grok").exists());
    }

    #[test]
    fn save_creates_a_new_private_config() {
        let temp = tempfile::tempdir().unwrap();
        let backup = tempfile::tempdir().unwrap();
        let payload = load_grok_config_from_home(temp.path()).unwrap();
        let mut next = request(&payload);
        next.models[0].source_alias.clear();
        let result = save_grok_config_at(temp.path(), &next, backup.path()).unwrap();
        assert!(result.backup_path.is_none());
        assert!(temp.path().join(CONFIG_FILE).is_file());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(temp.path().join(CONFIG_FILE))
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn validation_rejects_duplicate_aliases_and_zero_windows() {
        let payload = GrokConfigPayload {
            grok_home: String::new(),
            config_path: String::new(),
            config_exists: false,
            cli_path: None,
            cli_installed: false,
            revision: revision_for(&[]),
            default_model: String::new(),
            models_base_url: String::new(),
            models: Vec::new(),
        };
        let mut next = request(&payload);
        next.models.push(GrokModelInput {
            source_alias: String::new(),
            alias: "renamed".to_string(),
            model: String::new(),
            name: String::new(),
            base_url: String::new(),
            api_backend: "responses".to_string(),
            context_window: None,
            api_key_update: String::new(),
            remove_api_key: false,
        });
        assert!(validate_request(&next).is_err());
        next.models.pop();
        next.models[0].context_window = Some(0);
        assert!(validate_request(&next).is_err());
    }
}
