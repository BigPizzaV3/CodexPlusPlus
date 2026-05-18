use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Context;
use serde::Serialize;
use serde_json::{Map, Value, json};

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UserScriptConfig {
    pub enabled: bool,
    pub scripts: BTreeMap<String, bool>,
}

impl Default for UserScriptConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scripts: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct UserScriptManager {
    builtin_dir: PathBuf,
    user_dir: PathBuf,
    config_path: PathBuf,
    config_lock: Arc<Mutex<()>>,
}

impl UserScriptManager {
    pub fn new(
        builtin_dir: impl Into<PathBuf>,
        user_dir: impl Into<PathBuf>,
        config_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            builtin_dir: builtin_dir.into(),
            user_dir: user_dir.into(),
            config_path: config_path.into(),
            config_lock: Arc::new(Mutex::new(())),
        }
    }

    pub fn load_config(&self) -> UserScriptConfig {
        let _guard = self.config_lock.lock().unwrap();
        self.load_config_unlocked()
    }

    fn load_config_unlocked(&self) -> UserScriptConfig {
        let Ok(text) = fs::read_to_string(&self.config_path) else {
            return UserScriptConfig::default();
        };
        let Ok(Value::Object(raw)) = serde_json::from_str::<Value>(&text) else {
            return UserScriptConfig::default();
        };
        config_from_object(&raw)
    }

    pub fn save_config(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        let _guard = self.config_lock.lock().unwrap();
        self.save_config_unlocked(config)
    }

    fn save_config_unlocked(&self, config: &UserScriptConfig) -> anyhow::Result<()> {
        if let Some(parent) = self.config_path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "failed to create user script config directory {}",
                    parent.display()
                )
            })?;
        }
        crate::settings::atomic_write(
            &self.config_path,
            serde_json::to_string_pretty(config)?.as_bytes(),
        )
    }

    pub fn set_global_enabled(&self, enabled: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.enabled = enabled;
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn set_script_enabled(&self, key: &str, enabled: bool) -> anyhow::Result<UserScriptConfig> {
        let _guard = self.config_lock.lock().unwrap();
        let mut config = self.load_config_unlocked();
        config.scripts.insert(key.to_string(), enabled);
        self.save_config_unlocked(&config)?;
        Ok(config)
    }

    pub fn inventory(&self) -> anyhow::Result<Value> {
        let config = self.load_config();
        let scripts = self.scan_scripts(&config)?;
        Ok(json!({
            "enabled": config.enabled,
            "builtin_dir": self.builtin_dir.to_string_lossy(),
            "user_dir": self.user_dir.to_string_lossy(),
            "scripts": scripts
        }))
    }

    pub fn build_enabled_bundle(&self) -> anyhow::Result<String> {
        let config = self.load_config();
        if !config.enabled {
            return Ok(String::new());
        }
        let mut blocks = Vec::new();
        for script in self.scan_script_files(&config)? {
            if !script.enabled {
                continue;
            }
            let source = fs::read_to_string(&script.path)
                .unwrap_or_else(|error| format!("throw new Error({});", json!(error.to_string())));
            blocks.push(wrap_script(&script, &source));
        }
        Ok(blocks.join("\n"))
    }

    fn scan_scripts(&self, config: &UserScriptConfig) -> anyhow::Result<Vec<Value>> {
        Ok(self
            .scan_script_files(config)?
            .into_iter()
            .map(|script| {
                let status = if !config.enabled || !script.enabled {
                    "disabled"
                } else {
                    "not_loaded"
                };
                json!({
                    "key": script.key,
                    "name": script.name,
                    "source": script.source,
                    "enabled": script.enabled,
                    "status": status,
                    "error": ""
                })
            })
            .collect())
    }

    fn scan_script_files(&self, config: &UserScriptConfig) -> anyhow::Result<Vec<UserScriptFile>> {
        fs::create_dir_all(&self.user_dir).with_context(|| {
            format!(
                "failed to create user scripts directory {}",
                self.user_dir.display()
            )
        })?;
        let mut scripts = Vec::new();
        self.append_scripts("builtin", &self.builtin_dir, config, &mut scripts)?;
        self.append_scripts("user", &self.user_dir, config, &mut scripts)?;
        Ok(scripts)
    }

    fn append_scripts(
        &self,
        source: &str,
        directory: &std::path::Path,
        config: &UserScriptConfig,
        scripts: &mut Vec<UserScriptFile>,
    ) -> anyhow::Result<()> {
        let Ok(entries) = fs::read_dir(directory) else {
            return Ok(());
        };
        let mut paths = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("js"))
            .collect::<Vec<_>>();
        paths.sort_by_key(|path| {
            path.file_name()
                .map(|name| name.to_string_lossy().to_lowercase())
                .unwrap_or_default()
        });

        for path in paths {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
                .unwrap_or_default();
            let key = format!("{source}:{name}");
            scripts.push(UserScriptFile {
                enabled: config.scripts.get(&key).copied().unwrap_or(true),
                key,
                name,
                source: source.to_string(),
                path,
            });
        }
        Ok(())
    }
}

#[derive(Debug)]
struct UserScriptFile {
    key: String,
    name: String,
    source: String,
    path: PathBuf,
    enabled: bool,
}

fn wrap_script(script: &UserScriptFile, source: &str) -> String {
    format!(
        r#"
(() => {{
  window.__codexPlusUserScripts = window.__codexPlusUserScripts || {{ scripts: {{}} }};
  const key = {key};
  window.__codexPlusUserScripts.scripts[key] = {{ key, name: {name}, source: {source_name}, status: "loading", error: "", loadedAt: new Date().toISOString() }};
  try {{
{source}
    window.__codexPlusUserScripts.scripts[key].status = "loaded";
    window.__codexPlusUserScripts.scripts[key].loadedAt = new Date().toISOString();
  }} catch (error) {{
    window.__codexPlusUserScripts.scripts[key].status = "failed";
    window.__codexPlusUserScripts.scripts[key].error = String(error && (error.stack || error.message) || error);
  }}
}})();
"#,
        key = json!(script.key).to_string(),
        name = json!(script.name).to_string(),
        source_name = json!(script.source).to_string(),
        source = source
    )
}

fn config_from_object(raw: &Map<String, Value>) -> UserScriptConfig {
    let enabled = raw.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    let scripts = raw
        .get("scripts")
        .and_then(Value::as_object)
        .map(|items| {
            items
                .iter()
                .filter_map(|(key, value)| Some((key.clone(), value.as_bool()?)))
                .collect::<BTreeMap<_, _>>()
        })
        .unwrap_or_default();
    UserScriptConfig { enabled, scripts }
}
