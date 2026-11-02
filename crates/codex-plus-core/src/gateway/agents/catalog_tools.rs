//! 通用 Agent 规格：命令行 Agent（第二批）：YAML / TOML / .env 配置、订阅类 Agent 与本机常见的 Agent。
//!
//! 写法参考 [`super::catalog_core`]。文件末尾是几种格式共用的小工具：YAML 二级键的保注释改写、
//! TOML 表的逐级建立、JSON 值转 TOML 值、dsh 补丁列表的条目读取。
//!
//! 没有接入的：Antigravity CLI（agy）。它的 Gemini 端点和 Key 只从启动时的环境变量
//! （`GOOGLE_GEMINI_BASE_URL` / `GEMINI_API_KEY`）读，没有 `.env` 也没有配置项，
//! 只改文件接不进来。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table};

use crate::gateway::agents::catalog_core::output_limit;
use crate::gateway::agents::files::{parse_jsonc, yaml_block};
use crate::gateway::agents::{
    AgentEnv, AgentSpec, AgentWire, FileSet, Os, PROVIDER_ID, PROVIDER_NAME, Wiring, YamlDoc,
    object_entry,
};

pub static SPECS: &[AgentSpec] = &[
    GEMINI_CLI,
    GOOSE,
    DSH,
    REASONIX,
    HERMES,
    MISTER_MORPH,
    JCODE,
    EMPRYO,
    MINIMAX_CODE,
    DROID,
    CLINE,
    QODER,
    QODER_CN,
    GROK_APP,
    ZCODE,
    WORKBUDDY,
    WORKBUDDY_AI,
    CODEBUDDY,
    ATOMCODE,
    SNOW,
    ANTE,
];

/// 不知道窗口时告诉 Agent 的上下文大小。
const DEFAULT_WINDOW: u64 = 128_000;

/// 模型的窗口，不知道时用 [`DEFAULT_WINDOW`]。
fn window_or_default(window: Option<u64>) -> u64 {
    window.unwrap_or(DEFAULT_WINDOW)
}

/// 主模型带上供应商前缀：`codexplus/<模型>`。
fn prefixed(model: &str) -> String {
    format!("{PROVIDER_ID}/{model}")
}

// ---------------------------------------------------------------- Gemini CLI

/// `$GEMINI_CLI_HOME` 换的是用户目录，配置仍在它下面的 `.gemini`。
fn gemini_dir(env: &AgentEnv) -> PathBuf {
    env.dir_var("GEMINI_CLI_HOME")
        .unwrap_or_else(|| env.home.clone())
        .join(".gemini")
}

fn gemini_path(env: &AgentEnv) -> PathBuf {
    gemini_dir(env).join("settings.json")
}

/// Gemini CLI 只说 Gemini 协议：`.env` 里的 `GOOGLE_GEMINI_BASE_URL` 指向网关，
/// 网关 token 当 `GEMINI_API_KEY`，鉴权方式切到 API Key，模型名写进 `model.name`。
fn write_gemini(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = gemini_dir(env);
    let root = set.json(&dir.join("settings.json"))?;
    object_entry(object_entry(root, "security"), "auth")
        .insert("selectedType".to_string(), json!("gemini-api-key"));
    object_entry(root, "model").insert("name".to_string(), json!(wiring.default_model));
    let dotenv = set.env(&dir.join(".env"))?;
    dotenv.set("GOOGLE_GEMINI_BASE_URL", &wiring.root);
    dotenv.set("GEMINI_API_KEY", &wiring.token);
    Ok(())
}

pub const GEMINI_CLI: AgentSpec = AgentSpec {
    id: "gemini-cli",
    name: "Gemini CLI",
    binary: "gemini",
    wire: AgentWire::Gemini,
    // ~/.gemini 也是 Antigravity 的目录，只认命令
    detect: |env| env.has_binary("gemini"),
    config_path: gemini_path,
    write: write_gemini,
    hint: "Gemini CLI 启动时读配置，接入后重开 gemini；临时换模型用 gemini -m <模型>",
};

// ---------------------------------------------------------------- Goose

fn goose_dir(env: &AgentEnv) -> PathBuf {
    if let Some(root) = env.dir_var("GOOSE_PATH_ROOT") {
        return root.join("config");
    }
    if env.os == Os::Windows {
        return env.app_config().join("Block").join("goose").join("config");
    }
    env.xdg_config().join("goose")
}

fn goose_path(env: &AgentEnv) -> PathBuf {
    goose_dir(env).join("config.yaml")
}

/// Goose 的自定义供应商是 `custom_providers/<id>.json` 一个文件；`config.yaml` 里选中它。
/// 新版布局（有 `providers` 或 `active_provider`）用 `active_provider` + `providers.<id>.model`，
/// 旧版用 `GOOSE_PROVIDER` / `GOOSE_MODEL` 两个平铺键。
fn write_goose(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = goose_dir(env);
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({ "name": model.id });
            if let Some(window) = model.context_window {
                entry["context_limit"] = json!(window);
            }
            entry
        })
        .collect();
    let provider = set.json(
        &dir.join("custom_providers")
            .join(format!("{PROVIDER_ID}.json")),
    )?;
    let fields = [
        ("name", json!(PROVIDER_ID)),
        ("engine", json!("openai")),
        ("display_name", json!(PROVIDER_NAME)),
        ("description", json!("Codex++ 本地网关")),
        ("api_key_env", json!("")),
        ("base_url", json!(wiring.v1())),
        (
            "headers",
            json!({ "Authorization": format!("Bearer {}", wiring.token) }),
        ),
        ("requires_auth", json!(false)),
        ("skip_canonical_filtering", json!(true)),
        ("dynamic_models", json!(models.is_empty())),
        ("models", Value::Array(models)),
    ];
    for (key, value) in fields {
        provider.insert(key.to_string(), value);
    }

    let doc = set.yaml(&dir.join("config.yaml"))?;
    if doc.has("providers") || doc.has("active_provider") {
        doc.set("active_provider", &json!(PROVIDER_ID));
        yaml_set_child(
            doc,
            "providers",
            PROVIDER_ID,
            // 按字母序写：serde_json 开不开 preserve_order（随平台的依赖而变）写出来都一样。
            Some(&json!({ "configured": true, "enabled": true, "model": wiring.default_model })),
        );
    } else {
        doc.set("GOOSE_PROVIDER", &json!(PROVIDER_ID));
        doc.set("GOOSE_MODEL", &json!(wiring.default_model));
    }
    Ok(())
}

pub const GOOSE: AgentSpec = AgentSpec {
    id: "goose",
    name: "Goose",
    binary: "goose",
    wire: AgentWire::Chat,
    detect: |env| goose_dir(env).exists() || env.has_binary("goose"),
    config_path: goose_path,
    write: write_goose,
    hint: "Goose 启动时加载供应商，接入后重启 Goose；在它的模型设置里可换 Codex++ 下的模型",
};

// ---------------------------------------------------------------- DeepSeek Harness (dsh)

/// dsh 读网关 Key 用的变量名（`.env` 与 dsh 自己的密钥库里都写这个名字）。
const DSH_KEY_ENV: &str = "CODEXPLUS_GATEWAY_KEY";
const DSH_BEGIN: &str = "# >>> codex-plus-gateway";
const DSH_BEGIN_LINE: &str =
    "# >>> codex-plus-gateway：由 Codex++ 网关写入，断开网关时自动移除，请不要手改";
const DSH_END: &str = "# <<< codex-plus-gateway";
const DSH_HEAD: &str = "# dsh 的全局补丁层，叠加在每个 profile 的补丁层之上\n";

fn dsh_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["DSH_HOME"], || env.home_path(".dsh"))
}

/// 全局补丁层 `$DSH_HOME/cordis.patch.yml`：叠加在每个 profile 的补丁层之上，同 id 的条目以它为准。
fn dsh_path(env: &AgentEnv) -> PathBuf {
    dsh_dir(env).join("cordis.patch.yml")
}

/// 各 profile 的补丁层，desktop、web、headless 在前。
fn dsh_profiles(dir: &Path) -> Vec<PathBuf> {
    let root = dir.join("profiles");
    let mut names: Vec<String> = std::fs::read_dir(&root)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| entry.path().join("cordis.patch.yml").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort_by_key(|name| {
        let rank = match name.as_str() {
            "desktop" => 0,
            "web" => 1,
            "headless" => 2,
            _ => 3,
        };
        (rank, name.clone())
    });
    names
        .into_iter()
        .map(|name| root.join(name).join("cordis.patch.yml"))
        .collect()
}

/// dsh 的补丁列表是顶层 YAML 数组，`YamlDoc` 只认顶层键，所以全局补丁层按整份文件写：
/// 用户原有内容不动，末尾维护一段带首尾标记的块，里面两条补丁——
/// `llm-pi-ai`（自定义供应商，Codex++ 一条路由，用户已有的路由一并带上）与
/// `agent-default-model`（新会话起始的模型）。Key 写进 `$DSH_HOME/.env`（命令行读）和
/// dsh 自己的密钥库 `.credentials.yaml`（桌面版只读它）。
fn write_dsh(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = dsh_dir(env);
    let path = dir.join("cordis.patch.yml");
    let before = set.raw(&path)?.clone();
    let (outside, _) = split_marked(before.as_deref().unwrap_or_default(), &path)?;
    let outside = dsh_outside(&outside, &path)?;

    // 用户自己的 llm-pi-ai 配置（dsh 模型页加的供应商）：全局层块外的优先，其次第一个有它的 profile。
    // 补丁按 id 整块替换 config，不带上就会在接入期间把它们藏起来。
    let mut sources = vec![outside.clone()];
    for profile in dsh_profiles(&dir) {
        if let Ok(text) = std::fs::read_to_string(profile) {
            sources.push(text);
        }
    }
    let user_config = sources
        .iter()
        .find_map(|text| patch_entry_config(text, "llm-pi-ai"));

    let block = dsh_block(wiring, user_config.as_deref());
    let mut text = if before.is_none() {
        DSH_HEAD.to_string()
    } else {
        outside
    };
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(&block);
    *set.raw(&path)? = Some(text);

    set.env(&dir.join(".env"))?.set(DSH_KEY_ENV, &wiring.token);
    let credentials = dir.join(".credentials.yaml");
    if credentials.is_file() {
        let doc = set.yaml(&credentials)?;
        // 只认 version 1 的布局，别的交给 dsh 自己迁移
        if doc.scalar("version").as_deref() == Some("1") {
            yaml_set_child(doc, "refs", DSH_KEY_ENV, Some(&json!(wiring.token)));
        }
    }
    Ok(())
}

fn dsh_block(wiring: &Wiring, user_config: Option<&str>) -> String {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({ "id": model.id, "name": model.id });
            if let Some(window) = model.context_window {
                entry["contextWindow"] = json!(window);
                entry["maxTokens"] = json!(output_limit(window));
            }
            entry
        })
        .collect();
    let route = json!({
        "displayName": PROVIDER_NAME,
        "apiKeyEnv": DSH_KEY_ENV,
        "api": "openai-completions",
        "baseURL": wiring.v1(),
        "models": models,
    });
    let mut config = YamlDoc::parse(user_config.unwrap_or_default());
    yaml_set_child(&mut config, "providers", PROVIDER_ID, Some(&route));

    let mut out = format!(
        "{DSH_BEGIN_LINE}\n- id: llm-pi-ai\n  name: \"@deepseek-ai/dsh-llm-pi-ai\"\n  config:\n"
    );
    out.push_str(&indent_lines(&config.render(), 4));
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!(
        "- id: agent-default-model\n  name: \"@deepseek-ai/dsh-agent-default-model\"\n  config:\n    provider: {}\n    model: {}\n{DSH_END}\n",
        json_string(PROVIDER_ID),
        json_string(&wiring.default_model),
    ));
    out
}

/// 去掉块外的 `[]`（dsh 给空 profile 写的流式空列表）；块外不是块状列表时拒绝改动。
fn dsh_outside(text: &str, path: &Path) -> anyhow::Result<String> {
    let mut out = String::new();
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim();
        if trimmed == "[]" {
            continue;
        }
        if is_content(line)
            && leading_spaces(line) == 0
            && !line.starts_with("- ")
            && trimmed != "-"
            && !trimmed.starts_with("---")
        {
            bail!(
                "{} 不是块状的 YAML 列表，为免改坏没有改动它",
                path.display()
            );
        }
        out.push_str(line);
    }
    Ok(out)
}

/// 把文本拆成「标记块之外」和「标记块」两部分。
fn split_marked(text: &str, path: &Path) -> anyhow::Result<(String, Option<String>)> {
    let mut outside = String::new();
    let mut block: Option<String> = None;
    let mut inside = false;
    for line in text.split_inclusive('\n') {
        if !inside && line.starts_with(DSH_BEGIN) {
            inside = true;
            block = Some(line.to_string());
            continue;
        }
        if inside {
            block.get_or_insert_with(String::new).push_str(line);
            if line.trim_end() == DSH_END {
                inside = false;
            }
            continue;
        }
        outside.push_str(line);
    }
    if inside {
        bail!(
            "{} 里 Codex++ 写入的块缺少结束标记，为免改坏没有改动它",
            path.display()
        );
    }
    Ok((outside, block))
}

/// 断开只撤销网关的标记块，接入期间新增或改写的用户补丁仍保留。
pub(super) fn restore_dsh_patch(
    path: &Path,
    before: Option<&str>,
    current: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(current) = current else {
        return Ok(None);
    };
    let (outside, block) = split_marked(current, path)?;
    let Some(block) = block else {
        return Ok(Some(current.to_string()));
    };
    let (before_outside, before_block) = split_marked(before.unwrap_or_default(), path)?;
    let mut written_outside = if before.is_none() {
        DSH_HEAD.to_string()
    } else {
        dsh_outside(&before_outside, path)?
    };
    if !written_outside.is_empty() && !written_outside.ends_with('\n') {
        written_outside.push('\n');
    }
    if outside == written_outside {
        return Ok(before.map(str::to_string));
    }
    let mut restored = current.replacen(&block, before_block.as_deref().unwrap_or_default(), 1);
    if before.is_none() {
        restored = restored.strip_prefix(DSH_HEAD).unwrap_or(&restored).to_string();
    }
    Ok(if before.is_none() && restored.is_empty() { None } else { Some(restored) })
}

// ---------------------------------------------------------------- Reasonix Studio

const REASONIX_KEY_ENV: &str = "CODEXPLUS_REASONIX_API_KEY";

fn reasonix_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["REASONIX_HOME"], || {
        if env.os == Os::Windows {
            env.app_config().join("reasonix")
        } else {
            env.home_path(".reasonix")
        }
    })
}

fn reasonix_path(env: &AgentEnv) -> PathBuf {
    reasonix_dir(env).join("config.toml")
}

/// Studio 与命令行共用 `config.toml` 和 `.env`：`[[providers]]` 里一条 `name = "codexplus"`，
/// `default_model` 指向它；桌面版有供应商白名单时把它加进去。
fn write_reasonix(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = reasonix_dir(env);
    let doc = set.toml(&dir.join("config.toml"))?;
    let mut overrides = InlineTable::new();
    for model in &wiring.models {
        let mut entry = InlineTable::new();
        // 网关不知道模型支不支持思考强度，不让 Reasonix 发
        entry.insert("reasoning_protocol", "none".into());
        if let Some(window) = model.context_window {
            entry.insert("context_window", toml_int(window));
            entry.insert("max_output_tokens", toml_int(output_limit(window)));
        }
        overrides.insert(&model.id, toml_edit::Value::InlineTable(entry));
    }
    let mut provider = Table::new();
    toml_set(&mut provider, "name", &json!(PROVIDER_ID));
    toml_set(&mut provider, "kind", &json!("openai"));
    toml_set(&mut provider, "base_url", &json!(wiring.v1()));
    toml_set(&mut provider, "api_key_env", &json!(REASONIX_KEY_ENV));
    toml_set(
        &mut provider,
        "models",
        &Value::Array(wiring.model_ids().map(|id| json!(id)).collect()),
    );
    toml_set(&mut provider, "default", &json!(wiring.default_model));
    toml_set(&mut provider, "no_proxy", &json!(true));
    provider.insert(
        "model_overrides",
        Item::Value(toml_edit::Value::InlineTable(overrides)),
    );

    match doc.get_mut("providers") {
        None => {
            let mut list = ArrayOfTables::new();
            list.push(provider);
            doc.insert("providers", Item::ArrayOfTables(list));
        }
        Some(Item::ArrayOfTables(list)) => {
            let existing = list
                .iter()
                .position(|table| table.get("name").and_then(Item::as_str) == Some(PROVIDER_ID));
            match existing.and_then(|index| list.get_mut(index)) {
                Some(table) => *table = provider,
                None => list.push(provider),
            }
        }
        Some(_) => bail!(
            "Reasonix 的 config.toml 里 providers 不是 [[providers]] 表数组，为免改坏没有改动它"
        ),
    }
    doc.insert(
        "default_model",
        Item::Value(prefixed(&wiring.default_model).into()),
    );
    if let Some(access) = doc
        .get_mut("desktop")
        .and_then(|desktop| desktop.get_mut("provider_access"))
        .and_then(Item::as_array_mut)
        && !access
            .iter()
            .any(|value| value.as_str() == Some(PROVIDER_ID))
    {
        access.push(PROVIDER_ID);
    }
    set.env(&dir.join(".env"))?
        .set(REASONIX_KEY_ENV, &wiring.token);
    Ok(())
}

pub const REASONIX: AgentSpec = AgentSpec {
    id: "reasonix",
    name: "Reasonix Studio",
    binary: "reasonix",
    wire: AgentWire::Chat,
    detect: |env| reasonix_dir(env).exists() || env.has_binary("reasonix"),
    config_path: reasonix_path,
    write: write_reasonix,
    hint: "在 Reasonix 里选 Codex++ 下的模型；接入后重启 Studio 或 reasonix",
};

pub const DSH: AgentSpec = AgentSpec {
    id: "dsh",
    name: "DeepSeek Harness",
    binary: "dsh",
    wire: AgentWire::Chat,
    detect: |env| dsh_dir(env).exists() || env.has_binary("dsh"),
    config_path: dsh_path,
    write: write_dsh,
    hint: "在 dsh 的模型列表里选 Codex++ 下的模型；新会话生效",
};

// ---------------------------------------------------------------- Hermes Agent

fn hermes_root(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["HERMES_HOME"], || {
        if env.os == Os::Windows {
            env.local_app_data().join("hermes")
        } else {
            env.home_path(".hermes")
        }
    })
}

/// 当前生效的配置目录：`active_profile` 指向的 profile，否则根目录。
fn hermes_dir(env: &AgentEnv) -> PathBuf {
    let root = hermes_root(env);
    if let Ok(text) = std::fs::read_to_string(root.join("active_profile")) {
        let name = text.trim_start_matches('\u{feff}').trim();
        if !name.is_empty() && name != "default" && !name.contains(['/', '\\']) {
            let dir = root.join("profiles").join(name);
            if dir.is_dir() {
                return dir;
            }
        }
    }
    root
}

fn hermes_path(env: &AgentEnv) -> PathBuf {
    hermes_dir(env).join("config.yaml")
}

/// `providers.codexplus` 一条 OpenAI 兼容供应商，`model.provider` / `model.default` 选中它。
/// 两个顶层块都只改这几个二级键，`model:` 块里的大段注释原样保留。
fn write_hermes(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.yaml(&hermes_path(env))?;
    let provider = json!({
        "name": PROVIDER_NAME,
        "base_url": wiring.v1(),
        "api_key": wiring.token,
        "api_mode": "chat_completions",
        "models": wiring.model_ids().collect::<Vec<_>>(),
    });
    yaml_set_child(doc, "providers", PROVIDER_ID, Some(&provider));
    yaml_set_child(doc, "model", "provider", Some(&json!(PROVIDER_ID)));
    yaml_set_child(doc, "model", "default", Some(&json!(wiring.default_model)));
    Ok(())
}

pub const HERMES: AgentSpec = AgentSpec {
    id: "hermes",
    name: "Hermes Agent",
    binary: "hermes",
    wire: AgentWire::Chat,
    detect: |env| hermes_root(env).exists() || env.has_binary("hermes"),
    config_path: hermes_path,
    write: write_hermes,
    hint: "用 hermes model 或会话里的模型选择器换 Codex++ 下的模型；接入后重开 hermes",
};

// ---------------------------------------------------------------- Mister Morph

fn morph_path(env: &AgentEnv) -> PathBuf {
    env.var("MISTER_MORPH_CONFIG")
        .map(|value| env.expand(&value))
        .unwrap_or_else(|| env.home_path(".morph").join("config.yaml"))
}

/// 它没有供应商列表，只能接管 `llm` 下的端点、Key 和模型（Responses 协议）。
fn write_morph(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.yaml(&morph_path(env))?;
    let fields = [
        ("inference_provider", json!("openai_response_compatible")),
        ("endpoint", json!(wiring.v1())),
        ("api_key", json!(wiring.token)),
        ("model", json!(wiring.default_model)),
    ];
    for (key, value) in fields {
        yaml_set_child(doc, "llm", key, Some(&value));
    }
    // 原来模型的窗口不再适用：知道就写，不知道就去掉
    let window = wiring
        .window(&wiring.default_model)
        .map(|window| json!(window));
    yaml_set_child(doc, "llm", "context_window_tokens", window.as_ref());
    Ok(())
}

pub const MISTER_MORPH: AgentSpec = AgentSpec {
    id: "mister-morph",
    name: "Mister Morph",
    // morph 这个命令名别的工具也在用
    binary: "",
    wire: AgentWire::Responses,
    detect: |env| morph_path(env).exists() || env.home_path(".morph").exists(),
    config_path: morph_path,
    write: write_morph,
    hint: "Mister Morph 只用一个模型，换模型请在 Codex++ 里改；接入后重开 morph",
};

// ---------------------------------------------------------------- jcode

fn jcode_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["JCODE_HOME"], || env.home_path(".jcode"))
}

fn jcode_path(env: &AgentEnv) -> PathBuf {
    jcode_dir(env).join("config.toml")
}

/// `[providers.codexplus]`（openai-compatible，模型表一并写上），`[provider]` 的默认供应商与模型指向它。
fn write_jcode(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.toml(&jcode_path(env))?;
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            json!({
                "id": model.id,
                "context_window": window_or_default(model.context_window),
                "input": ["text"],
            })
        })
        .collect();
    let provider = toml_table(doc, &["providers", PROVIDER_ID])?;
    toml_set(provider, "type", &json!("openai-compatible"));
    toml_set(provider, "base_url", &json!(wiring.v1()));
    toml_set(provider, "api_key", &json!(wiring.token));
    // 回环地址上 jcode 默认不发 Key
    toml_set(provider, "requires_api_key", &json!(true));
    toml_set(provider, "default_model", &json!(wiring.default_model));
    toml_set(provider, "models", &Value::Array(models));
    let pick = toml_table(doc, &["provider"])?;
    toml_set(pick, "default_provider", &json!(PROVIDER_ID));
    toml_set(pick, "default_model", &json!(wiring.default_model));
    Ok(())
}

pub const JCODE: AgentSpec = AgentSpec {
    id: "jcode",
    name: "jcode",
    binary: "jcode",
    wire: AgentWire::Chat,
    detect: |env| jcode_dir(env).exists() || env.has_binary("jcode"),
    config_path: jcode_path,
    write: write_jcode,
    hint: "在 jcode 里用 /model 选 Codex++ 下的模型；接入后重启 jcode",
};

// ---------------------------------------------------------------- Empryo

const EMPRYO_KEY_ENV: &str = "CODEXPLUS_EMPRYO_KEY";

/// `~/.empryo`；改名前的 `~/.soulforge` 还在而新目录没有时用旧的。
fn empryo_dir(env: &AgentEnv) -> PathBuf {
    let dir = env.home_path(".empryo");
    let legacy = env.home_path(".soulforge");
    if !dir.exists() && legacy.exists() {
        return legacy;
    }
    dir
}

fn empryo_path(env: &AgentEnv) -> PathBuf {
    empryo_dir(env).join("config.json")
}

/// `providers` 数组里一条 Codex++，`defaultModel` 选中它。自定义供应商的 Key 只能按 `envVar`
/// 取：环境变量没有时 Empryo 会去同目录的 `secrets.json` 按变量名找，网关 token 就写在那里。
fn write_empryo(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = empryo_dir(env);
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({ "id": model.id, "name": model.id });
            if let Some(window) = model.context_window {
                entry["contextWindow"] = json!(window);
            }
            entry
        })
        .collect();
    let root = set.json(&dir.join("config.json"))?;
    let providers = array_entry(root, "providers");
    providers.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(PROVIDER_ID));
    providers.push(json!({
        "id": PROVIDER_ID,
        "name": PROVIDER_NAME,
        "baseURL": wiring.v1(),
        "envVar": EMPRYO_KEY_ENV,
        "modelsAPI": format!("{}/models", wiring.v1()),
        "models": models,
    }));
    root.insert(
        "defaultModel".to_string(),
        json!(prefixed(&wiring.default_model)),
    );
    set.json(&dir.join("secrets.json"))?
        .insert(EMPRYO_KEY_ENV.to_string(), json!(wiring.token));
    Ok(())
}

pub const EMPRYO: AgentSpec = AgentSpec {
    id: "empryo",
    name: "Empryo",
    binary: "empryo",
    wire: AgentWire::Chat,
    detect: |env| {
        env.home_path(".empryo").exists()
            || env.home_path(".soulforge").exists()
            || env.has_binary("empryo")
    },
    config_path: empryo_path,
    write: write_empryo,
    hint: "在 Empryo 的模型选择里选 Codex++ 下的模型；接入后重启 empryo",
};

// ---------------------------------------------------------------- MiniMax Code

fn minimax_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["MINIMAX_DATA_DIR"], || env.home_path(".minimax"))
        .join("config.yaml")
}

/// `custom_provider.codexplus`（Anthropic Messages），`defaultModel` 写成
/// `custom_provider:codexplus/<模型>`。用户的其它自定义供应商在同一块里，只改这一个二级键。
fn write_minimax(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let mut models = Map::new();
    for model in &wiring.models {
        let mut entry = json!({ "name": model.id });
        if let Some(window) = model.context_window {
            entry["limit"] = json!({ "context": window, "output": output_limit(window) });
        }
        models.insert(model.id.clone(), entry);
    }
    let provider = json!({
        "name": PROVIDER_NAME,
        "kind": "custom",
        "enabled": true,
        "api": "anthropic-messages",
        "options": { "apiKey": wiring.token, "baseURL": wiring.root, "authMode": "api-key" },
        "models": models,
    });
    let doc = set.yaml(&minimax_path(env))?;
    yaml_set_child(doc, "custom_provider", PROVIDER_ID, Some(&provider));
    doc.set(
        "defaultModel",
        &json!(format!(
            "custom_provider:{PROVIDER_ID}/{}",
            wiring.default_model
        )),
    );
    Ok(())
}

pub const MINIMAX_CODE: AgentSpec = AgentSpec {
    id: "minimax-code",
    name: "MiniMax Code",
    binary: "mcode",
    wire: AgentWire::Anthropic,
    detect: |env| minimax_path(env).parent().is_some_and(Path::exists) || env.has_binary("mcode"),
    config_path: minimax_path,
    write: write_minimax,
    hint: "在 mcode 里用 /model 选 Codex++ 下的模型；接入后重启 mcode",
};

// ---------------------------------------------------------------- Droid (Factory)

const DROID_ID_PREFIX: &str = "custom:codexplus/";

fn droid_path(env: &AgentEnv) -> PathBuf {
    env.dir_var("FACTORY_HOME_OVERRIDE")
        .map(|dir| dir.join(".factory"))
        .unwrap_or_else(|| env.home_path(".factory"))
        .join("settings.json")
}

/// Droid 按模型名猜厂商：名字像 OpenAI / Claude 的模型挂在别的协议上会在每次开会话时告警，
/// 这类模型按它要的协议写，其余走 Chat Completions。
fn droid_provider(model: &str) -> &'static str {
    let name = model
        .rsplit('/')
        .next()
        .unwrap_or(model)
        .to_ascii_lowercase();
    let mut chars = name.chars();
    let o_series = chars.next() == Some('o') && chars.next().is_some_and(|ch| ch.is_ascii_digit());
    if name.starts_with("gpt") || name.contains("codex") || o_series {
        "openai"
    } else if ["claude", "haiku", "sonnet", "opus"]
        .iter()
        .any(|word| name.contains(word))
    {
        "anthropic"
    } else {
        "generic-chat-completion-api"
    }
}

/// `customModels`（BYOK）里每个模型一条，id 统一 `custom:codexplus/<模型>`；
/// `sessionDefaultSettings.model` 选中主模型。
fn write_droid(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(&droid_path(env))?;
    let models = array_entry(root, "customModels");
    models.retain(|entry| {
        !entry
            .get("id")
            .and_then(Value::as_str)
            .is_some_and(|id| id.starts_with(DROID_ID_PREFIX))
    });
    for model in &wiring.models {
        let provider = droid_provider(&model.id);
        let base_url = if provider == "anthropic" {
            wiring.root.clone()
        } else {
            wiring.v1()
        };
        let mut entry = json!({
            "model": model.id,
            "id": format!("{DROID_ID_PREFIX}{}", model.id),
            "displayName": model.id,
            "baseUrl": base_url,
            "apiKey": wiring.token,
            "provider": provider,
            "noImageSupport": true,
        });
        if let Some(window) = model.context_window {
            entry["maxContextLimit"] = json!(window);
            entry["maxOutputTokens"] = json!(output_limit(window));
        }
        models.push(entry);
    }
    object_entry(root, "sessionDefaultSettings").insert(
        "model".to_string(),
        json!(format!("{DROID_ID_PREFIX}{}", wiring.default_model)),
    );
    Ok(())
}

pub const DROID: AgentSpec = AgentSpec {
    id: "droid",
    name: "Droid",
    binary: "droid",
    wire: AgentWire::Chat,
    detect: |env| droid_path(env).parent().is_some_and(Path::exists) || env.has_binary("droid"),
    config_path: droid_path,
    write: write_droid,
    hint: "在 droid 里用 /model 的自定义模型里选 Codex++ 下的模型；新会话生效",
};

// ---------------------------------------------------------------- Cline (CLI)

const CLINE_SLOT: &str = "openai-compatible";

fn cline_data(env: &AgentEnv) -> PathBuf {
    env.dir_var("CLINE_DATA_DIR").unwrap_or_else(|| {
        env.dir_or(&["CLINE_DIR"], || env.home_path(".cline"))
            .join("data")
    })
}

fn cline_path(env: &AgentEnv) -> PathBuf {
    cline_data(env).join("settings").join("providers.json")
}

/// 自建供应商会被 Cline 拒绝，只能占用它内置的 openai-compatible：`providers.json` 里填地址、
/// Key、模型并设为正在用的；`models.json` 里给它列上网关的模型（替换内置那份只有 gpt-4o 的列表）。
fn write_cline(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let settings_dir = cline_data(env).join("settings");
    let root = set.json(&settings_dir.join("providers.json"))?;
    if root.is_empty() {
        root.insert("version".to_string(), json!(1));
    }
    let settings = json!({
        "provider": CLINE_SLOT,
        "apiKey": wiring.token,
        "model": wiring.default_model,
        "baseUrl": wiring.v1(),
    });
    let providers = object_entry(root, "providers");
    // 内容没变就沿用原来的时间戳，免得每次重写都改动文件
    let updated_at = providers
        .get(CLINE_SLOT)
        .filter(|entry| entry.get("settings") == Some(&settings))
        .and_then(|entry| entry.get("updatedAt").cloned())
        .unwrap_or_else(|| json!(iso_now()));
    providers.insert(
        CLINE_SLOT.to_string(),
        json!({ "settings": settings, "updatedAt": updated_at, "tokenSource": "manual" }),
    );
    root.insert("lastUsedProvider".to_string(), json!(CLINE_SLOT));

    let mut models = Map::new();
    for model in &wiring.models {
        let mut entry = json!({
            "id": model.id,
            "name": model.id,
            "capabilities": ["streaming", "tools"],
        });
        if let Some(window) = model.context_window {
            entry["contextWindow"] = json!(window);
            entry["maxTokens"] = json!(output_limit(window));
        }
        models.insert(model.id.clone(), entry);
    }
    let list = set.json(&settings_dir.join("models.json"))?;
    if list.is_empty() {
        list.insert("version".to_string(), json!(1));
    }
    object_entry(list, "providers").insert(
        CLINE_SLOT.to_string(),
        json!({
            "provider": {
                "name": PROVIDER_NAME,
                "baseUrl": wiring.v1(),
                "defaultModelId": wiring.default_model,
            },
            "models": models,
        }),
    );
    Ok(())
}

pub const CLINE: AgentSpec = AgentSpec {
    id: "cline",
    name: "Cline",
    binary: "cline",
    wire: AgentWire::Chat,
    detect: |env| cline_data(env).exists() || env.has_binary("cline"),
    config_path: cline_path,
    write: write_cline,
    hint: "Cline 在会话开始时读配置，新开的 cline 会话生效",
};

// ---------------------------------------------------------------- Qoder / Qoder CN

fn qoder_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["QODER_CONFIG_DIR"], || env.home_path(".qoder"))
        .join("settings.json")
}

fn qoder_cn_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["QODERCN_CONFIG_DIR"], || env.home_path(".qoder-cn"))
        .join("settings.json")
}

/// `providers.codexplus`（openai 协议），`model.name` 写成 `codexplus/<模型>`。
fn write_qoder_at(path: &Path, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({
                "model": model.id,
                "displayName": model.id,
                "capabilities": { "tools": true },
            });
            if let Some(window) = model.context_window {
                entry["contextWindow"] = json!(window);
                entry["maxOutputTokens"] = json!(output_limit(window));
            }
            entry
        })
        .collect();
    let root = set.json(path)?;
    object_entry(root, "providers").insert(
        PROVIDER_ID.to_string(),
        json!({
            "displayName": PROVIDER_NAME,
            "protocol": "openai",
            "baseUrl": wiring.v1(),
            "apiKey": wiring.token,
            "model": wiring.default_model,
            "models": models,
        }),
    );
    object_entry(root, "model").insert("name".to_string(), json!(prefixed(&wiring.default_model)));
    Ok(())
}

pub const QODER: AgentSpec = AgentSpec {
    id: "qoder",
    name: "Qoder CLI",
    binary: "qodercli",
    wire: AgentWire::Chat,
    // ~/.qoder 也是 Qoder IDE 插件的目录，只看命令和 settings.json
    detect: |env| qoder_path(env).is_file() || env.has_binary("qodercli"),
    config_path: qoder_path,
    write: |env, wiring, set| write_qoder_at(&qoder_path(env), wiring, set),
    hint: "在 qodercli 里用 /model 选 Codex++ 下的模型（套餐需支持自带 Key）；接入后重开 qodercli",
};

pub const QODER_CN: AgentSpec = AgentSpec {
    id: "qoder-cn",
    name: "Qoder CN CLI",
    binary: "qoderclicn",
    wire: AgentWire::Chat,
    detect: |env| qoder_cn_path(env).is_file() || env.has_binary("qoderclicn"),
    config_path: qoder_cn_path,
    write: |env, wiring, set| write_qoder_at(&qoder_cn_path(env), wiring, set),
    hint: "在 qoderclicn 里用 /model 选 Codex++ 下的模型（套餐需支持自带 Key）；接入后重开 qoderclicn",
};

// ---------------------------------------------------------------- Grok App

/// 数据目录：`$GROK_APP_HOME`，否则各系统的应用数据目录（旧的 `~/.grok-app` 存在时也认）。
fn grok_app_dir(env: &AgentEnv) -> PathBuf {
    if let Some(dir) = env.dir_var("GROK_APP_HOME") {
        return dir;
    }
    let candidates = match env.os {
        Os::Mac => vec![
            env.app_config().join("com.grokapp.grok-app"),
            env.home_path(".grok-app"),
        ],
        Os::Windows => vec![
            env.app_config()
                .join("grokapp")
                .join("grok-app")
                .join("data"),
            env.app_config().join("grokapp").join("grok-app"),
            env.app_config().join("grok-app"),
        ],
        Os::Linux => vec![env.xdg_data().join("grok-app"), env.home_path(".grok-app")],
    };
    candidates
        .iter()
        .find(|dir| dir.is_dir())
        .unwrap_or(&candidates[0])
        .clone()
}

fn grok_app_path(env: &AgentEnv) -> PathBuf {
    grok_app_dir(env).join("agent-home").join("config.toml")
}

/// 「自定义供应商渠道」`[model.codexplus]`，按 Grok App 自己写的键。不改 `[models] default`：
/// 那是它的路由，改成渠道会让 App 丢掉自己的登录；由用户在 App 里点「使用」切过去。
fn write_grok_app(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let app_models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({ "id": model.id, "name": model.id });
            if let Some(window) = model.context_window {
                entry["contextWindow"] = json!(window);
            }
            entry
        })
        .collect();
    let doc = set.toml(&grok_app_path(env))?;
    let channel = toml_table(doc, &["model", PROVIDER_ID])?;
    let fields = [
        ("model", json!(wiring.default_model)),
        ("base_url", json!(wiring.v1())),
        ("name", json!(PROVIDER_NAME)),
        ("api_key", json!(wiring.token)),
        ("api_backend", json!("chat_completions")),
        ("app_provider_mode", json!("generic")),
        ("app_models", json!(serde_json::to_string(&app_models)?)),
        ("app_efforts", json!("[]")),
        ("supports_reasoning_effort", json!(false)),
    ];
    for (key, value) in fields {
        toml_set(channel, key, &value);
    }
    match wiring.window(&wiring.default_model) {
        Some(window) => toml_set(channel, "context_window", &json!(window)),
        None => {
            channel.remove("context_window");
        }
    }
    Ok(())
}

pub const GROK_APP: AgentSpec = AgentSpec {
    id: "grok-app",
    name: "Grok App",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| grok_app_dir(env).exists(),
    config_path: grok_app_path,
    write: write_grok_app,
    hint: "在 Grok App 的设置 › 供应商里对 Codex++ 点「使用」，再在对话里选模型",
};

// ---------------------------------------------------------------- ZCode

/// ZCode 的数据目录：用户在设置里挪过（`setting.json` 的 `dataBaseDir`）就在那里，否则用户目录。
fn zcode_dir(env: &AgentEnv) -> PathBuf {
    let setting = env.home_path(".zcode").join("v2").join("setting.json");
    if let Ok(text) = std::fs::read_to_string(setting)
        && let Ok(value) = parse_jsonc(&text)
        && let Some(base) = value.get("dataBaseDir").and_then(Value::as_str)
        && Path::new(base.trim()).is_absolute()
    {
        return PathBuf::from(base.trim()).join(".zcode");
    }
    env.home_path(".zcode")
}

fn zcode_path(env: &AgentEnv) -> PathBuf {
    zcode_dir(env).join("v2").join("provider_config.json")
}

/// 3.14 起供应商在 `provider_config.json`（规则）；旧版的 `config.json` 存在时也写一份。
/// 模型在 ZCode 自己的选择器里按任务选，这里只负责让 Codex++ 的模型出现在选择器里。
fn write_zcode(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = zcode_dir(env).join("v2");
    let ids: Vec<&str> = wiring.model_ids().collect();
    let is_ours =
        |rule: &Value| rule.get("providerId").and_then(Value::as_str) == Some(PROVIDER_ID);

    let root = set.json(&dir.join("provider_config.json"))?;
    root.entry("schemaVersion").or_insert(json!(1));
    let config = object_entry(root, "config");
    if let Some(order) = config
        .get_mut("providerOrder")
        .and_then(Value::as_array_mut)
        && !order.iter().any(|id| id == PROVIDER_ID)
    {
        order.push(json!(PROVIDER_ID));
    }
    let rules = array_entry(object_entry(config, "providerConfigRules"), "providerRules");
    rules.retain(|rule| !is_ours(rule));
    rules.push(json!({
        "providerId": PROVIDER_ID,
        "providerName": PROVIDER_NAME,
        "enabled": true,
        "config": {
            "group": "standard-personal",
            "access": { "type": "api-key", "apiKey": wiring.token },
            "api": { "type": "anthropic-messages", "baseUrl": wiring.root },
            "personalModelIds": ids,
            "modelOrder": ids,
        },
    }));
    let model_rules = object_entry(config, "modelConfigRules");
    array_entry(model_rules, "manualProviderModelRules");
    let list = array_entry(model_rules, "providerModelRules");
    list.retain(|rule| !is_ours(rule));
    for model in &wiring.models {
        let window = window_or_default(model.context_window);
        list.push(json!({
            "providerId": PROVIDER_ID,
            "modelId": model.id,
            "config": {
                "properties": { "contextWindow": window, "inputFormat": { "supportsImage": false } },
                "optionSpecs": { "maxOutputTokens": { "max": output_limit(window) } },
            },
        }));
    }

    let legacy = dir.join("config.json");
    if legacy.is_file() {
        let mut models = Map::new();
        for model in &wiring.models {
            let window = window_or_default(model.context_window);
            models.insert(
                model.id.clone(),
                json!({
                    "name": model.id,
                    "limit": { "context": window, "output": output_limit(window) },
                    "modalities": { "input": ["text"], "output": ["text"] },
                }),
            );
        }
        object_entry(set.json(&legacy)?, "provider").insert(
            PROVIDER_ID.to_string(),
            json!({
                "name": PROVIDER_NAME,
                "kind": "anthropic",
                "enabled": true,
                "source": "custom",
                "options": { "apiKey": wiring.token, "baseURL": wiring.root },
                "models": models,
            }),
        );
    }
    Ok(())
}

pub const ZCODE: AgentSpec = AgentSpec {
    id: "zcode",
    name: "ZCode",
    binary: "",
    wire: AgentWire::Anthropic,
    detect: |env| zcode_dir(env).exists(),
    config_path: zcode_path,
    write: write_zcode,
    hint: "在 ZCode 的模型选择器里选 Codex++ 下的模型；接入后重启 ZCode",
};

// ---------------------------------------------------------------- WorkBuddy / WorkBuddy AI / CodeBuddy

/// WorkBuddy 与 CodeBuddy 共用的 `models.json` 条目（OpenAI Chat Completions）。
fn buddy_entry(wiring: &Wiring, id: &str, window: Option<u64>) -> Value {
    let window = window.unwrap_or(200_000);
    json!({
        "id": id,
        "name": id,
        "vendor": PROVIDER_ID,
        "apiKey": wiring.token,
        "url": format!("{}/chat/completions", wiring.v1()),
        "maxInputTokens": window,
        "maxOutputTokens": output_limit(window),
        "supportsToolCall": true,
        "supportsImages": false,
        "supportsReasoning": false,
    })
}

fn buddy_model_is_ours(entry: &Value) -> bool {
    entry.get("vendor").and_then(Value::as_str) == Some(PROVIDER_ID)
}

/// 裸数组的旧 Raw 存档与对象的 JSON 存档都只还原我们拥有的模型。
pub(super) fn restore_buddy_models(
    saved: &crate::gateway::restore::FileStash,
    current: Option<&str>,
) -> anyhow::Result<Option<String>> {
    let Some(current) = current else {
        return Ok(None);
    };
    let original = parse_jsonc(saved.before.as_deref().unwrap_or_default())?;
    let mut root = parse_jsonc(current)?;
    let original_models = original
        .as_array()
        .or_else(|| original.get("models").and_then(Value::as_array));
    let models = match &root {
        Value::Array(models) => Some(models),
        Value::Object(object) => object.get("models").and_then(Value::as_array),
        _ => bail!("模型配置不是 JSON 数组或对象，为免覆盖没有改动它"),
    };
    let Some(models) = models else {
        return Ok(Some(current.to_string()));
    };
    let written_models = saved
        .changed
        .iter()
        .find(|key| key.path == ["models"])
        .and_then(|key| key.written.as_ref())
        .and_then(Value::as_array);
    let managed_ids: Vec<&Value> = models
        .iter()
        .chain(written_models.into_iter().flatten())
        .filter(|model| buddy_model_is_ours(model))
        .filter_map(|model| model.get("id"))
        .collect();
    let other_ids: Vec<&Value> = models
        .iter()
        .filter(|model| !buddy_model_is_ours(model))
        .filter_map(|model| model.get("id"))
        .collect();
    let mut restored_models: Vec<Value> = models
        .iter()
        .filter(|model| !buddy_model_is_ours(model))
        .cloned()
        .collect();
    for (index, model) in original_models.into_iter().flatten().enumerate() {
        if buddy_model_is_ours(model) {
            restored_models.insert(index.min(restored_models.len()), model.clone());
        }
    }
    let available = root.get("availableModels").and_then(Value::as_array);
    let written_available = saved
        .changed
        .iter()
        .find(|key| key.path == ["availableModels"])
        .and_then(|key| key.written.as_ref())
        .and_then(Value::as_array);
    let original_available = original.get("availableModels").and_then(Value::as_array);
    let restored_available = available.zip(written_available).map(|(available, written)| {
        available
            .iter()
            .filter(|id| {
                // 只移除网关追加且没有其它供应商使用的 ID；不还原整份白名单。
                !written.contains(id)
                    || original_available.is_some_and(|original| original.contains(id))
                    || !managed_ids.contains(id)
                    || other_ids.contains(id)
            })
            .cloned()
            .collect::<Vec<_>>()
    });
    match &mut root {
        Value::Array(models) => *models = restored_models,
        Value::Object(object) => {
            if restored_models.is_empty() && original.get("models").is_none() {
                object.remove("models");
            } else {
                let restored = if restored_models.is_empty() {
                    original
                        .get("models")
                        .filter(|models| !models.is_array())
                        .cloned()
                        .unwrap_or_else(|| json!([]))
                } else {
                    Value::Array(restored_models)
                };
                object.insert("models".to_string(), restored);
            }
            if let Some(available) = restored_available {
                object.insert("availableModels".to_string(), Value::Array(available));
            }
        }
        _ => unreachable!(),
    }
    if root == original {
        return Ok(saved.before.clone());
    }
    let mut text = serde_json::to_string_pretty(&root)?;
    text.push('\n');
    Ok(Some(text))
}

/// 把 Codex++ 的模型放进 `models.json`：它可以是裸数组，也可以是 `{"models":[…],"availableModels":[…]}`。
/// 裸数组用 Raw 存档，对象用 JSON 存档；断开时都按模型归属恢复。
fn write_buddy_models(path: &Path, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let ours: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| buddy_entry(wiring, &model.id, model.context_window))
        .collect();
    let current = match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error).with_context(|| format!("无法读取 {}", path.display())),
    };
    if let Some(text) = &current
        && let Ok(Value::Array(mut list)) = parse_jsonc(text)
    {
        list.retain(|entry| !buddy_model_is_ours(entry));
        list.extend(ours);
        let mut text = serde_json::to_string_pretty(&Value::Array(list))?;
        text.push('\n');
        *set.raw(path)? = Some(text);
        return Ok(());
    }
    let root = set.json(path)?;
    let models = array_entry(root, "models");
    models.retain(|entry| !buddy_model_is_ours(entry));
    models.extend(ours);
    // 有「显示哪些模型」的白名单时，把 Codex++ 的加进去
    if let Some(available) = root
        .get_mut("availableModels")
        .and_then(Value::as_array_mut)
        && !available.is_empty()
    {
        for id in wiring.model_ids() {
            if !available.iter().any(|value| value == id) {
                available.push(json!(id));
            }
        }
    }
    Ok(())
}

fn workbuddy_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["WORKBUDDY_CONFIG_DIR"], || env.home_path(".workbuddy"))
        .join("models.json")
}

fn workbuddy_ai_path(env: &AgentEnv) -> PathBuf {
    env.home_path(".workbuddy-ai").join("models.json")
}

pub const WORKBUDDY: AgentSpec = AgentSpec {
    id: "workbuddy",
    name: "WorkBuddy",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| workbuddy_path(env).parent().is_some_and(Path::exists),
    config_path: workbuddy_path,
    write: |env, wiring, set| write_buddy_models(&workbuddy_path(env), wiring, set),
    hint: "在 WorkBuddy 的模型选择器里选 Codex++ 的模型；接入后重启 WorkBuddy",
};

pub const WORKBUDDY_AI: AgentSpec = AgentSpec {
    id: "workbuddy-ai",
    name: "WorkBuddy AI",
    binary: "",
    wire: AgentWire::Chat,
    detect: |env| env.home_path(".workbuddy-ai").exists(),
    config_path: workbuddy_ai_path,
    write: |env, wiring, set| write_buddy_models(&workbuddy_ai_path(env), wiring, set),
    hint: "在 WorkBuddy AI 的模型选择器里选 Codex++ 的模型；接入后重启 WorkBuddy AI",
};

fn codebuddy_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["CODEBUDDY_CONFIG_DIR"], || env.home_path(".codebuddy"))
}

fn codebuddy_path(env: &AgentEnv) -> PathBuf {
    codebuddy_dir(env).join("models.json")
}

/// 命令行与 IDE 都读 `models.json`；命令行新会话的模型是 `settings.json` 的 `model`。
fn write_codebuddy(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = codebuddy_dir(env);
    write_buddy_models(&dir.join("models.json"), wiring, set)?;
    set.json(&dir.join("settings.json"))?
        .insert("model".to_string(), json!(wiring.default_model));
    Ok(())
}

pub const CODEBUDDY: AgentSpec = AgentSpec {
    id: "codebuddy",
    name: "CodeBuddy",
    binary: "codebuddy",
    wire: AgentWire::Chat,
    detect: |env| codebuddy_dir(env).exists() || env.has_binary("codebuddy"),
    config_path: codebuddy_path,
    write: write_codebuddy,
    hint: "在 CodeBuddy 里用 /model 选 Codex++ 的模型；新会话生效",
};

// ---------------------------------------------------------------- AtomCode

fn atomcode_dir(env: &AgentEnv) -> PathBuf {
    env.var("ATOMCODE_HOME")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| env.home_path(".atomcode"))
}

fn atomcode_path(env: &AgentEnv) -> PathBuf {
    atomcode_dir(env).join("config.toml")
}

/// 一个账号 `[provider_accounts.codexplus]`，每个模型一张 `[models."codexplus/<模型>"]`，
/// 顶层 `default_provider` / `default_model` 指向主模型那张表。
fn write_atomcode(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.toml(&atomcode_path(env))?;
    let account = toml_table(doc, &["provider_accounts", PROVIDER_ID])?;
    toml_set(account, "provider", &json!("openai-compatible"));
    toml_set(account, "base_url", &json!(wiring.v1()));
    toml_set(account, "api_key", &json!(wiring.token));

    let prefix = format!("{PROVIDER_ID}/");
    let models = toml_table(doc, &["models"])?;
    models.retain(|key, _| !key.starts_with(&prefix));
    for model in &wiring.models {
        let window = window_or_default(model.context_window);
        let mut table = Table::new();
        toml_set(&mut table, "account", &json!(PROVIDER_ID));
        toml_set(&mut table, "model", &json!(model.id));
        toml_set(&mut table, "context_window", &json!(window));
        toml_set(&mut table, "max_tokens", &json!(output_limit(window)));
        toml_set(&mut table, "supports_vision", &json!(false));
        models.insert(&prefixed(&model.id), Item::Table(table));
    }
    let default = prefixed(&wiring.default_model);
    doc.insert("default_provider", Item::Value(default.as_str().into()));
    doc.insert("default_model", Item::Value(default.as_str().into()));
    Ok(())
}

pub const ATOMCODE: AgentSpec = AgentSpec {
    id: "atomcode",
    name: "AtomCode",
    binary: "atomcode",
    wire: AgentWire::Chat,
    detect: |env| atomcode_dir(env).exists() || env.has_binary("atomcode"),
    config_path: atomcode_path,
    write: write_atomcode,
    hint: "在 AtomCode 里用 /model 选 codexplus/ 开头的模型；接入后重启 atomcode",
};

// ---------------------------------------------------------------- Snow CLI

fn snow_dir(env: &AgentEnv) -> PathBuf {
    env.var("SNOW_CONFIG_DIR")
        .map(PathBuf::from)
        .filter(|dir| dir.is_absolute())
        .unwrap_or_else(|| env.home_path(".snow"))
}

fn snow_path(env: &AgentEnv) -> PathBuf {
    snow_dir(env)
        .join("profiles")
        .join(format!("{PROVIDER_ID}.json"))
}

/// 读一个 JSON 文件的根对象；不存在或不是对象时为空。
fn read_json_object(path: &Path) -> Map<String, Value> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|text| parse_jsonc(&text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default()
}

/// 每套 API 配置是 `profiles/<名字>.json`，`active-profile.json` 指定用哪套，`config.json` 是它的副本。
/// 以用户正在用的那套为底（去掉订阅登录）新建 `profiles/codexplus.json`，设为当前并复制到 `config.json`，
/// 与 Snow 自己切换配置档的做法一致。
fn write_snow(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = snow_dir(env);
    let profiles = dir.join("profiles");
    let active_file = dir.join("active-profile.json");
    let active = read_json_object(&active_file)
        .get("activeProfile")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| {
            std::fs::read_to_string(dir.join("active-profile.txt"))
                .ok()
                .map(|text| text.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "default".to_string());
    let base = profiles.join(format!("{active}.json"));
    let mut profile = if base.is_file() {
        read_json_object(&base)
    } else {
        read_json_object(&dir.join("config.json"))
    };
    let snowcfg = object_entry(&mut profile, "snowcfg");
    snowcfg.remove("oauth");
    let context = window_or_default(wiring.window(&wiring.default_model));
    let fields = [
        ("baseUrl", json!(wiring.v1())),
        ("baseUrlMode", json!("base")),
        ("apiKey", json!(wiring.token)),
        ("requestMethod", json!("chat")),
        ("advancedModel", json!(wiring.default_model)),
        ("basicModel", json!(wiring.small_model)),
        ("maxContextTokens", json!(context)),
        ("maxTokens", json!(output_limit(context))),
        ("supportsVision", json!(false)),
    ];
    for (key, value) in fields {
        snowcfg.insert(key.to_string(), value);
    }

    for path in [snow_path(env), dir.join("config.json")] {
        let root = set.json(&path)?;
        root.clear();
        root.extend(profile.clone());
    }
    set.json(&active_file)?
        .insert("activeProfile".to_string(), json!(PROVIDER_ID));
    Ok(())
}

pub const SNOW: AgentSpec = AgentSpec {
    id: "snow",
    name: "Snow CLI",
    binary: "snow",
    wire: AgentWire::Chat,
    detect: |env| snow_dir(env).exists() || env.has_binary("snow"),
    config_path: snow_path,
    write: write_snow,
    hint: "已切到 Codex++ 配置档，接入后重开 snow；Snow App 可用「同步 Snow CLI API 配置」导入",
};

// ---------------------------------------------------------------- Ante

fn ante_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["ANTE_HOME"], || env.home_path(".ante"))
}

fn ante_path(env: &AgentEnv) -> PathBuf {
    ante_dir(env).join("catalog.json")
}

/// `catalog.json` 里一条 OpenAI 兼容供应商（token 放在固定请求头里），`settings.json` 的
/// `provider` / `model` 让 Ante 从它启动。
fn write_ante(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = ante_dir(env);
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|model| {
            let mut entry = json!({ "id": model.id, "display_name": model.id });
            if let Some(window) = model.context_window {
                entry["context_limit"] = json!(window);
                entry["max_tokens"] = json!(output_limit(window));
            }
            entry
        })
        .collect();
    object_entry(set.json(&dir.join("catalog.json"))?, "providers").insert(
        PROVIDER_ID.to_string(),
        json!({
            "display_name": PROVIDER_NAME,
            "base_url": wiring.v1(),
            "wire_style": "OpenAiCompatible",
            "http_headers": { "Authorization": format!("Bearer {}", wiring.token) },
            "preferred_models": models,
        }),
    );
    let settings = set.json(&dir.join("settings.json"))?;
    settings.insert("provider".to_string(), json!(PROVIDER_ID));
    settings.insert("model".to_string(), json!(wiring.default_model));
    Ok(())
}

pub const ANTE: AgentSpec = AgentSpec {
    id: "ante",
    name: "Ante",
    binary: "ante",
    wire: AgentWire::Chat,
    detect: |env| ante_dir(env).exists() || env.has_binary("ante"),
    config_path: ante_path,
    write: write_ante,
    hint: "在 Ante 的模型选择器里选 Codex++ 下的模型；接入后重启 ante",
};

// ---------------------------------------------------------------- 工具：JSON

/// 取对象里的数组，没有（或不是数组）时建一个空的。
fn array_entry<'a>(map: &'a mut Map<String, Value>, key: &str) -> &'a mut Vec<Value> {
    let slot = map
        .entry(key.to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    if !slot.is_array() {
        *slot = Value::Array(Vec::new());
    }
    slot.as_array_mut().expect("刚换成数组")
}

/// JSON 字符串字面量（也是合法的 YAML 双引号字符串）。
fn json_string(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_default()
}

/// 当前 UTC 时间，`2026-01-02T03:04:05.000Z`。
fn iso_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or_default();
    let (year, month, day) = civil_date((secs / 86_400) as i64);
    let rest = secs % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// 1970-01-01 起的天数 → 公历年月日。
fn civil_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

// ---------------------------------------------------------------- 工具：TOML

/// 按路径取（没有就建）一张表；中间建出来的表是隐式的，不单独写表头。
fn toml_table<'a>(doc: &'a mut DocumentMut, path: &[&str]) -> anyhow::Result<&'a mut Table> {
    let mut table = doc.as_table_mut();
    for key in path {
        let item = table.entry(key).or_insert_with(|| {
            let mut created = Table::new();
            created.set_implicit(true);
            Item::Table(created)
        });
        if let Some(inline) = item.as_inline_table() {
            *item = Item::Table(inline.clone().into_table());
        }
        table = item
            .as_table_mut()
            .with_context(|| format!("TOML 里 {key} 不是表，为免改坏没有改动它"))?;
    }
    Ok(table)
}

fn toml_int(number: u64) -> toml_edit::Value {
    i64::try_from(number).unwrap_or(i64::MAX).into()
}

/// JSON 值 → TOML 值（对象写成行内表）。
fn toml_value(value: &Value) -> toml_edit::Value {
    match value {
        Value::Null => "".into(),
        Value::Bool(flag) => (*flag).into(),
        Value::Number(number) => match number.as_i64() {
            Some(int) => int.into(),
            None => number.as_f64().unwrap_or_default().into(),
        },
        Value::String(text) => text.as_str().into(),
        Value::Array(items) => {
            let mut array = toml_edit::Array::new();
            for item in items {
                array.push(toml_value(item));
            }
            toml_edit::Value::Array(array)
        }
        Value::Object(map) => {
            let mut table = InlineTable::new();
            for (key, item) in map {
                table.insert(key, toml_value(item));
            }
            toml_edit::Value::InlineTable(table)
        }
    }
}

fn toml_set(table: &mut Table, key: &str, value: &Value) {
    table.insert(key, Item::Value(toml_value(value)));
}

// ---------------------------------------------------------------- 工具：YAML

fn leading_spaces(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

/// 不是空行也不是注释。
fn is_content(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

/// `key: value` 行的键名（去掉引号）。
fn line_key(text: &str) -> Option<String> {
    let (key, _) = text.split_once(':')?;
    let key = key.trim();
    let unquoted = key
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .or_else(|| {
            key.strip_prefix('\'')
                .and_then(|rest| rest.strip_suffix('\''))
        })
        .unwrap_or(key);
    Some(unquoted.to_string())
}

/// `key: value` 行冒号后面的值（去掉行尾注释）。
fn line_value(text: &str) -> String {
    let Some((_, rest)) = text.split_once(':') else {
        return String::new();
    };
    let rest = rest.trim();
    if rest.starts_with('#') {
        return String::new();
    }
    match rest.find(" #") {
        Some(index) if !rest.starts_with('"') && !rest.starts_with('\'') => {
            rest[..index].trim().to_string()
        }
        _ => rest.to_string(),
    }
}

/// 每个非空行前面加 `width` 个空格。
fn indent_lines(text: &str, width: usize) -> String {
    let pad = " ".repeat(width);
    text.split_inclusive('\n')
        .map(|line| {
            if line.trim().is_empty() {
                line.to_string()
            } else {
                format!("{pad}{line}")
            }
        })
        .collect()
}

/// 每行去掉最多 `width` 个前导空格。
fn dedent_lines(text: &str, width: usize) -> String {
    text.split_inclusive('\n')
        .map(|line| &line[leading_spaces(line).min(width)..])
        .collect()
}

/// 去掉块尾的空行与注释（至少留第一行），与 [`YamlDoc::set_block`] 保留原块尾部的做法对应。
fn strip_trailing_comments(block: &str) -> String {
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let mut end = lines.len();
    while end > 1 && !is_content(lines[end - 1]) {
        end -= 1;
    }
    lines[..end].concat()
}

/// 改 YAML 顶层映射 `top` 下的一个二级键：块里其它行（含注释）原样保留，`value` 为 `None` 时删掉这个键。
/// 顶层键不存在时新建；它原来不是块状映射（标量、行内 `{}`、列表）时整块换掉。
fn yaml_set_child(doc: &mut YamlDoc, top: &str, child: &str, value: Option<&Value>) {
    let whole = |doc: &mut YamlDoc| {
        if let Some(value) = value {
            let mut map = Map::new();
            map.insert(child.to_string(), value.clone());
            doc.set(top, &Value::Object(map));
        }
    };
    let Some(block) = doc.block(top).map(str::to_string) else {
        whole(doc);
        return;
    };
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let first_child = lines[1..].iter().find(|line| is_content(line));
    let mapping = line_value(lines[0]).is_empty()
        && first_child
            .is_none_or(|line| leading_spaces(line) > 0 && !line.trim_start().starts_with('-'));
    if !mapping {
        whole(doc);
        return;
    }
    let width = first_child.map(|line| leading_spaces(line)).unwrap_or(2);
    let rendered = value.map(|value| indent_lines(&yaml_block(child, value), width));

    let start = (1..lines.len()).find(|&index| {
        let line = lines[index];
        is_content(line)
            && leading_spaces(line) == width
            && line_key(line.trim_start()).as_deref() == Some(child)
    });
    let mut out: Vec<String> = lines.iter().map(|line| line.to_string()).collect();
    match start {
        Some(start) => {
            // 这个键的范围：后面缩进更深的行，以及与它同缩进的列表项；之后的空行与注释留给下一个键
            let mut last = start;
            for (index, line) in lines.iter().enumerate().skip(start + 1) {
                if !is_content(line) {
                    continue;
                }
                let indent = leading_spaces(line);
                if indent > width || (indent == width && line.trim_start().starts_with('-')) {
                    last = index;
                } else {
                    break;
                }
            }
            out.splice(start..=last, rendered);
        }
        None => {
            if let Some(rendered) = rendered {
                out.insert(1, rendered);
            }
        }
    }
    doc.set_block(top, strip_trailing_comments(&out.concat()));
}

/// 补丁列表（顶层数组）里 id 为 `id` 的最后一条的 `config` 子块，去掉 4 格缩进后的文本。
/// 没有这条、没有 `config` 或 `config` 写成行内形式时为 `None`。
fn patch_entry_config(text: &str, id: &str) -> Option<String> {
    let mut items: Vec<Vec<&str>> = Vec::new();
    for line in text.split_inclusive('\n') {
        if line.starts_with("- ") || line.trim_end() == "-" {
            items.push(vec![line]);
        } else if let Some(item) = items.last_mut() {
            item.push(line);
        }
    }
    // 条目第一层的内容：首行去掉 `- `，其余行去掉两格缩进
    let first_level = |index: usize, line: &'_ str| -> Option<String> {
        let rest = if index == 0 {
            line.strip_prefix("- ")?
        } else {
            line.strip_prefix("  ")
                .filter(|rest| !rest.starts_with(' '))?
        };
        Some(rest.to_string())
    };
    let item = items.iter().rev().find(|item| {
        item.iter().copied().enumerate().any(|(index, line)| {
            first_level(index, line).is_some_and(|rest| {
                line_key(&rest).as_deref() == Some("id")
                    && line_value(&rest).trim_matches(['"', '\'']) == id
            })
        })
    })?;
    let start = item.iter().copied().enumerate().position(|(index, line)| {
        first_level(index, line).is_some_and(|rest| line_key(&rest).as_deref() == Some("config"))
    })?;
    let header = first_level(start, item[start])?;
    if !line_value(&header).is_empty() {
        return None;
    }
    let mut body = String::new();
    for line in &item[start + 1..] {
        if is_content(line) && leading_spaces(line) <= 2 {
            break;
        }
        body.push_str(line);
    }
    Some(dedent_lines(&body, 4))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::agents::ModelEntry;
    use crate::gateway::agents::files::{AgentStash, restored_text};
    use std::collections::BTreeMap;

    fn wiring(agent: &str) -> Wiring {
        Wiring {
            agent: agent.to_string(),
            root: format!("http://127.0.0.1:57321/gw/{agent}"),
            token: "cpx-gw-test".to_string(),
            default_model: "ds/deepseek-chat".to_string(),
            small_model: "ds/deepseek-chat".to_string(),
            models: vec![
                ModelEntry {
                    id: "ds/deepseek-chat".to_string(),
                    context_window: Some(128_000),
                    reasoning_levels: Vec::new(),
                },
                ModelEntry {
                    id: "group/daily".to_string(),
                    context_window: None,
                    reasoning_levels: Vec::new(),
                },
            ],
        }
    }

    struct Home {
        _temp: tempfile::TempDir,
        env: AgentEnv,
    }

    impl Home {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            // 用例都按 Unix 布局放文件（~/.config、~/.hermes…）；固定成 Linux，
            // 免得在 Windows 上被解析到 %APPDATA% / %LOCALAPPDATA% 下。
            let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
            Self { _temp: temp, env }
        }

        fn put(&self, relative: &str, text: &str) -> PathBuf {
            let path = self.env.home_path(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
            path
        }

        /// 跑一次写入，返回 (相对路径 → 写入后的全文)。
        fn connect(&self, spec: &AgentSpec) -> BTreeMap<String, String> {
            let mut set = FileSet::new();
            (spec.write)(&self.env, &wiring(spec.id), &mut set).unwrap();
            set.changes()
                .unwrap()
                .into_iter()
                .map(|change| {
                    let relative = change
                        .path
                        .strip_prefix(&self.env.home)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/");
                    (relative, change.after.unwrap_or_default())
                })
                .collect()
        }
    }

    #[test]
    fn yaml_child_keeps_comments_and_siblings() {
        let text = "# 头\nmodel:\n  # 默认模型\n  default: \"gpt\"\n\n  # 供应商说明\n  #   多行\n  provider: \"openai-codex\"\n\n  # api_key: x\n\n# 下一段\nother:\n  a: 1\n";
        let mut doc = YamlDoc::parse(text);
        yaml_set_child(&mut doc, "model", "provider", Some(&json!("codexplus")));
        yaml_set_child(&mut doc, "model", "default", Some(&json!("ds/x")));
        yaml_set_child(&mut doc, "model", "base_url", Some(&json!("http://h")));
        let out = doc.render();
        assert_eq!(
            out,
            "# 头\nmodel:\n  base_url: \"http://h\"\n  # 默认模型\n  default: \"ds/x\"\n\n  # 供应商说明\n  #   多行\n  provider: \"codexplus\"\n\n  # api_key: x\n\n# 下一段\nother:\n  a: 1\n"
        );
        // 嵌套值整段替换，删掉也只删这一段
        yaml_set_child(&mut doc, "other", "nested", Some(&json!({ "x": [1, 2] })));
        yaml_set_child(&mut doc, "other", "nested", Some(&json!({ "y": 3 })));
        assert!(
            doc.render()
                .ends_with("other:\n  nested:\n    y: 3\n  a: 1\n")
        );
        yaml_set_child(&mut doc, "other", "nested", None);
        assert!(doc.render().ends_with("other:\n  a: 1\n"));
        // 原来是标量：整块换成映射
        let mut doc = YamlDoc::parse("model: gpt-4\n");
        yaml_set_child(&mut doc, "model", "default", Some(&json!("m")));
        assert_eq!(doc.render(), "model:\n  default: \"m\"\n");
    }

    #[test]
    fn hermes_keeps_comments_and_restores_exactly() {
        let home = Home::new();
        let original = "_config_version: 51\nmodel:\n  # Default model\n  default: \"gpt-6.1-sol\"\n\n  # Inference provider selection:\n  #   \"auto\" - detect\n  provider: \"openai-codex\"\n\n  # api_key: sk-xxx\n\nproviders:\n  mine:\n    base_url: http://mine\n\nterminal:\n  # shell\n  backend: local\n";
        let path = home.put(".hermes/config.yaml", original);
        let files = home.connect(&HERMES);
        let after = &files[".hermes/config.yaml"];
        assert!(after.contains("  # Inference provider selection:\n  #   \"auto\" - detect\n  provider: \"codexplus\"\n"));
        assert!(after.contains("  # Default model\n  default: \"ds/deepseek-chat\"\n"));
        assert!(after.contains("  # api_key: sk-xxx\n"));
        assert!(after.contains("providers:\n  codexplus:\n"));
        assert!(after.contains("    base_url: \"http://127.0.0.1:57321/gw/hermes/v1\"\n"));
        assert!(after.contains("  mine:\n    base_url: http://mine\n"));
        assert!(after.ends_with("terminal:\n  # shell\n  backend: local\n"));

        // 断开：按框架的存档逐字节还原
        let mut set = FileSet::new();
        (HERMES.write)(&home.env, &wiring("hermes"), &mut set).unwrap();
        let mut stash = AgentStash::default();
        for change in set.changes().unwrap() {
            stash.record(&change).unwrap();
        }
        let file = &stash.files[&path];
        assert_eq!(
            restored_text(file, Some(after)).unwrap().as_deref(),
            Some(original)
        );
    }

    #[test]
    fn hermes_follows_active_profile() {
        let home = Home::new();
        home.put(".hermes/active_profile", "work\n");
        home.put(
            ".hermes/profiles/work/config.yaml",
            "model:\n  default: x\n",
        );
        let files = home.connect(&HERMES);
        assert!(files.contains_key(".hermes/profiles/work/config.yaml"));
    }

    #[test]
    fn goose_uses_the_layout_it_finds() {
        let home = Home::new();
        home.put(
            ".config/goose/config.yaml",
            "active_provider: anthropic\nproviders:\n  anthropic:\n    enabled: true\n    model: claude\nextensions:\n  dev: {}\n",
        );
        let files = home.connect(&GOOSE);
        let config = &files[".config/goose/config.yaml"];
        assert!(config.starts_with("active_provider: \"codexplus\"\n"));
        assert!(config.contains("  anthropic:\n    enabled: true\n    model: claude\n"));
        assert!(config.contains("  codexplus:\n    configured: true\n    enabled: true\n    model: \"ds/deepseek-chat\"\n"));
        assert!(config.ends_with("extensions:\n  dev: {}\n"));
        let provider: Value =
            serde_json::from_str(&files[".config/goose/custom_providers/codexplus.json"]).unwrap();
        assert_eq!(provider["base_url"], "http://127.0.0.1:57321/gw/goose/v1");
        assert_eq!(provider["headers"]["Authorization"], "Bearer cpx-gw-test");
        assert_eq!(provider["models"][0]["context_limit"], 128_000);

        let old = Home::new();
        old.put(
            ".config/goose/config.yaml",
            "GOOSE_PROVIDER: openai\nGOOSE_MODEL: gpt-4o\n",
        );
        let files = old.connect(&GOOSE);
        assert_eq!(
            files[".config/goose/config.yaml"],
            "GOOSE_PROVIDER: \"codexplus\"\nGOOSE_MODEL: \"ds/deepseek-chat\"\n"
        );
    }

    #[test]
    fn dsh_writes_home_layer_and_keeps_user_routes() {
        let home = Home::new();
        home.put(
            ".dsh/profiles/desktop/cordis.patch.yml",
            "# 模板\n- id: agent-default-model\n  config:\n    provider: deepseek-account\n    model: deepseek-flash\n- id: llm-pi-ai\n  name: \"@deepseek-ai/dsh-llm-pi-ai\"\n  config:\n    timeoutMs: 1000\n    providers:\n      mine:\n        api: openai-completions\n        baseURL: http://mine/v1\n",
        );
        home.put(".dsh/profiles/web/cordis.patch.yml", "[]\n");
        home.put(
            ".dsh/.credentials.yaml",
            "version: 1\nrecords:\n  a/b:\n    kind: x\n",
        );
        let files = home.connect(&DSH);
        let layer = &files[".dsh/cordis.patch.yml"];
        assert!(layer.starts_with(DSH_HEAD));
        assert!(layer.contains(DSH_BEGIN_LINE));
        assert!(layer.ends_with(&format!("{DSH_END}\n")));
        assert!(
            layer.contains("  config:\n    timeoutMs: 1000\n    providers:\n      codexplus:\n")
        );
        assert!(layer.contains("        baseURL: \"http://127.0.0.1:57321/gw/dsh/v1\"\n"));
        assert!(layer.contains("        apiKeyEnv: \"CODEXPLUS_GATEWAY_KEY\"\n"));
        assert!(layer.contains("      mine:\n        api: openai-completions\n"));
        assert!(layer.contains("- id: agent-default-model\n  name: \"@deepseek-ai/dsh-agent-default-model\"\n  config:\n    provider: \"codexplus\"\n    model: \"ds/deepseek-chat\"\n"));
        assert_eq!(files[".dsh/.env"], "CODEXPLUS_GATEWAY_KEY=cpx-gw-test\n");
        assert_eq!(
            files[".dsh/.credentials.yaml"],
            "version: 1\nrecords:\n  a/b:\n    kind: x\nrefs:\n  CODEXPLUS_GATEWAY_KEY: \"cpx-gw-test\"\n"
        );
        // profile 自己的补丁层不动
        assert!(!files.contains_key(".dsh/profiles/desktop/cordis.patch.yml"));
    }

    #[test]
    fn dsh_rewrites_its_block_in_place() {
        let home = Home::new();
        let user = "- id: ui-chat\n  config:\n    usage: compact\n";
        let path = home.put(".dsh/cordis.patch.yml", user);
        let first = home.connect(&DSH);
        std::fs::write(&path, &first[".dsh/cordis.patch.yml"]).unwrap();
        let second = home.connect(&DSH);
        // 再写一次内容不变，所以没有改动
        assert!(!second.contains_key(".dsh/cordis.patch.yml"));
        let layer = &first[".dsh/cordis.patch.yml"];
        assert!(layer.starts_with(user));
        assert_eq!(layer.matches(DSH_BEGIN).count(), 1);

        let flow = Home::new();
        flow.put(".dsh/cordis.patch.yml", "# 说明\n[]\n");
        let files = flow.connect(&DSH);
        assert!(files[".dsh/cordis.patch.yml"].starts_with(&format!("# 说明\n{DSH_BEGIN}")));

        let odd = Home::new();
        odd.put(".dsh/cordis.patch.yml", "foo: bar\n");
        let mut set = FileSet::new();
        assert!((DSH.write)(&odd.env, &wiring("dsh"), &mut set).is_err());
    }

    #[test]
    fn dsh_restore_keeps_changes_outside_the_gateway_block() {
        let path = Path::new("cordis.patch.yml");
        let old = "- id: user-old\n  config:\n    value: old\n";
        let edited = "- id: user-edited\n  config:\n    value: new\n";
        let added = "- id: user-added\n";
        let block = dsh_block(&wiring("dsh"), None);
        let current = format!("{edited}{block}{added}");
        assert_eq!(
            restore_dsh_patch(path, Some(old), Some(&current)).unwrap(),
            Some(format!("{edited}{added}")),
        );
        let created = format!("{DSH_HEAD}{block}{added}");
        assert_eq!(
            restore_dsh_patch(path, None, Some(&created)).unwrap(),
            Some(added.to_string()),
        );
        assert_eq!(restore_dsh_patch(path, Some(old), None).unwrap(), None);
        assert_eq!(
            restore_dsh_patch(path, Some(old), Some(edited)).unwrap(),
            Some(edited.to_string()),
        );
        assert!(restore_dsh_patch(path, Some(old), Some(DSH_BEGIN_LINE)).is_err());
    }

    #[test]
    fn dsh_restore_without_user_edits_recovers_original_bytes() {
        let path = Path::new("cordis.patch.yml");
        for before in [None, Some(""), Some("# empty\n[]\n"), Some("- id: user")] {
            let home = Home::new();
            if let Some(before) = before {
                home.put(".dsh/cordis.patch.yml", before);
            }
            let files = home.connect(&DSH);
            let written = &files[".dsh/cordis.patch.yml"];
            assert_eq!(restore_dsh_patch(path, before, Some(written)).unwrap(), before.map(str::to_string));
        }
        let original_block = format!("{DSH_BEGIN_LINE}\n- id: old-gateway\n{DSH_END}\n");
        let before = format!("- id: old-user\n{original_block}");
        let current = format!("- id: new-user\n{}- id: later-user\n", dsh_block(&wiring("dsh"), None));
        assert_eq!(
            restore_dsh_patch(path, Some(&before), Some(&current)).unwrap(),
            Some(format!("- id: new-user\n{original_block}- id: later-user\n")),
        );
    }

    #[test]
    fn reasonix_adds_provider_and_access() {
        let home = Home::new();
        home.put(
            ".reasonix/config.toml",
            "default_model = \"deepseek/deepseek-chat\"\n\n[[providers]]\nname = \"deepseek\"\nkind = \"openai\"\n\n[desktop]\nprovider_access = [\"deepseek\"]\n",
        );
        home.put(".reasonix/.env", "DEEPSEEK_API_KEY=x\n");
        let files = home.connect(&REASONIX);
        let config = &files[".reasonix/config.toml"];
        let value: toml::Value = toml::from_str(config).unwrap();
        assert_eq!(
            value["default_model"].as_str(),
            Some("codexplus/ds/deepseek-chat")
        );
        let providers = value["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 2);
        assert_eq!(providers[0]["name"].as_str(), Some("deepseek"));
        assert_eq!(
            providers[1]["base_url"].as_str(),
            Some("http://127.0.0.1:57321/gw/reasonix/v1")
        );
        assert_eq!(providers[1]["api_key_env"].as_str(), Some(REASONIX_KEY_ENV));
        assert_eq!(
            providers[1]["model_overrides"]["ds/deepseek-chat"]["context_window"].as_integer(),
            Some(128_000)
        );
        let access = value["desktop"]["provider_access"].as_array().unwrap();
        assert_eq!(access.len(), 2);
        assert_eq!(
            files[".reasonix/.env"],
            "DEEPSEEK_API_KEY=x\nCODEXPLUS_REASONIX_API_KEY=cpx-gw-test\n"
        );
    }

    #[test]
    fn jcode_keeps_user_tables() {
        let home = Home::new();
        home.put(
            ".jcode/config.toml",
            "# 我的设置\n[provider]\ndefault_provider = \"openai\"\n\n[providers.mine]\ntype = \"openai-compatible\"\nbase_url = \"http://mine\"\n\n[ui]\ntheme = \"dark\"\n",
        );
        let files = home.connect(&JCODE);
        let config = &files[".jcode/config.toml"];
        assert!(config.starts_with("# 我的设置\n"));
        let value: toml::Value = toml::from_str(config).unwrap();
        assert_eq!(
            value["provider"]["default_provider"].as_str(),
            Some("codexplus")
        );
        assert_eq!(
            value["provider"]["default_model"].as_str(),
            Some("ds/deepseek-chat")
        );
        assert_eq!(
            value["providers"]["mine"]["base_url"].as_str(),
            Some("http://mine")
        );
        let ours = &value["providers"]["codexplus"];
        assert_eq!(
            ours["base_url"].as_str(),
            Some("http://127.0.0.1:57321/gw/jcode/v1")
        );
        assert_eq!(ours["requires_api_key"].as_bool(), Some(true));
        assert_eq!(
            ours["models"][1]["context_window"].as_integer(),
            Some(128_000)
        );
        assert_eq!(value["ui"]["theme"].as_str(), Some("dark"));
    }

    #[test]
    fn atomcode_replaces_only_its_model_tables() {
        let home = Home::new();
        home.put(
            ".atomcode/config.toml",
            "default_model = \"mine\"\n\n[models.mine]\nmodel = \"x\"\n\n[models.\"codexplus/old\"]\nmodel = \"old\"\n",
        );
        let files = home.connect(&ATOMCODE);
        let value: toml::Value = toml::from_str(&files[".atomcode/config.toml"]).unwrap();
        assert_eq!(
            value["default_model"].as_str(),
            Some("codexplus/ds/deepseek-chat")
        );
        assert_eq!(
            value["default_provider"].as_str(),
            Some("codexplus/ds/deepseek-chat")
        );
        let models = value["models"].as_table().unwrap();
        assert!(models.contains_key("mine"));
        assert!(!models.contains_key("codexplus/old"));
        assert_eq!(
            models["codexplus/ds/deepseek-chat"]["account"].as_str(),
            Some("codexplus")
        );
        assert_eq!(
            value["provider_accounts"]["codexplus"]["base_url"].as_str(),
            Some("http://127.0.0.1:57321/gw/atomcode/v1")
        );
    }

    #[test]
    fn grok_app_adds_channel_without_route() {
        let home = Home::new();
        let env = AgentEnv::at(home.env.home.clone()).with_os(Os::Linux);
        let path = env.home_path(".local/share/grok-app/agent-home/config.toml");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "[models]\ndefault = \"grok-4\"\n").unwrap();
        let mut set = FileSet::new();
        (GROK_APP.write)(&env, &wiring("grok-app"), &mut set).unwrap();
        let changes = set.changes().unwrap();
        let value: toml::Value = toml::from_str(changes[0].after.as_deref().unwrap()).unwrap();
        assert_eq!(value["models"]["default"].as_str(), Some("grok-4"));
        let channel = &value["model"]["codexplus"];
        assert_eq!(channel["api_backend"].as_str(), Some("chat_completions"));
        assert_eq!(channel["context_window"].as_integer(), Some(128_000));
        let models: Value = serde_json::from_str(channel["app_models"].as_str().unwrap()).unwrap();
        assert_eq!(models[1]["id"], "group/daily");
    }

    #[test]
    fn minimax_and_morph_edit_only_their_keys() {
        let home = Home::new();
        home.put(
            ".minimax/config.yaml",
            "defaultModel: minimax/MiniMax-M2\ncustom_provider:\n  mine:\n    kind: custom\n# 主题\ntheme: dark\n",
        );
        let files = home.connect(&MINIMAX_CODE);
        let config = &files[".minimax/config.yaml"];
        assert!(
            config.starts_with("defaultModel: \"custom_provider:codexplus/ds/deepseek-chat\"\n")
        );
        assert!(config.contains("  mine:\n    kind: custom\n"));
        assert!(config.contains("      baseURL: \"http://127.0.0.1:57321/gw/minimax-code\"\n"));
        assert!(config.ends_with("# 主题\ntheme: dark\n"));

        let morph = Home::new();
        morph.put(
            ".morph/config.yaml",
            "llm:\n  # 厂商\n  inference_provider: openai\n  model: gpt-5\n  context_window_tokens: 400000\n  profiles:\n    cheap:\n      model: mini\n",
        );
        let files = morph.connect(&MISTER_MORPH);
        let config = &files[".morph/config.yaml"];
        assert!(
            config.contains("  # 厂商\n  inference_provider: \"openai_response_compatible\"\n")
        );
        assert!(config.contains("  context_window_tokens: 128000\n"));
        assert!(config.contains("  profiles:\n    cheap:\n      model: mini\n"));
        assert!(config.contains("  endpoint: \"http://127.0.0.1:57321/gw/mister-morph/v1\"\n"));
    }

    #[test]
    fn gemini_sets_auth_model_and_env() {
        let home = Home::new();
        home.put(".gemini/settings.json", "{ \"theme\": \"x\", \"security\": { \"auth\": { \"selectedType\": \"oauth-personal\" } } }");
        home.put(".gemini/.env", "# 我的\nOTHER=1\n");
        let files = home.connect(&GEMINI_CLI);
        let settings: Value = serde_json::from_str(&files[".gemini/settings.json"]).unwrap();
        assert_eq!(settings["theme"], "x");
        assert_eq!(
            settings["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
        assert_eq!(settings["model"]["name"], "ds/deepseek-chat");
        assert_eq!(
            files[".gemini/.env"],
            "# 我的\nOTHER=1\nGOOGLE_GEMINI_BASE_URL=http://127.0.0.1:57321/gw/gemini-cli\nGEMINI_API_KEY=cpx-gw-test\n"
        );
    }

    #[test]
    fn workbuddy_bare_list_stays_a_list() {
        let home = Home::new();
        home.put(
            ".workbuddy/models.json",
            "[{\"id\":\"local\",\"vendor\":\"ollama\"},{\"id\":\"old\",\"vendor\":\"codexplus\"}]",
        );
        let files = home.connect(&WORKBUDDY);
        let list: Value = serde_json::from_str(&files[".workbuddy/models.json"]).unwrap();
        let ids: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["local", "ds/deepseek-chat", "group/daily"]);
        assert_eq!(
            list[1]["url"],
            "http://127.0.0.1:57321/gw/workbuddy/v1/chat/completions"
        );

        let cb = Home::new();
        cb.put(
            ".codebuddy/models.json",
            "{\"models\":[{\"id\":\"mine\"}],\"availableModels\":[\"mine\"]}",
        );
        let files = cb.connect(&CODEBUDDY);
        let models: Value = serde_json::from_str(&files[".codebuddy/models.json"]).unwrap();
        assert_eq!(
            models["availableModels"],
            json!(["mine", "ds/deepseek-chat", "group/daily"])
        );
        assert_eq!(models["models"].as_array().unwrap().len(), 3);
        let settings: Value = serde_json::from_str(&files[".codebuddy/settings.json"]).unwrap();
        assert_eq!(settings["model"], "ds/deepseek-chat");
    }

    #[test]
    fn snow_copies_active_profile() {
        let home = Home::new();
        home.put(".snow/active-profile.json", "{\"activeProfile\":\"work\"}");
        home.put(
            ".snow/profiles/work.json",
            "{\"snowcfg\":{\"baseUrl\":\"https://x\",\"oauth\":{\"t\":1},\"language\":\"zh\"},\"mcp\":{\"a\":1}}",
        );
        let files = home.connect(&SNOW);
        let profile: Value = serde_json::from_str(&files[".snow/profiles/codexplus.json"]).unwrap();
        assert_eq!(
            profile["snowcfg"]["baseUrl"],
            "http://127.0.0.1:57321/gw/snow/v1"
        );
        assert_eq!(profile["snowcfg"]["language"], "zh");
        assert!(profile["snowcfg"].get("oauth").is_none());
        assert_eq!(profile["mcp"]["a"], 1);
        let config: Value = serde_json::from_str(&files[".snow/config.json"]).unwrap();
        assert_eq!(config, profile);
        let active: Value = serde_json::from_str(&files[".snow/active-profile.json"]).unwrap();
        assert_eq!(active["activeProfile"], "codexplus");
        assert!(!files.contains_key(".snow/profiles/work.json"));
    }

    #[test]
    fn zcode_keeps_other_rules() {
        let home = Home::new();
        home.put(
            ".zcode/v2/provider_config.json",
            "{\"schemaVersion\":1,\"config\":{\"providerOrder\":[\"deepseek\"],\"providerConfigRules\":{\"providerRules\":[{\"providerId\":\"deepseek\"}]},\"modelConfigRules\":{\"providerModelRules\":[],\"manualProviderModelRules\":[]}}}",
        );
        let files = home.connect(&ZCODE);
        let value: Value = serde_json::from_str(&files[".zcode/v2/provider_config.json"]).unwrap();
        let config = &value["config"];
        assert_eq!(config["providerOrder"], json!(["deepseek", "codexplus"]));
        let rules = config["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(
            rules[1]["config"]["api"]["baseUrl"],
            "http://127.0.0.1:57321/gw/zcode"
        );
        assert_eq!(
            config["modelConfigRules"]["providerModelRules"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn droid_and_cline_and_qoder() {
        assert_eq!(droid_provider("openai/gpt-5.1"), "openai");
        assert_eq!(droid_provider("o3-mini"), "openai");
        assert_eq!(droid_provider("x/claude-sonnet-4"), "anthropic");
        assert_eq!(
            droid_provider("ds/deepseek-chat"),
            "generic-chat-completion-api"
        );

        let home = Home::new();
        home.put(
            ".factory/settings.json",
            "{\"customModels\":[{\"id\":\"custom:mine\"},{\"id\":\"custom:codexplus/old\"}],\"theme\":\"x\"}",
        );
        let files = home.connect(&DROID);
        let settings: Value = serde_json::from_str(&files[".factory/settings.json"]).unwrap();
        let ids: Vec<&str> = settings["customModels"]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["id"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            [
                "custom:mine",
                "custom:codexplus/ds/deepseek-chat",
                "custom:codexplus/group/daily"
            ]
        );
        assert_eq!(
            settings["sessionDefaultSettings"]["model"],
            "custom:codexplus/ds/deepseek-chat"
        );

        let files = home.connect(&CLINE);
        let providers: Value =
            serde_json::from_str(&files[".cline/data/settings/providers.json"]).unwrap();
        assert_eq!(providers["lastUsedProvider"], "openai-compatible");
        assert_eq!(
            providers["providers"]["openai-compatible"]["settings"]["baseUrl"],
            "http://127.0.0.1:57321/gw/cline/v1"
        );
        let models: Value =
            serde_json::from_str(&files[".cline/data/settings/models.json"]).unwrap();
        assert_eq!(
            models["providers"]["openai-compatible"]["models"]["ds/deepseek-chat"]["contextWindow"],
            128_000
        );

        let files = home.connect(&QODER_CN);
        let settings: Value = serde_json::from_str(&files[".qoder-cn/settings.json"]).unwrap();
        assert_eq!(settings["model"]["name"], "codexplus/ds/deepseek-chat");
        assert_eq!(
            settings["providers"]["codexplus"]["baseUrl"],
            "http://127.0.0.1:57321/gw/qoder-cn/v1"
        );
    }

    #[test]
    fn civil_date_matches_known_days() {
        assert_eq!(civil_date(0), (1970, 1, 1));
        assert_eq!(civil_date(19_782), (2024, 2, 29));
        assert_eq!(civil_date(20_756), (2026, 10, 30));
    }
}
