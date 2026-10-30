//! 拓展的网关请求中间件：清单校验与用真实 Node 跑通的串联改写。

use codex_plus_core::extension_gateway::apply_request_with;
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::json;
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, gateway: serde_json::Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(dir.join("gateway")).unwrap();
    fs::write(dir.join("codexpp.json"), json!({"id": id, "name": id, "gateway": gateway}).to_string()).unwrap();
    for (path, content) in files {
        fs::write(dir.join(path), content).unwrap();
    }
}

#[test]
fn gateway_declarations_are_validated() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-kind", json!({"response": []}), &[]);
    write_package(&temp, "bad-timeout", json!({"request": [{"script": "gateway/a.mjs", "timeout": 60}]}), &[("gateway/a.mjs", "export default () => null")]);
    write_package(&temp, "ok", json!({"request": [{"script": "gateway/a.mjs"}]}), &[("gateway/a.mjs", "export default () => null")]);
    let inventory = manager.inventory().unwrap();
    let find = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap().clone();
    assert!(find("user:bad-kind")["error"].as_str().unwrap().contains("只支持 request"));
    assert!(find("user:bad-timeout")["error"].as_str().unwrap().contains("timeout"));
    assert_eq!(find("user:ok")["error"], "");
    assert_eq!(find("user:ok")["package"]["gatewayRequest"], 1);
}

#[tokio::test]
async fn request_middlewares_chain_in_order_and_failures_keep_the_request() {
    if codex_plus_core::dsh_plugins::locate_node().is_none() {
        eprintln!("跳过：本机没有 Node");
        return;
    }
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "a-prefix", json!({"request": [{"script": "gateway/a.mjs"}]}),
        &[("gateway/a.mjs", r#"export default (body, { path }) => ({ ...body, instructions: `${body.instructions}|A:${path}` });"#)]);
    write_package(&temp, "b-broken", json!({"request": [{"script": "gateway/b.mjs"}]}),
        &[("gateway/b.mjs", "export default () => { throw new Error('坏中间件'); }")]);
    write_package(&temp, "c-suffix", json!({"request": [{"script": "gateway/c.mjs"}]}),
        &[("gateway/c.mjs", r#"export default async (body) => ({ ...body, instructions: `${body.instructions}|C` });"#)]);

    let mut request = json!({"model": "gpt", "instructions": "原始", "input": []});
    apply_request_with(&manager, &mut request, "/v1/responses").await;
    assert_eq!(request["instructions"], "原始|A:/v1/responses|C", "按 key 顺序串联，坏中间件被跳过");
    assert_eq!(request["model"], "gpt");

    // 停用后不再改写；全部停用时零开销、原样返回。
    manager.set_script_enabled("user:a-prefix", false).unwrap();
    manager.set_script_enabled("user:c-suffix", false).unwrap();
    let mut untouched = json!({"instructions": "原始"});
    apply_request_with(&manager, &mut untouched, "/v1/responses").await;
    assert_eq!(untouched["instructions"], "原始");
}
