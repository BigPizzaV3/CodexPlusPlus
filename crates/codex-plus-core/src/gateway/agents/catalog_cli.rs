//! 通用 Agent 规格：命令行 Agent（第一批）：OpenCode / Pi 系与简单 JSON 配置的 CLI。
//!
//! 写法参考 [`super::catalog_core`]。几类共用的写法：
//! - OpenCode 系（MiMo Code、DevEco Code、OpenChamber）：OpenCode 格式的供应商条目；
//! - Pi 系（Pi、OmO、Prime Agent、StepCode、Aside）：`models.json` 里 Pi 格式的供应商，
//!   `settings.json`（StepCode 是 `config.toml`）里的 `defaultProvider` / `defaultModel`；
//! - 其余各自的格式（omp 的 YAML、Kimi / PenguinHarness 的 TOML、Qwen / Copilot / Crush 的 JSON）。
//!
//! 没接入的：Cursor CLI 与 Devin 只能用各自云端的模型，配置里没有自定义地址；fx 与 Muse Code
//! 的自定义供应商只能不带 key 或从环境变量取 key，网关要求每个 Agent 带自己的 token，写文件做不到。

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde_json::{Map, Value, json};
use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table};

use crate::gateway::agents::catalog_core::{
    json_or_jsonc, opencode_provider, output_limit, write_opencode_style,
};
use crate::gateway::agents::files::{parse_jsonc, yaml_block};
use crate::gateway::agents::{
    AgentEnv, AgentSpec, AgentWire, FileSet, ModelEntry, Os, PROVIDER_ID, PROVIDER_NAME, Wiring,
    YamlDoc, object_entry,
};

pub static SPECS: &[AgentSpec] = &[
    OPENCHAMBER,
    MIMO_CODE,
    DEVECO_CODE,
    PI,
    OMP,
    OMO,
    PRIME_AGENT,
    STEPCODE,
    MOTHX,
    PENGUIN,
    ASIDE,
    COPILOT_CLI,
    CRUSH,
    COMMAND_CODE,
    KIMI_CODE,
    QWEN_CODE,
];

// ---------------------------------------------------------------- 共用小工具

/// 网关的模型在「按供应商区分」的 Agent 里的写法：`codexplus/<模型>`。
fn qualified(model: &str) -> String {
    format!("{PROVIDER_ID}/{model}")
}

/// 目录里第一个存在的文件；都不存在时用 `fallback`。
fn first_existing(dir: &Path, names: &[&str], fallback: &str) -> PathBuf {
    names
        .iter()
        .map(|name| dir.join(name))
        .find(|path| path.exists())
        .unwrap_or_else(|| dir.join(fallback))
}

/// OpenCode 系的配置文件：`<名字>.jsonc`、`<名字>.json`、`config.json` 里第一个存在的，都没有时新建 `<名字>.json`。
fn opencode_like_path(dir: &Path, stem: &str) -> PathBuf {
    let jsonc = format!("{stem}.jsonc");
    let json = format!("{stem}.json");
    first_existing(dir, &[&jsonc, &json, "config.json"], &json)
}

/// 环境变量里的绝对路径（`~` 开头按用户目录展开）；相对路径是 Agent 自己工作目录下的，认不出来，不用。
fn abs_dir_var(env: &AgentEnv, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .filter_map(|name| env.dir_var(name))
        .find(|path| path.is_absolute())
}

/// 当前毫秒时间戳（OpenChamber 的偏好项要带）。
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or_default()
}

/// 模型选择范围里的一项是否已经包含 `reference`：同名（可带 `:思考档位` 后缀，不分大小写），
/// 或 `*` / `?` 通配匹配完整名或去掉供应商后的模型名。
fn scope_takes(pattern: &str, reference: &str) -> bool {
    const LEVELS: &[&str] = &[
        "off", "minimal", "low", "medium", "high", "xhigh", "max", "auto",
    ];
    let mut pattern = pattern.trim().to_ascii_lowercase();
    if let Some((head, level)) = pattern.rsplit_once(':')
        && LEVELS.contains(&level)
    {
        pattern = head.to_string();
    }
    let reference = reference.to_ascii_lowercase();
    if pattern == reference {
        return true;
    }
    if !pattern.contains(['*', '?']) {
        return false;
    }
    let bare = reference
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(&reference);
    glob_match(&pattern, &reference) || glob_match(&pattern, bare)
}

/// 只认 `*`（任意串）和 `?`（单个字符）的通配匹配。
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let text: Vec<char> = text.chars().collect();
    let (mut p, mut t) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while t < text.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == text[t]) {
            p += 1;
            t += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, t));
            p += 1;
        } else if let Some((star_p, star_t)) = star {
            p = star_p + 1;
            t = star_t + 1;
            star = Some((star_p, star_t + 1));
        } else {
            return false;
        }
    }
    pattern[p..].iter().all(|ch| *ch == '*')
}

/// JSON 里的模型选择范围（字符串数组）：已有且非空、又没包含 `reference` 时追加上；
/// 没有这个列表时不新建（新建会把用户能用的模型限制成只剩这一个）。
fn json_scope_add(root: &mut Map<String, Value>, key: &str, reference: &str) {
    let Some(Value::Array(items)) = root.get_mut(key) else {
        return;
    };
    if items.is_empty() {
        return;
    }
    let taken = items
        .iter()
        .filter_map(Value::as_str)
        .any(|pattern| scope_takes(pattern, reference));
    if !taken {
        items.push(json!(reference));
    }
}

/// JSON 根对象里的数组（没有时建一个空的；是别的类型时报错，免得覆盖用户的内容）。
fn json_array<'a>(
    root: &'a mut Map<String, Value>,
    key: &str,
    file: &Path,
) -> anyhow::Result<&'a mut Vec<Value>> {
    let slot = root.entry(key.to_string()).or_insert_with(|| json!([]));
    if slot.is_null() {
        *slot = json!([]);
    }
    match slot {
        Value::Array(items) => Ok(items),
        _ => bail!("{} 里的 {key} 不是数组，为免覆盖没有改动它", file.display()),
    }
}

/// TOML 根上的子表（没有时建一个隐式表，只出现 `[父.子]` 表头；内联表转成普通表）。
fn toml_table<'a>(doc: &'a mut DocumentMut, key: &str) -> anyhow::Result<&'a mut Table> {
    let item = doc.as_table_mut().entry(key).or_insert_with(|| {
        let mut table = Table::new();
        table.set_implicit(true);
        Item::Table(table)
    });
    if let Item::Value(toml_edit::Value::InlineTable(inline)) = item {
        let table = std::mem::take(inline).into_table();
        *item = Item::Table(table);
    }
    item.as_table_mut()
        .with_context(|| format!("配置里的 {key} 不是表，为免覆盖没有改动它"))
}

/// TOML 根上的 `[[键]]` 表数组（没有或是空数组时建一个）。
fn toml_array_of_tables<'a>(
    doc: &'a mut DocumentMut,
    key: &str,
) -> anyhow::Result<&'a mut ArrayOfTables> {
    let item = doc
        .as_table_mut()
        .entry(key)
        .or_insert_with(|| Item::ArrayOfTables(ArrayOfTables::new()));
    if item.as_array().is_some_and(|array| array.is_empty()) {
        *item = Item::ArrayOfTables(ArrayOfTables::new());
    }
    item.as_array_of_tables_mut()
        .with_context(|| format!("配置里的 {key} 不是 [[{key}]] 表数组，为免覆盖没有改动它"))
}

/// 由一组键值建一张 TOML 表。
fn toml_table_of(entries: &[(&str, toml_edit::Value)]) -> Table {
    let mut table = Table::new();
    for (key, value) in entries {
        table.insert(key, Item::Value(value.clone()));
    }
    table
}

/// TOML 根上的字符串数组（模型选择范围）：已有且非空、又没包含 `reference` 时追加。
fn toml_scope_add(doc: &mut DocumentMut, key: &str, reference: &str) {
    let Some(array) = doc.get_mut(key).and_then(Item::as_array_mut) else {
        return;
    };
    if array.is_empty() {
        return;
    }
    let taken = array
        .iter()
        .filter_map(toml_edit::Value::as_str)
        .any(|pattern| scope_takes(pattern, reference));
    if !taken {
        array.push(reference);
    }
}

// ---------------------------------------------------------------- YAML 第二层

/// 顶层键块拆成「正文」和尾部的空行 / 注释（后者属于下一段，[`YamlDoc::set_block`] 会自己保留）。
fn split_yaml_trailing(block: &str) -> (String, String) {
    let lines: Vec<&str> = block.split_inclusive('\n').collect();
    let mut end = lines.len();
    while end > 1 {
        let trimmed = lines[end - 1].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            end -= 1;
        } else {
            break;
        }
    }
    (lines[..end].concat(), lines[end..].concat())
}

fn yaml_content_line(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with('#')
}

fn yaml_indent(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn yaml_unquote(text: &str) -> String {
    let text = text.trim();
    if text.len() >= 2
        && ((text.starts_with('"') && text.ends_with('"'))
            || (text.starts_with('\'') && text.ends_with('\'')))
    {
        return text[1..text.len() - 1].to_string();
    }
    text.to_string()
}

/// 键名那一行冒号后面的内容（去掉行尾注释）。
fn yaml_inline_value(header: &str) -> String {
    let value = header.split_once(':').map(|(_, rest)| rest).unwrap_or("");
    let value = value.trim();
    if value.starts_with('"') || value.starts_with('\'') {
        return value.to_string();
    }
    match value.find(" #") {
        Some(index) => value[..index].trim().to_string(),
        None if value.starts_with('#') => String::new(),
        None => value.to_string(),
    }
}

/// 给生成的 YAML 块每行加缩进。
fn yaml_indented(block: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    block
        .split_inclusive('\n')
        .map(|line| format!("{pad}{line}"))
        .collect()
}

/// 把顶层映射 `top` 下的一个子键 `child` 整块换成 `value`，其余子键（含注释）原样保留。
/// 顶层键不存在时新建；它是行内写法（`{a: 1}`）或列表时报错，免得覆盖用户的内容。
fn yaml_set_child(doc: &mut YamlDoc, top: &str, child: &str, value: &Value) -> anyhow::Result<()> {
    let Some(block) = doc.block(top).map(str::to_string) else {
        let mut map = Map::new();
        map.insert(child.to_string(), value.clone());
        doc.set(top, &Value::Object(map));
        return Ok(());
    };
    let (body, _) = split_yaml_trailing(&block);
    let mut lines = body.split_inclusive('\n');
    let header = lines.next().unwrap_or_default();
    let inline = yaml_inline_value(header);
    let header = match inline.as_str() {
        "" => {
            let mut header = header.to_string();
            if !header.ends_with('\n') {
                header.push('\n');
            }
            header
        }
        "{}" | "null" | "~" => format!("{}:\n", header.split_once(':').map_or(top, |(key, _)| key)),
        _ => bail!("YAML 里的 {top} 不是按行展开的映射，为免覆盖没有改动它"),
    };
    let rest: Vec<&str> = lines.collect();
    let indent = rest
        .iter()
        .find(|line| yaml_content_line(line))
        .map(|line| yaml_indent(line))
        .unwrap_or(2);
    // 子项：(键名, 原文)；第一个子键之前的注释记成没有键名的一项。
    let mut items: Vec<(Option<String>, String)> = Vec::new();
    for line in rest {
        if yaml_content_line(line) && yaml_indent(line) == indent {
            let trimmed = line.trim_start();
            if trimmed.starts_with('-') {
                bail!("YAML 里的 {top} 是列表，不是映射，为免覆盖没有改动它");
            }
            let key = trimmed.split_once(':').map(|(key, _)| yaml_unquote(key));
            items.push((key, line.to_string()));
        } else if let Some((_, raw)) = items.last_mut() {
            raw.push_str(line);
        } else {
            items.push((None, line.to_string()));
        }
    }
    let rendered = yaml_indented(&yaml_block(child, value), indent);
    match items
        .iter_mut()
        .find(|(key, _)| key.as_deref() == Some(child))
    {
        Some((_, raw)) => {
            let (_, trailing) = split_yaml_trailing(raw);
            *raw = rendered + &trailing;
        }
        None => {
            if let Some((_, raw)) = items.last_mut()
                && !raw.ends_with('\n')
            {
                raw.push('\n');
            }
            items.push((Some(child.to_string()), rendered));
        }
    }
    let mut out = header;
    for (_, raw) in items {
        out.push_str(&raw);
    }
    doc.set_block(top, out);
    Ok(())
}

/// YAML 顶层的模型选择范围（字符串列表）：已有且非空、又没包含 `reference` 时追加一项，
/// 列表的写法（展开或行内）和注释原样保留。认不出的写法不动。
fn yaml_scope_add(doc: &mut YamlDoc, top: &str, reference: &str) {
    let Some(block) = doc.block(top).map(str::to_string) else {
        return;
    };
    let (body, _) = split_yaml_trailing(&block);
    let mut lines = body.split_inclusive('\n');
    let header = lines.next().unwrap_or_default();
    let inline = yaml_inline_value(header);
    let quoted = serde_json::to_string(reference).unwrap_or_default();
    if let Some(inner) = inline
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
    {
        let items: Vec<String> = inner
            .split(',')
            .map(yaml_unquote)
            .filter(|item| !item.is_empty())
            .collect();
        if items.is_empty() || items.iter().any(|item| scope_takes(item, reference)) {
            return;
        }
        let key = header.split_once(':').map_or(top, |(key, _)| key);
        let mut out = format!(
            "{key}: [{}, {quoted}]\n",
            inner.trim().trim_end_matches(',')
        );
        out.extend(lines);
        doc.set_block(top, out);
        return;
    }
    if !inline.is_empty() {
        return;
    }
    let rest: Vec<&str> = lines.collect();
    let entries: Vec<&str> = rest
        .iter()
        .copied()
        .filter(|line| yaml_content_line(line))
        .collect();
    let Some(first) = entries.first() else {
        return;
    };
    let indent = yaml_indent(first);
    let mut items = Vec::new();
    for line in &entries {
        if yaml_indent(line) != indent {
            continue;
        }
        let Some(item) = line.trim_start().strip_prefix('-') else {
            return;
        };
        let item = match item.find(" #") {
            Some(index) => &item[..index],
            None => item,
        };
        items.push(yaml_unquote(item));
    }
    if items.iter().any(|item| scope_takes(item, reference)) {
        return;
    }
    let mut out = body.clone();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("{}- {quoted}\n", " ".repeat(indent)));
    doc.set_block(top, out);
}

// ---------------------------------------------------------------- OpenCode 系

fn opencode_config_path(env: &AgentEnv) -> PathBuf {
    json_or_jsonc(
        env.dir_or(&["OPENCODE_CONFIG_DIR"], || {
            env.xdg_config().join("opencode")
        }),
        "opencode",
    )
}

fn openchamber_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["OPENCHAMBER_DATA_DIR"], || {
        env.home_path(".config").join("openchamber")
    })
}

/// OpenChamber 在 OpenCode 配置里用的供应商名：与 OpenCode 自己接入时的条目分开，两边各接各断互不影响。
fn openchamber_provider_id() -> String {
    format!("{PROVIDER_ID}-openchamber")
}

/// OpenChamber 偏好项：`{ "value": …, "updatedAt": <毫秒> }`，值没变时不动时间戳。
fn set_openchamber_pref(fields: &mut Map<String, Value>, key: &str, value: Value) {
    if fields.get(key).and_then(|field| field.get("value")) == Some(&value) {
        return;
    }
    fields.insert(
        key.to_string(),
        json!({ "value": value, "updatedAt": now_ms() }),
    );
}

/// OpenChamber 跑的是它自带的 OpenCode，读用户的 OpenCode 配置；它自己的默认模型优先于 OpenCode 的。
/// 供应商写进 OpenCode 配置，默认模型 / 小模型写进 OpenChamber 的偏好（新版 `preferences.json`，
/// 旧版 `settings.json`，两个都在时两个都写）。
fn write_openchamber(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let provider_id = openchamber_provider_id();
    let mut provider = opencode_provider(wiring);
    provider["name"] = json!(format!("{PROVIDER_NAME} · OpenChamber"));
    let opencode = set.json(&opencode_config_path(env))?;
    object_entry(opencode, "provider").insert(provider_id.clone(), provider);

    let model = json!(format!("{provider_id}/{}", wiring.default_model));
    let small = json!(format!("{provider_id}/{}", wiring.small_model));
    let dir = openchamber_dir(env);
    let preferences = dir.join("preferences.json");
    let settings = dir.join("settings.json");
    let has_preferences = preferences.exists();
    if has_preferences {
        let root = set.json(&preferences)?;
        if root.get("version") != Some(&json!(1))
            || !root.get("fields").is_some_and(Value::is_object)
        {
            bail!(
                "{} 不是 OpenChamber 认得的偏好文件，为免覆盖没有改动它",
                preferences.display()
            );
        }
        let fields = object_entry(root, "fields");
        set_openchamber_pref(fields, "defaultModel", model.clone());
        set_openchamber_pref(fields, "smallModelUseDefault", json!(false));
        set_openchamber_pref(fields, "smallModelOverride", small.clone());
    }
    if !has_preferences || settings.exists() {
        let root = set.json(&settings)?;
        root.insert("defaultModel".to_string(), model);
        root.insert("smallModelUseDefault".to_string(), json!(false));
        root.insert("smallModelOverride".to_string(), small);
    }
    Ok(())
}

pub const OPENCHAMBER: AgentSpec = AgentSpec {
    id: "openchamber",
    name: "OpenChamber",
    binary: "openchamber",
    wire: AgentWire::Chat,
    detect: |env| openchamber_dir(env).exists() || env.has_binary("openchamber"),
    config_path: |env| openchamber_dir(env).join("preferences.json"),
    write: write_openchamber,
    hint: "新会话用 Codex++ 的模型，已打开的会话保持原模型；也可在模型列表里选「Codex++ · OpenChamber」下的模型",
};

fn mimo_dir(env: &AgentEnv) -> PathBuf {
    match abs_dir_var(env, &["MIMOCODE_HOME"]) {
        Some(home) => home.join("config"),
        None => env.xdg_config().join("mimocode"),
    }
}

fn mimo_path(env: &AgentEnv) -> PathBuf {
    opencode_like_path(&mimo_dir(env), "mimocode")
}

pub const MIMO_CODE: AgentSpec = AgentSpec {
    id: "mimo-code",
    name: "MiMo Code",
    binary: "mimo",
    wire: AgentWire::Chat,
    detect: |env| mimo_dir(env).exists() || env.has_binary("mimo"),
    config_path: mimo_path,
    write: |env, wiring, set| write_opencode_style(&mimo_path(env), wiring, set),
    hint: "在 MiMo Code 里用 /models 选 Codex++ 下的模型",
};

fn deveco_dir(env: &AgentEnv) -> PathBuf {
    abs_dir_var(env, &["DEVECO_CONFIG_DIR"]).unwrap_or_else(|| env.xdg_config().join("deveco"))
}

fn deveco_path(env: &AgentEnv) -> PathBuf {
    opencode_like_path(&deveco_dir(env), "deveco")
}

pub const DEVECO_CODE: AgentSpec = AgentSpec {
    id: "deveco-code",
    name: "DevEco Code",
    binary: "deveco",
    wire: AgentWire::Chat,
    detect: |env| deveco_dir(env).exists() || env.has_binary("deveco"),
    config_path: deveco_path,
    write: |env, wiring, set| write_opencode_style(&deveco_path(env), wiring, set),
    hint: "在 DevEco Code 里用 /models 选 Codex++ 下的模型",
};

// ---------------------------------------------------------------- Pi 系

/// Pi 格式 `models.json` 里的一个模型。
fn pi_model(entry: &ModelEntry) -> Value {
    let mut model = json!({
        "id": entry.id,
        "name": entry.display_name,
        "reasoning": false,
        "input": ["text"],
    });
    if let Some(window) = entry.context_window {
        model["contextWindow"] = json!(window);
        model["maxTokens"] = json!(output_limit(window));
    }
    model
}

/// Pi 格式的供应商条目（OpenAI Chat）。`sendSessionAffinityHeaders` 让 Pi 在请求里带上会话 id。
fn pi_provider(wiring: &Wiring) -> Value {
    json!({
        "name": PROVIDER_NAME,
        "baseUrl": wiring.v1(),
        "api": "openai-completions",
        "apiKey": wiring.token,
        "compat": { "sendSessionAffinityHeaders": true },
        "models": wiring.models.iter().map(pi_model).collect::<Vec<_>>(),
    })
}

/// Pi 格式 `models.json` 里写入网关供应商，其它供应商不动。
fn write_pi_models(path: &Path, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let root = set.json(path)?;
    object_entry(root, "providers").insert(PROVIDER_ID.to_string(), pi_provider(wiring));
    Ok(())
}

/// Pi 系的写入：`models.json` 的供应商 + `settings.json` 的默认供应商 / 模型。
/// `enabledModels`（Ctrl+P 轮换的范围）有内容时把选中的模型加进去，否则新会话不会用它。
fn write_pi_family(dir: &Path, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    write_pi_models(&dir.join("models.json"), wiring, set)?;
    let settings = set.json(&dir.join("settings.json"))?;
    settings.insert("defaultProvider".to_string(), json!(PROVIDER_ID));
    settings.insert("defaultModel".to_string(), json!(wiring.default_model));
    json_scope_add(settings, "enabledModels", &qualified(&wiring.default_model));
    Ok(())
}

fn pi_dir(env: &AgentEnv) -> PathBuf {
    abs_dir_var(env, &["PI_CODING_AGENT_DIR"]).unwrap_or_else(|| env.home_path(".pi/agent"))
}

pub const PI: AgentSpec = AgentSpec {
    id: "pi",
    name: "Pi",
    binary: "pi",
    wire: AgentWire::Chat,
    detect: |env| pi_dir(env).exists() || env.has_binary("pi"),
    config_path: |env| pi_dir(env).join("settings.json"),
    write: |env, wiring, set| write_pi_family(&pi_dir(env), wiring, set),
    hint: "新开 Pi 会话生效，用 /model 选 codexplus 下的模型",
};

fn omo_dir(env: &AgentEnv) -> PathBuf {
    abs_dir_var(env, &["OMO_CODING_AGENT_DIR", "SENPI_CODING_AGENT_DIR"])
        .unwrap_or_else(|| env.home_path(".omo/agent"))
}

pub const OMO: AgentSpec = AgentSpec {
    id: "omo",
    name: "OmO",
    binary: "omo",
    wire: AgentWire::Chat,
    detect: |env| omo_dir(env).exists() || env.has_binary("omo"),
    config_path: |env| omo_dir(env).join("settings.json"),
    write: |env, wiring, set| write_pi_family(&omo_dir(env), wiring, set),
    hint: "新开 OmO 会话生效，用 /model 选 codexplus 下的模型",
};

fn prime_agent_dir(env: &AgentEnv) -> PathBuf {
    abs_dir_var(env, &["PRIME_AGENT_CODING_AGENT_DIR"])
        .unwrap_or_else(|| env.home_path(".prime/agent"))
}

pub const PRIME_AGENT: AgentSpec = AgentSpec {
    id: "prime-agent",
    name: "Prime Agent",
    binary: "prime-agent",
    wire: AgentWire::Chat,
    detect: |env| prime_agent_dir(env).exists() || env.has_binary("prime-agent"),
    config_path: |env| prime_agent_dir(env).join("settings.json"),
    write: |env, wiring, set| write_pi_family(&prime_agent_dir(env), wiring, set),
    hint: "重启 Prime Agent 后生效，用 /model 选 codexplus 下的模型",
};

/// StepCode 的目录：`STEP_CODING_AGENT_DIR`（绝对路径）的上一级，或用户目录下的
/// `STEPCODE_CONFIG_DIR`，默认 `~/.stepcode`。
fn stepcode_dir(env: &AgentEnv) -> PathBuf {
    if let Some(agent_dir) = abs_dir_var(env, &["STEP_CODING_AGENT_DIR"])
        && let Some(parent) = agent_dir.parent()
    {
        return parent.to_path_buf();
    }
    match env.var("STEPCODE_CONFIG_DIR") {
        Some(name) => env.home.join(env.expand(&name)),
        None => env.home_path(".stepcode"),
    }
}

/// StepCode：`config.toml` 的 `defaultProvider` / `defaultModel`（`enabledModels` 有内容时加上选中的模型），
/// 模型表是 Pi 格式的 `models.json`。
fn write_stepcode(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = stepcode_dir(env);
    write_pi_models(&dir.join("models.json"), wiring, set)?;
    let doc = set.toml(&dir.join("config.toml"))?;
    doc["defaultProvider"] = toml_edit::value(PROVIDER_ID);
    doc["defaultModel"] = toml_edit::value(wiring.default_model.as_str());
    toml_scope_add(doc, "enabledModels", &qualified(&wiring.default_model));
    Ok(())
}

pub const STEPCODE: AgentSpec = AgentSpec {
    id: "stepcode",
    name: "StepCode",
    binary: "step",
    wire: AgentWire::Chat,
    // 只认目录：同名的 `step`（Smallstep 证书工具）很常见。
    detect: |env| stepcode_dir(env).exists(),
    config_path: |env| stepcode_dir(env).join("config.toml"),
    write: write_stepcode,
    hint: "重启 StepCode 或新开会话后生效；项目里的 .stepcode/config.toml 会覆盖全局选择",
};

/// Aside 当前账号的目录：`~/.aside/accounts.json` 的 `currentAccountId`，默认 0 号账号。
fn aside_dir(env: &AgentEnv) -> PathBuf {
    let root = env.home_path(".aside");
    let current = std::fs::read_to_string(root.join("accounts.json"))
        .ok()
        .and_then(|text| parse_jsonc(&text).ok())
        .and_then(|value| value.get("currentAccountId").cloned())
        .and_then(|id| match id {
            Value::Number(number) => number.as_u64(),
            Value::String(text) => text.trim().trim_start_matches('u').parse().ok(),
            _ => None,
        })
        .filter(|id| root.join("u").join(id.to_string()).is_dir())
        .unwrap_or(0);
    root.join("u").join(current.to_string())
}

pub const ASIDE: AgentSpec = AgentSpec {
    id: "aside",
    name: "Aside",
    binary: "aside",
    wire: AgentWire::Chat,
    detect: |env| env.home_path(".aside").exists() || env.has_binary("aside"),
    config_path: |env| aside_dir(env).join("models.json"),
    // 选中的模型在运行中的 Aside 里改（它的设置由应用自己维护），这里只登记模型列表。
    write: |env, wiring, set| write_pi_models(&aside_dir(env).join("models.json"), wiring, set),
    hint: "重启 Aside 后在模型设置里选 Codex++ 下的模型",
};

// ---------------------------------------------------------------- omp（oh-my-pi）

fn omp_root(env: &AgentEnv) -> PathBuf {
    match env.var("PI_CONFIG_DIR") {
        Some(name) => env.home.join(name),
        None => env.home_path(".omp"),
    }
}

/// omp 的 agent 目录：`OMP_PROFILE` / `PI_PROFILE` 指定的档案，或 `PI_CODING_AGENT_DIR`，默认 `~/.omp/agent`。
fn omp_dir(env: &AgentEnv) -> PathBuf {
    let root = omp_root(env);
    let profile = env.var("OMP_PROFILE").or_else(|| env.var("PI_PROFILE"));
    if let Some(profile) = profile.filter(|name| {
        name != "default"
            && name
                .chars()
                .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || "._-".contains(ch))
    }) {
        return root.join("profiles").join(profile).join("agent");
    }
    abs_dir_var(env, &["PI_CODING_AGENT_DIR"]).unwrap_or_else(|| root.join("agent"))
}

/// omp 读 `.yml`，没有时读 `.yaml`。
fn omp_yaml(dir: &Path, stem: &str) -> PathBuf {
    let yml = dir.join(format!("{stem}.yml"));
    let yaml = dir.join(format!("{stem}.yaml"));
    if !yml.exists() && yaml.exists() {
        yaml
    } else {
        yml
    }
}

fn omp_provider(wiring: &Wiring) -> Value {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let mut model =
                json!({ "id": entry.id, "name": entry.display_name, "reasoning": false });
            if let Some(window) = entry.context_window {
                model["contextWindow"] = json!(window);
                model["maxTokens"] = json!(output_limit(window));
            }
            model
        })
        .collect();
    json!({
        "baseUrl": wiring.v1(),
        "api": "openai-completions",
        "apiKey": wiring.token,
        "models": models,
    })
}

/// omp：`models.yml` 的 `providers.codexplus`，`config.yml` 的 `modelRoles.default`；
/// `enabledModels` 有内容时加上选中的模型。只有旧版 `models.json` 时先把它的内容搬进新建的
/// `models.yml`（omp 有了 `models.yml` 就不再读 `models.json`）。
fn write_omp(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = omp_dir(env);
    let models_path = omp_yaml(&dir, "models");
    let legacy = dir.join("models.json");
    let carry = (!models_path.exists())
        .then(|| std::fs::read_to_string(&legacy).ok())
        .flatten()
        .map(|text| parse_jsonc(&text))
        .transpose()
        .with_context(|| format!("无法解析 {}", legacy.display()))?;
    let models = set.yaml(&models_path)?;
    if let Some(Value::Object(old)) = carry
        && !models.has("providers")
    {
        for (key, value) in &old {
            models.set(key, value);
        }
    }
    yaml_set_child(models, "providers", PROVIDER_ID, &omp_provider(wiring))?;

    let reference = qualified(&wiring.default_model);
    let config = set.yaml(&omp_yaml(&dir, "config"))?;
    yaml_set_child(config, "modelRoles", "default", &json!(reference))?;
    yaml_scope_add(config, "enabledModels", &reference);
    Ok(())
}

pub const OMP: AgentSpec = AgentSpec {
    id: "omp",
    name: "oh-my-pi (omp)",
    binary: "omp",
    wire: AgentWire::Chat,
    detect: |env| omp_root(env).exists() || env.has_binary("omp"),
    config_path: |env| omp_yaml(&omp_dir(env), "config"),
    write: write_omp,
    hint: "重启 omp 后生效，用 /model 选 codexplus 下的模型",
};

// ---------------------------------------------------------------- MothX

fn mothx_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["MOTHX_DIR"], || match env.os {
        Os::Windows => env.app_config().join("mothx"),
        _ => env.home_path(".mothx"),
    })
}

/// MothX：`settings.json` 里加 `providers.codexplus`（OpenAI Chat，思考档位按 OpenAI 的
/// `reasoning_effort` 发），并设 `defaultProvider` / `defaultModel`。
fn write_mothx(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let models: Vec<Value> = wiring.models.iter().map(pi_model).collect();
    let root = set.json(&mothx_dir(env).join("settings.json"))?;
    object_entry(root, "providers").insert(
        PROVIDER_ID.to_string(),
        json!({
            "baseUrl": wiring.v1(),
            "api": "openai-chat",
            "apiKey": wiring.token,
            "thinkingFormat": "openai",
            "models": models,
        }),
    );
    root.insert("defaultProvider".to_string(), json!(PROVIDER_ID));
    root.insert("defaultModel".to_string(), json!(wiring.default_model));
    Ok(())
}

pub const MOTHX: AgentSpec = AgentSpec {
    id: "mothx",
    name: "MothX",
    binary: "mothx",
    wire: AgentWire::Chat,
    detect: |env| mothx_dir(env).exists() || env.has_binary("mothx"),
    config_path: |env| mothx_dir(env).join("settings.json"),
    write: write_mothx,
    hint: "重启 MothX 后生效，用 /model 选 codexplus 下的模型",
};

// ---------------------------------------------------------------- PenguinHarness

fn penguin_root(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["PENGUIN_HOME"], || env.home_path(".penguin/data"))
}

fn penguin_path(env: &AgentEnv) -> PathBuf {
    let project = env
        .var("PENGUIN_PROJECT_ID")
        .filter(|id| id != "." && id != ".." && !id.contains(['/', '\\']))
        .unwrap_or_else(|| "default_project".to_string());
    penguin_root(env).join(project).join(".project_config.toml")
}

/// PenguinHarness：项目配置里加 `[providers.codexplus]`（OpenAI Responses），每个模型一条
/// `[[models]]`（用户自己的模型保留），`default_model` 指向主模型。
fn write_penguin(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let doc = set.toml(&penguin_path(env))?;
    let mut default_model = InlineTable::new();
    default_model.insert("provider", PROVIDER_ID.into());
    default_model.insert("model_id", wiring.default_model.as_str().into());
    doc["default_model"] = toml_edit::value(default_model);

    let provider = toml_table_of(&[
        ("base_url", wiring.v1().into()),
        ("client_type", "openai-responses".into()),
        ("api_key", wiring.token.as_str().into()),
    ]);
    toml_table(doc, "providers")?.insert(PROVIDER_ID, Item::Table(provider));

    let models = toml_array_of_tables(doc, "models")?;
    models.retain(|table| table.get("provider").and_then(Item::as_str) != Some(PROVIDER_ID));
    for entry in &wiring.models {
        let mut fields: Vec<(&str, toml_edit::Value)> = vec![
            ("provider", PROVIDER_ID.into()),
            ("model_id", entry.id.as_str().into()),
        ];
        if let Some(window) = entry.context_window {
            fields.push(("context_window", (window as i64).into()));
            fields.push(("max_tokens", (output_limit(window) as i64).into()));
        }
        fields.push(("vision", false.into()));
        models.push(toml_table_of(&fields));
    }
    Ok(())
}

pub const PENGUIN: AgentSpec = AgentSpec {
    id: "penguin",
    name: "PenguinHarness",
    binary: "penguin",
    wire: AgentWire::Responses,
    detect: |env| {
        penguin_root(env).exists()
            || env.home_path(".penguin").exists()
            || env.has_binary("penguin")
    },
    config_path: penguin_path,
    write: write_penguin,
    hint: "PenguinHarness 会自动重读配置，无需重启",
};

// ---------------------------------------------------------------- Copilot CLI

fn copilot_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["COPILOT_HOME"], || env.home_path(".copilot"))
}

fn copilot_providers_path(env: &AgentEnv) -> PathBuf {
    env.dir_var("COPILOT_PROVIDERS_CONFIG")
        .unwrap_or_else(|| copilot_dir(env).join("providers.json"))
}

/// Copilot CLI 自带模型（BYOK）：`providers.json` 的 `providers` / `models` 两个列表里各换上网关的条目
/// （用户自己的保留），`settings.json` 的 `model` 写成 `codexplus/<模型>`。
fn write_copilot(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let providers_path = copilot_providers_path(env);
    let root = set.json(&providers_path)?;
    let providers = json_array(root, "providers", &providers_path)?;
    providers.retain(|item| item.get("name").and_then(Value::as_str) != Some(PROVIDER_ID));
    providers.push(json!({
        "name": PROVIDER_ID,
        "type": "openai",
        "baseUrl": wiring.v1(),
        "apiKey": wiring.token,
        "wireApi": "completions",
    }));
    let models = json_array(root, "models", &providers_path)?;
    models.retain(|item| item.get("provider").and_then(Value::as_str) != Some(PROVIDER_ID));
    for entry in &wiring.models {
        let mut model = json!({ "id": entry.id, "provider": PROVIDER_ID });
        if let Some(window) = entry.context_window {
            let output = output_limit(window);
            model["maxPromptTokens"] = json!(window.saturating_sub(output).max(1));
            model["maxOutputTokens"] = json!(output);
        }
        models.push(model);
    }

    let settings = set.json(&copilot_dir(env).join("settings.json"))?;
    settings.insert("model".to_string(), json!(qualified(&wiring.default_model)));
    Ok(())
}

pub const COPILOT_CLI: AgentSpec = AgentSpec {
    id: "copilot-cli",
    name: "Copilot CLI",
    binary: "copilot",
    wire: AgentWire::Chat,
    // ~/.copilot 也会被 IDE 插件建出来，单有目录不算装了命令行。
    detect: |env| {
        copilot_dir(env).join("settings.json").exists()
            || copilot_providers_path(env).exists()
            || env.has_binary("copilot")
    },
    config_path: |env| copilot_dir(env).join("settings.json"),
    write: write_copilot,
    hint: "重启 Copilot CLI 后生效，用 /model 选 codexplus 下的模型",
};

// ---------------------------------------------------------------- Crush

fn crush_config_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["CRUSH_GLOBAL_CONFIG"], || match env.os {
        Os::Windows => env.local_app_data().join("crush"),
        _ => env.xdg_config().join("crush"),
    })
    .join("crush.json")
}

/// Crush 自己的模型选择器把选择存在数据目录的 `crush.json`，读的时候它盖过配置里的。
fn crush_data_path(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["CRUSH_GLOBAL_DATA"], || match env.os {
        Os::Windows => env.local_app_data().join("crush"),
        _ => env.xdg_data().join("crush"),
    })
    .join("crush.json")
}

/// Crush：配置里加 `providers.codexplus`（OpenAI 兼容），`models.large` / `models.small` 指向网关模型；
/// 数据文件里已有模型选择时也一并改掉，否则它会盖过配置。
fn write_crush(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let models: Vec<Value> = wiring
        .models
        .iter()
        .map(|entry| {
            let window = entry.context_window.unwrap_or(128_000);
            json!({
                "id": entry.id,
                "name": entry.display_name,
                "context_window": window,
                "default_max_tokens": output_limit(window),
            })
        })
        .collect();
    let picks = [
        ("large", wiring.default_model.as_str()),
        ("small", wiring.small_model.as_str()),
    ];
    let config = set.json(&crush_config_path(env))?;
    object_entry(config, "providers").insert(
        PROVIDER_ID.to_string(),
        json!({
            "name": PROVIDER_NAME,
            "type": "openai-compat",
            "base_url": wiring.v1(),
            "api_key": wiring.token,
            "models": models,
        }),
    );
    let selected = object_entry(config, "models");
    for (kind, model) in picks {
        selected.insert(
            kind.to_string(),
            json!({ "provider": PROVIDER_ID, "model": model }),
        );
    }

    let data_path = crush_data_path(env);
    if data_path != crush_config_path(env) && data_path.exists() {
        let data = set.json(&data_path)?;
        if let Some(Value::Object(selected)) = data.get_mut("models") {
            for (kind, model) in picks {
                if selected.contains_key(kind) {
                    selected.insert(
                        kind.to_string(),
                        json!({ "provider": PROVIDER_ID, "model": model }),
                    );
                }
            }
        }
    }
    Ok(())
}

pub const CRUSH: AgentSpec = AgentSpec {
    id: "crush",
    name: "Crush",
    binary: "crush",
    wire: AgentWire::Chat,
    detect: |env| {
        crush_config_path(env).exists() || crush_data_path(env).exists() || env.has_binary("crush")
    },
    config_path: crush_config_path,
    write: write_crush,
    hint: "重启 Crush 后生效，用 Ctrl+L 切换 Codex++ 下的模型",
};

// ---------------------------------------------------------------- Command Code

fn command_code_dir(env: &AgentEnv) -> PathBuf {
    env.home_path(".commandcode")
}

/// Command Code 自带供应商：`providers.json` 的 `provider.codexplus`，`settings.json` 的
/// `model` / `modelProvider`。它拒绝在文件里明文写 key，key 用 `!命令` 的输出给它。
fn write_command_code(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let dir = command_code_dir(env);
    let mut models = Map::new();
    for entry in &wiring.models {
        models.insert(entry.id.clone(), json!({ "name": entry.display_name }));
    }
    let providers = set.json(&dir.join("providers.json"))?;
    object_entry(providers, "provider").insert(
        PROVIDER_ID.to_string(),
        json!({
            "name": PROVIDER_NAME,
            "api": "openai-completions",
            "baseURL": wiring.v1(),
            "apiKey": format!("!echo {}", wiring.token),
            "models": models,
        }),
    );
    let settings = set.json(&dir.join("settings.json"))?;
    settings.insert("model".to_string(), json!(qualified(&wiring.default_model)));
    settings.insert("modelProvider".to_string(), json!(PROVIDER_ID));
    Ok(())
}

pub const COMMAND_CODE: AgentSpec = AgentSpec {
    id: "command-code",
    name: "Command Code",
    binary: "command-code",
    wire: AgentWire::Chat,
    detect: |env| command_code_dir(env).exists() || env.has_binary("command-code"),
    config_path: |env| command_code_dir(env).join("settings.json"),
    write: write_command_code,
    hint: "仍需先用 command-code login 登录；重启后用 /model 选 Codex++ 下的模型",
};

// ---------------------------------------------------------------- Kimi Code

/// Kimi Code 的目录：`KIMI_CODE_HOME`、已有的 `~/.kimi-code`、旧版的 `KIMI_SHARE_DIR` / `~/.kimi`；
/// 都没有时用 `~/.kimi-code`。第二个值表示是不是旧版（kimi-cli）。
fn kimi_dir(env: &AgentEnv) -> (PathBuf, bool) {
    if let Some(dir) = env.dir_var("KIMI_CODE_HOME") {
        return (dir, false);
    }
    let code = env.home_path(".kimi-code");
    if code.is_dir() {
        return (code, false);
    }
    if let Some(dir) = env.dir_var("KIMI_SHARE_DIR") {
        return (dir, true);
    }
    let legacy = env.home_path(".kimi");
    if legacy.is_dir() {
        return (legacy, true);
    }
    (code, false)
}

fn kimi_path(env: &AgentEnv) -> PathBuf {
    kimi_dir(env).0.join("config.toml")
}

/// Kimi Code：`[providers.codexplus]`，每个模型一张 `[models."codexplus/<模型>"]`，`default_model` 指向主模型。
fn write_kimi(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let (dir, legacy) = kimi_dir(env);
    let doc = set.toml(&dir.join("config.toml"))?;
    doc["default_model"] = toml_edit::value(qualified(&wiring.default_model));
    let provider = toml_table_of(&[
        ("type", "kimi".into()),
        ("base_url", wiring.v1().into()),
        ("api_key", wiring.token.as_str().into()),
    ]);
    toml_table(doc, "providers")?.insert(PROVIDER_ID, Item::Table(provider));

    let models = toml_table(doc, "models")?;
    let prefix = format!("{PROVIDER_ID}/");
    // 上次接入写的模型（这次可能已经不在列表里了）先去掉。
    models.retain(|key, item| {
        !(key.starts_with(&prefix)
            && item.get("provider").and_then(Item::as_str) == Some(PROVIDER_ID))
    });
    for entry in &wiring.models {
        let window = entry.context_window.unwrap_or(128_000) as i64;
        let mut fields: Vec<(&str, toml_edit::Value)> = vec![
            ("provider", PROVIDER_ID.into()),
            ("model", entry.id.as_str().into()),
            ("max_context_size", window.into()),
        ];
        if !legacy {
            // 新版 Kimi Code 要显式声明能调工具。
            let mut capabilities = toml_edit::Array::new();
            capabilities.push("tool_use");
            fields.push(("capabilities", capabilities.into()));
        }
        models.insert(&qualified(&entry.id), Item::Table(toml_table_of(&fields)));
    }
    Ok(())
}

pub const KIMI_CODE: AgentSpec = AgentSpec {
    id: "kimi-code",
    name: "Kimi Code",
    binary: "kimi",
    wire: AgentWire::Chat,
    detect: |env| kimi_dir(env).0.exists() || env.has_binary("kimi"),
    config_path: kimi_path,
    write: write_kimi,
    hint: "重启 Kimi Code 后生效，用 /model 选 codexplus 下的模型",
};

// ---------------------------------------------------------------- Qwen Code

/// Qwen Code 的模型条目从这个环境变量取 key，变量的值写在 `settings.json` 的 `env` 里。
const QWEN_ENV_KEY: &str = "CODEXPLUS_QWEN_API_KEY";

fn qwen_dir(env: &AgentEnv) -> PathBuf {
    env.dir_or(&["QWEN_HOME"], || env.home_path(".qwen"))
}

/// Qwen Code：网关的模型作为 `modelProviders.openai` 里的条目（用户自己的保留），key 放在
/// `env.CODEXPLUS_QWEN_API_KEY`，鉴权方式切到 `openai`，`model.name` 指向主模型。
fn write_qwen(env: &AgentEnv, wiring: &Wiring, set: &mut FileSet) -> anyhow::Result<()> {
    let path = qwen_dir(env).join("settings.json");
    let root = set.json(&path)?;
    let providers = object_entry(root, "modelProviders");
    let slot = providers
        .entry("openai".to_string())
        .or_insert_with(|| json!([]));
    if slot.is_null() {
        *slot = json!([]);
    }
    let Value::Array(entries) = slot else {
        bail!(
            "{} 里的 modelProviders.openai 不是数组，为免覆盖没有改动它",
            path.display()
        );
    };
    entries.retain(|item| item.get("envKey").and_then(Value::as_str) != Some(QWEN_ENV_KEY));
    for entry in &wiring.models {
        let mut model = json!({
            "id": entry.id,
            "name": entry.display_name,
            "baseUrl": wiring.v1(),
            "envKey": QWEN_ENV_KEY,
        });
        if let Some(window) = entry.context_window {
            model["generationConfig"] = json!({ "contextWindowSize": window });
        }
        entries.push(model);
    }
    object_entry(root, "env").insert(QWEN_ENV_KEY.to_string(), json!(wiring.token));
    object_entry(object_entry(root, "security"), "auth")
        .insert("selectedType".to_string(), json!("openai"));
    let model = object_entry(root, "model");
    model.insert("name".to_string(), json!(wiring.default_model));
    model.insert("baseUrl".to_string(), json!(wiring.v1()));
    Ok(())
}

pub const QWEN_CODE: AgentSpec = AgentSpec {
    id: "qwen-code",
    name: "Qwen Code",
    binary: "qwen",
    wire: AgentWire::Chat,
    detect: |env| qwen_dir(env).exists() || env.has_binary("qwen"),
    config_path: |env| qwen_dir(env).join("settings.json"),
    write: write_qwen,
    hint: "重启 Qwen Code 后生效，用 /model 切换模型",
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway::agents::files::{AgentStash, restored_text, write_file};

    fn wiring() -> Wiring {
        Wiring {
            agent: "x".to_string(),
            root: "http://127.0.0.1:57321/gw/x".to_string(),
            token: "cpx-gw-x-1".to_string(),
            default_model: "ds/deepseek-chat".to_string(),
            small_model: "ds/deepseek-chat".to_string(),
            models: vec![
                ModelEntry {
                    id: "ds/deepseek-chat".to_string(),
                    display_name: "ds/deepseek-chat".to_string(),
                    context_window: Some(128_000),
                    reasoning_levels: Vec::new(),
                },
                ModelEntry {
                    id: "group/daily".to_string(),
                    display_name: "group/daily".to_string(),
                    context_window: None,
                    reasoning_levels: Vec::new(),
                },
            ],
        }
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap()
    }

    /// 接入：写入函数 → 存档 → 落盘；返回断开用的存档。
    fn connect(spec: &AgentSpec, env: &AgentEnv) -> AgentStash {
        let mut set = FileSet::new();
        (spec.write)(env, &wiring(), &mut set).unwrap();
        let mut stash = AgentStash::default();
        for change in set.changes().unwrap() {
            stash.record(&change).unwrap();
            write_file(&change.path, change.after.as_deref()).unwrap();
        }
        stash
    }

    /// 断开：按存档还原。
    fn disconnect(stash: &AgentStash) {
        for (path, file) in &stash.files {
            let current = std::fs::read_to_string(path).ok();
            let restored = restored_text(file, current.as_deref()).unwrap();
            write_file(path, restored.as_deref()).unwrap();
        }
    }

    #[test]
    fn scope_patterns() {
        assert!(scope_takes("codexplus/ds/m:high", "codexplus/ds/m"));
        assert!(scope_takes("CODEXPLUS/*", "codexplus/ds/m"));
        assert!(scope_takes("ds/*", "codexplus/ds/m"));
        assert!(!scope_takes("anthropic/*", "codexplus/ds/m"));
        assert!(!scope_takes("codexplus/ds", "codexplus/ds/m"));
    }

    #[test]
    fn pi_adds_to_existing_scope_and_keeps_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let dir = temp.path().join(".pi/agent");
        let settings = "{\n  \"defaultProvider\": \"anthropic\",\n  \"enabledModels\": [\"anthropic/claude\"],\n  \"theme\": \"dark\"\n}\n";
        let models = "{ \"providers\": { \"mine\": { \"baseUrl\": \"http://x\" } } }\n";
        write(&dir.join("settings.json"), settings);
        write(&dir.join("models.json"), models);

        let stash = connect(&PI, &env);
        let after: Value = serde_json::from_str(&read(&dir.join("settings.json"))).unwrap();
        assert_eq!(after["defaultProvider"], "codexplus");
        assert_eq!(after["defaultModel"], "ds/deepseek-chat");
        assert_eq!(after["theme"], "dark");
        assert_eq!(
            after["enabledModels"],
            json!(["anthropic/claude", "codexplus/ds/deepseek-chat"])
        );
        let after: Value = serde_json::from_str(&read(&dir.join("models.json"))).unwrap();
        assert_eq!(after["providers"]["mine"]["baseUrl"], "http://x");
        assert_eq!(
            after["providers"]["codexplus"]["baseUrl"],
            "http://127.0.0.1:57321/gw/x/v1"
        );
        assert_eq!(
            after["providers"]["codexplus"]["models"][0]["contextWindow"],
            128_000
        );

        disconnect(&stash);
        assert_eq!(read(&dir.join("settings.json")), settings);
        assert_eq!(read(&dir.join("models.json")), models);
    }

    #[test]
    fn omp_yaml_keeps_user_roles_and_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let dir = temp.path().join(".omp/agent");
        let config = "# omp 配置\nmodelRoles:\n  default: anthropic/claude:high\n  smol: openai/gpt-mini # 小模型\nenabledModels:\n  - anthropic/*\ntheme: dark\n";
        let models = "providers:\n  mine:\n    baseUrl: http://x\n    models:\n      - id: a\n\n# 结尾注释\n";
        write(&dir.join("config.yml"), config);
        write(&dir.join("models.yml"), models);

        let stash = connect(&OMP, &env);
        let after = read(&dir.join("config.yml"));
        assert!(after.starts_with("# omp 配置\nmodelRoles:\n"));
        assert!(after.contains("  default: \"codexplus/ds/deepseek-chat\"\n"));
        assert!(after.contains("  smol: openai/gpt-mini # 小模型\n"));
        assert!(after.contains("  - anthropic/*\n  - \"codexplus/ds/deepseek-chat\"\n"));
        assert!(after.ends_with("theme: dark\n"));
        let after = read(&dir.join("models.yml"));
        assert!(after.contains("  mine:\n    baseUrl: http://x\n    models:\n      - id: a\n"));
        assert!(after.contains("\n  codexplus:\n"));
        assert!(after.contains("\n    apiKey: \"cpx-gw-x-1\"\n"));
        assert!(after.contains("\n    baseUrl: \"http://127.0.0.1:57321/gw/x/v1\"\n"));
        assert!(after.contains("contextWindow: 128000\n"));
        assert!(after.ends_with("\n# 结尾注释\n"));

        // 再接一次：替换自己的条目，不重复追加。
        connect(&OMP, &env);
        assert_eq!(
            read(&dir.join("models.yml")).matches("codexplus:").count(),
            1
        );
        assert_eq!(
            read(&dir.join("config.yml"))
                .matches("codexplus/ds")
                .count(),
            2
        );

        disconnect(&stash);
        assert_eq!(read(&dir.join("config.yml")), config);
        assert_eq!(read(&dir.join("models.yml")), models);
    }

    #[test]
    fn omp_carries_legacy_models_json() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let dir = temp.path().join(".omp/agent");
        write(
            &dir.join("models.json"),
            "{ \"providers\": { \"mine\": { \"baseUrl\": \"http://x\" } } }",
        );
        let stash = connect(&OMP, &env);
        let after = read(&dir.join("models.yml"));
        assert!(after.contains("  mine:\n    baseUrl: \"http://x\"\n"));
        assert!(after.contains("  codexplus:\n"));
        disconnect(&stash);
        assert!(!dir.join("models.yml").exists());
        assert!(dir.join("models.json").exists());
    }

    #[test]
    fn penguin_keeps_user_models() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = penguin_path(&env);
        let original = "default_model = { provider = \"openai\", model_id = \"gpt-5\" }\n\n[providers.openai]\nbase_url = \"https://api.openai.com/v1\"\n\n[[models]]\nprovider = \"openai\"\nmodel_id = \"gpt-5\"\n";
        write(&path, original);

        let stash = connect(&PENGUIN, &env);
        let after: toml::Value = toml::from_str(&read(&path)).unwrap();
        assert_eq!(
            after["default_model"]["provider"].as_str(),
            Some("codexplus")
        );
        assert_eq!(
            after["default_model"]["model_id"].as_str(),
            Some("ds/deepseek-chat")
        );
        assert_eq!(
            after["providers"]["openai"]["base_url"].as_str(),
            Some("https://api.openai.com/v1")
        );
        assert_eq!(
            after["providers"]["codexplus"]["client_type"].as_str(),
            Some("openai-responses")
        );
        let models = after["models"].as_array().unwrap();
        assert_eq!(models.len(), 3);
        assert_eq!(models[0]["model_id"].as_str(), Some("gpt-5"));
        assert_eq!(models[1]["context_window"].as_integer(), Some(128_000));

        // 再接一次不会重复追加模型。
        connect(&PENGUIN, &env);
        let after: toml::Value = toml::from_str(&read(&path)).unwrap();
        assert_eq!(after["models"].as_array().unwrap().len(), 3);

        disconnect(&stash);
        assert_eq!(read(&path), original);
    }

    #[test]
    fn kimi_writes_quoted_model_tables() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = temp.path().join(".kimi/config.toml");
        let original = "default_model = \"kimi-k2\"\n\n[providers.kimi]\ntype = \"kimi\"\n\n[models.kimi-k2]\nprovider = \"kimi\"\nmodel = \"kimi-k2\"\nmax_context_size = 262144\n";
        write(&path, original);

        let stash = connect(&KIMI_CODE, &env);
        let after: toml::Value = toml::from_str(&read(&path)).unwrap();
        assert_eq!(
            after["default_model"].as_str(),
            Some("codexplus/ds/deepseek-chat")
        );
        assert_eq!(after["providers"]["kimi"]["type"].as_str(), Some("kimi"));
        assert_eq!(
            after["providers"]["codexplus"]["base_url"].as_str(),
            Some("http://127.0.0.1:57321/gw/x/v1")
        );
        let model = &after["models"]["codexplus/ds/deepseek-chat"];
        assert_eq!(model["model"].as_str(), Some("ds/deepseek-chat"));
        assert_eq!(model["max_context_size"].as_integer(), Some(128_000));
        // 旧版 kimi-cli（~/.kimi）不写 capabilities。
        assert!(model.get("capabilities").is_none());
        assert_eq!(
            after["models"]["kimi-k2"]["max_context_size"].as_integer(),
            Some(262_144)
        );

        disconnect(&stash);
        assert_eq!(read(&path), original);
    }

    #[test]
    fn qwen_keeps_user_model_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = temp.path().join(".qwen/settings.json");
        let original = "{\n  \"modelProviders\": {\n    \"openai\": [\n      { \"id\": \"mine\", \"envKey\": \"MY_KEY\", \"baseUrl\": \"http://x\" }\n    ]\n  },\n  \"security\": { \"auth\": { \"selectedType\": \"qwen-oauth\" } }\n}\n";
        write(&path, original);

        let stash = connect(&QWEN_CODE, &env);
        let after: Value = serde_json::from_str(&read(&path)).unwrap();
        let entries = after["modelProviders"]["openai"].as_array().unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0]["id"], "mine");
        assert_eq!(entries[1]["envKey"], QWEN_ENV_KEY);
        assert_eq!(entries[1]["generationConfig"]["contextWindowSize"], 128_000);
        assert_eq!(after["env"][QWEN_ENV_KEY], "cpx-gw-x-1");
        assert_eq!(after["security"]["auth"]["selectedType"], "openai");
        assert_eq!(after["model"]["name"], "ds/deepseek-chat");

        disconnect(&stash);
        assert_eq!(read(&path), original);
    }

    #[test]
    fn copilot_keeps_user_providers() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let providers = temp.path().join(".copilot/providers.json");
        let original = "{\n  \"providers\": [{ \"name\": \"ollama\", \"type\": \"openai\", \"baseUrl\": \"http://localhost:11434/v1\" }],\n  \"models\": [{ \"id\": \"qwen3\", \"provider\": \"ollama\" }]\n}\n";
        write(&providers, original);

        let stash = connect(&COPILOT_CLI, &env);
        let after: Value = serde_json::from_str(&read(&providers)).unwrap();
        assert_eq!(after["providers"].as_array().unwrap().len(), 2);
        assert_eq!(after["providers"][1]["name"], "codexplus");
        assert_eq!(after["providers"][1]["apiKey"], "cpx-gw-x-1");
        assert_eq!(after["models"].as_array().unwrap().len(), 3);
        assert_eq!(after["models"][1]["maxOutputTokens"], 32_000);
        let settings: Value =
            serde_json::from_str(&read(&temp.path().join(".copilot/settings.json"))).unwrap();
        assert_eq!(settings["model"], "codexplus/ds/deepseek-chat");

        disconnect(&stash);
        assert_eq!(read(&providers), original);
        assert!(!temp.path().join(".copilot/settings.json").exists());
    }

    #[test]
    fn crush_overrides_data_file_picks() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path()).with_os(Os::Linux);
        let data = temp.path().join(".local/share/crush/crush.json");
        let original = "{\n  \"models\": {\n    \"large\": { \"provider\": \"openai\", \"model\": \"gpt-5\", \"max_tokens\": 9000 }\n  }\n}\n";
        write(&data, original);

        let stash = connect(&CRUSH, &env);
        let after: Value = serde_json::from_str(&read(&data)).unwrap();
        assert_eq!(
            after["models"]["large"],
            json!({ "provider": "codexplus", "model": "ds/deepseek-chat" })
        );
        // 数据文件里原来没有 small，就不加。
        assert!(after["models"].get("small").is_none());
        let config: Value =
            serde_json::from_str(&read(&temp.path().join(".config/crush/crush.json"))).unwrap();
        assert_eq!(config["providers"]["codexplus"]["type"], "openai-compat");
        assert_eq!(config["models"]["small"]["provider"], "codexplus");

        disconnect(&stash);
        assert_eq!(read(&data), original);
    }

    #[test]
    fn openchamber_writes_preferences_and_opencode_provider() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let preferences = temp.path().join(".config/openchamber/preferences.json");
        let original =
            "{\"version\":1,\"fields\":{\"theme\":{\"value\":\"dark\",\"updatedAt\":1}}}";
        write(&preferences, original);
        let opencode = temp.path().join(".config/opencode/opencode.json");
        let opencode_original =
            "{\n  \"provider\": { \"codexplus\": { \"npm\": \"opencode 自己的\" } }\n}\n";
        write(&opencode, opencode_original);

        let stash = connect(&OPENCHAMBER, &env);
        let after: Value = serde_json::from_str(&read(&preferences)).unwrap();
        assert_eq!(after["fields"]["theme"]["value"], "dark");
        assert_eq!(
            after["fields"]["defaultModel"]["value"],
            "codexplus-openchamber/ds/deepseek-chat"
        );
        assert_eq!(after["fields"]["smallModelUseDefault"]["value"], false);
        let after: Value = serde_json::from_str(&read(&opencode)).unwrap();
        // OpenCode 自己接入时的条目不动。
        assert_eq!(after["provider"]["codexplus"]["npm"], "opencode 自己的");
        assert_eq!(
            after["provider"]["codexplus-openchamber"]["options"]["baseURL"],
            "http://127.0.0.1:57321/gw/x/v1"
        );
        // 没有旧版 settings.json 时不新建它。
        assert!(
            !temp
                .path()
                .join(".config/openchamber/settings.json")
                .exists()
        );

        disconnect(&stash);
        assert_eq!(read(&preferences), original);
        assert_eq!(read(&opencode), opencode_original);
    }

    #[test]
    fn stepcode_toml_and_scope() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        let path = temp.path().join(".stepcode/config.toml");
        let original = "# 我的\ndefaultProvider = \"step\"\nenabledModels = [\"step/step-3\"]\n";
        write(&path, original);
        let stash = connect(&STEPCODE, &env);
        let after: toml::Value = toml::from_str(&read(&path)).unwrap();
        assert_eq!(after["defaultProvider"].as_str(), Some("codexplus"));
        assert_eq!(after["defaultModel"].as_str(), Some("ds/deepseek-chat"));
        assert_eq!(after["enabledModels"].as_array().unwrap().len(), 2);
        disconnect(&stash);
        assert_eq!(read(&path), original);
        assert!(!temp.path().join(".stepcode/models.json").exists());
    }

    #[test]
    fn command_code_key_is_a_command() {
        let temp = tempfile::tempdir().unwrap();
        let env = AgentEnv::at(temp.path());
        connect(&COMMAND_CODE, &env);
        let providers: Value =
            serde_json::from_str(&read(&temp.path().join(".commandcode/providers.json"))).unwrap();
        assert_eq!(
            providers["provider"]["codexplus"]["apiKey"],
            "!echo cpx-gw-x-1"
        );
        let settings: Value =
            serde_json::from_str(&read(&temp.path().join(".commandcode/settings.json"))).unwrap();
        assert_eq!(settings["model"], "codexplus/ds/deepseek-chat");
        assert_eq!(settings["modelProvider"], "codexplus");
    }
}
