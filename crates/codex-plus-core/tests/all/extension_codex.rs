//! 拓展声明的 Codex 原生能力：技能装进 ~/.codex/skills、子 agent 与 MCP 服务器写进 config.toml、
//! 全局指令合并进 AGENTS.md 的托管段；停用后只撤掉自己写过的，不碰用户自己的同名条目。

use codex_plus_core::extension_codex::{SKILL_MARKER, sync, sync_instructions_text};
use codex_plus_core::user_scripts::UserScriptManager;
use serde_json::{Value, json};
use std::fs;

fn manager(temp: &tempfile::TempDir) -> UserScriptManager {
    UserScriptManager::new(temp.path().join("builtin"), temp.path().join("user"), temp.path().join("user_scripts.json"))
}

fn write_package(temp: &tempfile::TempDir, id: &str, manifest: Value, files: &[(&str, &str)]) {
    let dir = temp.path().join("user").join(id);
    fs::create_dir_all(&dir).unwrap();
    for (path, content) in files {
        let path = dir.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let mut manifest = manifest;
    manifest["id"] = json!(id);
    manifest["name"] = json!(format!("拓展 {id}"));
    fs::write(dir.join("codexpp.json"), manifest.to_string()).unwrap();
}

#[test]
fn skills_agents_mcp_servers_and_instructions_sync_into_codex_home() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    let home = temp.path().join("codex-home");
    fs::create_dir_all(home.join("skills/user-own")).unwrap();
    fs::write(home.join("skills/user-own/SKILL.md"), "用户自己的技能").unwrap();
    fs::write(home.join("config.toml"), "model = \"gpt\"\n\n[mcp_servers.kit-weather]\ncommand = \"user-owned\"\n").unwrap();
    fs::write(home.join("AGENTS.md"), "# 我的全局规则\n\n保持简洁。\n").unwrap();
    write_package(&temp, "kit", json!({
        "config": { "apiKey": { "type": "string", "secret": true, "default": "k-1" } },
        "agents": [{ "name": "reviewer", "description": "代码审查", "instructions": "prompts/reviewer.md", "model": "gpt-5", "reasoningEffort": "high", "nicknames": ["审"] }],
        "mcpServers": {
            "docs": { "command": "node", "args": ["${extensionDir}/mcp/server.mjs"], "env": { "KEY": "${config.apiKey}" }, "startupTimeoutSec": 20 },
            "weather": { "url": "https://example.com/mcp" }
        },
        "instructions": "rules.md"
    }), &[
        ("skills/review/SKILL.md", "---\nname: review\ndescription: 审查\n---\n先读 diff"),
        ("skills/review/notes.txt", "附带文件"),
        ("skills/not-a-skill/readme.md", "没有 SKILL.md 的目录不算"),
        ("prompts/reviewer.md", "你是审查员。"),
        ("mcp/server.mjs", ""),
        ("rules.md", "回答前先确认需求。"),
    ]);
    let summary = manager.inventory().unwrap();
    let item = summary["scripts"].as_array().unwrap().iter().find(|item| item["key"] == "user:kit").unwrap().clone();
    assert_eq!(item["error"], "", "{item}");
    assert_eq!(item["package"]["codex"], json!({ "skills": ["review"], "agents": ["reviewer"], "mcpServers": ["docs", "weather"], "instructions": true, "prompts": [], "resources": [] }));

    let report = sync(&home, &manager, &manager.enabled_codex_assets().unwrap()).unwrap();
    assert_eq!(report["skills"], json!(["kit-review"]));
    assert!(home.join("skills/kit-review/SKILL.md").is_file() && home.join("skills/kit-review/notes.txt").is_file());
    assert!(home.join("skills/kit-review").join(SKILL_MARKER).is_file());
    let config = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(config.contains("[agents.kit-reviewer]") && config.contains("description = \"代码审查\""), "{config}");
    assert!(config.contains("nickname_candidates = [\"审\"]"), "{config}");
    assert!(config.contains("[mcp_servers.kit-docs]") && config.contains("startup_timeout_sec = 20"), "{config}");
    assert!(config.contains("server.mjs") && config.contains("KEY = \"k-1\""), "占位符展开：{config}");
    assert!(config.contains("command = \"user-owned\"") && !config.contains("example.com/mcp"), "同名的用户条目不覆盖：{config}");
    assert_eq!(report["conflicts"], json!(["mcp_servers.kit-weather"]));
    // Windows 路径含反斜杠，toml_edit 会写成单引号字面量：按值开头的引号字符切，不写死双引号。
    let after_key = config.split("config_file = ").nth(1).unwrap();
    let quote = after_key.chars().next().unwrap();
    let role_file = after_key[1..].split(quote).next().unwrap();
    let role = fs::read_to_string(role_file).unwrap();
    assert!(role.contains("model = \"gpt-5\"") && role.contains("model_reasoning_effort = \"high\"") && role.contains("你是审查员。"), "{role}");
    let agents_md = fs::read_to_string(home.join("AGENTS.md")).unwrap();
    assert!(agents_md.starts_with("# 我的全局规则") && agents_md.contains("## 拓展 kit") && agents_md.contains("回答前先确认需求。"), "{agents_md}");

    // 再同步一次：没有变化就不改文件。
    let before = fs::read_to_string(home.join("config.toml")).unwrap();
    sync(&home, &manager, &manager.enabled_codex_assets().unwrap()).unwrap();
    assert_eq!(fs::read_to_string(home.join("config.toml")).unwrap(), before);

    // 停用：撤掉自己写的，用户的原样保留。
    manager.set_script_enabled("user:kit", false).unwrap();
    sync(&home, &manager, &manager.enabled_codex_assets().unwrap()).unwrap();
    assert!(!home.join("skills/kit-review").exists() && home.join("skills/user-own/SKILL.md").is_file());
    let config = fs::read_to_string(home.join("config.toml")).unwrap();
    assert!(!config.contains("kit-reviewer") && !config.contains("kit-docs") && config.contains("command = \"user-owned\""), "{config}");
    assert_eq!(fs::read_to_string(home.join("AGENTS.md")).unwrap(), "# 我的全局规则\n\n保持简洁。\n");
}

#[test]
fn invalid_declarations_are_reported_on_the_package() {
    let temp = tempfile::tempdir().unwrap();
    let manager = manager(&temp);
    write_package(&temp, "bad-mcp", json!({ "mcpServers": { "x": { "command": "a", "url": "https://b" } } }), &[]);
    write_package(&temp, "bad-agent", json!({ "agents": [{ "name": "a b", "description": "d" }] }), &[]);
    write_package(&temp, "bad-skill", json!({ "skills": ["skills/none"] }), &[("skills/none/readme.md", "")]);
    let inventory = manager.inventory().unwrap();
    let error = |key: &str| inventory["scripts"].as_array().unwrap().iter().find(|item| item["key"] == key).unwrap()["error"].as_str().unwrap().to_string();
    assert!(error("user:bad-mcp").contains("二选一"));
    assert!(error("user:bad-agent").contains("名字不合法"));
    assert!(error("user:bad-skill").contains("SKILL.md"));
}

#[test]
fn instructions_block_is_replaced_in_place_and_removed_cleanly() {
    let first = sync_instructions_text("", Some("规则一"));
    assert!(first.contains("规则一") && first.ends_with("-->\n"));
    let second = sync_instructions_text(&format!("开头\n\n{first}"), Some("规则二"));
    assert!(second.starts_with("开头") && second.contains("规则二") && !second.contains("规则一"));
    assert_eq!(sync_instructions_text(&second, None), "开头\n");
}

/// 实测用：把一个声明了技能、子 agent、MCP 服务器、全局指令的拓展同步进 `CODEXPP_LIVE_CODEX_HOME`，
/// 再用真实 Codex CLI 跑一轮看它是否识别。平时跳过：`CODEXPP_LIVE_CODEX_HOME=… cargo test … -- --ignored`。
#[test]
#[ignore]
fn live_sync_into_codex_home() {
    let Ok(home) = std::env::var("CODEXPP_LIVE_CODEX_HOME") else {
        return;
    };
    let home = std::path::PathBuf::from(home);
    let root = home.parent().unwrap().join("live-ext");
    let manager = UserScriptManager::new(root.join("builtin"), root.join("user"), root.join("user_scripts.json"));
    let dir = root.join("user/kit");
    fs::create_dir_all(dir.join("skills/probe-skill")).unwrap();
    fs::create_dir_all(dir.join("mcp")).unwrap();
    fs::write(dir.join("skills/probe-skill/SKILL.md"), "---\nname: probe-skill\ndescription: PROBE-SKILL-DESCRIPTION 用来实测技能同步\n---\n先说 PROBE-SKILL-BODY\n").unwrap();
    fs::write(dir.join("mcp/server.mjs"), include_str!("live_mcp_server.mjs")).unwrap();
    fs::write(dir.join("rules.md"), "PROBE-GLOBAL-INSTRUCTION：回答前先说「已读规则」。").unwrap();
    fs::write(dir.join("codexpp.json"), json!({
        "id": "kit", "name": "实测套件",
        "config": { "token": { "type": "string", "secret": true, "default": "PROBE-TOKEN" } },
        "agents": [{ "name": "reviewer", "description": "PROBE-AGENT-DESCRIPTION 代码审查", "instructions": "你是审查员" }],
        "mcpServers": { "probe": { "command": "node", "args": ["${extensionDir}/mcp/server.mjs"], "env": { "PROBE_TOKEN": "${config.token}" } } },
        "instructions": "rules.md"
    }).to_string()).unwrap();
    let report = sync(&home, &manager, &manager.enabled_codex_assets().unwrap()).unwrap();
    println!("{report}");
}
