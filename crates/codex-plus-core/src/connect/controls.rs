use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

static WRITE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionOptions {
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub reasoning_effort: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_request: Option<String>,
}

/// 与轮询游标分开保存，工具执行时落盘不会覆盖正在处理的消息边界。
#[derive(Debug, Clone)]
pub struct SessionOptionsStore {
    path: PathBuf,
    peer: String,
}

impl SessionOptionsStore {
    #[cfg(test)]
    pub(super) fn test_store(path: PathBuf, peer: &str) -> Self {
        Self {
            path,
            peer: peer.to_string(),
        }
    }

    pub fn for_peer(account: &str, peer: &str) -> Self {
        Self::in_directory(&crate::paths::default_app_state_dir(), account, peer)
    }

    fn in_directory(directory: &std::path::Path, account: &str, peer: &str) -> Self {
        let path = directory.join("weixin-session-options").join(format!(
            "{}.json",
            super::session_store::account_key(account)
        ));
        Self {
            path,
            peer: peer.to_string(),
        }
    }

    fn load_all(&self) -> anyhow::Result<BTreeMap<String, SessionOptions>> {
        match std::fs::read(&self.path) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes).context("微信会话设置损坏")?),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(error) => Err(error.into()),
        }
    }

    pub fn load(&self) -> anyhow::Result<SessionOptions> {
        Ok(self.load_all()?.remove(&self.peer).unwrap_or_default())
    }

    pub fn save(&self, options: SessionOptions) -> anyhow::Result<()> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let mut all = self.load_all()?;
        let reset_request = all
            .get(&self.peer)
            .and_then(|saved| saved.reset_request.clone());
        all.insert(
            self.peer.clone(),
            SessionOptions {
                reset_request,
                ..options
            },
        );
        crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&all)?)
    }

    pub fn request_reset(&self) -> anyhow::Result<()> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let mut all = self.load_all()?;
        all.insert(
            self.peer.clone(),
            SessionOptions {
                reset_request: Some(uuid::Uuid::new_v4().to_string()),
                ..SessionOptions::default()
            },
        );
        crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&all)?)
    }

    pub fn complete_reset(&self, request: &str) -> anyhow::Result<()> {
        let _guard = WRITE_LOCK.lock().unwrap_or_else(|error| error.into_inner());
        let mut all = self.load_all()?;
        if let Some(saved) = all.get_mut(&self.peer)
            && saved.reset_request.as_deref() == Some(request)
        {
            saved.reset_request = None;
            crate::settings::atomic_write(&self.path, &serde_json::to_vec_pretty(&all)?)?;
        }
        Ok(())
    }
}

pub fn effective_options(defaults: &SessionOptions, overrides: &SessionOptions) -> SessionOptions {
    SessionOptions {
        model: if overrides.model.is_empty() {
            defaults.model.clone()
        } else {
            overrides.model.clone()
        },
        reasoning_effort: if overrides.reasoning_effort.is_empty() {
            defaults.reasoning_effort.clone()
        } else {
            overrides.reasoning_effort.clone()
        },
        reset_request: None,
    }
}

pub fn session_tool_spec() -> Value {
    json!({
        "type": "function", "name": "weixin_session_settings",
        "description": "查看或按用户明确要求修改当前微信联系人的模型及推理强度。get 查看，set 保存 model/reasoningEffort，空字符串继承连接默认值，reset 显式重置会话并清除联系人覆盖值。设置及重置均从下一条消息生效，保留原历史，不改当前回合或全局配置。",
        "inputSchema": {
            "type": "object",
            "properties": {
                "action": {"type":"string","enum":["get","set","reset"]},
                "model": {"type":"string"},
                "reasoningEffort": {"type":"string"}
            },
            "required":["action"], "additionalProperties":false
        }
    })
}

impl SessionOptions {
    pub fn resolve_for_turn(&self, defaults: &Self, catalog: &Value) -> (Self, bool) {
        let resolved = self.resolve(defaults, catalog);
        if self.model.is_empty() && self.reasoning_effort.is_empty() {
            return (resolved, false);
        }
        // 连接默认模型可能来自原生配置而不在公开目录中，只校验联系人的覆盖值。
        let mut checked_catalog = catalog.clone();
        if self.model.is_empty()
            && let Some(models) = checked_catalog["models"].as_array_mut()
            && !models
                .iter()
                .any(|model| model.as_str() == Some(&resolved.model))
        {
            models.push(json!(resolved.model));
        }
        if validate_options(&checked_catalog, &resolved).is_ok() {
            return (resolved, false);
        }
        (Self::default().resolve(defaults, catalog), true)
    }

    pub fn with_defaults(&self, defaults: &Self) -> Self {
        effective_options(defaults, self)
    }

    pub fn resolve(&self, defaults: &Self, catalog: &Value) -> Self {
        let mut resolved = self.with_defaults(defaults);
        if resolved.model.is_empty() {
            resolved.model = catalog["model"]
                .as_str()
                .or_else(|| catalog["default_model"].as_str())
                .unwrap_or_default()
                .to_string();
        }
        if resolved.reasoning_effort.is_empty() {
            resolved.reasoning_effort =
                catalog["modelMetadata"][&resolved.model]["defaultReasoningEffort"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
        }
        resolved
    }
}

pub fn updated_options(
    catalog: &Value,
    defaults: &SessionOptions,
    previous: &SessionOptions,
    args: &serde_json::Map<String, Value>,
) -> anyhow::Result<SessionOptions> {
    if !args.contains_key("model") && !args.contains_key("reasoningEffort") {
        bail!("没有指定需要修改的设置");
    }
    let mut next = previous.clone();
    if let Some(value) = args.get("model") {
        next.model = value
            .as_str()
            .context("model 必须为字符串")?
            .trim()
            .to_string();
    }
    if let Some(value) = args.get("reasoningEffort") {
        next.reasoning_effort = value
            .as_str()
            .context("reasoningEffort 必须为字符串")?
            .trim()
            .to_string();
    }
    validate_options(catalog, &next.resolve(defaults, catalog))?;
    Ok(next)
}

pub fn apply_native_model_defaults(
    defaults: &mut SessionOptions,
    catalog: &mut Value,
    response: &Value,
) {
    let Some(models) = response["data"].as_array() else {
        return;
    };
    for model in models {
        let Some(name) = model["model"].as_str().or_else(|| model["id"].as_str()) else {
            continue;
        };
        if defaults.model.is_empty() && model["isDefault"] == true {
            defaults.model = name.to_string();
        }
        for key in ["defaultReasoningEffort", "supportedReasoningEfforts"] {
            if catalog["modelMetadata"][name][key].is_null()
                && let Some(value) = model.get(key)
            {
                catalog["modelMetadata"][name][key] = value.clone();
            }
        }
    }
}

pub fn validate_options(catalog: &Value, options: &SessionOptions) -> anyhow::Result<()> {
    if !options.model.is_empty() {
        let available = catalog["models"].as_array().context("当前模型目录不可用")?;
        if !available
            .iter()
            .any(|model| model.as_str() == Some(&options.model))
        {
            bail!("该模型不在当前模型列表中，请先查看可用模型");
        }
        let metadata = &catalog["modelMetadata"][&options.model];
        let levels = metadata
            .get("supportedReasoningEfforts")
            .or_else(|| metadata.get("supported_reasoning_levels"))
            .or_else(|| metadata.get("supportedReasoningLevels"))
            .and_then(Value::as_array);
        if let Some(levels) = levels
            && !options.reasoning_effort.is_empty()
            && !levels.iter().any(|level| {
                level
                    .as_str()
                    .or_else(|| level.get("effort").and_then(Value::as_str))
                    .or_else(|| level.get("reasoningEffort").and_then(Value::as_str))
                    == Some(&options.reasoning_effort)
            })
        {
            bail!("当前模型不支持该推理强度");
        }
        if levels.is_none()
            && ![
                "", "none", "minimal", "low", "medium", "high", "xhigh", "max", "ultra",
            ]
            .contains(&options.reasoning_effort.as_str())
        {
            bail!("当前模型目录未声明该推理强度");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_resets_clear_overrides_and_do_not_clear_newer_requests() {
        let temp = tempfile::tempdir().unwrap();
        let first = SessionOptionsStore::in_directory(temp.path(), "Bot.1@wx", "peer-a");
        let other = SessionOptionsStore::in_directory(temp.path(), "Bot.1@wx", "peer-b");
        other
            .save(SessionOptions {
                model: "other-model".into(),
                ..SessionOptions::default()
            })
            .unwrap();
        first.request_reset().unwrap();
        let request = first.load().unwrap().reset_request.unwrap();
        first
            .save(SessionOptions {
                model: "m".into(),
                ..SessionOptions::default()
            })
            .unwrap();
        assert_eq!(
            first.load().unwrap().reset_request.as_deref(),
            Some(request.as_str())
        );
        assert!(other.load().unwrap().reset_request.is_none());
        first.request_reset().unwrap();
        first.complete_reset(&request).unwrap();
        let next = first.load().unwrap().reset_request.unwrap();
        assert_ne!(request, next);
        first.complete_reset(&next).unwrap();
        assert!(first.load().unwrap().reset_request.is_none());
        assert_eq!(first.load().unwrap(), SessionOptions::default());
        assert_eq!(other.load().unwrap().model, "other-model");
    }

    #[test]
    fn stale_contact_options_fall_back_to_current_defaults_before_a_turn() {
        let defaults = SessionOptions {
            model: "current".into(),
            reasoning_effort: "low".into(),
            ..SessionOptions::default()
        };
        let catalog = json!({"models":["current","other"],"modelMetadata":{
            "current":{"supportedReasoningEfforts":["low"]},
            "other":{"supportedReasoningEfforts":["high"]}
        }});
        for options in [
            SessionOptions {
                model: "removed".into(),
                ..SessionOptions::default()
            },
            SessionOptions {
                model: "current".into(),
                reasoning_effort: "high".into(),
                ..SessionOptions::default()
            },
        ] {
            let (resolved, recovered) = options.resolve_for_turn(&defaults, &catalog);
            assert!(recovered);
            assert_eq!(resolved, defaults);
        }
        let valid = SessionOptions {
            model: "other".into(),
            reasoning_effort: "high".into(),
            ..SessionOptions::default()
        };
        assert_eq!(
            valid.resolve_for_turn(&defaults, &catalog),
            (valid.clone(), false)
        );
        assert_eq!(
            SessionOptions::default().resolve_for_turn(&defaults, &json!({})),
            (defaults, false)
        );
    }

    #[test]
    fn native_default_model_is_resolved_before_validating_saved_effort() {
        let mut defaults = SessionOptions::default();
        let mut catalog = json!({"model":"", "models":[], "modelMetadata":{}});
        apply_native_model_defaults(
            &mut defaults,
            &mut catalog,
            &json!({"data":[{
                "model":"native-default", "isDefault":true, "defaultReasoningEffort":"low",
                "supportedReasoningEfforts":[{"reasoningEffort":"low"}]
            }]}),
        );
        let saved = SessionOptions {
            reasoning_effort: "high".into(),
            ..SessionOptions::default()
        };
        let (resolved, recovered) = saved.resolve_for_turn(&defaults, &catalog);
        assert!(recovered);
        assert_eq!(resolved.model, "native-default");
        assert_eq!(resolved.reasoning_effort, "low");
        let saved = SessionOptions {
            reasoning_effort: "low".into(),
            ..SessionOptions::default()
        };
        assert!(!saved.resolve_for_turn(&defaults, &catalog).1);
        assert!(defaults.reasoning_effort.is_empty());
    }

    #[test]
    fn account_paths_do_not_collide_or_load_unpublished_options() {
        let temp = tempfile::tempdir().unwrap();
        let accounts = [
            "a/b",
            "a\\b",
            "a:b",
            "a\0b",
            "a_b",
            "A_b",
            "",
            "../a",
            "中文",
            "Bot@wx.example",
            "bot@wx.example",
            "default",
        ];
        let mut paths = std::collections::BTreeSet::new();
        for (index, account) in accounts.iter().enumerate() {
            let store = SessionOptionsStore::in_directory(temp.path(), account, "peer");
            assert!(paths.insert(store.path.to_string_lossy().to_lowercase()));
            store
                .save(SessionOptions {
                    model: index.to_string(),
                    reasoning_effort: String::new(),
                    reset_request: None,
                })
                .unwrap();
        }
        for (index, account) in accounts.iter().enumerate() {
            assert_eq!(
                SessionOptionsStore::in_directory(temp.path(), account, "peer")
                    .load()
                    .unwrap()
                    .model,
                index.to_string()
            );
        }
        let legacy = temp.path().join("weixin-session-options-ordinary_123.json");
        std::fs::write(
            &legacy,
            br#"{"peer":{"model":"legacy","reasoningEffort":"low"}}"#,
        )
        .unwrap();
        let store = SessionOptionsStore::in_directory(temp.path(), "ordinary_123", "peer");
        assert_ne!(store.path, legacy);
        assert_eq!(store.load().unwrap(), SessionOptions::default());
        let upper = temp.path().join("weixin-session-options-Upper_123.json");
        std::fs::write(&upper, br#"{"peer":{"model":"legacy-upper"}}"#).unwrap();
        let store = SessionOptionsStore::in_directory(temp.path(), "Upper_123", "peer");
        assert_eq!(store.load().unwrap(), SessionOptions::default());
        store
            .save(SessionOptions {
                model: "new".into(),
                reasoning_effort: String::new(),
                reset_request: None,
            })
            .unwrap();
        assert_eq!(store.load().unwrap().model, "new");
        assert!(
            std::fs::read_to_string(upper)
                .unwrap()
                .contains("legacy-upper")
        );
    }

    #[test]
    fn next_message_reloads_overrides_and_clearing_uses_connection_defaults() {
        let temp = tempfile::tempdir().unwrap();
        let a = SessionOptionsStore::in_directory(temp.path(), "account", "a");
        let b = SessionOptionsStore::in_directory(temp.path(), "account", "b");
        let defaults = SessionOptions {
            model: "global".into(),
            reasoning_effort: "low".into(),
            reset_request: None,
        };
        let current = effective_options(&defaults, &a.load().unwrap());
        let overrides = SessionOptions {
            model: "override".into(),
            reasoning_effort: "high".into(),
            reset_request: None,
        };
        a.save(overrides.clone()).unwrap();
        assert_eq!(current, defaults);
        assert_eq!(effective_options(&defaults, &a.load().unwrap()), overrides);
        assert_eq!(effective_options(&defaults, &b.load().unwrap()), defaults);
        a.save(SessionOptions::default()).unwrap();
        let next = effective_options(&defaults, &a.load().unwrap());
        assert_eq!(next, defaults);
        let catalog = json!({"models":["global","override"],"modelMetadata":{
            "global":{"supportedReasoningEfforts":[{"reasoningEffort":"low"}]},
            "override":{"supportedReasoningEfforts":[{"reasoningEffort":"high"}]}
        }});
        validate_options(&catalog, &next).unwrap();
        assert!(
            validate_options(
                &catalog,
                &SessionOptions {
                    model: "global".into(),
                    reasoning_effort: "high".into(),
                    reset_request: None,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn metadata_effort_shapes_are_validated() {
        for key in [
            "supportedReasoningEfforts",
            "supported_reasoning_levels",
            "supportedReasoningLevels",
        ] {
            for levels in [
                json!(["low"]),
                json!([{"effort":"low"}]),
                json!([{"reasoningEffort":"low"}]),
            ] {
                let mut catalog = json!({"models":["m"],"modelMetadata":{"m":{}}});
                catalog["modelMetadata"]["m"][key] = levels;
                validate_options(
                    &catalog,
                    &SessionOptions {
                        model: "m".into(),
                        reasoning_effort: "low".into(),
                        reset_request: None,
                    },
                )
                .unwrap();
                assert!(
                    validate_options(
                        &catalog,
                        &SessionOptions {
                            model: "m".into(),
                            reasoning_effort: "high".into(),
                            reset_request: None,
                        }
                    )
                    .is_err()
                );
            }
        }
    }

    #[test]
    fn session_options_are_scoped_and_survive_reload() {
        let temp = tempfile::tempdir().unwrap();
        let a = SessionOptionsStore {
            path: temp.path().join("options.json"),
            peer: "a".into(),
        };
        let b = SessionOptionsStore {
            peer: "b".into(),
            ..a.clone()
        };
        a.save(SessionOptions {
            model: "m".into(),
            reasoning_effort: "low".into(),
            reset_request: None,
        })
        .unwrap();
        assert_eq!(b.load().unwrap(), SessionOptions::default());
        b.save(SessionOptions {
            model: "n".into(),
            reasoning_effort: "high".into(),
            reset_request: None,
        })
        .unwrap();
        assert_eq!(a.load().unwrap().reasoning_effort, "low");
        assert_eq!(b.load().unwrap().model, "n");
    }

    #[test]
    fn invalid_models_and_unsupported_efforts_are_rejected() {
        let catalog = json!({"models":["m"],"modelMetadata":{"m":{"supportedReasoningEfforts":[{"reasoningEffort":"low"},{"reasoningEffort":"high"}]}}});
        let mut options = SessionOptions {
            model: "m".into(),
            reasoning_effort: "low".into(),
            reset_request: None,
        };
        validate_options(&catalog, &options).unwrap();
        options.reasoning_effort = "xhigh".into();
        assert!(validate_options(&catalog, &options).is_err());
        options.model = "unknown".into();
        assert!(validate_options(&catalog, &options).is_err());
    }

    #[test]
    fn remote_change_and_reset_use_connection_defaults_not_previous_turn() {
        let catalog = json!({"models":["m","n"],"modelMetadata":{
            "m":{"supportedReasoningEfforts":[{"reasoningEffort":"high"}]},
            "n":{"supportedReasoningEfforts":[{"reasoningEffort":"low"}]}
        }});
        let defaults = SessionOptions {
            model: "m".into(),
            reasoning_effort: "high".into(),
            reset_request: None,
        };
        let args = json!({"model":"n","reasoningEffort":"low"});
        let next = updated_options(
            &catalog,
            &defaults,
            &SessionOptions::default(),
            args.as_object().unwrap(),
        )
        .unwrap();
        assert_eq!(next.model, "n");
        let reset = json!({"model":"","reasoningEffort":""});
        let reset =
            updated_options(&catalog, &defaults, &next, reset.as_object().unwrap()).unwrap();
        assert_eq!(reset.with_defaults(&defaults), defaults);
        let invalid = json!({"reasoningEffort":"high"});
        assert!(updated_options(&catalog, &defaults, &next, invalid.as_object().unwrap()).is_err());
        assert_eq!(next.reasoning_effort, "low");
    }

    #[test]
    fn inherited_effort_uses_model_default_when_global_effort_is_unset() {
        let defaults = SessionOptions {
            model: "m".into(),
            reasoning_effort: String::new(),
            reset_request: None,
        };
        let catalog = json!({"model":"m","models":["m"],"modelMetadata":{
            "m":{"defaultReasoningEffort":"adaptive","supportedReasoningEfforts":[{"reasoningEffort":"adaptive"}]}
        }});
        let inherited = SessionOptions::default().resolve(&defaults, &catalog);
        assert_eq!(inherited.reasoning_effort, "adaptive");
        validate_options(&catalog, &inherited).unwrap();
    }
}
