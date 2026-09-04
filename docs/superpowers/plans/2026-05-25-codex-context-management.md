# Codex 上下文管理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`（推荐）或 `superpowers:executing-plans` 按任务执行。每一步使用 checkbox（`- [ ]`）跟踪。

**Goal:** 为 Codex++ 供应商配置添加 Codex-only 的 MCP、skills、plugins 上下文管理，并支持供应商级上下文勾选、上下文大小和压缩上下文大小。

**Architecture:** 核心层负责 TOML 结构化解析、过滤、合并和校验；Tauri 命令层负责把核心能力暴露给前端并维持 settings 保存流程；前端在供应商配置页展示公共上下文库和每个供应商的选择。切换供应商时只把当前供应商勾选的公共上下文项合并进 live `~/.codex/config.toml`。

**Tech Stack:** Rust、Tauri commands、`toml_edit`、Serde、React + TypeScript、现有 `BackendSettings` / `RelayProfile` / `relay_config`。

---

## 文件结构

- 修改 `crates/codex-plus-core/src/settings.rs`
  - 为 `RelayProfile` 增加 `context_selection`、`context_window`、`auto_compact_limit`，全部带默认值。
- 修改 `crates/codex-plus-core/src/relay_config.rs`
  - 增加 Codex 上下文 TOML 解析、upsert、delete、filter、上下文 token 字段写入。
  - 增加带上下文选择的完整文件切换函数。
- 修改 `crates/codex-plus-core/tests/relay_config.rs`
  - 覆盖公共上下文解析、增删改、过滤、切换写入和数字校验。
- 修改 `apps/codex-plus-manager/src-tauri/src/commands.rs`
  - 增加上下文管理命令和命令层测试。
  - 切换供应商时传入供应商上下文选择和上下文 token 字段。
- 修改 `apps/codex-plus-manager/src-tauri/src/lib.rs`
  - 注册新增 Tauri commands。
- 修改 `apps/codex-plus-manager/src/App.tsx`
  - 增加 TS 类型、默认值、normalize 兼容逻辑、供应商上下文 UI 和新增供应商默认全选逻辑。
- 修改 `apps/codex-plus-manager/src/styles.css`
  - 增加上下文管理区布局和行样式。

---

### Task 1: 设置模型字段

**Files:**
- Modify: `crates/codex-plus-core/src/settings.rs`
- Test: `crates/codex-plus-core/src/settings.rs`
- Modify: `apps/codex-plus-manager/src/App.tsx`

- [ ] **Step 1: 写 Rust 设置默认值失败测试**

在 `crates/codex-plus-core/src/settings.rs` 的 `#[cfg(test)] mod tests` 中增加：

```rust
#[test]
fn relay_profile_context_fields_default_to_empty() {
    let profile = RelayProfile::default();

    assert!(profile.context_selection.mcp_servers.is_empty());
    assert!(profile.context_selection.skills.is_empty());
    assert!(profile.context_selection.plugins.is_empty());
    assert!(profile.context_window.is_empty());
    assert!(profile.auto_compact_limit.is_empty());
}

#[test]
fn relay_profile_context_fields_deserialize_from_camel_case() {
    let profile: RelayProfile = serde_json::from_str(
        r#"{
            "id":"relay-a",
            "name":"供应商 A",
            "contextSelection":{
                "mcpServers":["context7"],
                "skills":["writer"],
                "plugins":["local"]
            },
            "contextWindow":"200000",
            "autoCompactLimit":"160000"
        }"#,
    )
    .unwrap();

    assert_eq!(profile.context_selection.mcp_servers, vec!["context7"]);
    assert_eq!(profile.context_selection.skills, vec!["writer"]);
    assert_eq!(profile.context_selection.plugins, vec!["local"]);
    assert_eq!(profile.context_window, "200000");
    assert_eq!(profile.auto_compact_limit, "160000");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```powershell
cargo test -p codex-plus-core settings::tests::relay_profile_context
```

Expected: 编译失败，提示 `context_selection`、`context_window` 或 `auto_compact_limit` 字段不存在。

- [ ] **Step 3: 实现 Rust 设置字段**

在 `RelayProfile` 上方增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RelayContextSelection {
    #[serde(default)]
    pub mcp_servers: Vec<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub plugins: Vec<String>,
}
```

在 `RelayProfile` 中增加：

```rust
#[serde(rename = "contextSelection", default)]
pub context_selection: RelayContextSelection,
#[serde(rename = "contextWindow", default)]
pub context_window: String,
#[serde(rename = "autoCompactLimit", default)]
pub auto_compact_limit: String,
```

在所有 `RelayProfile { ... }` 构造处补：

```rust
context_selection: RelayContextSelection::default(),
context_window: String::new(),
auto_compact_limit: String::new(),
```

- [ ] **Step 4: 更新前端类型和默认值**

在 `apps/codex-plus-manager/src/App.tsx` 增加：

```ts
type RelayContextSelection = {
  mcpServers: string[];
  skills: string[];
  plugins: string[];
};
```

在 `type RelayProfile` 增加：

```ts
contextSelection: RelayContextSelection;
contextWindow: string;
autoCompactLimit: string;
```

新增默认值 helper：

```ts
const emptyContextSelection = (): RelayContextSelection => ({
  mcpServers: [],
  skills: [],
  plugins: [],
});
```

所有前端 `RelayProfile` 默认对象补：

```ts
contextSelection: emptyContextSelection(),
contextWindow: "",
autoCompactLimit: "",
```

在 `normalizeRelayProfile` 中补：

```ts
contextSelection: normalizeContextSelection(profile.contextSelection),
contextWindow: profile.contextWindow || "",
autoCompactLimit: profile.autoCompactLimit || "",
```

新增 helper：

```ts
function normalizeContextSelection(selection?: Partial<RelayContextSelection>): RelayContextSelection {
  return {
    mcpServers: Array.isArray(selection?.mcpServers) ? selection.mcpServers.map(String) : [],
    skills: Array.isArray(selection?.skills) ? selection.skills.map(String) : [],
    plugins: Array.isArray(selection?.plugins) ? selection.plugins.map(String) : [],
  };
}
```

- [ ] **Step 5: 验证通过**

Run:

```powershell
cargo test -p codex-plus-core settings::tests::relay_profile_context
npm run check
```

Expected: Rust 测试通过，TypeScript 无错误。

- [ ] **Step 6: 提交**

```powershell
git add crates/codex-plus-core/src/settings.rs apps/codex-plus-manager/src/App.tsx
git commit -m "feat: add relay context selection settings"
```

---

### Task 2: 核心 TOML 上下文库操作

**Files:**
- Modify: `crates/codex-plus-core/src/relay_config.rs`
- Test: `crates/codex-plus-core/tests/relay_config.rs`

- [ ] **Step 1: 写失败测试**

在 `crates/codex-plus-core/tests/relay_config.rs` imports 增加：

```rust
use codex_plus_core::relay_config::{
    delete_context_entry_from_common_config, filter_common_config_for_selection,
    list_context_entries_from_common_config, upsert_context_entry_in_common_config,
};
use codex_plus_core::settings::RelayContextSelection;
```

增加测试：

```rust
#[test]
fn lists_codex_context_entries_from_common_config() {
    let entries = list_context_entries_from_common_config(
        r#"[mcp_servers.context7]
command = "npx"
args = ["-y", "@upstash/context7-mcp"]

[skills.writer]
enabled = true

[plugins.local]
path = "plugin.js"
"#,
    )
    .unwrap();

    assert_eq!(entries.mcp_servers[0].id, "context7");
    assert_eq!(entries.mcp_servers[0].summary, r#"command = "npx""#);
    assert_eq!(entries.skills[0].id, "writer");
    assert_eq!(entries.plugins[0].id, "local");
}

#[test]
fn upserts_and_deletes_context_entry_in_common_config() {
    let common = upsert_context_entry_in_common_config(
        "",
        "mcp",
        "context7",
        r#"command = "npx"
args = ["-y", "@upstash/context7-mcp"]
"#,
    )
    .unwrap();

    assert!(common.contains("[mcp_servers.context7]"));
    assert!(common.contains(r#"command = "npx""#));

    let updated = upsert_context_entry_in_common_config(
        &common,
        "mcp",
        "context7",
        r#"command = "bunx""#,
    )
    .unwrap();

    assert!(updated.contains(r#"command = "bunx""#));
    assert!(!updated.contains(r#"command = "npx""#));

    let deleted = delete_context_entry_from_common_config(&updated, "mcp", "context7").unwrap();
    assert!(!deleted.contains("[mcp_servers.context7]"));
}

#[test]
fn filters_common_config_for_supplier_selection() {
    let filtered = filter_common_config_for_selection(
        r#"[mcp_servers.context7]
command = "npx"

[mcp_servers.memory]
command = "memory"

[skills.writer]
enabled = true

[plugins.local]
path = "plugin.js"
"#,
        &RelayContextSelection {
            mcp_servers: vec!["memory".to_string()],
            skills: vec![],
            plugins: vec!["local".to_string()],
        },
    )
    .unwrap();

    assert!(!filtered.contains("[mcp_servers.context7]"));
    assert!(filtered.contains("[mcp_servers.memory]"));
    assert!(!filtered.contains("[skills.writer]"));
    assert!(filtered.contains("[plugins.local]"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```powershell
cargo test -p codex-plus-core --test relay_config lists_codex_context_entries_from_common_config
```

Expected: 编译失败，提示新增函数和类型未定义。

- [ ] **Step 3: 实现 public 类型**

在 `relay_config.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexContextEntry {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub toml_body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CodexContextEntries {
    pub mcp_servers: Vec<CodexContextEntry>,
    pub skills: Vec<CodexContextEntry>,
    pub plugins: Vec<CodexContextEntry>,
}
```

- [ ] **Step 4: 实现 kind 映射和 summary**

在 `relay_config.rs` 增加：

```rust
fn context_table_name(kind: &str) -> anyhow::Result<&'static str> {
    match kind {
        "mcp" | "mcpServer" | "mcpServers" => Ok("mcp_servers"),
        "skill" | "skills" => Ok("skills"),
        "plugin" | "plugins" => Ok("plugins"),
        other => anyhow::bail!("未知上下文类型：{other}"),
    }
}

fn context_kind_name(table: &str) -> &'static str {
    match table {
        "mcp_servers" => "mcp",
        "skills" => "skill",
        "plugins" => "plugin",
        _ => "unknown",
    }
}

fn context_entry_summary(body: &str) -> String {
    body.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .unwrap_or("")
        .chars()
        .take(96)
        .collect()
}
```

- [ ] **Step 5: 实现 list/upsert/delete/filter**

在 `relay_config.rs` 增加 public functions：

```rust
pub fn list_context_entries_from_common_config(
    common_config: &str,
) -> anyhow::Result<CodexContextEntries> {
    let doc = parse_toml_document(common_config)?;
    Ok(CodexContextEntries {
        mcp_servers: list_context_entries_for_table(&doc, "mcp_servers"),
        skills: list_context_entries_for_table(&doc, "skills"),
        plugins: list_context_entries_for_table(&doc, "plugins"),
    })
}

pub fn upsert_context_entry_in_common_config(
    common_config: &str,
    kind: &str,
    id: &str,
    toml_body: &str,
) -> anyhow::Result<String> {
    let id = id.trim();
    if id.is_empty() {
        anyhow::bail!("上下文 id 不能为空");
    }
    let table_name = context_table_name(kind)?;
    let body_doc = parse_toml_document(toml_body)?;
    let mut doc = parse_toml_document(common_config)?;
    if !doc.as_table().contains_key(table_name) {
        doc[table_name] = toml_edit::table();
    }
    if doc[table_name].as_table().is_none() {
        anyhow::bail!("{table_name} 必须是 TOML 表");
    }
    doc[table_name][id] = toml_edit::Item::Table(body_doc.as_table().clone());
    Ok(normalize_optional_toml(doc))
}

pub fn delete_context_entry_from_common_config(
    common_config: &str,
    kind: &str,
    id: &str,
) -> anyhow::Result<String> {
    let table_name = context_table_name(kind)?;
    let mut doc = parse_toml_document(common_config)?;
    if let Some(table) = doc[table_name].as_table_mut() {
        table.remove(id.trim());
        if table.is_empty() {
            doc.as_table_mut().remove(table_name);
        }
    }
    Ok(normalize_optional_toml(doc))
}

pub fn filter_common_config_for_selection(
    common_config: &str,
    selection: &crate::settings::RelayContextSelection,
) -> anyhow::Result<String> {
    let source = parse_toml_document(common_config)?;
    let mut filtered = DocumentMut::new();
    copy_selected_context_table(&source, &mut filtered, "mcp_servers", &selection.mcp_servers);
    copy_selected_context_table(&source, &mut filtered, "skills", &selection.skills);
    copy_selected_context_table(&source, &mut filtered, "plugins", &selection.plugins);
    Ok(normalize_optional_toml(filtered))
}
```

同时实现 private helpers：

```rust
fn list_context_entries_for_table(doc: &DocumentMut, table_name: &str) -> Vec<CodexContextEntry> {
    let Some(table) = doc.get(table_name).and_then(Item::as_table) else {
        return Vec::new();
    };
    table
        .iter()
        .filter_map(|(id, item)| {
            let table = item.as_table()?;
            let body = normalize_optional_toml(DocumentMut::from(table.clone()));
            Some(CodexContextEntry {
                id: id.to_string(),
                kind: context_kind_name(table_name).to_string(),
                title: id.to_string(),
                summary: context_entry_summary(&body),
                toml_body: body,
            })
        })
        .collect()
}

fn copy_selected_context_table(
    source: &DocumentMut,
    target: &mut DocumentMut,
    table_name: &str,
    selected_ids: &[String],
) {
    let Some(source_table) = source.get(table_name).and_then(Item::as_table) else {
        return;
    };
    for id in selected_ids {
        let id = id.trim();
        if id.is_empty() {
            continue;
        }
        if let Some(item) = source_table.get(id) {
            if !target.as_table().contains_key(table_name) {
                target[table_name] = toml_edit::table();
            }
            target[table_name][id] = item.clone();
        }
    }
}
```

If `DocumentMut::from(table.clone())` does not compile with the crate version, implement `table_body_to_string(table: &Table) -> String` by creating `let mut doc = DocumentMut::new(); merge_toml_table_like(doc.as_table_mut(), table); normalize_optional_toml(doc)`.

- [ ] **Step 6: 验证通过**

Run:

```powershell
cargo test -p codex-plus-core --test relay_config lists_codex_context_entries_from_common_config
cargo test -p codex-plus-core --test relay_config upserts_and_deletes_context_entry_in_common_config
cargo test -p codex-plus-core --test relay_config filters_common_config_for_supplier_selection
```

Expected: 三个测试通过。

- [ ] **Step 7: 提交**

```powershell
git add crates/codex-plus-core/src/relay_config.rs crates/codex-plus-core/tests/relay_config.rs
git commit -m "feat: manage codex context toml entries"
```

---

### Task 3: 切换时应用供应商选择和上下文 token 字段

**Files:**
- Modify: `crates/codex-plus-core/src/relay_config.rs`
- Test: `crates/codex-plus-core/tests/relay_config.rs`

- [ ] **Step 1: 写失败测试**

在 `crates/codex-plus-core/tests/relay_config.rs` 增加：

```rust
#[test]
fn apply_relay_files_with_context_selection_writes_only_selected_context() {
    let temp = tempfile::tempdir().unwrap();
    let selection = RelayContextSelection {
        mcp_servers: vec!["memory".to_string()],
        skills: vec![],
        plugins: vec!["local".to_string()],
    };

    codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
        temp.path(),
        r#"model_provider = "CodexPlusPlus"
[model_providers.CodexPlusPlus]
name = "CodexPlusPlus"
wire_api = "responses"
requires_openai_auth = true
base_url = "https://relay.example/v1"
experimental_bearer_token = "sk-new"
"#,
        r#"{"OPENAI_API_KEY":"sk-new"}"#,
        r#"[mcp_servers.context7]
command = "npx"

[mcp_servers.memory]
command = "memory"

[skills.writer]
enabled = true

[plugins.local]
path = "plugin.js"
"#,
        &selection,
        "200000",
        "160000",
    )
    .unwrap();

    let config = std::fs::read_to_string(temp.path().join("config.toml")).unwrap();
    assert!(config.contains("[mcp_servers.memory]"));
    assert!(!config.contains("[mcp_servers.context7]"));
    assert!(!config.contains("[skills.writer]"));
    assert!(config.contains("[plugins.local]"));
    assert!(config.contains("model_context_window = 200000"));
    assert!(config.contains("model_auto_compact_token_limit = 160000"));
}

#[test]
fn apply_relay_files_with_context_rejects_invalid_context_token_values() {
    let temp = tempfile::tempdir().unwrap();
    let selection = RelayContextSelection::default();

    let error = codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
        temp.path(),
        r#"model_provider = "CodexPlusPlus""#,
        r#"{"OPENAI_API_KEY":"sk-new"}"#,
        "",
        &selection,
        "abc",
        "",
    )
    .unwrap_err();

    assert!(error.to_string().contains("上下文大小"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```powershell
cargo test -p codex-plus-core --test relay_config apply_relay_files_with_context
```

Expected: 编译失败，提示 `apply_relay_files_to_home_with_context` 不存在。

- [ ] **Step 3: 实现上下文 token 校验和写入**

在 `relay_config.rs` 增加：

```rust
fn parse_optional_positive_u64(value: &str, label: &str) -> anyhow::Result<Option<u64>> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let parsed = trimmed
        .parse::<u64>()
        .with_context(|| format!("{label}必须是正整数"))?;
    if parsed == 0 {
        anyhow::bail!("{label}必须大于 0");
    }
    Ok(Some(parsed))
}

fn apply_context_limits_to_config(
    config_text: &str,
    context_window: &str,
    auto_compact_limit: &str,
) -> anyhow::Result<String> {
    let mut doc = parse_toml_document(config_text)?;
    if let Some(value) = parse_optional_positive_u64(context_window, "上下文大小")? {
        doc["model_context_window"] = toml_edit::value(value as i64);
    }
    if let Some(value) = parse_optional_positive_u64(auto_compact_limit, "压缩上下文大小")? {
        doc["model_auto_compact_token_limit"] = toml_edit::value(value as i64);
    }
    Ok(normalize_optional_toml(doc))
}
```

- [ ] **Step 4: 实现新切换函数**

在 `relay_config.rs` 增加：

```rust
pub fn apply_relay_files_to_home_with_context(
    home: &Path,
    config_contents: &str,
    auth_contents: &str,
    common_config_contents: &str,
    selection: &crate::settings::RelayContextSelection,
    context_window: &str,
    auto_compact_limit: &str,
) -> anyhow::Result<RelayApplyResult> {
    let selected_common = filter_common_config_for_selection(common_config_contents, selection)?;
    let config_with_common = merge_common_config_into_config(config_contents, &selected_common)?;
    let config_with_limits =
        apply_context_limits_to_config(&config_with_common, context_window, auto_compact_limit)?;
    apply_relay_files_to_home(home, &config_with_limits, auth_contents)
}
```

- [ ] **Step 5: 验证通过**

Run:

```powershell
cargo test -p codex-plus-core --test relay_config apply_relay_files_with_context
```

Expected: 两个新测试通过。

- [ ] **Step 6: 提交**

```powershell
git add crates/codex-plus-core/src/relay_config.rs crates/codex-plus-core/tests/relay_config.rs
git commit -m "feat: apply supplier context during relay switch"
```

---

### Task 4: Tauri 命令层接线

**Files:**
- Modify: `apps/codex-plus-manager/src-tauri/src/commands.rs`
- Modify: `apps/codex-plus-manager/src-tauri/src/lib.rs`
- Test: `apps/codex-plus-manager/src-tauri/src/commands.rs`

- [ ] **Step 1: 写失败测试**

在 `commands.rs` 测试模块增加：

```rust
#[test]
fn context_entry_commands_update_settings_payload() {
    let settings = BackendSettings::default();
    let upsert = upsert_context_entry(ContextEntryRequest {
        settings: settings.clone(),
        kind: "mcp".to_string(),
        id: "context7".to_string(),
        toml_body: "command = \"npx\"\n".to_string(),
    });

    assert_eq!(upsert.status, "ok");
    assert!(upsert
        .payload
        .settings
        .relay_common_config_contents
        .contains("[mcp_servers.context7]"));

    let listed = list_context_entries(ContextSettingsRequest {
        settings: upsert.payload.settings.clone(),
    });
    assert_eq!(listed.payload.entries.mcp_servers[0].id, "context7");

    let deleted = delete_context_entry(ContextDeleteRequest {
        settings: upsert.payload.settings,
        kind: "mcp".to_string(),
        id: "context7".to_string(),
    });
    assert_eq!(deleted.status, "ok");
    assert!(!deleted
        .payload
        .settings
        .relay_common_config_contents
        .contains("[mcp_servers.context7]"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```powershell
cargo test -p codex-plus-manager context_entry_commands_update_settings_payload
```

Expected: 编译失败，提示 request/payload/commands 未定义。

- [ ] **Step 3: 增加 payload 和 request 类型**

在 `commands.rs` 增加：

```rust
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEntriesPayload {
    pub settings: BackendSettings,
    pub entries: codex_plus_core::relay_config::CodexContextEntries,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextSettingsRequest {
    pub settings: BackendSettings,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextEntryRequest {
    pub settings: BackendSettings,
    pub kind: String,
    pub id: String,
    pub toml_body: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextDeleteRequest {
    pub settings: BackendSettings,
    pub kind: String,
    pub id: String,
}
```

- [ ] **Step 4: 增加 commands**

在 `commands.rs` 增加：

```rust
#[tauri::command]
pub fn list_context_entries(request: ContextSettingsRequest) -> CommandResult<ContextEntriesPayload> {
    match codex_plus_core::relay_config::list_context_entries_from_common_config(
        &request.settings.relay_common_config_contents,
    ) {
        Ok(entries) => ok(
            "上下文列表已读取。",
            ContextEntriesPayload {
                settings: request.settings,
                entries,
            },
        ),
        Err(error) => failed(
            &format!("读取上下文列表失败：{error}"),
            ContextEntriesPayload {
                settings: request.settings,
                entries: codex_plus_core::relay_config::CodexContextEntries {
                    mcp_servers: Vec::new(),
                    skills: Vec::new(),
                    plugins: Vec::new(),
                },
            },
        ),
    }
}

#[tauri::command]
pub fn upsert_context_entry(request: ContextEntryRequest) -> CommandResult<ContextEntriesPayload> {
    let mut settings = request.settings;
    match codex_plus_core::relay_config::upsert_context_entry_in_common_config(
        &settings.relay_common_config_contents,
        &request.kind,
        &request.id,
        &request.toml_body,
    ) {
        Ok(common) => {
            settings.relay_common_config_contents = common;
            list_context_entries(ContextSettingsRequest { settings })
        }
        Err(error) => failed(
            &format!("保存上下文失败：{error}"),
            ContextEntriesPayload {
                settings,
                entries: codex_plus_core::relay_config::CodexContextEntries {
                    mcp_servers: Vec::new(),
                    skills: Vec::new(),
                    plugins: Vec::new(),
                },
            },
        ),
    }
}

#[tauri::command]
pub fn delete_context_entry(request: ContextDeleteRequest) -> CommandResult<ContextEntriesPayload> {
    let mut settings = request.settings;
    match codex_plus_core::relay_config::delete_context_entry_from_common_config(
        &settings.relay_common_config_contents,
        &request.kind,
        &request.id,
    ) {
        Ok(common) => {
            settings.relay_common_config_contents = common;
            list_context_entries(ContextSettingsRequest { settings })
        }
        Err(error) => failed(
            &format!("删除上下文失败：{error}"),
            ContextEntriesPayload {
                settings,
                entries: codex_plus_core::relay_config::CodexContextEntries {
                    mcp_servers: Vec::new(),
                    skills: Vec::new(),
                    plugins: Vec::new(),
                },
            },
        ),
    }
}
```

- [ ] **Step 5: 切换命令改用新 core 函数**

在 `apply_relay_injection` 和 `apply_pure_api_injection` 的完整文件分支中，把：

```rust
codex_plus_core::relay_config::apply_relay_files_to_home_with_common(
    &home,
    &relay.config_contents,
    &relay.auth_contents,
    &settings.relay_common_config_contents,
)
```

替换为：

```rust
codex_plus_core::relay_config::apply_relay_files_to_home_with_context(
    &home,
    &relay.config_contents,
    &relay.auth_contents,
    &settings.relay_common_config_contents,
    &relay.context_selection,
    &relay.context_window,
    &relay.auto_compact_limit,
)
```

- [ ] **Step 6: 注册 commands**

在 `apps/codex-plus-manager/src-tauri/src/lib.rs` 的 `generate_handler!` 中加入：

```rust
commands::list_context_entries,
commands::upsert_context_entry,
commands::delete_context_entry,
```

- [ ] **Step 7: 验证通过**

Run:

```powershell
cargo test -p codex-plus-manager context_entry_commands_update_settings_payload
cargo test -p codex-plus-manager
```

Expected: 命令测试和 manager 测试通过。

- [ ] **Step 8: 提交**

```powershell
git add apps/codex-plus-manager/src-tauri/src/commands.rs apps/codex-plus-manager/src-tauri/src/lib.rs
git commit -m "feat: expose codex context management commands"
```

---

### Task 5: 前端上下文管理 UI

**Files:**
- Modify: `apps/codex-plus-manager/src/App.tsx`
- Modify: `apps/codex-plus-manager/src/styles.css`

- [ ] **Step 1: 增加前端类型**

在 `App.tsx` 增加：

```ts
type ContextKind = "mcp" | "skill" | "plugin";

type CodexContextEntry = {
  id: string;
  kind: ContextKind;
  title: string;
  summary: string;
  tomlBody: string;
};

type CodexContextEntries = {
  mcpServers: CodexContextEntry[];
  skills: CodexContextEntry[];
  plugins: CodexContextEntry[];
};

type ContextEntriesResult = CommandResult<{
  settings: BackendSettings;
  entries: CodexContextEntries;
}>;
```

- [ ] **Step 2: 增加本地解析 fallback**

在 `App.tsx` helper 区增加：

```ts
function contextEntriesFromSettings(settings: BackendSettings): CodexContextEntries {
  return {
    mcpServers: parseContextEntries(settings.relayCommonConfigContents, "mcp", "mcp_servers"),
    skills: parseContextEntries(settings.relayCommonConfigContents, "skill", "skills"),
    plugins: parseContextEntries(settings.relayCommonConfigContents, "plugin", "plugins"),
  };
}

function parseContextEntries(commonConfig: string, kind: ContextKind, tableName: string): CodexContextEntry[] {
  const lines = commonConfig.split(/\r?\n/);
  const entries: CodexContextEntry[] = [];
  let currentId = "";
  let body: string[] = [];
  const headerPattern = new RegExp(`^\\s*\\[${tableName.replace("_", "\\_")}\\.([^\\]]+)\\]\\s*$`);
  const anyHeaderPattern = /^\s*\[[^\]]+\]\s*$/;
  const flush = () => {
    if (!currentId) return;
    const tomlBody = body.join("\n").trimEnd() + "\n";
    const summary = tomlBody.split(/\r?\n/).map((line) => line.trim()).find((line) => line && !line.startsWith("#")) || "";
    entries.push({ id: currentId, kind, title: currentId, summary, tomlBody });
  };
  for (const line of lines) {
    const header = headerPattern.exec(line);
    if (header) {
      flush();
      currentId = header[1];
      body = [];
      continue;
    }
    if (anyHeaderPattern.test(line)) {
      flush();
      currentId = "";
      body = [];
      continue;
    }
    if (currentId) body.push(line);
  }
  flush();
  return entries;
}
```

- [ ] **Step 3: 在 `RelayProfileEditor` 增加上下文大小输入**

在 `relay-fields` 中靠近配置模型处增加：

```tsx
<Field className="relay-field-context-window" label="上下文大小">
  <Input
    inputMode="numeric"
    value={profile.contextWindow}
    onChange={(event) => updateDraft({ contextWindow: event.currentTarget.value.replace(/[^\d]/g, "") })}
    placeholder="留空不写，例如 200000"
  />
</Field>
<Field className="relay-field-auto-compact" label="压缩上下文大小">
  <Input
    inputMode="numeric"
    value={profile.autoCompactLimit}
    onChange={(event) => updateDraft({ autoCompactLimit: event.currentTarget.value.replace(/[^\d]/g, "") })}
    placeholder="留空不写，例如 160000"
  />
</Field>
```

- [ ] **Step 4: 增加上下文管理组件**

在 `RelayProfileDetail` 的 `RelayFileEditors` 前插入：

```tsx
<RelayContextManager profile={draft} form={form} onFormChange={onFormChange} onProfileChange={setDraft} />
```

新增组件：

```tsx
function RelayContextManager({
  profile,
  form,
  onFormChange,
  onProfileChange,
}: {
  profile: RelayProfile;
  form: BackendSettings;
  onFormChange: (value: BackendSettings) => void;
  onProfileChange: (value: RelayProfile) => void;
}) {
  const [kind, setKind] = useState<ContextKind>("mcp");
  const [editing, setEditing] = useState<CodexContextEntry | null>(null);
  const entries = contextEntriesFromSettings(form);
  const visible = kind === "mcp" ? entries.mcpServers : kind === "skill" ? entries.skills : entries.plugins;
  const selected = kind === "mcp" ? profile.contextSelection.mcpServers : kind === "skill" ? profile.contextSelection.skills : profile.contextSelection.plugins;
  const toggle = (id: string, enabled: boolean) => {
    const nextIds = enabled ? Array.from(new Set([...selected, id])) : selected.filter((item) => item !== id);
    onProfileChange({
      ...profile,
      contextSelection: {
        ...profile.contextSelection,
        ...(kind === "mcp" ? { mcpServers: nextIds } : kind === "skill" ? { skills: nextIds } : { plugins: nextIds }),
      },
    });
  };
  const deleteEntry = (entry: CodexContextEntry) => {
    const nextCommon = removeContextEntryText(form.relayCommonConfigContents, entry);
    onFormChange({ ...form, relayCommonConfigContents: nextCommon });
  };
  return (
    <div className="relay-context-panel">
      <div className="relay-context-head">
        <div>
          <strong>上下文</strong>
          <span>按供应商选择要合并的 MCP、skills 和插件。</span>
        </div>
        <div className="segmented">
          <button className={kind === "mcp" ? "active" : ""} onClick={() => setKind("mcp")} type="button">MCP</button>
          <button className={kind === "skill" ? "active" : ""} onClick={() => setKind("skill")} type="button">Skills</button>
          <button className={kind === "plugin" ? "active" : ""} onClick={() => setKind("plugin")} type="button">插件</button>
        </div>
      </div>
      <div className="relay-context-list">
        {visible.length ? visible.map((entry) => (
          <div className="relay-context-row" key={`${entry.kind}-${entry.id}`}>
            <label className="inline-check">
              <input checked={selected.includes(entry.id)} onChange={(event) => toggle(entry.id, event.currentTarget.checked)} type="checkbox" />
              <span>{entry.id}</span>
            </label>
            <code>{entry.summary || "空配置"}</code>
            <Button onClick={() => setEditing(entry)} size="sm" variant="secondary">编辑</Button>
            <Button onClick={() => deleteEntry(entry)} size="sm" variant="outline">删除</Button>
          </div>
        )) : <div className="empty">还没有此类上下文。</div>}
      </div>
      <Button onClick={() => setEditing({ id: "", kind, title: "", summary: "", tomlBody: "" })} size="sm" variant="secondary">新增</Button>
      {editing ? (
        <ContextEntryEditor
          entry={editing}
          form={form}
          onClose={() => setEditing(null)}
          onSave={(nextForm) => {
            onFormChange(nextForm);
            setEditing(null);
          }}
        />
      ) : null}
    </div>
  );
}
```

- [ ] **Step 5: 增加文本 upsert/delete helpers**

在 `App.tsx` helper 区增加：

```ts
function tableNameForContextKind(kind: ContextKind) {
  if (kind === "mcp") return "mcp_servers";
  if (kind === "skill") return "skills";
  return "plugins";
}

function removeContextEntryText(commonConfig: string, entry: CodexContextEntry): string {
  const tableName = tableNameForContextKind(entry.kind);
  const header = `[${tableName}.${entry.id}]`;
  const lines = commonConfig.split(/\r?\n/);
  const output: string[] = [];
  let skipping = false;
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.startsWith("[") && trimmed.endsWith("]")) {
      if (trimmed === header) {
        skipping = true;
        continue;
      }
      skipping = false;
    }
    if (!skipping) output.push(line);
  }
  return output.join("\n").replace(/\n{3,}/g, "\n\n").trim() ? `${output.join("\n").replace(/\n{3,}/g, "\n\n").trimEnd()}\n` : "";
}

function upsertContextEntryText(commonConfig: string, entry: CodexContextEntry): string {
  const without = entry.id ? removeContextEntryText(commonConfig, entry) : commonConfig;
  const tableName = tableNameForContextKind(entry.kind);
  const body = entry.tomlBody.trimEnd();
  const block = `[${tableName}.${entry.id.trim()}]\n${body}\n`;
  return `${without.trimEnd() ? `${without.trimEnd()}\n\n` : ""}${block}`;
}
```

- [ ] **Step 6: 增加编辑弹窗组件**

在 `App.tsx` 增加：

```tsx
function ContextEntryEditor({
  entry,
  form,
  onClose,
  onSave,
}: {
  entry: CodexContextEntry;
  form: BackendSettings;
  onClose: () => void;
  onSave: (form: BackendSettings) => void;
}) {
  const [id, setId] = useState(entry.id);
  const [tomlBody, setTomlBody] = useState(entry.tomlBody);
  const save = () => {
    const trimmedId = id.trim();
    if (!trimmedId) return;
    const nextEntry = { ...entry, id: trimmedId, tomlBody };
    onSave({ ...form, relayCommonConfigContents: upsertContextEntryText(form.relayCommonConfigContents, nextEntry) });
  };
  return (
    <div className="context-editor">
      <Field label="ID">
        <Input value={id} onChange={(event) => setId(event.currentTarget.value)} />
      </Field>
      <Field label="TOML">
        <Textarea className="relay-file-textarea compact" value={tomlBody} onChange={(event) => setTomlBody(event.currentTarget.value)} spellCheck={false} />
      </Field>
      <div className="toolbar">
        <Button onClick={save} size="sm">保存</Button>
        <Button onClick={onClose} size="sm" variant="secondary">取消</Button>
      </div>
    </div>
  );
}
```

- [ ] **Step 7: 新供应商默认全选公共上下文**

修改 `createRelayProfile(settings)`，创建 `next` 前增加：

```ts
const entries = contextEntriesFromSettings(settings);
```

新 profile 使用：

```ts
contextSelection: {
  mcpServers: entries.mcpServers.map((entry) => entry.id),
  skills: entries.skills.map((entry) => entry.id),
  plugins: entries.plugins.map((entry) => entry.id),
},
```

- [ ] **Step 8: 增加 CSS**

在 `styles.css` 增加：

```css
.relay-context-panel {
  display: grid;
  gap: 12px;
  padding: 14px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--panel);
}

.relay-context-head,
.relay-context-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}

.relay-context-head > div:first-child,
.relay-context-row code {
  min-width: 0;
}

.relay-context-head span {
  display: block;
  color: var(--muted);
  font-size: 12px;
}

.relay-context-list {
  display: grid;
  gap: 8px;
}

.relay-context-row {
  min-height: 40px;
  padding: 8px;
  border: 1px solid var(--border);
  border-radius: 8px;
}

.relay-context-row code {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.segmented {
  display: inline-flex;
  gap: 4px;
}

.segmented button {
  min-height: 32px;
}

.segmented button.active {
  border-color: var(--accent);
  color: var(--accent);
}

.context-editor {
  display: grid;
  gap: 10px;
}
```

如果 CSS 变量名不存在，改用项目当前已有的 panel/border/muted/accent 变量名。

- [ ] **Step 9: 验证 TypeScript**

Run:

```powershell
npm run check
```

Expected: TypeScript 无错误。

- [ ] **Step 10: 提交**

```powershell
git add apps/codex-plus-manager/src/App.tsx apps/codex-plus-manager/src/styles.css
git commit -m "feat: add supplier context management UI"
```

---

### Task 6: 前端改用后端上下文命令

**Files:**
- Modify: `apps/codex-plus-manager/src/App.tsx`

- [ ] **Step 1: 增加 actions**

在 `Actions` 类型增加：

```ts
upsertContextEntry: (settings: BackendSettings, kind: ContextKind, id: string, tomlBody: string) => Promise<BackendSettings | null>;
deleteContextEntry: (settings: BackendSettings, kind: ContextKind, id: string) => Promise<BackendSettings | null>;
```

在 `App` 内实现：

```ts
const upsertContextEntry = async (settings: BackendSettings, kind: ContextKind, id: string, tomlBody: string) => {
  const result = await run(() => call<ContextEntriesResult>("upsert_context_entry", {
    request: { settings, kind, id, tomlBody },
  }));
  if (!result) return null;
  setSettingsForm(normalizeSettings(result.settings));
  showResultNotice("上下文", result);
  return normalizeSettings(result.settings);
};

const deleteContextEntry = async (settings: BackendSettings, kind: ContextKind, id: string) => {
  const result = await run(() => call<ContextEntriesResult>("delete_context_entry", {
    request: { settings, kind, id },
  }));
  if (!result) return null;
  setSettingsForm(normalizeSettings(result.settings));
  showResultNotice("上下文", result);
  return normalizeSettings(result.settings);
};
```

在 actions object 中加入两项。

- [ ] **Step 2: `RelayContextManager` 使用 actions**

给 `RelayContextManager` props 增加：

```ts
actions: Actions;
```

删除时改成：

```ts
void actions.deleteContextEntry(form, entry.kind, entry.id).then((next) => {
  if (next) onFormChange(next);
});
```

保存时 `ContextEntryEditor` 改成接收 async `onSave`：

```tsx
onSave={async (kind, id, tomlBody) => {
  const next = await actions.upsertContextEntry(form, kind, id, tomlBody);
  if (next) onFormChange(next);
  setEditing(null);
}}
```

- [ ] **Step 3: 保留本地 fallback**

保留 `contextEntriesFromSettings`，用于渲染当前表单状态，不强依赖 list command。保存/删除使用后端命令，让 TOML 修改由 `toml_edit` 保证。

- [ ] **Step 4: 验证 TypeScript**

Run:

```powershell
npm run check
```

Expected: TypeScript 无错误。

- [ ] **Step 5: 提交**

```powershell
git add apps/codex-plus-manager/src/App.tsx
git commit -m "feat: wire context UI to tauri commands"
```

---

### Task 7: 全量验证和构建

**Files:**
- No source changes expected unless verification exposes defects.

- [ ] **Step 1: 格式检查**

Run:

```powershell
cargo fmt --check
```

Expected: exit code 0。若失败，运行 `cargo fmt`，然后重新运行 `cargo fmt --check`。

- [ ] **Step 2: 核心测试**

Run:

```powershell
cargo test -p codex-plus-core
```

Expected: 所有 core 测试通过。

- [ ] **Step 3: Manager 测试**

Run:

```powershell
cargo test -p codex-plus-manager
```

Expected: 所有 manager 测试通过。

- [ ] **Step 4: TypeScript 检查**

Run:

```powershell
npm run check
```

Working directory: `apps/codex-plus-manager`

Expected: `tsc --noEmit` 通过。

- [ ] **Step 5: 构建前检查目标进程**

Run:

```powershell
Get-Process | Where-Object { $_.ProcessName -in @('codex-plus-plus','codex-plus-plus-manager') } | Select-Object ProcessName,Id,Path
```

Expected: 无输出。如果有输出，运行：

```powershell
Get-Process | Where-Object { $_.ProcessName -in @('codex-plus-plus','codex-plus-plus-manager') } | Stop-Process -Force
```

- [ ] **Step 6: Release 构建**

Run:

```powershell
npm run build
```

Working directory: `apps/codex-plus-manager`

Expected: 构建成功，并输出：

```text
Built application at: E:\Desktop\CodexPlusPlus\target\release\codex-plus-plus-manager.exe
```

- [ ] **Step 7: 最终状态检查**

Run:

```powershell
git status --short
```

Expected: 只剩用户已有未提交改动或本次已知改动；不要回滚用户未要求的文件。

- [ ] **Step 8: 提交收尾**

如果前面任务因为工作树已有改动无法按任务提交，本步骤只提交本功能相关文件：

```powershell
git add crates/codex-plus-core/src/settings.rs crates/codex-plus-core/src/relay_config.rs crates/codex-plus-core/tests/relay_config.rs apps/codex-plus-manager/src-tauri/src/commands.rs apps/codex-plus-manager/src-tauri/src/lib.rs apps/codex-plus-manager/src/App.tsx apps/codex-plus-manager/src/styles.css
git commit -m "feat: add codex context management"
```

Expected: 提交成功，且不包含 `node_modules`、`target-tauri-build` 或无关用户改动。

---

## 自查

- Spec 覆盖：公共上下文库、供应商勾选、切换过滤、上下文 token 字段、Codex-only 范围、测试要求均有对应任务。
- 类型一致性：Rust 使用 `RelayContextSelection { mcp_servers, skills, plugins }`；前端使用 `RelayContextSelection { mcpServers, skills, plugins }`；serde camelCase 对齐。
- 无未决项：跨应用、远程安装、Codex 运行时是否完全执行 token 字段均明确不在范围内。
