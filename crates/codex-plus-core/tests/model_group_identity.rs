//! 模型聚合的供应商分组身份：`codex_plus_group` / `codex_plus_group_id`。
//!
//! 原生 Codex 模型选择器把聚合目录里的 40+ 个模型平铺成一行一个的列表，供应商身份
//! 只能靠 `display_name` 尾部的「（供应商）」辨认。这里提供的两个键是前端分组折叠的
//! 权威数据源：
//! - `codex_plus_group_id`：稳定分组 id（持久化 relay id），同名成员靠它区分；
//! - `codex_plus_group`：展示名，只用于渲染分组表头。
//!
//! 分组键是加法：既有的 slug / display_name / description 行为必须原样保留
//! （前端还要靠「（供应商）」后缀给老目录兜底）。

use codex_plus_core::aggregate_catalog::{build_catalog_json, catalog_summary};
use codex_plus_core::model_suffix::model_ui_metadata_with_override;
use codex_plus_core::settings::{
    AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, BackendSettings, RelayMode,
    RelayProfile, RelaySessionProvider,
};
use serde_json::{Value, json};

/// 成员供应商：模型名故意用自定义 slug，避免内置元数据改写 display_name，
/// 让分组键之外的断言都保持确定性。
fn member(id: &str, name: &str) -> RelayProfile {
    RelayProfile {
        id: id.to_string(),
        name: name.to_string(),
        model: format!("{id}-main"),
        model_list: format!("{id}-mini"),
        base_url: format!("https://{id}.example/v1"),
        api_key: format!("sk-{id}"),
        relay_mode: RelayMode::PureApi,
        ..RelayProfile::default()
    }
}

fn aggregate(member_ids: &[&str], default_relay_id: &str) -> AggregateRelayProfile {
    AggregateRelayProfile {
        id: "agg".to_string(),
        name: "模型聚合".to_string(),
        session_provider: RelaySessionProvider::Custom,
        model_aggregation_enabled: true,
        strategy: AggregateRelayStrategy::Failover,
        members: member_ids
            .iter()
            .map(|relay_id| AggregateRelayMember {
                relay_id: (*relay_id).to_string(),
                weight: 1,
            })
            .collect(),
        routes: Vec::new(),
        default_relay_id: default_relay_id.to_string(),
    }
}

fn settings(members: Vec<RelayProfile>, aggregate: &AggregateRelayProfile) -> BackendSettings {
    let mut settings = BackendSettings::default();
    let mut profiles = members;
    // catalog_summary 会按 id 回查聚合 entry 本身（取名称与 model_provider）。
    profiles.push(RelayProfile {
        id: aggregate.id.clone(),
        name: aggregate.name.clone(),
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    });
    settings.relay_profiles = profiles;
    settings.active_relay_id = aggregate.id.clone();
    settings.active_aggregate_relay_id = aggregate.id.clone();
    settings.aggregate_relay_profiles = vec![aggregate.clone()];
    settings
}

fn catalog(settings: &BackendSettings, aggregate: &AggregateRelayProfile) -> Value {
    serde_json::from_str(&build_catalog_json(settings, aggregate).unwrap()).unwrap()
}

fn catalog_model<'a>(catalog: &'a Value, slug: &str) -> &'a Value {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["slug"] == slug)
        .unwrap_or_else(|| panic!("目录里缺少模型 {slug}"))
}

fn catalog_slugs(catalog: &Value) -> Vec<&str> {
    catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| model["slug"].as_str())
        .collect()
}

/// 默认成员：显示名仍是裸名（不加「（供应商）」后缀），但分组身份照写——
/// 它是「默认供应商」分组，不能留空。
#[test]
fn default_member_models_publish_bare_names_with_group_identity() {
    let aggregate = aggregate(&["relay-a", "relay-b"], "relay-a");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );
    let catalog = catalog(&settings, &aggregate);

    for slug in ["relay-a-main", "relay-a-mini"] {
        let entry = catalog_model(&catalog, slug);
        assert_eq!(entry["display_name"], slug, "默认成员的显示名仍是裸名");
        assert_eq!(entry["codex_plus_group"], "哈基米 国模");
        assert_eq!(entry["codex_plus_group_id"], "relay-a");
        assert_eq!(entry["description"], json!(format!("{slug} · 哈基米 国模")));
    }

    // 默认成员不发布带前缀的副本，裸名天然落回它。
    assert!(
        !catalog_slugs(&catalog).iter().any(|slug| slug.starts_with("cpx-relay-a/")),
        "默认成员不该再发布带前缀的副本：{:?}",
        catalog_slugs(&catalog)
    );
}

/// 非默认成员：`display_name` 仍带「（供应商）」后缀（前端兜底还要用），
/// 两个分组键同时给出权威身份。
#[test]
fn non_default_member_models_keep_provider_suffix_and_group_identity() {
    let aggregate = aggregate(&["relay-a", "relay-b"], "relay-a");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );
    let catalog = catalog(&settings, &aggregate);

    let entry = catalog_model(&catalog, "cpx-relay-b/relay-b-main");
    assert_eq!(entry["display_name"], "relay-b-main（jojocodemax）");
    assert_eq!(entry["codex_plus_group"], "jojocodemax");
    assert_eq!(entry["codex_plus_group_id"], "relay-b");
    assert_eq!(entry["description"], "relay-b-main · jojocodemax");
}

/// 注入层从同一份目录派生 modelMetadata：两个分组键必须能流到前端，
/// 且既有字段（displayName 等）不受影响。
#[test]
fn catalog_summary_model_metadata_carries_group_identity() {
    let temp = tempfile::tempdir().unwrap();
    let aggregate = aggregate(&["relay-a", "relay-b"], "relay-a");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );

    let summary = catalog_summary(temp.path(), &settings, &aggregate).unwrap();
    let metadata = &summary["modelMetadata"];

    // 默认成员（裸名 slug）也要带分组键。
    assert_eq!(metadata["relay-a-mini"]["codex_plus_group"], "哈基米 国模");
    assert_eq!(metadata["relay-a-mini"]["codex_plus_group_id"], "relay-a");
    // 非默认成员同样带分组键，既有展示字段原样保留。
    assert_eq!(metadata["cpx-relay-b/relay-b-mini"]["codex_plus_group"], "jojocodemax");
    assert_eq!(metadata["cpx-relay-b/relay-b-mini"]["codex_plus_group_id"], "relay-b");
    assert_eq!(
        metadata["cpx-relay-b/relay-b-mini"]["displayName"],
        "relay-b-mini（jojocodemax）"
    );
    // modelMetadata 覆盖目录里的全部 slug（只做加法，不新增/删除键）。
    assert_eq!(
        summary["models"].as_array().unwrap().len(),
        metadata.as_object().unwrap().len()
    );
}

/// 成员列表里同一个 relay id 出现两次（配置漂移/重复添加）：模型只发布一份，
/// 不会出现两组。
#[test]
fn duplicate_member_entries_do_not_create_duplicate_groups() {
    let aggregate = aggregate(&["relay-a", "relay-a", "relay-b"], "relay-a");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );
    let catalog = catalog(&settings, &aggregate);

    let all_slugs = catalog_slugs(&catalog);
    let mut unique = all_slugs.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(all_slugs.len(), unique.len(), "重复成员不该发布重复模型：{all_slugs:?}");
    assert_eq!(all_slugs.len(), 4, "两个成员各 2 个模型：{all_slugs:?}");

    let group_ids: Vec<&str> = catalog["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|model| model["codex_plus_group_id"].as_str())
        .collect();
    assert_eq!(
        group_ids.iter().filter(|id| **id == "relay-a").count(),
        2,
        "relay-a 只作为一个分组出现：{group_ids:?}"
    );
    assert_eq!(
        group_ids.iter().filter(|id| **id == "relay-b").count(),
        2,
        "relay-b 只作为一个分组出现：{group_ids:?}"
    );
}

/// 两个成员展示名相同时（用户截图里默认成员就叫「供应商 4」）：分组以
/// `codex_plus_group_id` 为准分成两组，展示名相同不合并。
#[test]
fn same_display_name_members_stay_separate_groups_by_id() {
    let aggregate = aggregate(&["relay-1", "relay-2"], "relay-1");
    let settings = settings(
        vec![member("relay-1", "供应商 4"), member("relay-2", "供应商 4")],
        &aggregate,
    );
    let catalog = catalog(&settings, &aggregate);

    let default_model = catalog_model(&catalog, "relay-1-main");
    let other_model = catalog_model(&catalog, "cpx-relay-2/relay-2-main");
    assert_eq!(default_model["codex_plus_group"], "供应商 4");
    assert_eq!(other_model["codex_plus_group"], "供应商 4");
    assert_eq!(default_model["codex_plus_group_id"], "relay-1");
    assert_eq!(other_model["codex_plus_group_id"], "relay-2");
    assert_ne!(
        default_model["codex_plus_group_id"], other_model["codex_plus_group_id"],
        "展示名相同也必须分成两组"
    );
}

/// 单供应商直连（非聚合目录）不会凭空多出分组：目录条目没有这两个键时，
/// UI 元数据里也不写；空白值同样忽略。
#[test]
fn direct_relay_catalog_gains_no_empty_group() {
    let plain = json!({
        "slug": "custom-alpha",
        "display_name": "custom-alpha",
        "description": "custom-alpha"
    });
    let ui = model_ui_metadata_with_override("custom-alpha", Some(&plain)).unwrap();
    assert!(ui.get("codex_plus_group").is_none(), "{ui}");
    assert!(ui.get("codex_plus_group_id").is_none(), "{ui}");

    let blank = json!({"codex_plus_group": "   ", "codex_plus_group_id": ""});
    let ui = model_ui_metadata_with_override("custom-alpha", Some(&blank)).unwrap();
    assert!(ui.get("codex_plus_group").is_none(), "{ui}");
    assert!(ui.get("codex_plus_group_id").is_none(), "{ui}");
}

/// 分组键与 `codex_plus_fast_support` 的既有特殊处理互不干扰：
/// 私有声明键不外泄、Fast 档位照旧生效，分组键照旧透出。
#[test]
fn group_identity_does_not_disturb_fast_support_handling() {
    let entry = json!({
        "slug": "cpx-relay-b/relay-b-main",
        "display_name": "relay-b-main（jojocodemax）",
        "description": "relay-b-main · jojocodemax",
        "codex_plus_group": "jojocodemax",
        "codex_plus_group_id": "relay-b",
        "codex_plus_fast_support": "supported"
    });
    let ui = model_ui_metadata_with_override("cpx-relay-b/relay-b-main", Some(&entry)).unwrap();

    assert_eq!(ui["codex_plus_group"], "jojocodemax");
    assert_eq!(ui["codex_plus_group_id"], "relay-b");
    assert!(ui.get("codex_plus_fast_support").is_none(), "{ui}");
    assert_eq!(ui["prioritySupportOverride"], true);
    assert_eq!(ui["serviceTiers"][0]["id"], "priority");
}

/// 默认成员要能被"认出来"：模型选择器只在默认供应商那一组打「默认」标志。
///
/// 默认成员的模型是裸名发布的，只看名字无法与"另一家没起名的成员"区分，
/// 所以身份必须由目录显式给出：目录条目带 `codex_plus_default`，
/// `/codex-model-catalog` 顶层带 `codex_plus_default_group_id`。
#[test]
fn default_member_is_marked_in_catalog_and_summary() {
    let temp = tempfile::tempdir().unwrap();
    let aggregate = aggregate(&["relay-a", "relay-b"], "relay-a");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );

    let catalog = catalog(&settings, &aggregate);
    for slug in ["relay-a-main", "relay-a-mini"] {
        assert_eq!(
            catalog_model(&catalog, slug)["codex_plus_default"],
            json!(true),
            "默认成员的模型必须带默认标记：{slug}"
        );
    }
    // 非默认成员不写这个键（不是 false，避免污染目录条目）。
    assert!(catalog_model(&catalog, "cpx-relay-b/relay-b-main")
        .get("codex_plus_default")
        .is_none());

    let summary = catalog_summary(temp.path(), &settings, &aggregate).unwrap();
    assert_eq!(summary["codex_plus_default_group_id"], "relay-a");
    // 标记要能流到前端读的 modelMetadata（白名单结构，不显式转发就丢）。
    assert_eq!(metadata_default_flag(&summary, "relay-a-mini"), Some(true));
    assert_eq!(metadata_default_flag(&summary, "cpx-relay-b/relay-b-mini"), None);
}

fn metadata_default_flag(summary: &Value, slug: &str) -> Option<bool> {
    summary["modelMetadata"][slug]
        .get("codex_plus_default")
        .and_then(Value::as_bool)
}

/// 没有配置默认成员时不能凭空造一个：标记与顶层 id 都应当是空的。
#[test]
fn unset_default_member_publishes_no_default_marker() {
    let temp = tempfile::tempdir().unwrap();
    let aggregate = aggregate(&["relay-a", "relay-b"], "");
    let settings = settings(
        vec![member("relay-a", "哈基米 国模"), member("relay-b", "jojocodemax")],
        &aggregate,
    );

    let catalog = catalog(&settings, &aggregate);
    assert!(
        catalog["models"]
            .as_array()
            .unwrap()
            .iter()
            .all(|model| model.get("codex_plus_default").is_none()),
        "没有默认成员时任何模型都不该被标成默认"
    );

    let summary = catalog_summary(temp.path(), &settings, &aggregate).unwrap();
    assert_eq!(summary["codex_plus_default_group_id"], "");
}
