//! 用真实 settings.json 跑一遍目录生成，核对"成员自指前缀"不再套两层。
//! 默认忽略（依赖本机用户设置），用 `--ignored` 显式运行：
//!   cargo test -p codex-plus-core --test real_catalog_probe -- --ignored --nocapture
use codex_plus_core::aggregate_catalog::build_catalog_json;
use codex_plus_core::settings::SettingsStore;

#[test]
#[ignore = "依赖本机 ~/.codex-session-delete/settings.json"]
fn real_settings_catalog_has_no_nested_prefix() {
    let settings = SettingsStore::default().load().expect("读取真实设置失败");
    let Some(aggregate) = settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.model_aggregation_enabled)
    else {
        eprintln!("本机没有启用的模型聚合，跳过");
        return;
    };
    let payload: serde_json::Value =
        serde_json::from_str(&build_catalog_json(&settings, &aggregate).unwrap()).unwrap();
    let models = payload["models"].as_array().unwrap();
    // 自指嵌套（同一条目里出现两次同一个成员前缀）是缺陷；跨成员引用
    // （`cpx-A/cpx-B/model`）是成员显式写了别家的 ID，属于配置意图，这里只报告。
    let self_nested: Vec<&str> = models
        .iter()
        .filter_map(|model| model["slug"].as_str())
        .filter(|slug| {
            slug.split_once('/')
                .and_then(|(member, rest)| {
                    rest.strip_prefix(member).and_then(|rest| rest.strip_prefix('/'))
                })
                .is_some()
        })
        .collect();
    let cross_member: Vec<&str> = models
        .iter()
        .filter_map(|model| model["slug"].as_str())
        .filter(|slug| {
            slug.split_once('/')
                .map(|(member, rest)| rest.contains('/') && !rest.starts_with(member))
                .unwrap_or(false)
        })
        .collect();
    println!("模型总数: {}", models.len());
    for model in models.iter().filter(|model| {
        model["slug"]
            .as_str()
            .map(|slug| slug.contains("kimi-k3") || slug.contains("deepseek-flash"))
            .unwrap_or(false)
    }) {
        println!(
            "  slug={:<52} display={}",
            model["slug"].as_str().unwrap_or(""),
            model["display_name"].as_str().unwrap_or("")
        );
    }
    println!("自指嵌套条目: {self_nested:?}");
    println!("跨成员引用条目（配置意图，非缺陷）: {cross_member:?}");
    assert!(self_nested.is_empty(), "仍有自指嵌套前缀：{self_nested:?}");
}
