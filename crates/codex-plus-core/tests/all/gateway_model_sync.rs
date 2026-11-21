//! 供应商详情「从上游同步」分两步（#2785）：先按上游列表算预览（只读），用户勾选后只删所选、只加所选。
//! 被删模型的价格、上下文窗口和思考等级一并清掉；默认模型被删时换成列表第一个；
//! 上游空列表不改；比对忽略大小写和 `[1M]` 窗口后缀。

use codex_plus_core::gateway::apply::{ProviderInput, create_provider};
use codex_plus_core::gateway::model_sync::{
    apply_upstream_sync, check_provider, preview_provider_models, preview_upstream_sync,
    sync_upstream_models,
};
use codex_plus_core::gateway::route::relay_models;
use codex_plus_core::gateway::store::{GatewayStore, ProviderPrice};
use codex_plus_core::gateway::subscriptions::models_or_fallback;
use codex_plus_core::relay_config::relay_profile_model;
use codex_plus_core::settings::RelayProtocol;

fn strings(items: &[&str]) -> Vec<String> {
    items.iter().map(|item| item.to_string()).collect()
}

fn price(input: f64) -> ProviderPrice {
    ProviderPrice {
        currency: "USD".into(),
        input,
        cached_input: 0.0,
        output: input,
    }
}

/// 建一个供应商：默认模型 `default`，列表 `models`；每个模型都填上价格、窗口和思考等级。
fn setup(name: &str, default: &str, models: &[&str]) -> (tempfile::TempDir, GatewayStore, String) {
    let temp = tempfile::tempdir().unwrap();
    let store = GatewayStore::at(temp.path().join("gateway.json"));
    let input = ProviderInput {
        name: name.into(),
        base_url: "https://api.sync.test/v1".into(),
        protocol: RelayProtocol::ChatCompletions,
        keys: vec!["sk-sync-0123456789abcdefghij-SYNC".into()],
        model: default.into(),
        model_list: models.join("\n"),
        ..ProviderInput::default()
    };
    let (_, id) = create_provider(&store, &input, "custom", None).unwrap();
    store
        .update(|gateway| {
            let provider = gateway
                .providers
                .iter_mut()
                .find(|provider| provider.relay.id == id)
                .unwrap();
            let mut metadata = serde_json::Map::new();
            for (index, model) in models.iter().enumerate() {
                provider
                    .model_prices
                    .insert(model.to_string(), price(index as f64 + 1.0));
                provider.model_windows.insert(model.to_string(), 200_000);
                metadata.insert(
                    model.to_string(),
                    serde_json::json!({ "default_reasoning_level": "high" }),
                );
            }
            provider.relay.model_metadata =
                serde_json::to_string_pretty(&serde_json::Value::Object(metadata)).unwrap();
            provider.upstream_new = strings(&["hint-new"]);
            provider.upstream_gone = strings(&["hint-gone"]);
            Ok(())
        })
        .unwrap();
    (temp, store, id)
}

fn stored(store: &GatewayStore) -> String {
    std::fs::read_to_string(&store.path).unwrap()
}

fn models(store: &GatewayStore, id: &str) -> Vec<String> {
    relay_models(&store.load().unwrap().provider(id).unwrap().relay)
}

#[test]
fn preview_only_computes_and_never_writes() {
    let (_temp, store, id) = setup("Sync Preview 2785", "b", &["a", "b", "old", "long[1M]"]);
    let before = stored(&store);

    let preview = preview_upstream_sync(
        &store,
        &id,
        &strings(&["A", "b", "long", "new-1", " ", "NEW-1[1M]", "new-2"]),
    )
    .unwrap();
    // 默认模型在最前（列表里的 [1M] 后缀读出来不带）；上游的空名去掉，只差大小写 / 后缀的算同一个。
    assert_eq!(preview.current, strings(&["b", "a", "old", "long"]));
    assert_eq!(preview.default_model, "b");
    assert_eq!(preview.added, strings(&["new-1", "new-2"]));
    assert_eq!(preview.removed, strings(&["old"]));
    assert_eq!(preview.upstream.len(), 6);
    // 只读：文件一个字节都不变（包括上游检查的提示）。
    assert_eq!(stored(&store), before);
}

#[test]
fn preview_with_an_empty_upstream_fails_without_writing() {
    let (_temp, store, id) = setup("Sync Empty 2785", "a", &["a", "b"]);
    let before = stored(&store);
    let error = preview_upstream_sync(&store, &id, &strings(&["", "  "])).unwrap_err();
    assert!(format!("{error:#}").contains("列表保持不变"), "{error:#}");
    assert!(sync_upstream_models(&store, &id, &[]).is_err());
    assert_eq!(stored(&store), before);
    assert_eq!(models(&store, &id), strings(&["a", "b"]));
}

#[test]
fn applying_only_removes_and_adds_what_was_picked() {
    let (_temp, store, id) = setup(
        "Sync Apply 2785",
        "b",
        &["a", "b", "old", "older", "long[1M]"],
    );
    let preview = preview_upstream_sync(
        &store,
        &id,
        &strings(&["a", "b", "long", "new-1", "new-2", "new-3"]),
    )
    .unwrap();
    assert_eq!(preview.removed, strings(&["old", "older"]));
    assert_eq!(preview.added, strings(&["new-1", "new-2", "new-3"]));

    // 只勾了移除 old（older 取消勾选留着），只勾了新增 new-2（大小写不同、带后缀也认）。
    let (saved, changes) = apply_upstream_sync(
        &store,
        &id,
        &strings(&["OLD"]),
        &strings(&["new-2[1M]", " ", "b"]),
    )
    .unwrap();
    assert_eq!(changes.removed, strings(&["old"]));
    // 已在列表里的 b 不重复加。
    assert_eq!(changes.added, strings(&["new-2[1M]"]));
    let provider = saved.provider(&id).unwrap();
    assert_eq!(
        relay_models(&provider.relay),
        strings(&["b", "a", "older", "long", "new-2"])
    );
    // 没勾的上游新增没加进来。
    assert!(!provider.relay.model_list.contains("new-1"));
    assert!(!provider.relay.model_list.contains("new-3"));
    assert_eq!(relay_profile_model(&provider.relay), "b");
    // 被删的 old 的价格、窗口、思考等级一并清掉；留着的 older 不动。
    assert!(!provider.model_prices.contains_key("old"));
    assert!(!provider.model_windows.contains_key("old"));
    assert!(!provider.relay.model_metadata.contains("\"old\""));
    assert!(provider.model_prices.contains_key("older"));
    assert!(provider.model_windows.contains_key("older"));
    assert!(provider.relay.model_metadata.contains("\"older\""));
    // 用户看过了，清掉上游检查的提示。
    assert!(provider.upstream_new.is_empty() && provider.upstream_gone.is_empty());
    // 写盘了，重新读一样。
    assert_eq!(
        models(&store, &id),
        strings(&["b", "a", "older", "long", "new-2"])
    );
}

#[test]
fn applying_nothing_keeps_the_list() {
    let (_temp, store, id) = setup("Sync Nothing 2785", "a", &["a", "old"]);
    let (_, changes) = apply_upstream_sync(&store, &id, &[], &[]).unwrap();
    assert!(changes.added.is_empty() && changes.removed.is_empty());
    assert_eq!(models(&store, &id), strings(&["a", "old"]));
}

#[test]
fn removing_the_default_falls_back_to_the_first_remaining_model() {
    let (_temp, store, id) = setup(
        "Sync Default 2785",
        "retired",
        &["retired", "keep", "other"],
    );
    let (saved, _) =
        apply_upstream_sync(&store, &id, &strings(&["retired"]), &strings(&["fresh"])).unwrap();
    let relay = &saved.provider(&id).unwrap().relay;
    assert_eq!(relay_profile_model(relay), "keep");
    // 重新读盘后旧默认模型不能被配置快照改回来、也不能并回列表。
    assert_eq!(models(&store, &id), strings(&["keep", "other", "fresh"]));
}

#[test]
fn removing_every_model_is_refused() {
    let (_temp, store, id) = setup("Sync All Gone 2785", "a", &["a", "b"]);
    let before = stored(&store);
    assert!(apply_upstream_sync(&store, &id, &strings(&["a", "b"]), &[]).is_err());
    assert_eq!(stored(&store), before);
}

#[test]
fn window_suffix_is_ignored_when_removing() {
    // 列表里写成 big[1M] 的模型（读出来是 big）和上游的 BIG 算同一个；移除时按后缀名记的窗口一起清掉。
    let (_temp, store, id) = setup("Sync Suffix 2785", "a", &["a", "big[1M]"]);
    let preview = preview_upstream_sync(&store, &id, &strings(&["a", "BIG"])).unwrap();
    assert!(
        preview.added.is_empty() && preview.removed.is_empty(),
        "{preview:?}"
    );
    let (saved, changes) = apply_upstream_sync(&store, &id, &strings(&["big"]), &[]).unwrap();
    assert_eq!(changes.removed, strings(&["big"]));
    let provider = saved.provider(&id).unwrap();
    assert!(!provider.model_windows.contains_key("big[1M]"));
    assert_eq!(models(&store, &id), strings(&["a"]));
}

#[test]
fn full_sync_still_removes_gone_and_appends_every_new_model() {
    // 不经用户挑选的多删少补（网页导入链接加进池后自动拉模型用）保持 2ecaceac 的语义。
    let (_temp, store, id) = setup("Sync Full 2785", "b", &["a", "b", "old"]);
    let (_, diff) =
        sync_upstream_models(&store, &id, &strings(&["a", "B", "x", "y", "X[1M]"])).unwrap();
    assert_eq!(diff.removed, strings(&["old"]));
    assert_eq!(diff.added, strings(&["x", "y"]));
    assert_eq!(models(&store, &id), strings(&["b", "a", "x", "y"]));
}

// ---- 订阅账号从上游同步（#3221）--------------------------------------------

/// 把供应商改成订阅账号：订阅供应商不存 Base URL（地址请求前才指到本机适配入口）。
fn make_subscription(store: &GatewayStore, id: &str, kind: &str, credentials: serde_json::Value) {
    store
        .update(|gateway| {
            let provider = gateway
                .providers
                .iter_mut()
                .find(|provider| provider.relay.id == id)
                .unwrap();
            provider.relay.base_url = String::new();
            provider.relay.upstream_base_url = String::new();
            provider.relay.auth_contents = serde_json::json!({
                codex_plus_core::gateway::accounts::ACCOUNT_KIND_FIELD: kind,
                "account": "user@example.com",
                "credentials": credentials,
            })
            .to_string();
            Ok(())
        })
        .unwrap();
}

fn diagnostic_log() -> String {
    std::fs::read_to_string(codex_plus_core::diagnostic_log::diagnostic_log_path())
        .unwrap_or_default()
}

#[tokio::test]
async fn subscription_without_model_list_api_reports_it_instead_of_base_url() {
    let _state = crate::common::IsolatedTestState::new();
    let (_temp, store, id) = setup("Sub Factory 3221", "a", &["a", "b"]);
    // Factory 用 API Key 登录：不用刷新、不联网；它没有可查询的模型列表接口。
    make_subscription(&store, &id, "sub:factory", serde_json::json!({ "api_key": "fk-3221" }));
    let before = models(&store, &id);

    let error = preview_provider_models(&store, &id).await.unwrap_err();
    let text = format!("{error:#}");
    assert!(!text.contains("Base URL"), "{text}");
    assert!(text.contains("Factory 没有可查询的模型列表接口"), "{text}");
    assert_eq!(models(&store, &id), before);

    let log = diagnostic_log();
    assert!(log.contains("gateway.subscription_models_failed"), "{log}");
    assert!(log.contains(&id), "{log}");
    assert!(log.contains("sub:factory"), "{log}");
}

#[tokio::test]
async fn antigravity_sync_surfaces_the_credential_error() {
    let _state = crate::common::IsolatedTestState::new();
    let (_temp, store, id) = setup("Sub Antigravity 3221", "a", &["a", "b"]);
    // 凭据里没有 token：按订阅自己的刷新流程报「重新登录」，而不是去拉空的 Base URL。
    make_subscription(&store, &id, "sub:antigravity", serde_json::json!({}));
    let before = models(&store, &id);

    let error = preview_provider_models(&store, &id).await.unwrap_err();
    let text = format!("{error:#}");
    assert!(!text.contains("Base URL"), "{text}");
    assert!(text.contains("重新登录"), "{text}");

    // 后台检查同样走订阅自己的接口：失败只记检查时间，不改列表和提示。
    let error = check_provider(&store, &id).await.unwrap_err();
    assert!(!format!("{error:#}").contains("Base URL"), "{error:#}");
    let gateway = store.load().unwrap();
    let provider = gateway.provider(&id).unwrap();
    assert!(provider.models_checked_at_ms > 0);
    assert_eq!(provider.upstream_new, strings(&["hint-new"]));
    assert_eq!(models(&store, &id), before);
    assert!(diagnostic_log().contains("sub:antigravity"));
}

#[tokio::test]
async fn unknown_subscription_kind_is_named() {
    let _state = crate::common::IsolatedTestState::new();
    let (_temp, store, id) = setup("Sub Unknown 3221", "a", &["a"]);
    make_subscription(&store, &id, "sub:not-a-real-plan", serde_json::json!({}));
    let error = preview_provider_models(&store, &id).await.unwrap_err();
    let text = format!("{error:#}");
    assert!(!text.contains("Base URL"), "{text}");
    assert!(text.contains("不认识的订阅类型 sub:not-a-real-plan"), "{text}");
}

#[test]
fn lenient_subscription_models_fall_back_with_a_diagnostic() {
    let _state = crate::common::IsolatedTestState::new();
    let fallback = || strings(&["default-a", "default-b"]);
    // 拉到了就原样用，不记日志。
    assert_eq!(
        models_or_fallback("antigravity", Ok(strings(&["live"])), fallback),
        strings(&["live"])
    );
    assert!(!diagnostic_log().contains("gateway.subscription_models_fallback"));
    // 拉不到时登录、本机适配入口仍用默认列表，但要留下原因可查。
    assert_eq!(
        models_or_fallback(
            "antigravity",
            Err(anyhow::anyhow!("fetchAvailableModels 返回 503")),
            fallback
        ),
        strings(&["default-a", "default-b"])
    );
    assert_eq!(
        models_or_fallback("antigravity", Ok(Vec::new()), fallback),
        strings(&["default-a", "default-b"])
    );
    let log = diagnostic_log();
    assert!(log.contains("gateway.subscription_models_fallback"), "{log}");
    assert!(log.contains("fetchAvailableModels 返回 503"), "{log}");
    assert!(log.contains("上游没有返回模型"), "{log}");
}
