//! Claude 页面里的 `window.codexPlus` 与 Codex 里那份的契约。
//!
//! Claude 的页面运行时在 claude-local 里另写了一份（ClaudeMom 仓库 `runtime/page-runtime.js`），它有哪些成员写在
//! `page-api.json` 里（那边的测试保证运行时和清单一致）。这里守住另一头：清单里的每个名字都得是 Codex++ 公开类型
//! 里的成员，常量的键和共用的取值得和 Codex 里的一样。这样一份拓展代码两边都能跑，谁也不会悄悄多出一个对方没有的名字。

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

fn repo_file(relative: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("读不到 {}: {error}", path.display()))
}

/// `interface <name> {` 这一块里两格缩进的成员名（属性、方法、getter）。
fn interface_members(types: &str, name: &str) -> BTreeSet<String> {
    let start = types.find(&format!("interface {name} {{")).unwrap_or_else(|| panic!("类型里没有 {name}"));
    let body = &types[start..];
    let end = body.find("\n}\n").unwrap_or(body.len());
    body[..end]
        .lines()
        .filter_map(|line| line.strip_prefix("  ").filter(|rest| !rest.starts_with(' ') && !rest.starts_with('/') && !rest.starts_with('*')))
        .filter_map(|line| {
            let line = line.strip_prefix("readonly ").unwrap_or(line);
            let name: String = line.chars().take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_').collect();
            (!name.is_empty()).then_some(name)
        })
        .collect()
}

fn names(contract: &Value, group: &str) -> BTreeSet<String> {
    contract[group].as_array().unwrap().iter().map(|name| name.as_str().unwrap().to_string()).collect()
}

#[test]
fn every_member_claude_offers_exists_in_the_public_types() {
    let contract: Value = serde_json::from_str(claude_local::extensions::PAGE_API_CONTRACT).unwrap();
    let types = repo_file("types/codex-plus-extensions.d.ts");

    let api = interface_members(&types, "CodexPlusExtensionApi");
    let missing: Vec<_> = names(&contract, "api").difference(&api).cloned().collect();
    assert!(missing.is_empty(), "Claude 的 codexPlus 有、公开类型里没有的成员：{missing:?}");

    let ctx = interface_members(&types, "CodexPlusExtensionContext");
    let missing: Vec<_> = names(&contract, "ctx").difference(&ctx).cloned().collect();
    assert!(missing.is_empty(), "Claude 的 ctx 有、公开类型里没有的成员：{missing:?}");

    // 判断宿主用的两个成员两边都有。
    for name in contract["newInClaude"].as_array().unwrap() {
        assert!(api.contains(name.as_str().unwrap()), "公开类型里缺 {name}");
    }
    assert_eq!(contract["apiVersion"], codex_plus_core::user_scripts::EXTENSION_API_VERSION);
}

#[test]
fn constants_keep_the_same_keys_and_the_shared_values() {
    let contract: Value = serde_json::from_str(claude_local::extensions::PAGE_API_CONTRACT).unwrap();
    let api_source = repo_file("assets/inject/renderer-inject/91-extension-api.js");
    let start = api_source.find("const codexPlusExtensionConstants = {").expect("Codex 侧的常量表");
    let block = &api_source[start..start + api_source[start..].find("\n  };").unwrap()];
    let codex_keys: BTreeSet<String> = block
        .lines()
        .skip(1)
        .filter_map(|line| line.trim().split([':', ',']).next().map(str::to_string))
        .filter(|key| !key.is_empty() && key.chars().all(|ch| ch.is_ascii_alphanumeric()))
        .collect();
    assert_eq!(names(&contract, "constants"), codex_keys, "常量的键两边要一样");

    // Codex++ 自己的类名和属性名两边取同一个值；rail 两项指向各自宿主的入口，不在此列。
    let prelude = repo_file("assets/inject/renderer-inject/00-prelude.js");
    for (key, value) in contract["sharedConstants"].as_object().unwrap() {
        let literal = format!("\"{}\"", value.as_str().unwrap());
        assert!(
            block.contains(&literal) || prelude.contains(&literal),
            "Codex 侧找不到常量 {key} 的值 {literal}：已发布的类名不能改名"
        );
    }
}

#[test]
fn routes_open_in_claude_are_a_subset_of_the_extension_whitelist() {
    let contract: Value = serde_json::from_str(claude_local::extensions::PAGE_API_CONTRACT).unwrap();
    let api_source = repo_file("assets/inject/renderer-inject/91-extension-api.js");
    let routes: Vec<&str> = contract["routes"].as_array().unwrap().iter().map(|route| route.as_str().unwrap()).collect();
    assert_eq!(routes, claude_local::extensions::PAGE_ROUTES);
    for route in routes {
        assert!(api_source.contains(&format!("\"{route}\"")), "{route} 不在 Codex 的拓展路由白名单里");
    }
}
