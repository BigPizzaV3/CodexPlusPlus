use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use codex_plus_core::launcher::{
    CodexLaunch, LaunchHooks, LaunchOptions, ProcessWaitStrategy, launch_and_inject_with_hooks,
};
use codex_plus_core::models::{DeleteResult, DeleteStatus, ExportResult, ExportStatus, SessionRef};
use codex_plus_core::routes::{
    BridgeContext, BridgeDataService, BridgeRuntimeService, BridgeSettingsService,
    CoreRuntimeService, evaluate_extension_cdp, handle_bridge_request,
};
use codex_plus_core::agent_pet;
use codex_plus_core::settings::{AgentPetSettings, BackendSettings};
use codex_plus_core::status::StatusStore;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};

#[tokio::test]
async fn bridge_routes_cover_all_current_paths() {
    let ctx = test_context();

    let cases = [
        ("/settings/get", json!({})),
        ("/settings/set", json!({"providerSyncEnabled": true})),
        ("/settings/catalog", json!({})),
        ("/user-scripts/list", json!({})),
        ("/user-scripts/set-enabled", json!({"enabled": false})),
        (
            "/user-scripts/set-script-enabled",
            json!({"key": "user:a.js", "enabled": false}),
        ),
        ("/user-scripts/delete", json!({"key": "user:a.js"})),
        ("/user-scripts/reload", json!({})),
        ("/user-scripts/load", json!({})),
        ("/cdp/targets", json!({})),
        ("/cdp/evaluate", json!({"kind":"avatar-overlay","expression":"1"})),
        ("/devtools/open", json!({})),
        ("/manager/open", json!({})),
        ("/manager/open-transient", json!({})),
        ("/backend/status", json!({})),
        ("/agent-pet/image", json!({})),
        ("/codex-model-catalog", json!({})),
        ("/codex-config-model", json!({})),
        (
            "/llm-proxy",
            json!({"url": "http://example.com", "method": "POST"}),
        ),
        ("/ads", json!({})),
        ("/script-market/list", json!({})),
        (
            "/script-market/install",
            json!({"id": "codex-relay-balance"}),
        ),
        ("/stepwise/settings", json!({})),
        (
            "/stepwise/generate",
            json!({"request": {"lastUserMessage": "请继续", "lastAssistantMessage": "已完成"}}),
        ),
        ("/stepwise/test", json!({})),
        (
            "/delete",
            json!({"session_id": "s1", "title": "First", "host_id": "local"}),
        ),
        ("/undo", json!({"undo_token": "undo-1"})),
        (
            "/export-markdown",
            json!({"session_id": "s1", "title": "First"}),
        ),
        (
            "/thread-usage-history",
            json!({"session_id": "s1", "title": "First"}),
        ),
        (
            "/thread-usage-live",
            json!({"session_id": "s1", "title": "First"}),
        ),
        ("/archived-thread", json!({"title": "Archived"})),
    ];

    for (path, payload) in cases {
        let result = handle_bridge_request(ctx.clone(), path, payload).await;
        assert_ne!(
            result["message"], "Unknown bridge path",
            "{path} should be routed"
        );
    }
}

#[tokio::test]
async fn cdp_evaluate_rejects_empty_expression_without_touching_cdp() {
    let error = evaluate_extension_cdp(1, &json!({"kind": "avatar-overlay"}))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("expression"),
        "{error}"
    );
}

#[tokio::test]
async fn cdp_routes_are_wired_for_extensions() {
    let result = handle_bridge_request(test_context(), "/cdp/targets", json!({})).await;
    assert_eq!(result["status"], "failed");
    assert_ne!(result["message"], "Unknown bridge path");
    assert!(
        result["message"].as_str().unwrap_or("").contains("CDP targets"),
        "{}",
        result
    );
}

#[tokio::test]
async fn thread_usage_live_route_returns_recent_token_events() {
    // 内置状态栏走的就是这条路由；它必须与 /thread-usage-history 分开，
    // 因为状态栏按秒级轮询，拿不动全量历史。
    let result = handle_bridge_request(
        test_context(),
        "/thread-usage-live",
        json!({"session_id": "s1", "title": "First"}),
    )
    .await;
    assert_eq!(result["status"], "ok");
    assert_eq!(result["session_id"], "s1");
    assert_eq!(result["events"][0]["cached_input_tokens"], json!(900));
    assert_eq!(result["events"][0]["context_window"], json!(258400));
}

fn agent_pet_context(pet: AgentPetSettings) -> (BridgeContext, std::path::PathBuf) {
    let service = Arc::new(FakeSettings::with_settings(BackendSettings {
        agent_pet: pet,
        ..BackendSettings::default()
    }));
    let cache_dir = service.agent_pet_cache_dir();
    let ctx = BridgeContext::new(
        service,
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData),
    );
    (ctx, cache_dir)
}

#[tokio::test]
async fn agent_pet_image_is_disabled_until_enabled_for_codex() {
    for pet in [
        AgentPetSettings::default(),
        AgentPetSettings {
            enabled: true,
            agents: Vec::new(),
            ..AgentPetSettings::default()
        },
    ] {
        let (ctx, _) = agent_pet_context(pet);
        let result = handle_bridge_request(ctx, "/agent-pet/image", json!({})).await;
        assert_eq!(result, json!({"status": "disabled"}));
    }
}

#[tokio::test]
async fn agent_pet_image_returns_cached_data_uri_without_network() {
    let (ctx, cache_dir) = agent_pet_context(AgentPetSettings {
        enabled: true,
        image: "fox".into(),
        ..AgentPetSettings::default()
    });
    // 预先放好缓存：路由命中缓存就不会联网。
    let png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR";
    let url = agent_pet::builtin_image_url("fox", agent_pet::CDN_HOSTS[0]).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();
    std::fs::write(
        cache_dir.join(agent_pet::cache_file_name(&url, agent_pet::ImageKind::Png)),
        png,
    )
    .unwrap();
    // payload 被忽略，图片只按当前设置选择。
    let result = handle_bridge_request(ctx, "/agent-pet/image", json!({"image": "cat"})).await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["id"], "fox");
    assert!(
        result["image"]
            .as_str()
            .unwrap()
            .starts_with("data:image/png;base64,")
    );
}

#[tokio::test]
async fn agent_pet_image_reports_invalid_custom_url() {
    let (ctx, _) = agent_pet_context(AgentPetSettings {
        enabled: true,
        image: "custom".into(),
        custom_image_url: "http://example.com/pet.png".into(),
        ..AgentPetSettings::default()
    });
    let result = handle_bridge_request(ctx, "/agent-pet/image", json!({})).await;
    assert_eq!(result["status"], "failed");
    assert!(
        result["message"].as_str().unwrap().contains("https://"),
        "{result}"
    );
}

#[tokio::test]
async fn llm_proxy_rejects_local_addresses() {
    let ctx = test_context();

    let result = handle_bridge_request(
        ctx,
        "/llm-proxy",
        json!({
            "url": "https://127.0.0.1/v1/chat/completions",
            "method": "POST",
            "headers": {"Authorization": "Bearer sk-test"},
            "body": "{}"
        }),
    )
    .await;

    assert_eq!(result["status"], json!("failed"));
    assert_eq!(result["message"], json!("Base URL 不得指向本机或私有网络"));
}

#[tokio::test]
async fn llm_proxy_requires_https() {
    let ctx = test_context();

    let result = handle_bridge_request(
        ctx,
        "/llm-proxy",
        json!({
            "url": "http://api.example.com/v1/chat/completions",
            "method": "POST",
            "body": "{}"
        }),
    )
    .await;

    assert_eq!(result["status"], json!("failed"));
    assert_eq!(result["message"], json!("Base URL 必须使用 HTTPS"));
}

#[tokio::test]
async fn llm_proxy_rejects_non_post_methods() {
    let ctx = test_context();

    let result = handle_bridge_request(
        ctx,
        "/llm-proxy",
        json!({
            "url": "https://api.example.com/v1/chat/completions",
            "method": "GET",
            "body": "{}"
        }),
    )
    .await;

    assert_eq!(result["status"], json!("failed"));
    assert_eq!(result["message"], json!("LLM Bridge 仅支持 POST 请求"));
}

fn enhancement_settings_catalog_file() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("assets")
        .join("enhancement-settings.json");
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[tokio::test]
async fn settings_catalog_route_returns_the_single_catalog_file() {
    // 注入页的设置行只从这条路由取文案；返回的必须就是管理器 import 的那份 JSON。
    let result = handle_bridge_request(test_context(), "/settings/catalog", json!({})).await;
    assert_eq!(result, enhancement_settings_catalog_file());
    assert!(result["items"].as_array().is_some_and(|items| !items.is_empty()));
}

#[test]
fn settings_catalog_items_map_to_real_backend_settings() {
    // 清单里的 key 就是 /settings/set 写入的字段：拼错或字段被删时这里先失败，
    // 不会等到用户在 Codex 里点了开关才发现没存上。
    let catalog = enhancement_settings_catalog_file();
    let defaults = serde_json::to_value(BackendSettings::default()).unwrap();
    let groups: Vec<&str> = catalog["groups"]
        .as_array()
        .unwrap()
        .iter()
        .map(|group| group["id"].as_str().unwrap())
        .collect();
    let mut keys = std::collections::HashSet::new();
    for item in catalog["items"].as_array().unwrap() {
        let key = item["key"].as_str().unwrap();
        assert!(keys.insert(key), "{key} 在清单里重复");
        assert!(
            groups.contains(&item["group"].as_str().unwrap()),
            "{key} 引用了不存在的分组"
        );
        let value = defaults
            .get(key)
            .unwrap_or_else(|| panic!("{key} 不是 BackendSettings 字段"));
        match item["control"].as_str().unwrap() {
            "bool" => assert!(value.is_boolean(), "{key} 应为布尔字段"),
            "select" => {
                let options: Vec<&str> = item["options"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|option| option["value"].as_str().unwrap())
                    .collect();
                assert!(
                    options.contains(&value.as_str().unwrap()),
                    "{key} 的默认值不在选项里"
                );
            }
            other => panic!("{key} 使用了未知控件类型 {other}"),
        }
    }
}

#[tokio::test]
async fn settings_get_includes_runtime_codex_app_version() {
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_codex_app_version("26.601.21317")),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert_eq!(result["codexAppVersion"], json!("26.601.21317"));
    assert_eq!(result["activeRelaySessionProvider"], json!("custom"));
    assert_eq!(result["codexAppPluginMarketplaceUnlock"], json!(true));
    assert_eq!(result.get("codexAppForcePluginInstall"), None);
    assert_eq!(result["codexAppThreadIdBadge"], json!(false));
}

#[tokio::test]
async fn settings_get_exposes_active_openai_session_provider() {
    let settings = BackendSettings {
        relay_profiles: vec![codex_plus_core::settings::RelayProfile {
            config_contents: "model_provider = \"openai\"\n".to_string(),
            ..codex_plus_core::settings::RelayProfile::default()
        }],
        ..BackendSettings::default()
    };
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_settings(settings)),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert_eq!(result["activeRelaySessionProvider"], json!("openai"));
    assert_eq!(result["activeRelayCodexProvider"], json!("openai"));
}

#[tokio::test]
async fn settings_get_preserves_arbitrary_custom_provider_id() {
    let settings = BackendSettings {
        relay_profiles: vec![codex_plus_core::settings::RelayProfile {
            config_contents: "model_provider = \"ccswitch\"\n".to_string(),
            ..codex_plus_core::settings::RelayProfile::default()
        }],
        ..BackendSettings::default()
    };
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_settings(settings)),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert_eq!(result["activeRelaySessionProvider"], json!("custom"));
    assert_eq!(result["activeRelayCodexProvider"], json!("ccswitch"));
}

#[tokio::test]
async fn settings_get_does_not_expose_stepwise_api_key_to_renderer() {
    let settings = BackendSettings {
        codex_app_stepwise_api_key: "sk-secret".to_string(),
        ..BackendSettings::default()
    };
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_settings(settings)),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert!(result.get("codexAppStepwiseApiKey").is_none());
    assert_eq!(
        result["codexAppStepwiseApiKeyEnv"],
        json!("CODEX_STEPWISE_API_KEY")
    );
}

#[tokio::test]
async fn settings_set_notifies_external_settings_change_listener() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static NOTIFIED: AtomicUsize = AtomicUsize::new(0);
    codex_plus_core::settings::set_external_settings_change_listener(|| {
        NOTIFIED.fetch_add(1, Ordering::SeqCst);
    });
    let before = NOTIFIED.load(Ordering::SeqCst);

    let result = handle_bridge_request(
        test_context(),
        "/settings/set",
        json!({"providerSyncEnabled": true}),
    )
    .await;

    assert_eq!(result["providerSyncEnabled"], json!(true));
    // 同进程其它用例也可能写设置，只断言至少通知了一次。
    assert!(NOTIFIED.load(Ordering::SeqCst) > before, "Codex 写入设置后应通知管理器");
}

#[tokio::test]
async fn settings_set_does_not_persist_runtime_codex_app_version() {
    let settings = Arc::new(FakeSettings::with_codex_app_version("26.601.21317"));
    let ctx = BridgeContext::new(
        settings.clone(),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(
        ctx,
        "/settings/set",
        json!({
            "codexAppVersion": "1.2.3",
            "codexAppPluginMarketplaceUnlock": false
        }),
    )
    .await;

    assert_eq!(result["codexAppVersion"], json!("26.601.21317"));
    assert_eq!(result["codexAppPluginMarketplaceUnlock"], json!(false));

    let persisted = settings.settings.lock().unwrap().clone();
    let persisted_value = serde_json::to_value(persisted).unwrap();
    assert!(persisted_value.get("codexAppVersion").is_none());
}

#[tokio::test]
async fn bridge_context_core_with_app_dir_exposes_runtime_codex_app_version() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp
        .path()
        .join("OpenAI.Codex_26.601.21317.0_x64__abc")
        .join("app");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::write(app_dir.join("Codex.exe"), "").unwrap();
    let ctx = BridgeContext::core_with_data_and_app_dir(
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
        app_dir,
    );

    let result = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert_eq!(result["codexAppVersion"], json!("26.601.21317.0"));
}

#[tokio::test]
async fn stepwise_routes_use_settings_service() {
    let settings = BackendSettings {
        codex_app_stepwise_enabled: false,
        codex_app_stepwise_generation_mode: "manual".to_string(),
        codex_app_stepwise_direct_send: true,
        codex_app_stepwise_model: "settings-service-stepwise".to_string(),
        codex_app_stepwise_max_items: 3,
        ..BackendSettings::default()
    };
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_settings(settings)),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let public_settings = handle_bridge_request(ctx.clone(), "/stepwise/settings", json!({})).await;
    assert_eq!(public_settings["settings"]["enabled"], json!(false));
    assert_eq!(
        public_settings["settings"]["generationMode"],
        json!("manual")
    );
    assert_eq!(
        public_settings["settings"]["answerOutlineEnabled"],
        json!(false)
    );
    assert_eq!(public_settings["settings"]["directSend"], json!(true));
    assert_eq!(
        public_settings["settings"]["model"],
        json!("settings-service-stepwise")
    );
    assert_eq!(public_settings["settings"]["maxItems"], json!(3));
    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/stepwise/generate",
            json!({"request": {"lastUserMessage": "请继续", "lastAssistantMessage": "已完成"}}),
        )
        .await,
        json!({
            "status": "ok",
            "disabled": true,
            "protocol": "chat_completions",
            "items": []
        })
    );
    assert_eq!(
        handle_bridge_request(ctx, "/stepwise/test", json!({})).await,
        json!({
            "status": "ok",
            "disabled": true,
            "protocol": "chat_completions",
            "items": []
        })
    );
}

#[tokio::test]
async fn unknown_bridge_path_preserves_empty_session_id_shape() {
    let result = handle_bridge_request(
        test_context(),
        "/missing",
        json!({"session_id": "should-not-leak"}),
    )
    .await;

    assert_eq!(
        result,
        json!({
            "status": "failed",
            "session_id": "",
            "message": "Unknown bridge path"
        })
    );
}

#[tokio::test]
async fn settings_routes_use_settings_service() {
    let ctx = test_context();

    let updated = handle_bridge_request(
        ctx.clone(),
        "/settings/set",
        json!({"providerSyncEnabled": true, "codexAppSessionDelete": false, "codexAppNativeFastUnlock": true, "codexAppPetRealMouseLook": true, "codexAppActivityOverlayRight": true}),
    )
    .await;
    let loaded = handle_bridge_request(ctx, "/settings/get", json!({})).await;

    assert_eq!(updated["providerSyncEnabled"], true);
    assert_eq!(updated["codexAppSessionDelete"], false);
    assert_eq!(updated["codexAppNativeFastUnlock"], true);
    assert!(updated.get("codexAppServiceTierControls").is_none());
    assert_eq!(updated["codexAppPetRealMouseLook"], true);
    assert_eq!(updated["codexAppActivityOverlayRight"], true);
    assert_eq!(loaded, updated);
}

#[tokio::test]
async fn runtime_routes_keep_user_script_inventory_shape() {
    let ctx = test_context();

    let listed = handle_bridge_request(ctx.clone(), "/user-scripts/list", json!({})).await;
    let global = handle_bridge_request(
        ctx.clone(),
        "/user-scripts/set-enabled",
        json!({"enabled": false}),
    )
    .await;
    let script = handle_bridge_request(
        ctx.clone(),
        "/user-scripts/set-script-enabled",
        json!({"key": "user:a.js", "enabled": false}),
    )
    .await;
    let loaded = handle_bridge_request(ctx.clone(), "/user-scripts/load", json!({})).await;
    let reloaded = handle_bridge_request(ctx, "/user-scripts/reload", json!({})).await;

    assert_eq!(listed["enabled"], true);
    assert_eq!(listed["scripts"][0]["key"], "builtin:demo.js");
    assert_eq!(global["enabled"], false);
    assert_eq!(script["scripts"][1]["enabled"], false);
    assert_eq!(reloaded["reloaded"], true);
    assert_eq!(loaded["reloaded"], false);
    assert_eq!(reloaded["scripts"][0]["key"], "builtin:demo.js");
}

#[tokio::test]
async fn runtime_status_devtools_repair_and_ads_routes_are_dispatched() {
    let runtime = Arc::new(FakeRuntime::default());
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::default()),
        runtime.clone(),
        Arc::new(FakeData::default()),
    );

    assert_eq!(
        handle_bridge_request(ctx.clone(), "/devtools/open", json!({})).await,
        json!({"status": "ok", "opened": true})
    );
    let manager_payload = json!({"page": "settings", "section": "stepwise"});
    assert_eq!(
        handle_bridge_request(ctx.clone(), "/manager/open", manager_payload.clone()).await,
        json!({"status": "ok", "opened": "manager"})
    );
    assert_eq!(*runtime.manager_payload.lock().unwrap(), manager_payload);

    let transient_payload = json!({"page": "settings"});
    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/manager/open-transient",
            transient_payload.clone(),
        )
        .await,
        json!({"status": "ok", "opened": "manager-transient"})
    );
    assert_eq!(*runtime.manager_payload.lock().unwrap(), transient_payload);
    assert_eq!(
        handle_bridge_request(ctx.clone(), "/backend/status", json!({})).await,
        json!({"status": "ok", "message": "后端已连接", "version": codex_plus_core::version::VERSION, "hideOfficialUsageAlert": false})
    );
    assert_eq!(
        handle_bridge_request(ctx.clone(), "/ads", json!({})).await,
        json!({"version": 1, "ads": [{"id": "runtime-ad"}]})
    );
}

#[tokio::test]
async fn backend_status_includes_active_official_usage_alert_setting() {
    let settings = BackendSettings {
        active_relay_id: "official".to_string(),
        relay_profiles: vec![codex_plus_core::settings::RelayProfile {
            id: "official".to_string(),
            relay_mode: codex_plus_core::settings::RelayMode::Official,
            official_mix_api_key: true,
            hide_official_usage_alert: false,
            ..Default::default()
        }],
        ..Default::default()
    };
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::with_settings(settings)),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(ctx, "/backend/status", json!({})).await;

    assert_eq!(result["hideOfficialUsageAlert"], true);
}

#[tokio::test]
async fn data_routes_forward_payloads_to_data_service() {
    let ctx = test_context();

    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/delete",
            json!({"session_id": "s1", "title": "First", "host_id": "local"}),
        )
        .await["undo_token"],
        "undo-s1"
    );
    assert_eq!(
        handle_bridge_request(ctx.clone(), "/undo", json!({"undo_token": "undo-s1"})).await,
        json!({
            "status": "undone",
            "session_id": "s1",
            "message": "undone",
            "undo_token": "undo-s1",
            "backup_path": null
        })
    );
    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/export-markdown",
            json!({"session_id": "s1", "title": "First"}),
        )
        .await["filename"],
        "First.md"
    );
    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/thread-usage-history",
            json!({"session_id": "s1", "title": "First"}),
        )
        .await,
        json!({
            "status": "ok",
            "session_id": "s1",
            "history": [
                {
                    "source": "rollout-history",
                    "conversation_id": "local:s1",
                    "turn_id": "turn-1",
                    "observed_at": "2026-06-02T05:00:00Z",
                    "usage": {
                        "inputTokens": 1200,
                        "outputTokens": 120,
                        "totalTokens": 1320,
                        "cachedTokens": 900,
                        "cacheReadTokens": 0,
                        "cacheCreationTokens": 0,
                        "contextUsed": 1320,
                        "contextLimit": 258400,
                        "hasBreakdown": true
                    }
                }
            ]
        })
    );
    assert_eq!(
        handle_bridge_request(
            ctx.clone(),
            "/archived-thread",
            json!({"title": "Archived"})
        )
        .await,
        json!({"session_id": "archived-1", "title": "Archived"})
    );
}

#[tokio::test]
async fn bridge_context_core_with_data_uses_injected_data_service() {
    let ctx = BridgeContext::core_with_data(
        Arc::new(CoreRuntimeService::new(9229, StatusStore::default())),
        Arc::new(FakeData::default()),
    );

    let result = handle_bridge_request(
        ctx,
        "/delete",
        json!({"session_id": "s1", "title": "First", "host_id": "local"}),
    )
    .await;

    assert_eq!(result["status"], "local_deleted");
    assert_eq!(result["undo_token"], "undo-s1");
    assert_ne!(
        result["message"],
        "Delete service is not wired in core launcher hooks"
    );
}

#[tokio::test]
async fn user_script_manager_scans_and_persists_inventory_shape() {
    let temp = tempfile::tempdir().unwrap();
    let builtin_dir = temp.path().join("builtin");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&builtin_dir).unwrap();
    std::fs::write(builtin_dir.join("demo.js"), "window.demo = true;").unwrap();
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("a.js"), "window.a = true;").unwrap();
    std::fs::write(user_dir.join("ignore.txt"), "not js").unwrap();
    let manager = UserScriptManager::new(
        builtin_dir.clone(),
        user_dir.clone(),
        temp.path().join("user_scripts.json"),
    );

    let listed = manager.inventory().unwrap();
    manager.set_global_enabled(false).unwrap();
    let disabled = manager.inventory().unwrap();
    manager.set_script_enabled("user:a.js", false).unwrap();
    let script_disabled = manager.inventory().unwrap();
    manager.delete_user_script("user:a.js").unwrap();
    let deleted = manager.inventory().unwrap();

    assert_eq!(listed["enabled"], true);
    assert_eq!(
        listed["builtin_dir"].as_str().unwrap(),
        builtin_dir.to_string_lossy()
    );
    assert_eq!(
        listed["user_dir"].as_str().unwrap(),
        user_dir.to_string_lossy()
    );
    assert_eq!(listed["scripts"][0]["key"], "builtin:demo.js");
    assert_eq!(listed["scripts"][0]["source"], "builtin");
    assert_eq!(listed["scripts"][0]["enabled"], true);
    assert_eq!(listed["scripts"][0]["status"], "not_loaded");
    assert_eq!(listed["scripts"][0]["error"], "");
    assert_eq!(listed["scripts"][1]["key"], "user:a.js");
    assert_eq!(disabled["enabled"], false);
    assert_eq!(disabled["scripts"][0]["status"], "disabled");
    assert_eq!(script_disabled["scripts"][1]["enabled"], false);
    assert_eq!(deleted["scripts"].as_array().unwrap().len(), 1);
    assert!(!user_dir.join("a.js").exists());
    assert_eq!(
        serde_json::from_str::<Value>(
            &std::fs::read_to_string(temp.path().join("user_scripts.json")).unwrap()
        )
        .unwrap(),
        json!({"enabled": false, "scripts": {}})
    );
}

#[tokio::test]
async fn delete_refuses_remote_and_unknown_hosts_instead_of_local_fallback() {
    let ctx = test_context();
    for host in [
        json!(null),
        json!(""),
        json!("remote-ssh:fixture"),
        json!({"id":"local"}),
    ] {
        let result = handle_bridge_request(
            ctx.clone(),
            "/delete",
            json!({
                "session_id": "s1", "title": "Same ID on another host", "host_id": host,
            }),
        )
        .await;
        // FakeData 必定返回 local_deleted/undo-s1，失败说明没有进入本地服务。
        assert_eq!(result["status"], "failed");
        assert!(result.get("undo_token").is_none());
    }
    let conflict = handle_bridge_request(
        ctx.clone(),
        "/delete",
        json!({
            "session_id":"s1", "host_id":"local", "hostId":"remote-ssh:fixture",
        }),
    )
    .await;
    assert_eq!(conflict["status"], "failed");
    assert!(conflict.get("undo_token").is_none());
    let local = handle_bridge_request(
        ctx,
        "/delete",
        json!({
            "session_id":"s1", "title":"Local", "hostId":"local",
        }),
    )
    .await;
    assert_eq!(local["status"], "local_deleted");
    assert_eq!(local["undo_token"], "undo-s1");
}

#[tokio::test]
async fn user_script_inventory_merges_renderer_runtime_status() {
    let temp = tempfile::tempdir().unwrap();
    let builtin_dir = temp.path().join("builtin");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&builtin_dir).unwrap();
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("loaded.js"), "window.loaded = true;").unwrap();
    std::fs::write(user_dir.join("failed.js"), "throw new Error('boom');").unwrap();
    let manager =
        UserScriptManager::new(builtin_dir, user_dir, temp.path().join("user_scripts.json"));
    let runtime_status = json!({
        "user:loaded.js": {"status": "loaded", "error": ""},
        "user:failed.js": {"status": "failed", "error": "boom"}
    });

    let inventory = manager
        .inventory_with_runtime_status(Some(&runtime_status))
        .unwrap();
    let scripts = inventory["scripts"].as_array().unwrap();
    let loaded = scripts
        .iter()
        .find(|script| script["key"] == "user:loaded.js")
        .unwrap();
    let failed = scripts
        .iter()
        .find(|script| script["key"] == "user:failed.js")
        .unwrap();

    assert_eq!(loaded["status"], "loaded");
    assert_eq!(loaded["error"], "");
    assert_eq!(failed["status"], "failed");
    assert_eq!(failed["error"], "boom");
}

#[tokio::test]
async fn user_script_manager_deletes_market_script_metadata_and_rejects_builtin_delete() {
    let temp = tempfile::tempdir().unwrap();
    let builtin_dir = temp.path().join("builtin");
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&builtin_dir).unwrap();
    std::fs::write(builtin_dir.join("demo.js"), "window.demo = true;").unwrap();
    std::fs::create_dir_all(&user_dir).unwrap();
    let manager = UserScriptManager::new(
        builtin_dir,
        user_dir.clone(),
        temp.path().join("user_scripts.json"),
    );
    let script = codex_plus_core::script_market::MarketScript {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        description: String::new(),
        version: "1.0.0".to_string(),
        author: String::new(),
        tags: Vec::new(),
        homepage: "https://example.com/demo".to_string(),
        script_url: "https://example.com/demo.js".to_string(),
        requirements: Vec::new(),
        limitations: Vec::new(),
        icon: String::new(),
    };

    codex_plus_core::script_market::install_market_script_content(
        &manager,
        &script,
        b"window.demo = true;",
    )
    .unwrap();
    manager
        .set_script_enabled("user:market-demo.js", false)
        .unwrap();

    let error = manager.delete_user_script("builtin:demo.js").unwrap_err();
    assert!(error.to_string().contains("only user scripts"));
    manager.delete_user_script("user:market-demo.js").unwrap();

    assert!(!user_dir.join("market-demo.js").exists());
    assert!(
        manager.inventory().unwrap()["scripts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|script| script["market_id"] != "demo")
    );
    let saved = serde_json::from_str::<Value>(
        &std::fs::read_to_string(temp.path().join("user_scripts.json")).unwrap(),
    )
    .unwrap();
    assert!(saved.get("market").is_none());
    assert_eq!(saved["scripts"], json!({}));
}

#[tokio::test]
async fn core_runtime_reload_evaluates_enabled_user_bundle_and_status_is_ok() {
    let temp = tempfile::tempdir().unwrap();
    let builtin_dir = temp.path().join("builtin");
    std::fs::create_dir_all(&builtin_dir).unwrap();
    std::fs::write(builtin_dir.join("demo.js"), "window.demo = true;").unwrap();
    let manager = UserScriptManager::new(
        builtin_dir,
        temp.path().join("user"),
        temp.path().join("user_scripts.json"),
    );
    let evaluated = Arc::new(Mutex::new(Vec::<String>::new()));
    let runtime = CoreRuntimeService::new(9229, StatusStore::default())
        .with_user_scripts(manager)
        .with_user_script_evaluator({
            let evaluated = evaluated.clone();
            Arc::new(move |websocket_url, script| {
                evaluated
                    .lock()
                    .unwrap()
                    .push(format!("{websocket_url}:{script}"));
                Ok(json!({"status": "ok"}))
            })
        })
        .with_websocket_url("ws://page");
    let ctx = BridgeContext::new(
        Arc::new(FakeSettings::default()),
        Arc::new(runtime),
        Arc::new(FakeData::default()),
    );

    let status = handle_bridge_request(ctx.clone(), "/backend/status", json!({})).await;
    let loaded = handle_bridge_request(ctx.clone(), "/user-scripts/load", json!({})).await;
    let reloaded = handle_bridge_request(ctx, "/user-scripts/reload", json!({})).await;

    assert_eq!(
        status,
        json!({"status": "ok", "message": "后端已连接", "version": codex_plus_core::version::VERSION, "hideOfficialUsageAlert": false})
    );
    assert_eq!(reloaded["scripts"][0]["key"], "builtin:demo.js");
    assert_eq!(loaded["scripts"][0]["key"], "builtin:demo.js");
    let evaluated = evaluated.lock().unwrap();
    assert_eq!(evaluated.len(), 2);
    assert!(evaluated[0].starts_with("ws://page:"));
    assert!(evaluated[0].contains("window.demo = true;"));
    assert!(!evaluated[0].contains(".prepareReload();"));
    assert!(evaluated[1].contains(".prepareReload();"));
}

#[tokio::test]
async fn core_runtime_open_devtools_uses_inspector_url_opener() {
    let opened = Arc::new(Mutex::new(Vec::<String>::new()));
    let runtime = CoreRuntimeService::new(9229, StatusStore::default())
        .with_devtools_opener({
            let opened = opened.clone();
            Arc::new(move |url| {
                opened.lock().unwrap().push(url.to_string());
                Ok(())
            })
        })
        .with_devtools_target_id("page-1");
    let ctx = BridgeContext::core_with_data(Arc::new(runtime), Arc::new(FakeData::default()));

    let result = handle_bridge_request(ctx, "/devtools/open", json!({})).await;

    assert_eq!(result["status"], "ok");
    assert_eq!(result["target_id"], "page-1");
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["http://127.0.0.1:9229/devtools/inspector.html?ws=127.0.0.1:9229/devtools/page/page-1"]
    );
}

#[tokio::test]
async fn core_runtime_manager_route_attempts_to_open_manager_binary() {
    let ctx = BridgeContext::core(Arc::new(CoreRuntimeService::new(
        9229,
        StatusStore::default(),
    )));

    let result = handle_bridge_request(ctx, "/manager/open", json!({})).await;

    assert_ne!(result["message"], "管理工具启动未接入当前运行时");
}

#[tokio::test]
async fn bridge_backend_status_writes_diagnostic_log() {
    let temp = tempfile::tempdir().unwrap();
    let log_path = temp.path().join("codex-plus.log");
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path.clone()));
    let ctx = BridgeContext::core(Arc::new(CoreRuntimeService::new(
        9229,
        StatusStore::default(),
    )));

    let result = handle_bridge_request(ctx, "/backend/status", json!({})).await;

    assert_eq!(result["status"], "ok");
    let contents = std::fs::read_to_string(&log_path).unwrap();
    assert!(contents.contains("bridge.request"));
    assert!(contents.contains("bridge.backend_status_ok"));
    assert!(contents.contains("/backend/status"));
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(None);
}

#[test]
fn user_script_manager_tolerates_bad_config_fields_and_updates_atomically() {
    let temp = tempfile::tempdir().unwrap();
    let config_path = temp.path().join("user_scripts.json");
    std::fs::write(
        &config_path,
        r#"{"enabled":"not bool","scripts":{"user:a.js":false,"user:b.js":"bad"},"custom":true}"#,
    )
    .unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        config_path.clone(),
    );

    assert_eq!(manager.load_config().enabled, true);
    assert_eq!(manager.load_config().scripts.get("user:a.js"), Some(&false));
    assert!(!manager.load_config().scripts.contains_key("user:b.js"));

    manager.set_script_enabled("user:c.js", false).unwrap();
    let saved = serde_json::from_str::<Value>(&std::fs::read_to_string(config_path).unwrap())
        .expect("config should remain valid JSON");

    assert_eq!(saved["enabled"], true);
    assert_eq!(saved["scripts"]["user:a.js"], false);
    assert_eq!(saved["scripts"]["user:c.js"], false);
}

#[test]
fn script_market_manifest_filters_invalid_entries() {
    let raw = serde_json::json!({
        "version": 1,
        "updated_at": "2026-05-21T00:00:00Z",
        "scripts": [
            {
                "id": "demo",
                "name": "Demo",
                "description": "Useful demo",
                "version": "1.0.0",
                "author": "BigPizzaV3",
                "tags": ["ui", 42],
                "homepage": "https://example.com/demo",
                "script_url": "https://example.com/demo.js",
            },
            { "id": "", "name": "Bad", "version": "1", "script_url": "https://example.com/bad.js" },
            { "id": "missing-url", "name": "Bad", "version": "1" }
        ]
    });

    let manifest = codex_plus_core::script_market::parse_market_manifest(raw).unwrap();

    assert_eq!(manifest.version, 1);
    assert_eq!(manifest.updated_at.as_deref(), Some("2026-05-21T00:00:00Z"));
    assert_eq!(manifest.scripts.len(), 1);
    assert_eq!(manifest.scripts[0].id, "demo");
    assert_eq!(manifest.scripts[0].tags, vec!["ui"]);
}

#[test]
fn user_script_inventory_includes_market_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("market-demo.js"), "window.demo = true;").unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        user_dir,
        temp.path().join("user_scripts.json"),
    );

    manager
        .record_market_install(&codex_plus_core::script_market::MarketScript {
            id: "demo".to_string(),
            name: "Demo".to_string(),
            description: "Useful demo".to_string(),
            version: "1.0.0".to_string(),
            author: "BigPizzaV3".to_string(),
            tags: vec!["ui".to_string()],
            homepage: "https://example.com/demo".to_string(),
            script_url: "https://example.com/demo.js".to_string(),
            requirements: Vec::new(),
            limitations: Vec::new(),
            icon: String::new(),
        })
        .unwrap();

    let inventory = manager.inventory().unwrap();

    assert_eq!(inventory["scripts"][0]["key"], "user:market-demo.js");
    assert_eq!(inventory["scripts"][0]["market_id"], "demo");
    assert_eq!(inventory["scripts"][0]["version"], "1.0.0");
    assert_eq!(inventory["scripts"][0]["installed"], true);
    assert_eq!(
        inventory["scripts"][0]["source_url"],
        "https://example.com/demo.js"
    );
    assert_eq!(
        inventory["scripts"][0]["homepage"],
        "https://example.com/demo"
    );
}

#[test]
fn install_market_script_writes_file_and_records_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        temp.path().join("user"),
        temp.path().join("user_scripts.json"),
    );
    let script = codex_plus_core::script_market::MarketScript {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        description: String::new(),
        version: "1.0.0".to_string(),
        author: String::new(),
        tags: Vec::new(),
        homepage: "https://example.com/demo".to_string(),
        script_url: "https://example.com/demo.js".to_string(),
        requirements: Vec::new(),
        limitations: Vec::new(),
        icon: String::new(),
    };

    codex_plus_core::script_market::install_market_script_content(
        &manager,
        &script,
        b"window.demo = true;",
    )
    .unwrap();

    assert_eq!(
        std::fs::read_to_string(temp.path().join("user").join("market-demo.js")).unwrap(),
        "window.demo = true;"
    );
    let inventory = manager.inventory().unwrap();
    assert_eq!(inventory["scripts"][0]["market_id"], "demo");
}

#[test]
fn install_market_script_ignores_checksum_mismatch_and_replaces_existing_file() {
    let temp = tempfile::tempdir().unwrap();
    let user_dir = temp.path().join("user");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("market-demo.js"), "old").unwrap();
    let manager = UserScriptManager::new(
        temp.path().join("builtin"),
        user_dir.clone(),
        temp.path().join("user_scripts.json"),
    );
    let script = codex_plus_core::script_market::MarketScript {
        id: "demo".to_string(),
        name: "Demo".to_string(),
        description: String::new(),
        version: "1.0.0".to_string(),
        author: String::new(),
        tags: Vec::new(),
        homepage: String::new(),
        script_url: "https://example.com/demo.js".to_string(),
        requirements: Vec::new(),
        limitations: Vec::new(),
        icon: String::new(),
    };

    codex_plus_core::script_market::install_market_script_content(&manager, &script, b"new")
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(user_dir.join("market-demo.js")).unwrap(),
        "new"
    );
}

#[tokio::test]
async fn launch_lifecycle_uses_hook_supplied_bridge_context_for_injection() {
    let temp = tempfile::tempdir().unwrap();
    let app_dir = temp.path().join("Codex.app");
    std::fs::create_dir_all(&app_dir).unwrap();
    let events = Arc::new(Mutex::new(Vec::<String>::new()));
    let hooks = ContextHooks {
        events: events.clone(),
    };

    launch_and_inject_with_hooks(
        LaunchOptions {
            app_dir: Some(app_dir),
            debug_port: 9229,
            helper_port: 57321,
            status_store: StatusStore::new(temp.path().join("latest-status.json")),
            ..Default::default()
        },
        &hooks,
    )
    .await
    .unwrap();

    assert_eq!(
        *events.lock().unwrap(),
        vec![
            "bridge-context:9229",
            "inject-bridge:9229:57321",
            "watchdog:9229:57321",
            "status:running",
        ]
    );
}

fn test_context() -> BridgeContext {
    BridgeContext::new(
        Arc::new(FakeSettings::default()),
        Arc::new(FakeRuntime::default()),
        Arc::new(FakeData::default()),
    )
}

struct FakeSettings {
    settings: Mutex<BackendSettings>,
    codex_app_version: Mutex<String>,
    agent_pet_cache: tempfile::TempDir,
}

impl Default for FakeSettings {
    fn default() -> Self {
        Self {
            settings: Mutex::new(BackendSettings::default()),
            codex_app_version: Mutex::new(String::new()),
            agent_pet_cache: tempfile::TempDir::new().unwrap(),
        }
    }
}

impl FakeSettings {
    fn with_settings(settings: BackendSettings) -> Self {
        Self {
            settings: Mutex::new(settings),
            codex_app_version: Mutex::new(String::new()),
            agent_pet_cache: tempfile::TempDir::new().unwrap(),
        }
    }

    fn with_codex_app_version(version: &str) -> Self {
        Self {
            settings: Mutex::new(BackendSettings::default()),
            codex_app_version: Mutex::new(version.to_string()),
            agent_pet_cache: tempfile::TempDir::new().unwrap(),
        }
    }
}

#[async_trait]
impl BridgeSettingsService for FakeSettings {
    fn agent_pet_cache_dir(&self) -> std::path::PathBuf {
        self.agent_pet_cache.path().join("agent-pet-cache")
    }
    async fn get_settings(&self) -> anyhow::Result<BackendSettings> {
        Ok(self.settings.lock().unwrap().clone())
    }

    async fn set_settings(&self, payload: Value) -> anyhow::Result<BackendSettings> {
        let current = self.settings.lock().unwrap().clone();
        let mut raw = serde_json::to_value(current).unwrap();
        let raw = raw.as_object_mut().unwrap();
        if let Some(value) = payload.get("providerSyncEnabled").and_then(Value::as_bool) {
            raw.insert("providerSyncEnabled".to_string(), json!(value));
        }
        if let Some(value) = payload.get("enhancementsEnabled").and_then(Value::as_bool) {
            raw.insert("enhancementsEnabled".to_string(), json!(value));
        }
        for key in [
            "codexAppPluginMarketplaceUnlock",
            "codexAppModelWhitelistUnlock",
            "codexAppSessionDelete",
            "codexAppMarkdownExport",
            "codexAppThreadIdBadge",
            "codexAppConversationView",
            "codexAppThreadScrollRestore",
            "codexAppNativeMenuPlacement",
            "codexAppNativeFastUnlock",
            "codexAppPetRealMouseLook",
            "codexAppActivityOverlayRight",
        ] {
            if let Some(value) = payload.get(key).and_then(Value::as_bool) {
                raw.insert(key.to_string(), json!(value));
            }
        }
        if let Some(value) = payload.get("relayBaseUrl").and_then(Value::as_str) {
            raw.insert("relayBaseUrl".to_string(), json!(value));
        }
        if let Some(value) = payload.get("relayApiKey").and_then(Value::as_str) {
            raw.insert("relayApiKey".to_string(), json!(value));
        }
        let updated: BackendSettings = serde_json::from_value(Value::Object(raw.clone())).unwrap();
        *self.settings.lock().unwrap() = updated.clone();
        Ok(updated)
    }

    async fn codex_app_version(&self) -> anyhow::Result<String> {
        Ok(self.codex_app_version.lock().unwrap().clone())
    }
}

struct FakeRuntime {
    enabled: Mutex<bool>,
    script_enabled: Mutex<bool>,
    manager_payload: Mutex<Value>,
}

impl Default for FakeRuntime {
    fn default() -> Self {
        Self {
            enabled: Mutex::new(true),
            script_enabled: Mutex::new(true),
            manager_payload: Mutex::new(json!({})),
        }
    }
}

#[async_trait]
impl BridgeRuntimeService for FakeRuntime {
    async fn user_script_inventory(&self) -> anyhow::Result<Value> {
        Ok(self.inventory(false))
    }

    async fn set_user_scripts_enabled(&self, enabled: bool) -> anyhow::Result<Value> {
        *self.enabled.lock().unwrap() = enabled;
        Ok(self.inventory(false))
    }

    async fn set_user_script_enabled(&self, key: String, enabled: bool) -> anyhow::Result<Value> {
        assert_eq!(key, "user:a.js");
        *self.script_enabled.lock().unwrap() = enabled;
        Ok(self.inventory(false))
    }

    async fn delete_user_script(&self, key: String) -> anyhow::Result<Value> {
        assert_eq!(key, "user:a.js");
        *self.script_enabled.lock().unwrap() = false;
        Ok(self.inventory(false))
    }

    async fn reload_user_scripts(&self) -> anyhow::Result<Value> {
        Ok(self.inventory(true))
    }

    async fn open_devtools(&self) -> anyhow::Result<Value> {
        Ok(json!({"status": "ok", "opened": true}))
    }

    async fn open_manager(&self, payload: Value) -> anyhow::Result<Value> {
        *self.manager_payload.lock().unwrap() = payload;
        Ok(json!({"status": "ok", "opened": "manager"}))
    }

    async fn open_transient_manager(&self, payload: Value) -> anyhow::Result<Value> {
        *self.manager_payload.lock().unwrap() = payload;
        Ok(json!({"status": "ok", "opened": "manager-transient"}))
    }

    async fn backend_status(&self) -> anyhow::Result<Value> {
        Ok(
            json!({"status": "ok", "message": "后端已连接", "version": codex_plus_core::version::VERSION}),
        )
    }

    async fn codex_model_catalog(&self) -> anyhow::Result<Value> {
        Ok(json!({
            "status": "ok",
            "model": "qwen3-coder",
            "default_model": "qwen3-coder",
            "model_provider": "relay",
            "provider_name": "Relay",
            "models": ["qwen3-coder"],
            "sources": []
        }))
    }

    async fn ads(&self) -> anyhow::Result<Value> {
        Ok(json!({"version": 1, "ads": [{"id": "runtime-ad"}]}))
    }
}

impl FakeRuntime {
    fn inventory(&self, reloaded: bool) -> Value {
        json!({
            "enabled": *self.enabled.lock().unwrap(),
            "reloaded": reloaded,
            "scripts": [
                {"key": "builtin:demo.js", "name": "demo.js", "enabled": true},
                {"key": "user:a.js", "name": "a.js", "enabled": *self.script_enabled.lock().unwrap()}
            ]
        })
    }
}

struct FakeData;

impl Default for FakeData {
    fn default() -> Self {
        Self
    }
}

#[async_trait]
impl BridgeDataService for FakeData {
    async fn delete(&self, session: SessionRef) -> anyhow::Result<DeleteResult> {
        Ok(DeleteResult {
            status: DeleteStatus::LocalDeleted,
            session_id: session.session_id.clone(),
            message: format!("deleted {}", session.title),
            undo_token: Some(format!("undo-{}", session.session_id)),
            backup_path: None,
        })
    }

    async fn undo(&self, undo_token: String) -> anyhow::Result<DeleteResult> {
        Ok(DeleteResult {
            status: DeleteStatus::Undone,
            session_id: "s1".to_string(),
            message: "undone".to_string(),
            undo_token: Some(undo_token),
            backup_path: None,
        })
    }

    async fn export_markdown(&self, session: SessionRef) -> anyhow::Result<ExportResult> {
        Ok(ExportResult {
            status: ExportStatus::Exported,
            session_id: session.session_id,
            message: "exported".to_string(),
            filename: Some("First.md".to_string()),
            markdown: Some("# First\n".to_string()),
        })
    }

    async fn thread_usage_live(&self, session: SessionRef) -> anyhow::Result<Value> {
        Ok(json!({
            "status": "ok",
            "session_id": session.session_id,
            "events": [
                {
                    "t": "2026-06-02T05:00:00Z",
                    "input_tokens": 1200,
                    "cached_input_tokens": 900,
                    "output_tokens": 120,
                    "reasoning_output_tokens": 40,
                    "total_tokens": 1320,
                    "context_window": 258400,
                    "turn_output_tokens": 120
                }
            ]
        }))
    }

    async fn thread_usage_history(&self, session: SessionRef) -> anyhow::Result<Value> {
        Ok(json!({
            "status": "ok",
            "session_id": session.session_id,
            "history": [
                {
                    "source": "rollout-history",
                    "conversation_id": "local:s1",
                    "turn_id": "turn-1",
                    "observed_at": "2026-06-02T05:00:00Z",
                    "usage": {
                        "inputTokens": 1200,
                        "outputTokens": 120,
                        "totalTokens": 1320,
                        "cachedTokens": 900,
                        "cacheReadTokens": 0,
                        "cacheCreationTokens": 0,
                        "contextUsed": 1320,
                        "contextLimit": 258400,
                        "hasBreakdown": true
                    }
                }
            ]
        }))
    }

    async fn find_archived_thread_by_title(
        &self,
        title: String,
    ) -> anyhow::Result<Option<SessionRef>> {
        Ok(Some(SessionRef {
            session_id: "archived-1".to_string(),
            title,
            host_id: None,
        }))
    }
}

#[derive(Clone)]
struct ContextHooks {
    events: Arc<Mutex<Vec<String>>>,
}

impl ContextHooks {
    fn event(&self, event: impl Into<String>) {
        self.events.lock().unwrap().push(event.into());
    }
}

#[async_trait(?Send)]
impl LaunchHooks for ContextHooks {
    fn resolve_app_dir(
        &self,
        app_dir: Option<&std::path::Path>,
        _settings: &BackendSettings,
    ) -> anyhow::Result<std::path::PathBuf> {
        app_dir
            .map(std::path::Path::to_path_buf)
            .ok_or_else(|| anyhow::anyhow!("missing app dir"))
    }

    fn select_debug_port(&self, requested: u16) -> u16 {
        requested
    }

    fn select_helper_port(&self, requested: u16) -> u16 {
        requested
    }

    async fn load_settings(&self) -> anyhow::Result<BackendSettings> {
        Ok(BackendSettings::default())
    }

    async fn run_provider_sync(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn run_remote_control_session_recovery(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn start_helper(&self, _helper_port: u16) -> anyhow::Result<()> {
        Ok(())
    }

    async fn launch_codex(
        &self,
        _app_dir: &std::path::Path,
        _debug_port: u16,
        _settings: &BackendSettings,
        _extra_args: &[String],
        _instance: &codex_plus_core::launcher::LaunchInstance,
    ) -> anyhow::Result<CodexLaunch> {
        Ok(CodexLaunch::Process {
            command: vec!["codex".to_string()],
            wait_strategy: ProcessWaitStrategy::TrackedChild,
            macos_cleanup_policy: None,
        })
    }

    async fn bridge_context(
        &self,
        debug_port: u16,
        _app_dir: &std::path::Path,
    ) -> anyhow::Result<Option<BridgeContext>> {
        self.event(format!("bridge-context:{debug_port}"));
        Ok(Some(test_context()))
    }

    async fn inject(&self, _debug_port: u16, _helper_port: u16) -> anyhow::Result<()> {
        anyhow::bail!("legacy inject should not run when bridge context is supplied")
    }

    async fn inject_bridge(
        &self,
        debug_port: u16,
        helper_port: u16,
        _ctx: BridgeContext,
    ) -> anyhow::Result<()> {
        self.event(format!("inject-bridge:{debug_port}:{helper_port}"));
        Ok(())
    }

    async fn start_bridge_watchdog(&self, debug_port: u16, helper_port: u16) -> anyhow::Result<()> {
        self.event(format!("watchdog:{debug_port}:{helper_port}"));
        Ok(())
    }

    async fn write_status(&self, status: &str) {
        self.event(format!("status:{status}"));
    }

    async fn wait_for_codex_exit(
        &self,
        _launch: &CodexLaunch,
        _debug_port: u16,
    ) -> anyhow::Result<()> {
        Ok(())
    }

    async fn shutdown_helper(&self, _helper_port: u16) {}

    async fn terminate_codex(&self, _launch: &CodexLaunch) {}
}

#[tokio::test]
async fn retired_remote_routes_are_unavailable() {
    let ctx = test_context();
    for path in [
        "/zed-remote/status",
        "/zed-remote/resolve-host",
        "/zed-remote/fallback-request",
        "/zed-remote/open",
        "/zed-remote/projects",
        "/zed-remote/remember-project",
        "/zed-remote/forget-project",
        "/upstream-worktree/status",
        "/upstream-worktree/defaults",
        "/upstream-worktree/prepare",
        "/upstream-worktree/create",
    ] {
        let result = handle_bridge_request(ctx.clone(), path, json!({})).await;
        assert_eq!(result["status"], "failed", "{path}");
        assert!(
            result["message"]
                .as_str()
                .unwrap()
                .contains("Unknown bridge path"),
            "{path}: {result}"
        );
    }
}
