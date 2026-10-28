//! 网关模式的开启 / 关闭，以及网关自己的供应商池、路由组的增删改。
//!
//! - Claude Code：在 `claude-providers.json` 里放一个托管供应商（[`GATEWAY_PROVIDER_ID`]），
//!   用现有的「关键字段」切换逻辑写进 `~/.claude/settings.json`，用户的 hooks 等键不动；
//!   关闭时切回开启前的供应商。
//! - Grok：合成一个指向网关的 Responses 供应商写进 `~/.grok/config.toml`；
//!   关闭时重新应用 Grok 页里当前选中的供应商。
//! - Codex：在直连模式的供应商列表里放一个托管供应商（[`CODEX_GATEWAY_RELAY_ID`]），
//!   用直连模式自己的切换流程写进 `~/.codex`（回填、冲突检测、模型目录都照旧）；
//!   关闭时切回开启前的供应商并删掉托管供应商。直连模式里切到别的供应商等于断开网关。
//! - 供应商池与路由组只存在 `gateway.json`；从直连模式导入是复制，之后互不影响。

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Context, bail};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::gateway::drift::{self, Drift};
use crate::gateway::restore::{self as stash, ChangedKey, FileStash, KeyPath};
use crate::gateway::route::{self, binding_default_model};
use crate::gateway::store::{
    AgentBinding, AutoGroupSettings, GROUP_MODEL_PREFIX, GatewayGroup, GatewayProvider,
    GatewaySettings, GatewayStore, GroupMember, GroupMode, GroupSticky, ProviderPrice,
    new_client_token,
};
use crate::settings::{
    BackendSettings, RelayApiKey, RelayMode, RelayProfile, RelayProtocol, SettingsStore,
};
use crate::tools::ToolId;
use crate::tools::claude::{self, ClaudeContext, ClaudeProvider, GATEWAY_PROVIDER_ID};

/// 开启/关闭时用到的全部路径，命令层用 [`ApplyContext::default_paths`]，测试指向临时目录。
#[derive(Debug, Clone)]
pub struct ApplyContext {
    pub gateway: GatewayStore,
    pub claude: ClaudeContext,
    pub grok_home: PathBuf,
    /// Codex 的配置目录与直连模式的设置（Codex 接入网关走直连模式的切换流程）。
    pub codex_home: PathBuf,
    pub settings: SettingsStore,
    pub backup_root: PathBuf,
    pub port: u16,
}

/// Codex 直连模式供应商列表里托管的网关供应商 id。
pub const CODEX_GATEWAY_RELAY_ID: &str = "codex-plus-gateway";

/// Codex 当前在用网关托管的供应商：本地代理收到的 Codex 请求只是转给网关的一跳。
pub fn codex_hops_through_gateway() -> bool {
    SettingsStore::default()
        .load()
        .is_ok_and(|settings| settings.active_relay_id == CODEX_GATEWAY_RELAY_ID)
}

impl ApplyContext {
    pub fn default_paths(port: u16) -> Self {
        Self {
            gateway: GatewayStore::default(),
            claude: ClaudeContext::default_paths(),
            grok_home: crate::grok_config::default_grok_home_dir(),
            codex_home: crate::relay_config::default_codex_home_dir(),
            settings: SettingsStore::default(),
            backup_root: crate::paths::default_app_state_dir().join("backups"),
            port,
        }
    }
}

/// Agent 访问网关的根地址（不含 `/v1`）。
pub fn gateway_base_url(port: u16, agent: &ToolId) -> String {
    format!("http://127.0.0.1:{port}/gw/{}", agent.as_str())
}

/// 开启前检查：池里有这个 Agent 能用的供应商，默认模型能路由。
fn ensure_routable(gateway: &GatewaySettings, binding: &AgentBinding) -> anyhow::Result<()> {
    if route::binding_pool_relays(gateway, binding).is_empty() {
        bail!("供应商池是空的或都已关闭：先在「供应商」里添加供应商");
    }
    let model = binding_default_model(gateway, binding);
    if model.is_empty() {
        bail!("没有可用的默认模型：请给池里的供应商配置模型");
    }
    if !route::model_is_routable(gateway, binding, &model) {
        bail!("供应商池里没有供应商提供模型 {model}");
    }
    Ok(())
}

/// 写给 Claude Code 的托管供应商。
pub fn claude_gateway_provider(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
) -> ClaudeProvider {
    let default_model = binding_default_model(gateway, binding);
    let role = |value: &str| {
        let (slug, _) = crate::model_suffix::parse_model_suffix(value.trim());
        if slug.trim().is_empty() {
            default_model.clone()
        } else {
            slug.trim().to_string()
        }
    };
    let mut env = Map::new();
    env.insert(
        "ANTHROPIC_BASE_URL".to_string(),
        json!(gateway_base_url(port, &ToolId::Claude)),
    );
    env.insert("ANTHROPIC_AUTH_TOKEN".to_string(), json!(binding.token));
    env.insert("ANTHROPIC_MODEL".to_string(), json!(default_model));
    let opus = role(&binding.claude_roles.opus);
    env.insert("ANTHROPIC_DEFAULT_OPUS_MODEL".to_string(), json!(opus));
    env.insert("ANTHROPIC_DEFAULT_FABLE_MODEL".to_string(), json!(opus));
    env.insert(
        "ANTHROPIC_DEFAULT_SONNET_MODEL".to_string(),
        json!(role(&binding.claude_roles.sonnet)),
    );
    env.insert(
        "ANTHROPIC_DEFAULT_HAIKU_MODEL".to_string(),
        json!(role(&binding.claude_roles.haiku)),
    );
    if !binding.claude_roles.subagent.trim().is_empty() {
        env.insert(
            "CLAUDE_CODE_SUBAGENT_MODEL".to_string(),
            json!(role(&binding.claude_roles.subagent)),
        );
    }
    // Claude Code 只有一个窗口设置：按默认模型的窗口（供应商池里写的），没写用 Agent 的默认窗口。
    let window = route::model_entry_window(gateway, binding, &default_model)
        .or(binding.context_window)
        .or_else(|| {
            let (_, window) = crate::model_suffix::parse_model_suffix(binding.model.trim());
            window
        });
    if let Some(window) = window {
        env.insert(
            "CLAUDE_CODE_MAX_CONTEXT_TOKENS".to_string(),
            json!(window.to_string()),
        );
    }
    if binding.claude_enable_effort {
        env.insert("CLAUDE_CODE_ALWAYS_ENABLE_EFFORT".to_string(), json!("1"));
    }
    ClaudeProvider {
        id: GATEWAY_PROVIDER_ID.to_string(),
        name: "Codex++ 网关".to_string(),
        website_url: String::new(),
        notes: "由 Codex++ 网关模式管理，请在网关模式里修改或关闭。".to_string(),
        settings_config: json!({ "env": Value::Object(env) }),
        created_at: 0,
        ccs_source_id: String::new(),
    }
}

/// 写给 Grok 的合成供应商：Responses 协议，地址是网关，Key 是网关 token。
pub fn grok_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
) -> RelayProfile {
    agent_gateway_profile(gateway, binding, port, &ToolId::Grok)
}

/// Responses 类 Agent（Grok、Codex）的合成供应商：地址是该 Agent 的网关入口，模型列表是整个池。
fn agent_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
    agent: &ToolId,
) -> RelayProfile {
    let default_model = binding_default_model(gateway, binding);
    let mut models = vec![default_model.clone()];
    // 池里所有能用的模型和模型组都进 Agent 的模型列表；Codex 按供应商分组显示，多家都有的模型每家各列一份。
    let pool = if *agent == ToolId::Codex {
        route::codex_pool_models_led_by(gateway, binding, &default_model)
    } else {
        route::pool_models(gateway, binding)
    };
    for model in pool.into_iter().chain(binding.declared_models()) {
        if !models.contains(&model) {
            models.push(model);
        }
    }
    let model_list = models
        .iter()
        .filter(|model| !model.is_empty())
        // 每个模型带自己的窗口（供应商池里按模型写的）；没写的用 Agent 的默认窗口，都没有就不带，
        // 交给 Agent 自带的模型资料。
        .map(|model| {
            match route::model_entry_window(gateway, binding, model).or(binding.context_window) {
                Some(window) => format!("{model}[{window}]"),
                None => model.clone(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    let base = format!("{}/v1", gateway_base_url(port, agent));
    RelayProfile {
        id: "codex-plus-gateway".to_string(),
        name: "Codex++ 网关".to_string(),
        protocol: RelayProtocol::Responses,
        relay_mode: RelayMode::PureApi,
        base_url: base.clone(),
        upstream_base_url: base,
        api_key: binding.token.clone(),
        model_list,
        ..RelayProfile::default()
    }
}

/// 写 Agent 配置。Claude 首次切换时若把 live 配置存成了「切换前的配置」，返回它的 id。
fn write_agent_config(
    ctx: &ApplyContext,
    agent: &ToolId,
    binding: &AgentBinding,
) -> anyhow::Result<Option<String>> {
    let gateway = ctx.gateway.load()?;
    match agent {
        ToolId::Claude => {
            // 还没指向网关：先把 settings.json 原样存一份，断开时据此还原（见 [`restore_claude`]）。
            let live_path = ctx.claude.claude_home.join("settings.json");
            let fresh = claude::current_provider_id(&ctx.claude)? != GATEWAY_PROVIDER_ID;
            let live_before = stash::read_optional(&live_path)?;
            let mut file_stash = begin_stash(&claude_stash_path(ctx), fresh, &live_before)?;
            claude::save_provider(
                &ctx.claude,
                claude_gateway_provider(&gateway, binding, ctx.port),
            )?;
            let before = claude::load(&ctx.claude)?
                .providers
                .into_iter()
                .map(|provider| provider.id)
                .collect::<Vec<_>>();
            let payload = claude::switch_provider(&ctx.claude, GATEWAY_PROVIDER_ID)?;
            let snapshot = payload
                .providers
                .iter()
                .find(|provider| !before.contains(&provider.id))
                .map(|provider| provider.id.clone());
            if let Some(file_stash) = &mut file_stash {
                let live_after = stash::read_optional(&live_path)?;
                file_stash.record_write(
                    &stash::json_value(live_before.as_deref())?,
                    &stash::json_value(live_after.as_deref())?,
                );
                stash::save_stash(&claude_stash_path(ctx), file_stash)?;
            }
            return Ok(snapshot);
        }
        ToolId::Grok => {
            // 同上：Grok 的 config.toml 接入前原样存一份。
            let live_path = ctx.grok_home.join("config.toml");
            let live_before = stash::read_optional(&live_path)?;
            let fresh = !live_before.as_deref().is_some_and(grok_points_at_gateway);
            let mut file_stash = begin_stash(&grok_stash_path(ctx), fresh, &live_before)?;
            let profile = grok_gateway_profile(&gateway, binding, ctx.port);
            crate::tools::grok::apply_profile_to_grok_at(
                &ctx.grok_home,
                &profile,
                &ctx.backup_root,
            )?;
            if let Some(file_stash) = &mut file_stash {
                let live_after = stash::read_optional(&live_path)?;
                file_stash.record_write(
                    &stash::toml_value(live_before.as_deref())?,
                    &stash::toml_value(live_after.as_deref())?,
                );
                stash::save_stash(&grok_stash_path(ctx), file_stash)?;
            }
        }
        ToolId::Codex => write_codex(ctx, &gateway, binding)?,
        other => bail!("{} 不支持网关模式", other.as_str()),
    }
    Ok(None)
}

/// 开始改写一个 Agent 的配置文件前准备存档：
/// - `fresh`（还没指向网关）：新建存档，记下接入前的原文；先落盘再改配置，存不下就不改；
/// - 已经指向网关：沿用已有的存档（之后把这次改到的键追加进去）；没有存档（旧版接入的）就不记。
fn begin_stash(
    path: &std::path::Path,
    fresh: bool,
    live_before: &Option<String>,
) -> anyhow::Result<Option<FileStash>> {
    if fresh {
        let file_stash = FileStash::new(live_before.clone());
        stash::save_stash(path, &file_stash)?;
        return Ok(Some(file_stash));
    }
    Ok(stash::load_stash(path))
}

fn claude_stash_path(ctx: &ApplyContext) -> PathBuf {
    ctx.backup_root.join("claude-before-gateway.json")
}

fn grok_stash_path(ctx: &ApplyContext) -> PathBuf {
    ctx.backup_root.join("grok-before-gateway.json")
}

/// Grok 的配置还指向网关（与 [`crate::gateway::drift`] 的判断一致）。
fn grok_points_at_gateway(text: &str) -> bool {
    text.contains("/gw/grok")
}

/// Codex 的托管网关供应商：与直连模式里手加的 Responses 供应商一样存储（配置快照里带地址和 token）。
///
/// `signed_in`：Codex 用 ChatGPT 账号登录着。这时供应商带 openai 会话身份（与直连模式的
/// 「官方会话身份」一样）：Codex 保留内置的 OpenAI 供应商与登录，会话、额度、远程控制照旧，
/// 请求经本地代理（`openai_base_url`）转给网关。没登录或用 API Key 时是普通的自定义供应商。
pub fn codex_gateway_profile(
    gateway: &GatewaySettings,
    binding: &AgentBinding,
    port: u16,
    signed_in: bool,
) -> anyhow::Result<RelayProfile> {
    let synthetic = agent_gateway_profile(gateway, binding, port, &ToolId::Codex);
    let mut profile = crate::provider_import::pool_relay_profile(
        "Codex++ 网关",
        &synthetic.base_url,
        &synthetic.api_key,
        RelayProtocol::Responses,
        &binding_default_model(gateway, binding),
        &synthetic.model_list,
        &[],
    )?;
    profile.id = CODEX_GATEWAY_RELAY_ID.to_string();
    profile.relay_mode = RelayMode::PureApi;
    profile.model_metadata = codex_model_metadata(gateway, binding);
    if signed_in {
        profile.config_contents = with_openai_session_identity(&profile.config_contents)?;
    }
    crate::relay_config::normalize_relay_profile_for_storage(&mut profile)?;
    Ok(profile)
}

/// 「模型名后带供应商」关掉时用的看不见的标记：各家那份追加 N 个零宽空格（N 是这家在池里的位置），
/// 自动那份追加一个零宽非连接符。看起来和模型名一样，分组脚本按原文比对时又互不相同。
const PROVIDER_MARK: &str = "\u{200B}";
const AUTO_MARK: &str = "\u{200C}";

/// Codex 模型选择器的注入层能从显示名尾部 `（…）` 认出的供应商名最长多少字（见 71-model-picker-groups.js）。
const CODEX_GROUP_SUFFIX_MAX: usize = 24;

/// 多家都有的模型的「自动切换」入口在 Codex 模型选择器里的分组身份。
const CODEX_AUTO_GROUP_ID: &str = "codex-plus-gateway-auto";

/// 模型组在 Codex 模型选择器里的分组身份。
const CODEX_GROUPS_GROUP_ID: &str = "codex-plus-gateway-groups";

/// Codex 模型目录里每个模型的元数据（显示名、推理档位、按供应商分组），与直连模式的按模型聚合一致：
/// 只有一家的模型取那一家的成员目录，分组是那一家；多家都有的模型每家各列一份 `<供应商 id>/<模型>`
/// 归在各家名下，另有一份裸名的「自动切换」入口单独一组（见 [`route::codex_pool_models`]）；
/// `group/<id>` 显示组名，单独归在「模型组」一组。没有可写的条目时返回空串。
///
/// 写进托管供应商的 `model_metadata`：生成 `model_catalog_json` 时覆盖进目录，
/// 注入层读的 `/codex-model-catalog` 也按它给出推理档位与分组。
fn codex_model_metadata(gateway: &GatewaySettings, binding: &AgentBinding) -> String {
    let relays = route::binding_pool_relays(gateway, binding);
    let mut member_catalogs: Vec<Option<Vec<Value>>> = vec![None; relays.len()];
    let mut metadata = Map::new();
    for model in route::codex_pool_models(gateway, binding) {
        if let Some(id) = model.strip_prefix(GROUP_MODEL_PREFIX) {
            let Some(group) = gateway.group(id) else {
                continue;
            };
            // 组的推理档位取第一个成员的；Fast 要每个成员都支持才开（#2740）：
            // 请求轮到不支持的那家会被拒。成员一个都对不上时退回按组名生成的通用条目。
            let mut member_fields = Vec::new();
            for (relay, upstream) in route::model_entry_members(gateway, binding, &model) {
                let Some(index) = relays.iter().position(|candidate| candidate.id == relay.id)
                else {
                    continue;
                };
                if let Some(entry) =
                    member_catalog_entry(&mut member_catalogs, &relays, index, &upstream)
                {
                    member_fields.push(catalog_ui_fields(&entry));
                }
            }
            let mut fields = match member_fields.first() {
                Some(first) => first.clone(),
                None => {
                    let entry = crate::model_suffix::collect_catalog_entries(
                        &model,
                        &Default::default(),
                        &Default::default(),
                        &model,
                    );
                    let template: Value = serde_json::from_str(
                        &crate::model_suffix::build_model_catalog_json(&entry, None),
                    )
                    .unwrap_or(Value::Null);
                    catalog_ui_fields(&template["models"][0])
                }
            };
            let has_tiers = |fields: &Map<String, Value>| {
                fields
                    .get("service_tiers")
                    .and_then(Value::as_array)
                    .is_some_and(|tiers| !tiers.is_empty())
            };
            if member_fields.is_empty() || !member_fields.iter().all(has_tiers) {
                fields.insert("service_tiers".into(), json!([]));
                fields.insert("additional_speed_tiers".into(), json!([]));
            }
            fields.insert("display_name".into(), json!(group.name));
            fields.insert("description".into(), json!(format!("{model} · 模型组")));
            fields.insert("codex_plus_group".into(), json!("模型组"));
            fields.insert("codex_plus_group_id".into(), json!(CODEX_GROUPS_GROUP_ID));
            if codex_favorite(gateway, &relays, &model) {
                fields.insert("codex_plus_favorite".into(), json!(true));
            }
            metadata.insert(model, Value::Object(fields));
            continue;
        }
        // 归哪家：`<供应商 id>/<模型>` 是那一家；裸名归第一个提供它的供应商（多家都有时它是自动切换入口）。
        let Some((index, upstream)) = route::codex_model_owner(&relays, &model) else {
            continue;
        };
        let relay = relays[index];
        let Some(entry) = member_catalog_entry(&mut member_catalogs, &relays, index, &upstream)
        else {
            continue;
        };
        let mut fields = catalog_ui_fields(&entry);
        // 没有元数据的模型，成员目录的显示名就是原始 ID：换成规范写法（gpt-6.1-sol → GPT-6.1-Sol）。
        let display = fields
            .get("display_name")
            .and_then(Value::as_str)
            .filter(|name| !name.eq_ignore_ascii_case(&upstream))
            .map(str::to_string)
            .unwrap_or_else(|| pretty_model_name(&upstream));
        let providers = route::serving_count(&relays, &upstream);
        if providers > 1 && upstream == model {
            // 多家自动切换入口：单独一组，按池顺序依次尝试各家。
            let marked = if gateway.model_suffix {
                format!("{display}（自动）")
            } else {
                format!("{display}{AUTO_MARK}")
            };
            fields.insert("display_name".into(), json!(marked));
            fields.insert(
                "description".into(),
                json!(format!("{model} · {providers} 家按顺序自动切换")),
            );
            fields.insert("codex_plus_group".into(), json!("自动组"));
            fields.insert("codex_plus_group_id".into(), json!(CODEX_AUTO_GROUP_ID));
            if codex_favorite(gateway, &relays, &model) {
                fields.insert("codex_plus_favorite".into(), json!(true));
            }
            metadata.insert(model, Value::Object(fields));
            continue;
        }
        // 分组名和描述会显示在 Codex 的模型选择器里：账号名要打码。
        let group = if relay.name.trim().is_empty() {
            relay.id.clone()
        } else {
            crate::gateway::accounts::mask_account_name(relay.name.trim())
        };
        // 多家都有的模型，每家那份显示名带 `（供应商）` 后缀：选择器按整行文字认分组，同名的几行
        // 不加后缀就分不开（与直连模式按模型聚合的写法一致，注入层会去掉后缀再显示），
        // 选中后输入框里也看得出是哪一家。注入层只认 24 个字以内的后缀，账号名太长时用供应商 id。
        // 「模型名后带供应商」关掉时，换成看不见的标记（每家不同），选择器照样分得清，界面上只显示模型名。
        let display = if providers <= 1 {
            display
        } else if gateway.model_suffix {
            format!(
                "{display}（{}）",
                codex_group_suffix(relay, &group, &relays)
            )
        } else {
            format!("{display}{}", PROVIDER_MARK.repeat(index + 1))
        };
        fields.insert("display_name".into(), json!(display));
        fields.insert("description".into(), json!(format!("{upstream} · {group}")));
        fields.insert("codex_plus_group".into(), json!(group));
        fields.insert("codex_plus_group_id".into(), json!(relay.id));
        if codex_favorite(gateway, &relays, &model) {
            fields.insert("codex_plus_favorite".into(), json!(true));
        }
        metadata.insert(model, Value::Object(fields));
    }
    if metadata.is_empty() {
        return String::new();
    }
    serde_json::to_string(&Value::Object(metadata)).unwrap_or_default()
}

/// 这个模型在管理器里收藏过没有（与管理器的 `isFavoriteModel` 一致）：直接收藏过这个名字算；
/// 旧版收藏的是裸名，那时多家都有的模型只有一份——这个模型还没有任何一家的那份被单独收藏过时，
/// 裸名收藏也点亮各家那几份。
fn codex_favorite(gateway: &GatewaySettings, relays: &[&RelayProfile], model: &str) -> bool {
    let favorites = &gateway.favorite_models;
    let has = |name: &str| favorites.iter().any(|item| item.eq_ignore_ascii_case(name));
    if has(model) {
        return true;
    }
    let Some((_, upstream)) = route::codex_model_owner(relays, model) else {
        return false;
    };
    if upstream == model || !has(&upstream) {
        return false;
    }
    !relays
        .iter()
        .filter(|relay| {
            route::relay_models(relay)
                .iter()
                .any(|known| known.eq_ignore_ascii_case(&upstream))
        })
        .any(|relay| has(&format!("{}/{upstream}", relay.id)))
}

/// 同名模型显示名后的 `（供应商）` 后缀：注入层只认 24 个字以内的后缀。打码后的完整名放得下就用它；
/// 放不下时用 `·` 前那段（如「ChatGPT Pro」），前提是池里没有别家同样以它开头（否则几行又分不开）；
/// 都不行才用供应商 id。
fn codex_group_suffix(relay: &RelayProfile, group: &str, relays: &[&RelayProfile]) -> String {
    if group.chars().count() <= CODEX_GROUP_SUFFIX_MAX {
        return group.to_string();
    }
    let head = |name: &str| {
        name.split('·')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let short = head(group);
    let unique = relays
        .iter()
        .filter(|other| other.id != relay.id)
        .all(|other| {
            head(&crate::gateway::accounts::mask_account_name(
                other.name.trim(),
            )) != short
        });
    if !short.is_empty() && short.chars().count() <= CODEX_GROUP_SUFFIX_MAX && unique {
        short
    } else {
        relay.id.clone()
    }
}

/// 池里第 `index` 家的成员目录里这个上游模型的条目（不分大小写）；每家的目录只生成一次。
fn member_catalog_entry(
    cache: &mut [Option<Vec<Value>>],
    relays: &[&RelayProfile],
    index: usize,
    upstream: &str,
) -> Option<Value> {
    let catalog = cache[index].get_or_insert_with(|| {
        crate::aggregate_catalog::build_member_catalog(relays[index])
            .ok()
            .and_then(|payload| payload["models"].as_array().cloned())
            .unwrap_or_default()
    });
    catalog
        .iter()
        .find(|entry| {
            entry["slug"]
                .as_str()
                .is_some_and(|slug| slug.eq_ignore_ascii_case(upstream))
        })
        .cloned()
}

/// 目录条目里选择器要用的字段；整条（含基础指令等大字段）不进设置文件。
fn catalog_ui_fields(entry: &Value) -> Map<String, Value> {
    let mut fields = Map::new();
    for key in [
        "display_name",
        "supported_reasoning_levels",
        "default_reasoning_level",
        "supports_reasoning_summaries",
        // Fast（优先处理）按这两个字段判断能不能开；不带过去 Codex 里的 Fast 按钮就一直是灰的。
        "service_tiers",
        "additional_speed_tiers",
    ] {
        if let Some(value) = entry.get(key).filter(|value| !value.is_null()) {
            fields.insert(key.to_string(), value.clone());
        }
    }
    fields
}

/// 模型 ID 的显示写法，和管理器界面一致：品牌词按正式写法（GPT、DeepSeek、GLM），
/// `v4`、`k2` 这类字母 + 数字大写，其余首字母大写。已经带大写的原样保留。
fn pretty_model_name(model: &str) -> String {
    const WORDS: &[(&str, &str)] = &[
        ("gpt", "GPT"),
        ("glm", "GLM"),
        ("deepseek", "DeepSeek"),
        ("minimax", "MiniMax"),
        ("qwen", "Qwen"),
        ("qwq", "QwQ"),
        ("kimi", "Kimi"),
        ("grok", "Grok"),
        ("claude", "Claude"),
        ("gemini", "Gemini"),
        ("gemma", "Gemma"),
        ("llama", "Llama"),
        ("mistral", "Mistral"),
        ("codestral", "Codestral"),
        ("doubao", "Doubao"),
        ("ernie", "ERNIE"),
        ("hunyuan", "Hunyuan"),
        ("step", "Step"),
        ("mimo", "MiMo"),
        ("longcat", "LongCat"),
    ];
    let model = model.trim();
    let (prefix, name) = match model.rfind('/') {
        Some(index) => model.split_at(index + 1),
        None => ("", model),
    };
    if name.chars().any(|ch| ch.is_ascii_uppercase()) {
        return model.to_string();
    }
    let token = |part: &str| -> String {
        let lower = part.to_ascii_lowercase();
        let word = |key: &str| {
            WORDS
                .iter()
                .find(|(known, _)| *known == key)
                .map(|(_, word)| *word)
        };
        if let Some(word) = word(&lower) {
            return word.to_string();
        }
        let bytes = lower.as_bytes();
        // 数字串（可带一个小数部分）从 `from` 开始，返回结束位置。
        let number_end = |from: usize| {
            let mut end = from;
            while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                end += 1;
            }
            if end > from
                && bytes.get(end) == Some(&b'.')
                && bytes.get(end + 1).is_some_and(u8::is_ascii_digit)
            {
                end += 1;
                while bytes.get(end).is_some_and(u8::is_ascii_digit) {
                    end += 1;
                }
            }
            (end > from).then_some(end)
        };
        let Some(&first) = bytes.first() else {
            return String::new();
        };
        // `o3` 这类 OpenAI 推理模型保持小写。
        if first == b'o' && bytes.get(1).is_some_and(u8::is_ascii_digit) {
            return lower;
        }
        // `v4`、`k2.5`、`r1b`：一个字母 + 数字（+ 至多一个字母）。
        if first.is_ascii_alphabetic()
            && let Some(end) = number_end(1)
            && (end == bytes.len() || (end + 1 == bytes.len() && bytes[end].is_ascii_alphabetic()))
        {
            return lower.to_ascii_uppercase();
        }
        if first.is_ascii_digit() {
            // `235b`、`32k` 大写，其余数字开头的原样。
            return match number_end(0) {
                Some(end) if end + 1 == bytes.len() && matches!(bytes[end], b'b' | b'k' | b'm') => {
                    lower.to_ascii_uppercase()
                }
                _ => lower,
            };
        }
        let letters = lower.bytes().take_while(u8::is_ascii_alphabetic).count();
        if letters == 0 {
            return part.to_string();
        }
        let (head, rest) = lower.split_at(letters);
        let head = word(head)
            .map(str::to_string)
            .unwrap_or_else(|| head[..1].to_ascii_uppercase() + &head[1..]);
        head + rest
    };
    let mut out = String::from(prefix);
    let mut current = String::new();
    for ch in name.chars() {
        if ch == '-' || ch == '_' {
            out.push_str(&token(&current));
            out.push(ch);
            current.clear();
        } else {
            current.push(ch);
        }
    }
    out.push_str(&token(&current));
    out
}

/// 把供应商配置改成 openai 会话身份：`model_provider = "openai"`，传输放进 `[model_providers.custom]`。
fn with_openai_session_identity(contents: &str) -> anyhow::Result<String> {
    let mut doc = contents
        .parse::<toml_edit::DocumentMut>()
        .context("托管供应商配置不是合法 TOML")?;
    let provider = doc
        .get("model_provider")
        .and_then(toml_edit::Item::as_str)
        .unwrap_or_default()
        .to_string();
    if provider == "openai" {
        return Ok(doc.to_string());
    }
    let mut table = doc
        .get_mut("model_providers")
        .and_then(toml_edit::Item::as_table_mut)
        .and_then(|providers| providers.remove(&provider))
        .and_then(|item| item.into_table().ok())
        .context("托管供应商配置里没有传输供应商")?;
    table["name"] = toml_edit::value("custom");
    doc["model_providers"]["custom"] = toml_edit::Item::Table(table);
    doc["model_provider"] = toml_edit::value("openai");
    Ok(doc.to_string())
}

/// 池里能借给 Codex 登录的账号：打开着的、网关自己登录的 ChatGPT 账号（按池里的顺序取第一个）。
fn lendable_login(gateway: &GatewaySettings) -> Option<&GatewayProvider> {
    gateway.providers.iter().find(|provider| {
        provider.enabled && crate::official_login::is_gateway_owned_login(&provider.relay)
    })
}

/// 把网关的 ChatGPT 账号借给 Codex 登录：token 写进 Codex 的 auth.json（其余字段不动），
/// 网关里标记「借出」——之后这个账号以 Codex 那份为准，网关刷新也写回那里，两边始终同一代。
fn lend_login_to_codex(ctx: &ApplyContext, provider_id: &str) -> anyhow::Result<()> {
    // 带锁读改写：借出用的是锁内读到的最新一代 token，也不会把同时刷新写回的 token 盖掉。
    ctx.gateway.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == provider_id)
        else {
            return Ok(());
        };
        let path = ctx.codex_home.join("auth.json");
        let live = std::fs::read_to_string(&path).unwrap_or_else(|_| "{}".to_string());
        let merged = crate::official_login::auth_contents_with_member_login(
            &live,
            &provider.relay.auth_contents,
            false,
        )
        .context("Codex 已经登录了别的 ChatGPT 账号，不能再借用网关的账号")?;
        crate::settings::atomic_write(&path, merged.as_bytes())?;
        let mut auth: Value = serde_json::from_str(&provider.relay.auth_contents)?;
        auth[crate::official_login::LENT_TO_CODEX_MARKER] = Value::Bool(true);
        provider.relay.auth_contents = serde_json::to_string_pretty(&auth)?;
        Ok(())
    })?;
    Ok(())
}

/// 收回借给 Codex 的账号：Codex 那份是最新一代，token 写回网关、去掉「借出」标记，
/// 再从 Codex 的 auth.json 里拿掉这个登录（借之前 Codex 本来就没登录）。
pub fn return_lent_login(store: &GatewayStore, codex_home: &std::path::Path) -> anyhow::Result<()> {
    let path = codex_home.join("auth.json");
    let live = std::fs::read_to_string(&path).unwrap_or_default();
    let live_login = crate::official_login::official_login_from_auth_contents(&live);
    // 带锁读改写：只动借出的那几个供应商，不拿旧状态覆盖同时写进来的改动（没有借出的就不写盘）。
    let (_, strip_live) = store.update(|gateway| {
        let mut strip_live = false;
        for provider in gateway
            .providers
            .iter_mut()
            .filter(|provider| crate::official_login::is_lent_to_codex(&provider.relay))
        {
            let mut auth: Value = serde_json::from_str(&provider.relay.auth_contents)?;
            let account = auth
                .pointer("/tokens/account_id")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if let Some(login) = live_login
                .as_ref()
                .filter(|login| !account.is_empty() && login.account_id == account)
            {
                auth["tokens"]["access_token"] = json!(login.access_token);
                if !login.refresh_token.is_empty() {
                    auth["tokens"]["refresh_token"] = json!(login.refresh_token);
                }
                strip_live = true;
            }
            if let Some(object) = auth.as_object_mut() {
                object.remove(crate::official_login::LENT_TO_CODEX_MARKER);
            }
            provider.relay.auth_contents = serde_json::to_string_pretty(&auth)?;
        }
        Ok(strip_live)
    })?;
    if strip_live && let Ok(Value::Object(mut object)) = serde_json::from_str::<Value>(&live) {
        for key in ["tokens", "auth_mode", "last_refresh"] {
            object.remove(key);
        }
        crate::settings::atomic_write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string_pretty(&Value::Object(object))?
            )
            .as_bytes(),
        )?;
    }
    Ok(())
}

/// Codex 现在是否用 ChatGPT 账号登录（auth.json 里有登录 token）。
fn codex_signed_in(codex_home: &std::path::Path) -> bool {
    std::fs::read_to_string(codex_home.join("auth.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .is_some_and(|auth| {
            auth.pointer("/tokens/access_token")
                .and_then(Value::as_str)
                .is_some_and(|token| !token.trim().is_empty())
        })
}

/// 接入网关前 Codex 实时配置的副本。只在实时配置与直连模式的记录对不上时才存：
/// 那时直连模式没法从它自己的记录还原，断开时据此还原网关改过的键（见 [`restore_codex_files`]）。
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CodexLiveStash {
    #[serde(default)]
    config_toml: Option<String>,
    #[serde(default)]
    auth_json: Option<String>,
    /// 网关改过的 config.toml 键；None 是旧版的存档（没记），还原时按已知的网关键兜底。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    config_changed: Option<Vec<ChangedKey>>,
    /// 网关改过的 auth.json 键与写进去的值。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    auth_changed: Option<Vec<ChangedKey>>,
}

impl CodexLiveStash {
    /// 记下一次写入改到的键（`before` / `after` 是写入前后的实时文件）。
    /// 文件解析不了时当作空的（那样的文件直连模式本来也切不过去）。
    fn record_write(&mut self, before: &CodexLiveStash, after: &CodexLiveStash) {
        let empty = || Value::Object(Map::new());
        let mut config = FileStash {
            before: None,
            changed: self.config_changed.take().unwrap_or_default(),
        };
        config.record_write(
            &stash::toml_value(before.config_toml.as_deref()).unwrap_or_else(|_| empty()),
            &stash::toml_value(after.config_toml.as_deref()).unwrap_or_else(|_| empty()),
        );
        self.config_changed = Some(config.changed);
        let mut auth = FileStash {
            before: None,
            changed: self.auth_changed.take().unwrap_or_default(),
        };
        auth.record_write(
            &stash::json_value(before.auth_json.as_deref()).unwrap_or_else(|_| empty()),
            &stash::json_value(after.auth_json.as_deref()).unwrap_or_else(|_| empty()),
        );
        self.auth_changed = Some(auth.changed);
    }
}

const CODEX_LIVE_FILES: [&str; 2] = ["config.toml", "auth.json"];

fn codex_stash_path(ctx: &ApplyContext) -> PathBuf {
    ctx.backup_root.join("codex-before-gateway.json")
}

fn capture_codex_live(ctx: &ApplyContext) -> CodexLiveStash {
    let read = |name: &str| std::fs::read_to_string(ctx.codex_home.join(name)).ok();
    CodexLiveStash {
        config_toml: read(CODEX_LIVE_FILES[0]),
        auth_json: read(CODEX_LIVE_FILES[1]),
        ..CodexLiveStash::default()
    }
}

/// 旧版存档没记网关改过哪些键：按网关会写的键兜底（这些键与接入前不同就改回去）。
const CODEX_GATEWAY_KEYS: &[&str] = &[
    "model_provider",
    "model",
    "model_catalog_json",
    "openai_base_url",
    "model_reasoning_effort",
    "model_context_window",
    "model_auto_compact_token_limit",
    "preferred_auth_method",
];

fn legacy_codex_config_paths(before: Option<&str>, current: Option<&str>) -> Vec<KeyPath> {
    let (Ok(before), Ok(current)) = (stash::toml_value(before), stash::toml_value(current)) else {
        return Vec::new();
    };
    stash::changed_paths(&before, &current)
        .into_iter()
        .filter(|path| {
            path.first().is_some_and(|key| {
                key == "model_providers" || CODEX_GATEWAY_KEYS.contains(&key.as_str())
            })
        })
        .collect()
}

/// 断开时还原 Codex 的实时文件（只在接入时存过副本的情况下）。`live` 是切回直连之前的实时文件。
///
/// - config.toml：只把网关改过的键改回接入前的值，其余（例如接入期间加的 MCP）照当前的留着；
/// - auth.json：只处理网关改过、之后 Codex 没再动过的键（Codex 刷新过的 token、新登录的账号都保留），
///   其余照切回前的样子；接入前没有、现在也没别的内容时才删掉。
fn restore_codex_files(
    ctx: &ApplyContext,
    saved: &CodexLiveStash,
    live: &CodexLiveStash,
) -> anyhow::Result<()> {
    let config_paths = match &saved.config_changed {
        Some(changed) => changed.iter().map(|key| key.path.clone()).collect(),
        None => {
            legacy_codex_config_paths(saved.config_toml.as_deref(), live.config_toml.as_deref())
        }
    };
    let config = stash::restore_toml(
        saved.config_toml.as_deref(),
        live.config_toml.as_deref(),
        &config_paths,
    )
    .context("还原 Codex 的 config.toml 失败")?;
    stash::write_or_remove(&ctx.codex_home.join(CODEX_LIVE_FILES[0]), config.as_deref())?;

    let auth_changed = match &saved.auth_changed {
        Some(changed) => changed.clone(),
        // 旧版存档：只认得出网关自己的 token（cpx-gw- 开头），把它们换回接入前的值。
        None => stash::json_value(live.auth_json.as_deref())
            .ok()
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
            .into_iter()
            .filter(|(_, value)| {
                value.as_str().is_some_and(|text| {
                    text.starts_with(crate::gateway::store::CLIENT_TOKEN_PREFIX)
                })
            })
            .map(|(key, value)| ChangedKey {
                path: vec![key],
                written: Some(value),
            })
            .collect(),
    };
    let auth = stash::restore_json(
        saved.auth_json.as_deref(),
        live.auth_json.as_deref(),
        &auth_changed,
        |key, _, now| stash::value_at(now, &key.path) != key.written.as_ref(),
    )
    .context("还原 Codex 的 auth.json 失败")?;
    stash::write_or_remove(&ctx.codex_home.join(CODEX_LIVE_FILES[1]), auth.as_deref())?;
    Ok(())
}

/// 把托管供应商放进直连模式的列表并切过去（已经在用时只刷新它的配置）。
///
/// 实时配置被别的工具改过、与直连模式的记录对不上（直连模式里切换会弹「实时配置冲突」）时，
/// 不往上一个供应商回填，先把实时文件原样存下来再切；断开时原样写回。
fn write_codex(
    ctx: &ApplyContext,
    gateway: &GatewaySettings,
    binding: &AgentBinding,
) -> anyhow::Result<()> {
    let mut settings = ctx.settings.load()?;
    if !settings.relay_profiles_enabled {
        bail!("Codex 的供应商配置总开关已关闭，无法接入网关：先在 Codex 设置里打开");
    }
    // Codex 自己没登录、池里有网关登录的 ChatGPT 账号：借给 Codex，让它以官方身份接入。
    let signed_in = codex_signed_in(&ctx.codex_home);
    let lend = (!signed_in)
        .then(|| lendable_login(gateway).map(|provider| provider.relay.id.clone()))
        .flatten();
    let profile = codex_gateway_profile(gateway, binding, ctx.port, signed_in || lend.is_some())?;
    match settings
        .relay_profiles
        .iter_mut()
        .find(|item| item.id == CODEX_GATEWAY_RELAY_ID)
    {
        Some(existing) => *existing = profile,
        None => settings.relay_profiles.push(profile),
    }
    let previous = std::mem::replace(
        &mut settings.active_relay_id,
        CODEX_GATEWAY_RELAY_ID.to_string(),
    );
    let switching_in = previous != CODEX_GATEWAY_RELAY_ID;
    let live = capture_codex_live(ctx);
    let switched = crate::relay_switch::switch_relay_profile_in_home(
        &ctx.settings,
        &ctx.codex_home,
        settings.clone(),
        &previous,
    );
    match switched {
        Ok(_) => {
            if switching_in {
                // 直连模式能从自己的记录还原，不需要原样副本；清掉以前留下的。
                stash::remove_stash(&codex_stash_path(ctx));
            } else if let Some(mut saved) =
                stash::load_stash::<CodexLiveStash>(&codex_stash_path(ctx))
                && saved.config_changed.is_some()
            {
                // 接入时存过副本：这次重写改到的键也记上，断开时一起改回去。
                saved.record_write(&live, &capture_codex_live(ctx));
                stash::save_stash(&codex_stash_path(ctx), &saved)?;
            }
        }
        Err(error)
            if switching_in
                && error
                    .downcast_ref::<crate::relay_switch::LiveEndpointConflict>()
                    .is_some() =>
        {
            // 先存接入前的原文（还没记改了哪些键：万一切换中途退出，还原时按已知的网关键兜底）。
            stash::save_stash(&codex_stash_path(ctx), &live)?;
            crate::relay_switch::switch_relay_profile_in_home_resolving_conflict(
                &ctx.settings,
                &ctx.codex_home,
                settings,
                &previous,
                crate::relay_switch::LiveConflictResolution::DiscardLiveChanges,
            )
            .context("把 Codex 切到网关失败")?;
            let mut saved = live.clone();
            saved.record_write(&live, &capture_codex_live(ctx));
            stash::save_stash(&codex_stash_path(ctx), &saved)?;
        }
        Err(error) => return Err(error.context("把 Codex 切到网关失败")),
    }
    // 切换时回填上一个供应商用的还是借之前的 auth.json；切过去之后再写入借来的登录。
    if let Some(provider_id) = lend {
        lend_login_to_codex(ctx, &provider_id)?;
    }
    Ok(())
}

/// 开启（或更新）一个 Agent 的网关绑定，并把它的配置指向网关。
///
/// 已经开着时只更新绑定与配置，开启前记下的「恢复现场」保持不变。
pub fn enable_binding(
    ctx: &ApplyContext,
    agent: &ToolId,
    mut binding: AgentBinding,
) -> anyhow::Result<GatewaySettings> {
    let gateway = ctx.gateway.load()?;
    ensure_routable(&gateway, &binding)?;
    let previous = gateway.binding(agent).cloned();
    binding.token = previous
        .as_ref()
        .map(|previous| previous.token.clone())
        .filter(|token| !token.trim().is_empty())
        .unwrap_or_else(|| new_client_token(agent));
    // 更新版本写的未知字段：界面传回的绑定里没有的，沿用原来的。
    if let Some(previous) = &previous {
        for (key, value) in &previous.extra {
            binding
                .extra
                .entry(key.clone())
                .or_insert_with(|| value.clone());
        }
    }
    binding.restore = match previous.filter(|previous| previous.enabled) {
        Some(previous) => previous.restore,
        None => {
            let mut restore = crate::gateway::store::GatewayRestoreState::default();
            if *agent == ToolId::Claude {
                let current = claude::current_provider_id(&ctx.claude)?;
                if current != GATEWAY_PROVIDER_ID {
                    restore.claude_previous_provider_id = current;
                }
            }
            if *agent == ToolId::Codex {
                restore.codex_previous_relay_id = codex_active_relay_id(ctx)?;
            }
            restore
        }
    };
    binding.enabled = true;
    binding.restore.applied_port = Some(ctx.port);
    binding.restore.restored_endpoint = None;
    if let Some(snapshot) = write_agent_config(ctx, agent, &binding)?
        && binding.restore.claude_previous_provider_id.is_empty()
    {
        binding.restore.claude_previous_provider_id = snapshot;
    }
    // 写配置时可能改过 gateway.json（借出账号等），以最新的为准再写绑定（带锁，不盖掉同时刷新的 token）。
    let (gateway, ()) = ctx.gateway.update(|gateway| {
        gateway.bindings.insert(agent.as_str().to_string(), binding);
        Ok(())
    })?;
    Ok(gateway)
}

/// 关闭一个 Agent 的网关绑定，把它的配置恢复成直连。绑定本身（目标、模型）保留，方便再次开启。
pub fn disable_binding(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
) -> anyhow::Result<GatewaySettings> {
    let gateway = match ctx.gateway.load() {
        Ok(gateway) => gateway,
        // gateway.json 读不出来（例如损坏），但 Agent 还指向网关：照样还原，不碰 gateway.json。
        Err(error) if agent_points_at_gateway(ctx, agent) => {
            restore_agent(ctx, settings, agent, &AgentBinding::default())
                .with_context(|| format!("网关状态读取失败（{error:#}），按直连还原也失败"))?;
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "gateway.disable_without_store",
                serde_json::json!({ "agent": agent.as_str(), "message": format!("{error:#}") }),
            );
            return Ok(GatewaySettings::default());
        }
        Err(error) => return Err(error),
    };
    // 没有绑定（例如降级后又升级、gateway.json 被换过），但 Agent 还停在网关上：照样还原，
    // 否则界面一直显示「被网关接管」却断不开。
    let binding = match gateway.bindings.get(agent.as_str()) {
        Some(binding) => binding.clone(),
        None if agent_points_at_gateway(ctx, agent) => AgentBinding::default(),
        None => return Ok(gateway),
    };
    restore_agent(ctx, settings, agent, &binding)?;
    // 恢复时可能改过 gateway.json（收回借出的账号等），以最新的为准。
    let (gateway, ()) = ctx.gateway.update(|gateway| {
        if let Some(binding) = gateway.bindings.get_mut(agent.as_str()) {
            binding.enabled = false;
            binding.restore = Default::default();
        }
        Ok(())
    })?;
    Ok(gateway)
}

/// 还原一个 Agent 的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreOutcome {
    /// 改回了接入前的样子（或本来就不用改）。
    Restored,
    /// 配置已经被别的工具改走：没写 Agent 的文件，只清掉了网关自己的记录，绑定该关掉。
    TakenOver,
}

/// 把一个 Agent 的配置还原成接入网关前的样子。
///
/// Codex++ 的记录里它还接在网关上、配置却已经指向别处（用户用别的工具改过）时，写回存档
/// 会盖掉用户的新选择：这时不写 Agent 的文件，只丢掉存档和网关自己的记录。
fn restore_agent(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    agent: &ToolId,
    binding: &AgentBinding,
) -> anyhow::Result<RestoreOutcome> {
    if let Some(found) = taken_over(ctx, agent) {
        forget_gateway(ctx, agent, binding)?;
        log_taken_over("gateway.restore_skipped_taken_over", agent, &found);
        return Ok(RestoreOutcome::TakenOver);
    }
    match agent {
        ToolId::Claude => restore_claude(ctx, binding),
        ToolId::Grok => restore_grok(ctx, settings),
        ToolId::Codex => restore_codex(ctx, binding),
        _ => Ok(()),
    }?;
    Ok(RestoreOutcome::Restored)
}

/// Codex++ 的记录里这个 Agent 还接在网关上（Codex：直连模式当前是托管供应商；Claude：当前是
/// 网关供应商；Grok：接入时的存档还在，没还原过）。旧版接入、没有存档的 Grok 认不出，按没接算。
fn recorded_on_gateway(ctx: &ApplyContext, agent: &ToolId) -> bool {
    match agent {
        ToolId::Grok => grok_stash_path(ctx).exists(),
        ToolId::Codex | ToolId::Claude => agent_points_at_gateway(ctx, agent),
        _ => false,
    }
}

/// 记录上接在网关、配置却已经指向别处：被别的工具接管了。
fn taken_over(ctx: &ApplyContext, agent: &ToolId) -> Option<Drift> {
    if !recorded_on_gateway(ctx, agent) {
        return None;
    }
    drift::check(ctx, agent)
}

/// Agent 配置现在连的地址；还指向网关时为 None。
fn live_endpoint(ctx: &ApplyContext, agent: &ToolId) -> Option<String> {
    drift::check(ctx, agent).map(|found| found.endpoint)
}

/// 两个地址算同一个：相同，或都是 Codex++ 自己的本地代理（之间换过端口、启动时重写过也算）。
fn same_endpoint(left: &str, right: &str) -> bool {
    left.trim_end_matches('/') == right.trim_end_matches('/')
        || (crate::protocol_proxy::is_managed_protocol_proxy_base_url(left)
            && crate::protocol_proxy::is_managed_protocol_proxy_base_url(right))
}

/// 只记认得出的工具名，不记地址（地址里可能带凭据）。
fn log_taken_over(event: &str, agent: &ToolId, found: &Drift) {
    let _ = crate::diagnostic_log::append_diagnostic_log(
        event,
        serde_json::json!({ "agent": agent.as_str(), "by": found.by }),
    );
}

/// Agent 已被别的工具接管时的断开：不写 Agent 的配置文件，只清掉网关自己的记录与存档。
fn forget_gateway(
    ctx: &ApplyContext,
    agent: &ToolId,
    binding: &AgentBinding,
) -> anyhow::Result<()> {
    match agent {
        ToolId::Claude => {
            if claude::current_provider_id(&ctx.claude)? == GATEWAY_PROVIDER_ID {
                claude::forget_current(&ctx.claude)?;
            }
            if claude::load(&ctx.claude)?
                .providers
                .iter()
                .any(|provider| provider.id == GATEWAY_PROVIDER_ID)
            {
                claude::delete_provider(&ctx.claude, GATEWAY_PROVIDER_ID)?;
            }
            stash::remove_stash(&claude_stash_path(ctx));
        }
        ToolId::Codex => {
            // 借给 Codex 的账号照样收回：那是网关的登录，留在 Codex 里两边会抢着刷新 token。
            if ctx.gateway.load().is_ok() {
                return_lent_login(&ctx.gateway, &ctx.codex_home)?;
            }
            // 直连模式的记录改回开启前的供应商（不切换、不写 ~/.codex），删掉托管供应商。
            let mut settings = ctx.settings.load()?;
            if settings.active_relay_id == CODEX_GATEWAY_RELAY_ID {
                let previous = binding.restore.codex_previous_relay_id.trim();
                let known = |id: &str| {
                    !id.is_empty()
                        && id != CODEX_GATEWAY_RELAY_ID
                        && (settings.relay_profiles.iter().any(|item| item.id == id)
                            || settings
                                .aggregate_relay_profiles
                                .iter()
                                .any(|item| item.id == id))
                };
                settings.active_relay_id = if known(previous) {
                    previous.to_string()
                } else {
                    settings
                        .relay_profiles
                        .iter()
                        .find(|profile| profile.id != CODEX_GATEWAY_RELAY_ID)
                        .map(|profile| profile.id.clone())
                        .unwrap_or_else(crate::settings::default_active_relay_id)
                };
            }
            settings
                .relay_profiles
                .retain(|profile| profile.id != CODEX_GATEWAY_RELAY_ID);
            ctx.settings.save(&settings)?;
            stash::remove_stash(&codex_stash_path(ctx));
        }
        ToolId::Grok => stash::remove_stash(&grok_stash_path(ctx)),
        _ => {}
    }
    Ok(())
}

/// Agent 的配置现在还指向网关（不看 gateway.json 里的绑定）。
fn agent_points_at_gateway(ctx: &ApplyContext, agent: &ToolId) -> bool {
    match agent {
        ToolId::Codex => ctx
            .settings
            .load()
            .is_ok_and(|settings| settings.active_relay_id == CODEX_GATEWAY_RELAY_ID),
        ToolId::Claude => claude::current_provider_id(&ctx.claude)
            .is_ok_and(|current| current == GATEWAY_PROVIDER_ID),
        ToolId::Grok => stash::read_optional(&ctx.grok_home.join("config.toml"))
            .ok()
            .flatten()
            .is_some_and(|text| grok_points_at_gateway(&text)),
        _ => false,
    }
}

/// 切回开启前的供应商（不在了就清掉当前供应商），删掉托管供应商；
/// 接入时存过 settings.json 的话，再把网关改过的键改回接入前的值（接入前的配置如果没存成供应商，
/// 只靠切换会丢）。用户已经在 Claude 页切走了的，只删托管供应商。
fn restore_claude(ctx: &ApplyContext, binding: &AgentBinding) -> anyhow::Result<()> {
    let payload = claude::load(&ctx.claude)?;
    let stash_path = claude_stash_path(ctx);
    if payload.current_id == GATEWAY_PROVIDER_ID {
        let live_path = ctx.claude.claude_home.join("settings.json");
        // 切回之前的样子（还指向网关）：存档没记改了哪些键时据此兜底。
        let on_gateway = stash::read_optional(&live_path)?;
        let previous = binding.restore.claude_previous_provider_id.trim();
        let can_switch_back = !previous.is_empty()
            && payload
                .providers
                .iter()
                .any(|provider| provider.id == previous);
        if can_switch_back {
            claude::switch_provider(&ctx.claude, previous)?;
        } else {
            claude::clear_current(&ctx.claude)?;
        }
        if let Some(saved) = stash::load_stash::<FileStash>(&stash_path) {
            let live = stash::read_optional(&live_path)?;
            let changed = if saved.changed.is_empty() {
                claude_fallback_changes(saved.before.as_deref(), on_gateway.as_deref())?
            } else {
                saved.changed.clone()
            };
            let restored = stash::restore_json(
                saved.before.as_deref(),
                live.as_deref(),
                &changed,
                |_, _, _| false,
            )
            .context("还原 Claude Code 的 settings.json 失败")?;
            stash::write_or_remove(&live_path, restored.as_deref())
                .context("写回 Claude Code 接入网关前的 settings.json 失败")?;
        }
    }
    stash::remove_stash(&stash_path);
    if payload
        .providers
        .iter()
        .any(|provider| provider.id == GATEWAY_PROVIDER_ID)
    {
        claude::delete_provider(&ctx.claude, GATEWAY_PROVIDER_ID)?;
    }
    Ok(())
}

/// 存档里没记改了哪些键（接入时存下原文后、记下改动前进程就退出了，或第二次写存档失败），
/// 配置却指向网关：按切换供应商会动的关键字段兜底，与接入前不同的改回去。
fn claude_fallback_changes(
    before: Option<&str>,
    on_gateway: Option<&str>,
) -> anyhow::Result<Vec<ChangedKey>> {
    let paths = stash::changed_paths(&stash::json_value(before)?, &stash::json_value(on_gateway)?);
    Ok(paths
        .into_iter()
        .filter(|path| match path.as_slice() {
            [env, key] if env == "env" => {
                claude::is_floor_env(key) || claude::is_exclusive_env(key)
            }
            [key] => claude::is_floor_top(key),
            _ => false,
        })
        .map(|path| ChangedKey {
            path,
            written: None,
        })
        .collect())
}

/// Codex 当前在用的直连供应商（不是托管的网关供应商时）。
fn codex_active_relay_id(ctx: &ApplyContext) -> anyhow::Result<String> {
    let active = ctx.settings.load()?.active_relay_id;
    Ok(if active == CODEX_GATEWAY_RELAY_ID {
        String::new()
    } else {
        active
    })
}

/// 切回开启前的供应商（不在了就用列表里第一个），再删掉托管供应商。
/// 用户已经在直连模式里切走了的，只删托管供应商。
fn restore_codex(ctx: &ApplyContext, binding: &AgentBinding) -> anyhow::Result<()> {
    // 先收回借给 Codex 的账号，切回去时上一个供应商写的是它自己的 auth.json。
    // gateway.json 读不出来时没法知道借没借，跳过。
    if ctx.gateway.load().is_ok() {
        return_lent_login(&ctx.gateway, &ctx.codex_home)?;
    }
    let mut settings = ctx.settings.load()?;
    let is_managed = |profile: &RelayProfile| profile.id == CODEX_GATEWAY_RELAY_ID;
    let has_managed = settings.relay_profiles.iter().any(is_managed);
    let on_gateway = settings.active_relay_id == CODEX_GATEWAY_RELAY_ID;
    if !has_managed && !on_gateway {
        return Ok(());
    }
    if on_gateway {
        let previous = binding.restore.codex_previous_relay_id.trim();
        let exists = |id: &str| {
            !id.is_empty()
                && id != CODEX_GATEWAY_RELAY_ID
                && (settings
                    .relay_profiles
                    .iter()
                    .any(|profile| profile.id == id)
                    || settings
                        .aggregate_relay_profiles
                        .iter()
                        .any(|profile| profile.id == id))
        };
        let target = if exists(previous) {
            previous.to_string()
        } else {
            settings
                .relay_profiles
                .iter()
                .find(|profile| !is_managed(profile))
                .map(|profile| profile.id.clone())
                .context("Codex 的直连模式里没有别的供应商可切回：先在 Codex 的供应商页添加一个")?
        };
        // 切回前的实时文件：接入时存过副本的话，据此只改回网关改过的键。
        let live = capture_codex_live(ctx);
        let saved = stash::load_stash::<CodexLiveStash>(&codex_stash_path(ctx));
        settings.active_relay_id = target;
        // 托管供应商还在列表里时按直连模式的正常流程切（会回填公共配置，接入期间加的 MCP 等不丢）；
        // 不在了（被删、或旧版留下的状态），或实时配置被别的工具改过，就不回填、实时文件先备份再切。
        let discard_live = if has_managed {
            match crate::relay_switch::switch_relay_profile_in_home(
                &ctx.settings,
                &ctx.codex_home,
                settings.clone(),
                CODEX_GATEWAY_RELAY_ID,
            ) {
                Ok(_) => false,
                Err(error)
                    if error
                        .downcast_ref::<crate::relay_switch::LiveEndpointConflict>()
                        .is_some() =>
                {
                    true
                }
                Err(error) => return Err(error.context("把 Codex 切回直连失败")),
            }
        } else {
            true
        };
        if discard_live {
            crate::relay_switch::switch_relay_profile_in_home_resolving_conflict(
                &ctx.settings,
                &ctx.codex_home,
                settings,
                CODEX_GATEWAY_RELAY_ID,
                crate::relay_switch::LiveConflictResolution::DiscardLiveChanges,
            )
            .context("把 Codex 切回直连失败")?;
        }
        settings = ctx.settings.load()?;
        if let Some(saved) = saved {
            restore_codex_files(ctx, &saved, &live)?;
        }
        stash::remove_stash(&codex_stash_path(ctx));
    }
    // 切换会把请求里缺的供应商从磁盘补回，所以托管供应商要在切走之后单独删。
    settings
        .relay_profiles
        .retain(|profile| !is_managed(profile));
    ctx.settings.save(&settings)?;
    Ok(())
}

/// 还原 Grok 的 config.toml：接入时存过原文的，把网关改过的键（地址、api_backend、Key、默认模型、
/// 模型别名）改回接入前的值；没有存档（旧版接入的）时重新应用 Grok 页当前选中的供应商。
/// 配置已经不指向网关（用户在 Grok 页换过）时不动。写不回去时报错，不假装成功。
fn restore_grok(ctx: &ApplyContext, settings: &BackendSettings) -> anyhow::Result<()> {
    let live_path = ctx.grok_home.join("config.toml");
    let live = stash::read_optional(&live_path)?;
    let on_gateway = live.as_deref().is_some_and(grok_points_at_gateway);
    let stash_path = grok_stash_path(ctx);
    if let Some(saved) = stash::load_stash::<FileStash>(&stash_path) {
        if on_gateway {
            let paths = if saved.changed.is_empty() {
                grok_fallback_paths(saved.before.as_deref(), live.as_deref())
                    .context("还原 Grok 的 config.toml 失败")?
            } else {
                saved.paths()
            };
            let restored = stash::restore_toml(saved.before.as_deref(), live.as_deref(), &paths)
                .context("还原 Grok 的 config.toml 失败")?;
            stash::write_or_remove(&live_path, restored.as_deref())
                .context("写回 Grok 接入网关前的 config.toml 失败")?;
        }
        stash::remove_stash(&stash_path);
        return Ok(());
    }
    if !on_gateway {
        return Ok(());
    }
    let profile = settings
        .tools
        .get(&ToolId::Grok)
        .and_then(crate::tools::grok::active_grok_profile)
        .filter(|profile| {
            !crate::tools::grok::parse_grok_model_list(&profile.model_list).is_empty()
        })
        .context(
            "Grok 的配置还指向网关，但没有接入前的存档，也没有可应用的 Grok 供应商：\
             请在 Grok 页添加并应用一个供应商",
        )?;
    crate::tools::grok::apply_profile_to_grok_at(&ctx.grok_home, &profile, &ctx.backup_root)?;
    Ok(())
}

/// Grok 应用供应商时会写的顶层键：默认模型、模型列表地址、各模型的配置。
const GROK_GATEWAY_KEYS: &[&str] = &["models", "endpoints", "model"];

/// 存档里没记改了哪些键、配置却指向网关（同 [`claude_fallback_changes`]）：
/// 按网关会写的键兜底，与接入前不同的改回去。
fn grok_fallback_paths(before: Option<&str>, live: Option<&str>) -> anyhow::Result<Vec<KeyPath>> {
    let paths = stash::changed_paths(&stash::toml_value(before)?, &stash::toml_value(live)?);
    Ok(paths
        .into_iter()
        .filter(|path| {
            path.first()
                .is_some_and(|key| GROK_GATEWAY_KEYS.contains(&key.as_str()))
        })
        .collect())
}

/// 用户在 Claude 供应商列表里直接切到别的供应商：网关绑定随之失效，不再尝试恢复。
pub fn note_claude_switched(store: &GatewayStore, target_id: &str) -> anyhow::Result<()> {
    if target_id == GATEWAY_PROVIDER_ID {
        return Ok(());
    }
    store.update(|gateway| {
        if let Some(binding) = gateway.bindings.get_mut(ToolId::Claude.as_str())
            && binding.enabled
        {
            binding.enabled = false;
            binding.restore = Default::default();
        }
        Ok(())
    })?;
    Ok(())
}

/// 用户在 Grok 页直接应用了别的供应商：网关绑定随之失效。
pub fn note_grok_switched(store: &GatewayStore) -> anyhow::Result<()> {
    store.update(|gateway| {
        if let Some(binding) = gateway.bindings.get_mut(ToolId::Grok.as_str())
            && binding.enabled
        {
            binding.enabled = false;
            binding.restore = Default::default();
        }
        Ok(())
    })?;
    Ok(())
}

/// 用户在 Codex 的供应商页直接切换：切到别的供应商等于断开网关；
/// 切回托管的「Codex++ 网关」等于重新接入（断开时它还留在列表里，没被删）。
pub fn note_codex_switched(
    store: &GatewayStore,
    target_id: &str,
    previous_id: &str,
) -> anyhow::Result<()> {
    store.update(|gateway| {
        let Some(binding) = gateway.bindings.get_mut(ToolId::Codex.as_str()) else {
            return Ok(());
        };
        let connected = target_id == CODEX_GATEWAY_RELAY_ID;
        if binding.enabled == connected {
            return Ok(());
        }
        binding.enabled = connected;
        binding.restore = Default::default();
        if connected && previous_id != CODEX_GATEWAY_RELAY_ID {
            binding.restore.codex_previous_relay_id = previous_id.to_string();
        }
        Ok(())
    })?;
    Ok(())
}

/// 直连模式里切换供应商前：Codex 正被网关接管（当前是托管供应商、绑定开着）且要切到别的供应商时拦下。
pub fn codex_switch_blocked_by_gateway(settings: &SettingsStore, target_id: &str) -> bool {
    if target_id == CODEX_GATEWAY_RELAY_ID {
        return false;
    }
    let on_gateway = settings
        .load()
        .is_ok_and(|saved| saved.active_relay_id == CODEX_GATEWAY_RELAY_ID);
    on_gateway
        && GatewayStore::default()
            .load()
            .ok()
            .and_then(|gateway| {
                gateway
                    .binding(&ToolId::Codex)
                    .map(|binding| binding.enabled)
            })
            .unwrap_or(false)
}

/// Codex 开着网关时，托管供应商的会话身份要跟 Codex 的登录状态一致（登录了 ChatGPT 就保留官方身份）：
/// 不一致（之后才登录 / 退出，或旧版写的）就重写一次。返回是否重写了。
pub fn sync_codex_identity(ctx: &ApplyContext) -> anyhow::Result<bool> {
    let gateway = ctx.gateway.load()?;
    let Some(binding) = gateway
        .binding(&ToolId::Codex)
        .filter(|binding| binding.enabled)
    else {
        return Ok(false);
    };
    let settings = ctx.settings.load()?;
    if settings.active_relay_id != CODEX_GATEWAY_RELAY_ID {
        return Ok(false);
    }
    if let Some(found) = taken_over(ctx, &ToolId::Codex) {
        log_taken_over(
            "gateway.identity_sync_skipped_taken_over",
            &ToolId::Codex,
            &found,
        );
        return Ok(false);
    }
    let keeps_login =
        settings.active_relay_session_provider() == crate::settings::RelaySessionProvider::Openai;
    let can_sign_in = codex_signed_in(&ctx.codex_home) || lendable_login(&gateway).is_some();
    if keeps_login == can_sign_in {
        return Ok(false);
    }
    write_codex(ctx, &gateway, &binding.clone())?;
    Ok(true)
}

/// 网关端口变了（例如 Windows 端口被占后换了新端口）：重写所有开着的 Agent 配置。
/// 某个 Agent 失败时继续其它的，只记录成功项的端口，最后返回第一个错误。
pub fn reapply_if_port_changed(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let gateway = ctx.gateway.load()?;
    let mut changed = 0;
    let mut first_error = None;
    let agents = gateway
        .bindings
        .iter()
        .filter(|(_, binding)| binding.enabled && binding.restore.applied_port != Some(ctx.port))
        .map(|(agent, _)| ToolId::parse(agent))
        .collect::<Vec<_>>();
    for agent in agents {
        // 已被别的工具接管的不改回来（漂移检查只看是不是网关地址，不看端口，换端口不算接管）。
        if taken_over(ctx, &agent).is_some() {
            continue;
        }
        let Some(binding) = gateway.bindings.get(agent.as_str()) else {
            continue;
        };
        let mut binding = binding.clone();
        binding.restore.applied_port = Some(ctx.port);
        let result = write_agent_config(ctx, &agent, &binding).and_then(|_| {
            // 写配置时可能改过 gateway.json：只更新本次成功项，不回写旧快照。
            ctx.gateway.update(|latest| {
                if let Some(target) = latest.bindings.get_mut(agent.as_str()) {
                    target.restore.applied_port = Some(ctx.port);
                }
                Ok(())
            })?;
            Ok(())
        });
        match result {
            Ok(()) => changed += 1,
            Err(error) => {
                first_error
                    .get_or_insert(error.context(format!("重新接入 {} 失败", agent.as_str())));
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    Ok(changed)
}

/// 退出 Codex++（用户打开了「退出时恢复直连」，默认开）或安装更新前（`force`，不看开关）
/// 恢复所有走网关的 Agent。绑定保留为开启状态，并记下「已恢复」，下次启动时由 [`reapply_enabled`]
/// 重新指回网关。某个 Agent 还原失败时继续还原其它的，最后返回第一个错误。
pub fn restore_all_for_exit(
    ctx: &ApplyContext,
    settings: &BackendSettings,
    force: bool,
) -> anyhow::Result<()> {
    let gateway = ctx.gateway.load()?;
    if !force && !gateway.restore_on_exit {
        return Ok(());
    }
    let mut outcomes = Vec::new();
    let mut first_error = None;
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        let tool = ToolId::parse(agent);
        match restore_agent(ctx, settings, &tool, binding) {
            Ok(outcome) => outcomes.push((agent.clone(), tool, outcome)),
            Err(error) => {
                first_error.get_or_insert(error.context(format!("还原 {agent} 失败")));
            }
        }
    }
    if !outcomes.is_empty() {
        // 带锁读改写：只动这几个绑定和标记，不盖掉同时写进来的 token。
        ctx.gateway.update(|latest| {
            for (agent, tool, outcome) in &outcomes {
                let Some(binding) = latest.bindings.get_mut(agent) else {
                    continue;
                };
                match outcome {
                    // 记下还原后连的地址：下次启动前用户又改了的话就不再接回。
                    RestoreOutcome::Restored => {
                        binding.restore.restored_endpoint = live_endpoint(ctx, tool);
                        latest.restored_at_exit = true;
                    }
                    // 已被别的工具接管：等于用户自己断开了，绑定关掉，下次启动不再接回。
                    RestoreOutcome::TakenOver => {
                        binding.enabled = false;
                        binding.restore = Default::default();
                    }
                }
            }
            Ok(())
        })?;
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}

/// 供应商池或路由组变化后重写开着的 Agent 配置：Grok 的模型列表、Claude 的默认模型随池更新。
/// 不碰恢复现场；某个 Agent 写失败时继续写其它的，最后返回第一个错误。
/// 已被别的工具接管的 Agent 不重写（界面提示漂移，由用户决定要不要重新接入）。
pub fn refresh_enabled(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let gateway = ctx.gateway.load()?;
    let mut count = 0;
    let mut first_error = None;
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        let agent = ToolId::parse(agent);
        if taken_over(ctx, &agent).is_some() {
            continue;
        }
        match write_agent_config(ctx, &agent, binding) {
            Ok(_) => count += 1,
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(count),
    }
}

/// 启动时把开着的绑定重新写进 Agent 配置（退出 / 更新前恢复过直连，或端口变了），并清掉「已恢复」标记。
///
/// 某个 Agent 写失败时继续写其它的，最后返回第一个错误；有失败时「已恢复」标记留着，下次启动再试。
/// 不接回的：还原之后用户又用别的工具改过的（绑定关掉），以及接着网关时被别的工具接管的（界面提示漂移）。
pub fn reapply_enabled(ctx: &ApplyContext) -> anyhow::Result<usize> {
    let gateway = ctx.gateway.load()?;
    let mut count = 0;
    let mut first_error = None;
    for (agent, binding) in &gateway.bindings {
        if !binding.enabled {
            continue;
        }
        let agent = ToolId::parse(agent);
        match reapply_binding(ctx, &agent, binding, gateway.restored_at_exit) {
            Ok(true) => count += 1,
            Ok(false) => {}
            Err(error) => {
                first_error
                    .get_or_insert(error.context(format!("重新接入 {} 失败", agent.as_str())));
            }
        }
    }
    if let Some(error) = first_error {
        return Err(error);
    }
    ctx.gateway.update(|latest| {
        latest.restored_at_exit = false;
        Ok(())
    })?;
    Ok(count)
}

/// 把一个开着的绑定重新写进 Agent 配置；没写（被别的工具改过）时返回 false。
fn reapply_binding(
    ctx: &ApplyContext,
    agent: &ToolId,
    binding: &AgentBinding,
    restored_at_exit: bool,
) -> anyhow::Result<bool> {
    // 退出 / 更新前还原过，之后用户又把 Agent 指向了别处：等于用户自己断开了，绑定关掉。
    if restored_at_exit
        && let Some(expected) = &binding.restore.restored_endpoint
        && let Some(found) = drift::check(ctx, agent)
        && !same_endpoint(&found.endpoint, expected)
    {
        ctx.gateway.update(|latest| {
            if let Some(saved) = latest.bindings.get_mut(agent.as_str()) {
                saved.enabled = false;
                saved.restore = Default::default();
            }
            Ok(())
        })?;
        log_taken_over("gateway.reapply_skipped_taken_over", agent, &found);
        return Ok(false);
    }
    if taken_over(ctx, agent).is_some() {
        return Ok(false);
    }
    let mut binding = binding.clone();
    binding.restore.applied_port = Some(ctx.port);
    binding.restore.restored_endpoint = None;
    // Claude 恢复过直连时，重新开启前的「上一个供应商」要重新记录。
    if *agent == ToolId::Claude {
        let current = claude::current_provider_id(&ctx.claude)?;
        if current != GATEWAY_PROVIDER_ID && !current.is_empty() {
            binding.restore.claude_previous_provider_id = current;
        }
    }
    if *agent == ToolId::Codex {
        let current = codex_active_relay_id(ctx)?;
        if !current.is_empty() {
            binding.restore.codex_previous_relay_id = current;
        }
    }
    if let Some(snapshot) = write_agent_config(ctx, agent, &binding)?
        && binding.restore.claude_previous_provider_id.is_empty()
    {
        binding.restore.claude_previous_provider_id = snapshot;
    }
    ctx.gateway.update(|latest| {
        latest.bindings.insert(agent.as_str().to_string(), binding);
        Ok(())
    })?;
    Ok(true)
}

/// 把一个 Claude 供应商复制成网关供应商（Anthropic 协议）。只读 Claude 的数据，不改它。
pub fn copy_claude_provider(provider: &ClaudeProvider) -> anyhow::Result<RelayProfile> {
    let env = provider
        .settings_config
        .get("env")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let text = |key: &str| {
        env.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .unwrap_or_default()
            .to_string()
    };
    let base_url = text("ANTHROPIC_BASE_URL");
    if base_url.is_empty() {
        bail!(
            "Claude 供应商「{}」没有 ANTHROPIC_BASE_URL，无法导入",
            provider.name
        );
    }
    if base_url.contains("/gw/claude") {
        bail!("「{}」是网关托管的供应商，不能导入", provider.name);
    }
    let (api_key, bearer) = match (text("ANTHROPIC_AUTH_TOKEN"), text("ANTHROPIC_API_KEY")) {
        (token, _) if !token.is_empty() => (token, true),
        (_, key) if !key.is_empty() => (key, false),
        _ => bail!("Claude 供应商「{}」没有 Key，无法导入", provider.name),
    };
    let mut models = Vec::new();
    for key in [
        "ANTHROPIC_MODEL",
        "ANTHROPIC_DEFAULT_OPUS_MODEL",
        "ANTHROPIC_DEFAULT_SONNET_MODEL",
        "ANTHROPIC_DEFAULT_HAIKU_MODEL",
        "ANTHROPIC_DEFAULT_FABLE_MODEL",
    ] {
        let model = text(key);
        if !model.is_empty() && !models.contains(&model) {
            models.push(model);
        }
    }
    let mut profile = build_relay(&ProviderInput {
        name: provider.name.clone(),
        base_url,
        protocol: RelayProtocol::AnthropicMessages,
        keys: vec![api_key],
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        price: None,
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
    })?;
    if bearer {
        profile.anthropic_auth_header =
            crate::anthropic_bridge::ANTHROPIC_AUTH_HEADER_BEARER.to_string();
    }
    // 直连时给 Claude Code 设的窗口当作这家的默认窗口（模型名里的 `[1m]` 已按模型收进窗口表）。
    let window = text("CLAUDE_CODE_MAX_CONTEXT_TOKENS");
    if crate::model_suffix::parse_window_token(&window).is_some() {
        profile.context_window = window;
    }
    Ok(profile)
}

/// 把 Grok 页的一个供应商复制成网关供应商。只读 Grok 的数据，不改它。
pub fn copy_grok_profile(
    settings: &BackendSettings,
    profile_id: &str,
) -> anyhow::Result<RelayProfile> {
    let source = settings
        .tools
        .get(&ToolId::Grok)
        .and_then(|config| {
            config
                .relay_profiles
                .iter()
                .find(|profile| profile.id == profile_id)
                .cloned()
        })
        .with_context(|| format!("Grok 供应商不存在：{profile_id}"))?;
    let base_url = crate::tools::grok::profile_base_url(&source);
    if base_url.contains("/gw/grok") {
        bail!("「{}」指向网关本身，不能导入", source.name);
    }
    let parsed = crate::tools::grok::parse_grok_model_list(&source.model_list);
    let models = parsed
        .iter()
        .map(|(slug, _)| slug.clone())
        .collect::<Vec<_>>();
    // 模型带的 `[1M]` 窗口放进上游配置的窗口表，加进池时收进网关自己的按模型窗口。
    let windows: BTreeMap<String, String> = parsed
        .into_iter()
        .filter_map(|(slug, window)| window.map(|window| (slug, window.to_string())))
        .collect();
    let mut relay = build_relay(&ProviderInput {
        name: source.name.clone(),
        base_url,
        protocol: source.protocol,
        keys: vec![source.api_key.clone()],
        model: models.first().cloned().unwrap_or_default(),
        model_list: models.join("\n"),
        price: None,
        model_prices: None,
        proxy: None,
        strip_image_generation_tool: None,
    })?;
    if !windows.is_empty() {
        relay.model_windows = serde_json::to_string(&windows)?;
    }
    relay.context_window = source.context_window.clone();
    Ok(relay)
}

/// 可以从 Codex 复制过来的供应商：去掉聚合供应商和官方登录账号。
///
/// 官方登录的 refresh token 每用一次就换新的，刷新结果只写回 Codex 自己那一条；
/// 复制一份会让两边互相把对方的登录刷失效，所以网关里的订阅账号要单独登录。
pub fn codex_importable(settings: &BackendSettings) -> Vec<&RelayProfile> {
    settings
        .relay_profiles
        .iter()
        .filter(|relay| {
            relay.relay_mode != RelayMode::Aggregate
                && !crate::official_login::member_uses_official_login_transport(relay)
        })
        .collect()
}

/// 直连模式里能搬成网关模型组的聚合供应商。`imported`：网关里已有同名模型组。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AggregateSource {
    pub id: String,
    pub name: String,
    pub members: usize,
    /// 搬过来后模型组的模式与会话固定方式。
    pub mode: GroupMode,
    pub sticky: GroupSticky,
    pub imported: bool,
}

/// 只列搬过来会建模型组的聚合：按模型聚合的不列（它的成员就是上面那些普通供应商，池本身按模型名分发），
/// 没有成员的也不列（搬过来什么都不会发生）。
pub fn codex_aggregates(
    settings: &BackendSettings,
    gateway: &GatewaySettings,
) -> Vec<AggregateSource> {
    settings
        .aggregate_relay_profiles
        .iter()
        .filter(|aggregate| !aggregate.model_aggregation_enabled && !aggregate.members.is_empty())
        .map(|aggregate| {
            let (mode, sticky) = aggregate_group_mode(aggregate.strategy);
            AggregateSource {
                id: aggregate.id.clone(),
                name: aggregate.name.clone(),
                members: aggregate.members.len(),
                mode,
                sticky,
                imported: gateway
                    .groups
                    .iter()
                    .any(|group| group.name == aggregate.name),
            }
        })
        .collect()
}

/// 聚合策略对应的模型组模式：故障转移 → 依次；按会话轮换 → 轮换 + 整个会话固定；
/// 按请求轮换 → 轮换 + 不固定；加权 → 按权重。
fn aggregate_group_mode(
    strategy: crate::settings::AggregateRelayStrategy,
) -> (GroupMode, GroupSticky) {
    use crate::settings::AggregateRelayStrategy;
    match strategy {
        AggregateRelayStrategy::Failover => (GroupMode::InOrder, GroupSticky::Cache),
        AggregateRelayStrategy::ConversationRoundRobin => (GroupMode::Rotate, GroupSticky::Session),
        AggregateRelayStrategy::RequestRoundRobin => (GroupMode::Rotate, GroupSticky::Off),
        AggregateRelayStrategy::WeightedRoundRobin => (GroupMode::Weighted, GroupSticky::Cache),
    }
}

/// 直连模式里的官方登录账号（不能复制，要在网关里单独登录）。
pub fn codex_official_accounts(settings: &BackendSettings) -> Vec<&RelayProfile> {
    settings
        .relay_profiles
        .iter()
        .filter(|relay| {
            relay.relay_mode != RelayMode::Aggregate
                && crate::official_login::member_uses_official_login_transport(relay)
        })
        .collect()
}

/// 已从直连模式导入过的 Codex 供应商在网关里的 id。
fn imported_codex_provider(gateway: &GatewaySettings, relay_id: &str) -> Option<String> {
    let source = format!("codex:{relay_id}");
    gateway
        .providers
        .iter()
        .find(|provider| provider.source == source)
        .map(|provider| provider.relay.id.clone())
}

/// 聚合供应商搬进网关的结果。
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AggregateImport {
    /// 新导入池里的成员供应商数。
    pub added: usize,
    /// 建好的模型组 id；按模型聚合或没有可用成员时为空。
    pub group: Option<String>,
    /// 没法照搬的地方，原样告诉用户。
    pub notes: Vec<String>,
}

/// 把直连模式的聚合供应商搬进网关：成员导进池（导入过的复用），再按策略建同名模型组。
/// - 故障转移 → 依次；按会话轮换 → 轮换 + 整个会话固定；按请求轮换 → 轮换 + 不固定；加权 → 按权重；
/// - 按模型聚合不建组：网关池本身就按模型名分发；
/// - 官方登录成员不能复制，用池里已登录的 ChatGPT 账号代替，没有就提示去登录；
/// - 模型路由规则网关里没有对应功能，写进 notes。
pub fn import_codex_aggregate(
    store: &GatewayStore,
    settings: &BackendSettings,
    aggregate_id: &str,
    skip_agents: Option<&[String]>,
) -> anyhow::Result<AggregateImport> {
    let aggregate = settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.id == aggregate_id)
        .with_context(|| format!("聚合供应商不存在：{aggregate_id}"))?;
    let mut result = AggregateImport::default();
    let mut members: Vec<GroupMember> = Vec::new();
    for member in &aggregate.members {
        let Some(relay) = settings
            .relay_profiles
            .iter()
            .find(|relay| relay.id == member.relay_id)
        else {
            result
                .notes
                .push(format!("成员 {} 已经不存在，跳过", member.relay_id));
            continue;
        };
        let provider_id = if crate::official_login::member_uses_official_login_transport(relay) {
            let lendable = store.load().map(|gateway| {
                gateway
                    .providers
                    .iter()
                    .find(|provider| {
                        crate::official_login::member_uses_official_login_transport(&provider.relay)
                    })
                    .map(|provider| provider.relay.id.clone())
            });
            match lendable {
                Ok(Some(id)) => id,
                Ok(None) => {
                    result.notes.push(format!(
                        "「{}」是官方登录账号，不能复制：在网关里登录 ChatGPT 后再把它加进模型组",
                        crate::gateway::accounts::mask_account_name(&relay.name)
                    ));
                    continue;
                }
                Err(error) => {
                    result.notes.push(member_failure(relay, &error));
                    continue;
                }
            }
        } else if relay.relay_mode == RelayMode::Aggregate {
            result
                .notes
                .push(format!("成员「{}」本身是聚合供应商，跳过", relay.name));
            continue;
        } else {
            // 单个成员导入失败只记一笔，不影响已经导入的和后面的成员。
            let imported = (|| -> anyhow::Result<(String, bool)> {
                if let Some(id) = imported_codex_provider(&store.load()?, &relay.id) {
                    return Ok((id, false));
                }
                let copy = copy_codex_provider(settings, &relay.id)?;
                let (_, id) =
                    add_provider(store, copy, &format!("codex:{}", relay.id), skip_agents)?;
                Ok((id, true))
            })();
            match imported {
                Ok((id, added)) => {
                    if added {
                        result.added += 1;
                    }
                    id
                }
                Err(error) => {
                    result.notes.push(member_failure(relay, &error));
                    continue;
                }
            }
        };
        if aggregate.model_aggregation_enabled {
            continue;
        }
        let model = crate::relay_config::relay_profile_model(relay);
        let model = if model.trim().is_empty() {
            route::relay_models(relay)
                .into_iter()
                .next()
                .unwrap_or_default()
        } else {
            model.trim().to_string()
        };
        if model.is_empty() {
            result
                .notes
                .push(format!("成员「{}」没有配置模型，没加进模型组", relay.name));
            continue;
        }
        members.push(GroupMember {
            model: format!("{provider_id}/{model}"),
            weight: member.weight.max(1),
        });
    }
    if !aggregate.routes.is_empty() {
        result.notes.push(format!(
            "{} 条模型路由规则网关里没有对应功能，没有转换；可以把对应模型直接加进供应商池",
            aggregate.routes.len()
        ));
    }
    if aggregate.model_aggregation_enabled {
        result
            .notes
            .push("按模型聚合：网关的供应商池本身按模型名分发，不用建模型组".to_string());
        return Ok(result);
    }
    if members.is_empty() {
        return Ok(result);
    }
    let (mode, sticky) = aggregate_group_mode(aggregate.strategy);
    // 成员已经进池了：建组失败也照实返回已导入的数目，失败原因写进 notes。
    let saved = (|| {
        // 同名模型组已存在就覆盖它，重复搬不会多出一个组。
        let existing = store
            .load()?
            .groups
            .iter()
            .find(|group| group.name == aggregate.name)
            .map(|group| group.id.clone())
            .unwrap_or_default();
        save_group(
            store,
            &GroupInput {
                id: existing,
                name: aggregate.name.clone(),
                mode,
                sticky,
                members,
                ..GroupInput::default()
            },
        )
    })();
    match saved {
        Ok((_, id)) => result.group = Some(id),
        Err(error) => result
            .notes
            .push(format!("模型组「{}」没建成：{error:#}", aggregate.name)),
    }
    Ok(result)
}

/// 聚合成员导入失败时写给用户的一句话（账号名打码）。
fn member_failure(relay: &RelayProfile, error: &anyhow::Error) -> String {
    format!(
        "成员「{}」没导入：{error:#}",
        crate::gateway::accounts::mask_account_name(&relay.name)
    )
}

/// 复制一个 Codex 供应商（地址、协议、多 Key、请求头等原样带上）。
pub fn copy_codex_provider(
    settings: &BackendSettings,
    relay_id: &str,
) -> anyhow::Result<RelayProfile> {
    let relay = settings
        .relay_profiles
        .iter()
        .find(|relay| relay.id == relay_id)
        .with_context(|| format!("Codex 供应商不存在：{relay_id}"))?;
    if relay.relay_mode == RelayMode::Aggregate {
        bail!(
            "「{}」是聚合供应商，网关的组请在「路由」页里新建",
            relay.name
        );
    }
    if crate::official_login::member_uses_official_login_transport(relay) {
        bail!(
            "「{}」是官方登录账号，不能复制：两边共用登录会互相把对方刷失效，请在网关里单独登录",
            relay.name
        );
    }
    if relay.base_url.contains("/gw/") {
        bail!("「{}」指向网关本身，不能导入", relay.name);
    }
    Ok(relay.clone())
}

/// 新建 / 编辑网关供应商的输入。`keys` 每个一行；编辑时为空表示沿用原来的 Key。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInput {
    pub name: String,
    pub base_url: String,
    pub protocol: RelayProtocol,
    #[serde(default)]
    pub keys: Vec<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub model_list: String,
    /// 每百万 token 的价格；不填就不估算费用。按模型的价格优先，这里是其余模型的默认价。
    #[serde(default)]
    pub price: Option<ProviderPrice>,
    /// 按模型的价格；不传表示不改（编辑时保留原来的）。
    #[serde(default)]
    pub model_prices: Option<BTreeMap<String, ProviderPrice>>,
    /// 这个供应商单独走的 HTTP 代理；不传表示不改，空串表示不单独走代理。
    #[serde(default)]
    pub proxy: Option<String>,
    /// 转发前移除托管生图工具（#228）；不传表示不改（编辑时保留原来的）。
    #[serde(default)]
    pub strip_image_generation_tool: Option<bool>,
}

/// 代理地址只接受 http / https / socks5，空串表示不单独走代理。
fn clean_proxy(proxy: &str) -> anyhow::Result<String> {
    let proxy = proxy.trim();
    if proxy.is_empty() {
        return Ok(String::new());
    }
    let ok = ["http://", "https://", "socks5://", "socks5h://"]
        .iter()
        .any(|scheme| proxy.to_ascii_lowercase().starts_with(scheme));
    if !ok || reqwest::Url::parse(proxy).is_err() {
        bail!("代理地址要写成 http://主机:端口（也支持 https:// 与 socks5://）");
    }
    Ok(proxy.to_string())
}

fn clean_keys(keys: &[String]) -> Vec<String> {
    let mut cleaned: Vec<String> = Vec::new();
    for key in keys {
        let key = key.trim();
        if !key.is_empty() && !cleaned.iter().any(|known| known == key) {
            cleaned.push(key.to_string());
        }
    }
    cleaned
}

/// 按输入组装上游配置：多个 Key 写成命名 Key，参与多 Key 轮换。
fn build_relay(input: &ProviderInput) -> anyhow::Result<RelayProfile> {
    if input.base_url.contains("/gw/") {
        bail!("不能把网关自己加进供应商池");
    }
    let keys = clean_keys(&input.keys);
    let Some(first) = keys.first() else {
        bail!("API Key 为空");
    };
    let mut relay = crate::provider_import::pool_relay_profile(
        &input.name,
        &input.base_url,
        first,
        input.protocol,
        &input.model,
        &input.model_list,
        &[],
    )?;
    relay.relay_mode = RelayMode::PureApi;
    relay.strip_image_generation_tool = input.strip_image_generation_tool.unwrap_or(false);
    if keys.len() > 1 {
        relay.api_keys = keys
            .iter()
            .enumerate()
            .map(|(index, key)| RelayApiKey {
                id: format!("key-{}", index + 1),
                name: format!("Key {}", index + 1),
                api_key: key.clone(),
            })
            .collect();
        relay.active_api_key_id = "key-1".to_string();
    }
    crate::relay_config::normalize_relay_profile_for_storage(&mut relay)?;
    Ok(relay)
}

/// 把一份上游配置加进网关池，id 换成网关里不重复的短名。返回新供应商的 id。
///
/// `skip_agents` 是添加时取消勾选的 Agent：新供应商不给它们用。为 `None` 时按各 Agent
/// 自己的「添加供应商时自动加入」设置。
pub fn add_provider(
    store: &GatewayStore,
    relay: RelayProfile,
    source: &str,
    skip_agents: Option<&[String]>,
) -> anyhow::Result<(GatewaySettings, String)> {
    add_provider_with(store, relay, source, skip_agents, |_| Ok(()))
}

/// 同 [`add_provider`]，加进池的同一次带锁写入里再用 `extra` 补设价格、代理等字段。
fn add_provider_with(
    store: &GatewayStore,
    mut relay: RelayProfile,
    source: &str,
    skip_agents: Option<&[String]>,
    extra: impl FnOnce(&mut GatewayProvider) -> anyhow::Result<()>,
) -> anyhow::Result<(GatewaySettings, String)> {
    store.update(|gateway| {
        // 登录得到的账号名里带邮箱，短名用来源（如 `login:chatgpt` → `chatgpt`）更干净。
        let slug_base = source
            .strip_prefix("login:")
            .unwrap_or(&relay.name)
            .to_string();
        relay.id = gateway.unique_slug(&slug_base, false);
        let id = relay.id.clone();
        let mut provider = GatewayProvider {
            enabled: true,
            source: source.to_string(),
            price: None,
            model_prices: BTreeMap::new(),
            model_windows: BTreeMap::new(),
            context_window: None,
            proxy: String::new(),
            upstream_new: Vec::new(),
            upstream_gone: Vec::new(),
            models_checked_at_ms: 0,
            relay,
            extra: Default::default(),
        };
        extra(&mut provider)?;
        gateway.providers.push(provider);
        exclude_new_provider(gateway, &id, skip_agents);
        Ok(id)
    })
}

/// 新供应商不给哪些 Agent 用：勾选优先，没勾选时看 Agent 的 `auto_join_providers`。
fn exclude_new_provider(gateway: &mut GatewaySettings, id: &str, skip_agents: Option<&[String]>) {
    match skip_agents {
        Some(agents) => {
            for agent in agents {
                let binding = gateway.bindings.entry(agent.clone()).or_default();
                if !binding.pool_exclude.iter().any(|excluded| excluded == id) {
                    binding.pool_exclude.push(id.to_string());
                }
            }
        }
        None => {
            for binding in gateway.bindings.values_mut() {
                if !binding.auto_join_providers {
                    binding.pool_exclude.push(id.to_string());
                }
            }
        }
    }
}

/// 用预设或手填的地址、Key 新建网关供应商。
pub fn create_provider(
    store: &GatewayStore,
    input: &ProviderInput,
    source: &str,
    skip_agents: Option<&[String]>,
) -> anyhow::Result<(GatewaySettings, String)> {
    let proxy = clean_proxy(input.proxy.as_deref().unwrap_or_default())?;
    add_provider_with(
        store,
        build_relay(input)?,
        source,
        skip_agents,
        |provider| {
            provider.price = clean_price(input.price.clone());
            provider.model_prices =
                clean_model_prices(input.model_prices.clone().unwrap_or_default());
            provider.proxy = proxy;
            Ok(())
        },
    )
}

/// 全是 0 的价格等于没填。
fn clean_price(price: Option<ProviderPrice>) -> Option<ProviderPrice> {
    price.filter(|price| price.input > 0.0 || price.cached_input > 0.0 || price.output > 0.0)
}

/// 按模型的价格：去掉空模型名和没填的项。
fn clean_model_prices(prices: BTreeMap<String, ProviderPrice>) -> BTreeMap<String, ProviderPrice> {
    prices
        .into_iter()
        .filter_map(|(model, price)| {
            let model = model.trim().to_string();
            let price = clean_price(Some(price))?;
            (!model.is_empty()).then_some((model, price))
        })
        .collect()
}

/// 编辑网关供应商：id 不变（Agent、路由组里的引用都还有效）；`keys` 为空时沿用原来的 Key。
pub fn update_provider(
    store: &GatewayStore,
    id: &str,
    input: &ProviderInput,
) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| update_provider_in(gateway, id, input))?;
    Ok(gateway)
}

fn update_provider_in(
    gateway: &mut GatewaySettings,
    id: &str,
    input: &ProviderInput,
) -> anyhow::Result<()> {
    let Some(provider) = gateway
        .providers
        .iter_mut()
        .find(|provider| provider.relay.id == id)
    else {
        bail!("供应商不存在：{id}");
    };
    let mut input = input.clone();
    let keep_keys = clean_keys(&input.keys).is_empty();
    if keep_keys {
        input.keys = if provider.relay.api_keys.len() > 1 {
            provider
                .relay
                .api_keys
                .iter()
                .map(|key| key.api_key.clone())
                .collect()
        } else {
            vec![provider.relay.api_key.clone()]
        };
    }
    let mut relay = build_relay(&input)?;
    relay.id = id.to_string();
    // 原来的请求头、User-Agent、Anthropic 鉴权方式这些高级设置保留。
    relay.custom_headers = provider.relay.custom_headers.clone();
    relay.user_agent = provider.relay.user_agent.clone();
    relay.anthropic_auth_header = provider.relay.anthropic_auth_header.clone();
    // 按模型的思考等级等模型资料也保留（在供应商池详情里单独编辑）。
    relay.model_metadata = provider.relay.model_metadata.clone();
    // 没传「移除托管生图工具」时沿用原来的选择。
    if input.strip_image_generation_tool.is_none() {
        relay.strip_image_generation_tool = provider.relay.strip_image_generation_tool;
    }
    provider.relay = relay;
    provider.price = clean_price(input.price.clone());
    if let Some(prices) = &input.model_prices {
        provider.model_prices = clean_model_prices(prices.clone());
    }
    if let Some(proxy) = &input.proxy {
        provider.proxy = clean_proxy(proxy)?;
    }
    Ok(())
}

/// 只改供应商的价格与独立代理（订阅账号也能改；地址、Key、模型不动）。
/// `model_prices` 为 `None` 时保留原来按模型的价格。
pub fn set_provider_extras(
    store: &GatewayStore,
    id: &str,
    price: Option<ProviderPrice>,
    model_prices: Option<BTreeMap<String, ProviderPrice>>,
    proxy: &str,
) -> anyhow::Result<GatewaySettings> {
    let proxy = clean_proxy(proxy)?;
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        provider.price = clean_price(price);
        if let Some(prices) = model_prices {
            provider.model_prices = clean_model_prices(prices);
        }
        provider.proxy = proxy;
        Ok(())
    })?;
    store.load()
}

/// 供应商的上下文窗口：按模型的（键是上游模型名）和其余模型用的默认值。0 或不写表示不设。
/// 窗口会写进 Agent 的模型资料，保存后要重新写一遍 Agent 配置（调用方负责）。
pub fn set_provider_windows(
    store: &GatewayStore,
    id: &str,
    windows: BTreeMap<String, u64>,
    fallback: Option<u64>,
) -> anyhow::Result<GatewaySettings> {
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        provider.model_windows = windows
            .into_iter()
            .map(|(model, window)| (model.trim().to_string(), window))
            .filter(|(model, window)| !model.is_empty() && *window > 0)
            .collect();
        provider.context_window = fallback.filter(|window| *window > 0);
        Ok(())
    })?;
    store.load()
}

/// 供应商一个模型的思考等级：内置资料里的（不含用户改过的），界面拿它当没改时的显示值。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelReasoningView {
    pub model: String,
    pub builtin_levels: Vec<String>,
    pub builtin_default: Option<String>,
}

/// 供应商各模型内置的思考等级（按这家的模型列表顺序）。认不出的模型借用通用模板的档位。
pub fn provider_reasoning(
    gateway: &GatewaySettings,
    id: &str,
) -> anyhow::Result<Vec<ModelReasoningView>> {
    let provider = gateway
        .provider(id)
        .with_context(|| format!("供应商不存在：{id}"))?;
    let mut builtin = provider.relay.clone();
    builtin.model_metadata.clear();
    let catalog = crate::aggregate_catalog::build_member_catalog(&builtin)?;
    let entries = catalog["models"].as_array().cloned().unwrap_or_default();
    Ok(route::relay_models(&provider.relay)
        .into_iter()
        .map(|model| {
            let entry = entries.iter().find(|entry| {
                entry["slug"]
                    .as_str()
                    .is_some_and(|slug| slug.eq_ignore_ascii_case(&model))
            });
            let builtin_levels = entry
                .and_then(|entry| entry["supported_reasoning_levels"].as_array())
                .map(|levels| {
                    levels
                        .iter()
                        .filter_map(|level| {
                            level["effort"]
                                .as_str()
                                .or_else(|| level.as_str())
                                .map(str::to_string)
                        })
                        .collect()
                })
                .unwrap_or_default();
            let builtin_default = entry
                .and_then(|entry| entry["default_reasoning_level"].as_str())
                .map(str::to_string);
            ModelReasoningView {
                model,
                builtin_levels,
                builtin_default,
            }
        })
        .collect())
}

/// 保存供应商的模型资料（按模型的思考等级、默认等级、思考参数怎么发），格式与直连模式的
/// 「模型配置」一致。写进 Agent 的模型目录，保存后要重新写一遍 Agent 配置（调用方负责）。
pub fn set_provider_metadata(
    store: &GatewayStore,
    id: &str,
    metadata: &str,
) -> anyhow::Result<GatewaySettings> {
    crate::relay_config::parse_model_metadata_map(metadata)?;
    store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        provider.relay.model_metadata = metadata.trim().to_string();
        Ok(())
    })?;
    store.load()
}

/// 删除网关供应商：同时清掉路由组里指向它的成员和 Agent「挑选」里的记录，以及内存里按 id 记的
/// Key 健康、额度、速度（id 会被之后添加的供应商复用，比如重新登录的 `chatgpt`）。
/// 直连模式的数据不受影响。
pub fn delete_provider(store: &GatewayStore, id: &str) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        let before = gateway.providers.len();
        gateway.providers.retain(|provider| provider.relay.id != id);
        if gateway.providers.len() == before {
            bail!("供应商不存在：{id}");
        }
        // `<id>/…` 的成员一般指向它；但模型名本身带 `/`、被别的供应商声明的（如 OpenRouter 的
        // `openai/gpt-5`），以及还能匹配到别家模型的通配符，留着。
        let remaining = &gateway.providers;
        for group in &mut gateway.groups {
            group
                .members
                .retain(|member| route::member_outlives_provider(remaining, id, &member.model));
        }
        for binding in gateway.bindings.values_mut() {
            binding.pool_exclude.retain(|excluded| excluded != id);
        }
        Ok(())
    })?;
    crate::gateway::key_pool::reset(id);
    crate::gateway::quota::forget(id);
    crate::gateway::speed::forget(id);
    Ok(gateway)
}

/// 打开或关掉池里的某个供应商：关掉后不参与路由，也不出现在 Agent 的模型列表里。
pub fn set_provider_enabled(
    store: &GatewayStore,
    id: &str,
    enabled: bool,
) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        let Some(provider) = gateway
            .providers
            .iter_mut()
            .find(|provider| provider.relay.id == id)
        else {
            bail!("供应商不存在：{id}");
        };
        provider.enabled = enabled;
        Ok(())
    })?;
    Ok(gateway)
}

/// 调整池里供应商的顺序（同一个模型有多家时按这个顺序尝试）。没列出的保持原顺序排在后面。
pub fn reorder_providers(store: &GatewayStore, ids: &[String]) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        let mut ordered: Vec<GatewayProvider> = Vec::new();
        for id in ids {
            if let Some(position) = gateway
                .providers
                .iter()
                .position(|provider| &provider.relay.id == id)
            {
                ordered.push(gateway.providers.remove(position));
            }
        }
        ordered.append(&mut gateway.providers);
        gateway.providers = ordered;
        Ok(())
    })?;
    Ok(gateway)
}

/// 新建 / 编辑模型组的输入。`id` 为空表示新建。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupInput {
    #[serde(default)]
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mode: GroupMode,
    #[serde(default)]
    pub sticky: GroupSticky,
    #[serde(default)]
    pub members: Vec<GroupMember>,
    /// 隐藏成员：组里的模型不再单独出现在 Agent 的模型列表里。
    #[serde(default)]
    pub hide_members: bool,
    /// 手动模式下选中的成员。
    #[serde(default)]
    pub pick: String,
}

/// 保存模型组。成员去空、去重；不能直接包含自己。返回组 id。
pub fn save_group(
    store: &GatewayStore,
    input: &GroupInput,
) -> anyhow::Result<(GatewaySettings, String)> {
    store.update(|gateway| save_group_in(gateway, input))
}

fn save_group_in(gateway: &mut GatewaySettings, input: &GroupInput) -> anyhow::Result<String> {
    let name = input.name.trim();
    if name.is_empty() {
        bail!("请填写模型组名称");
    }
    let id = if input.id.trim().is_empty() {
        gateway.unique_slug(name, true)
    } else {
        input.id.trim().to_string()
    };
    let own = format!("{GROUP_MODEL_PREFIX}{id}");
    let mut members: Vec<GroupMember> = Vec::new();
    for member in &input.members {
        let model = member.model.trim();
        if model.is_empty() || model.eq_ignore_ascii_case(&own) {
            continue;
        }
        if !members.iter().any(|known| known.model == model) {
            members.push(GroupMember {
                model: model.to_string(),
                weight: member.weight.max(1),
            });
        }
    }
    if members.is_empty() {
        bail!("模型组至少要有一个成员");
    }
    let mut group = GatewayGroup {
        id: id.clone(),
        name: name.to_string(),
        mode: input.mode,
        sticky: input.sticky,
        members,
        hide_members: input.hide_members,
        pick: input.pick.trim().to_string(),
        extra: Default::default(),
    };
    match gateway.groups.iter_mut().find(|known| known.id == id) {
        Some(existing) => {
            // 更新版本写的未知字段跟着原来的组走。
            group.extra = std::mem::take(&mut existing.extra);
            *existing = group;
        }
        None => gateway.groups.push(group),
    }
    Ok(id)
}

/// 保存自动组的设置（键是模型名，不分大小写）。和默认一样时删掉记录，自动组回到「没改过」。
pub fn save_auto_group(
    store: &GatewayStore,
    model: &str,
    settings: AutoGroupSettings,
) -> anyhow::Result<GatewaySettings> {
    let key = model.trim().to_ascii_lowercase();
    if key.is_empty() {
        bail!("没有指定自动组的模型");
    }
    let (gateway, _) = store.update(|gateway| {
        let mut settings = settings;
        settings.pick = settings.pick.trim().to_string();
        if settings == AutoGroupSettings::default() {
            gateway.auto_groups.remove(&key);
        } else {
            gateway.auto_groups.insert(key.clone(), settings);
        }
        Ok(())
    })?;
    Ok(gateway)
}

/// 删除模型组：其它组里引用它的成员一并去掉。
pub fn delete_group(store: &GatewayStore, id: &str) -> anyhow::Result<GatewaySettings> {
    let (gateway, ()) = store.update(|gateway| {
        let before = gateway.groups.len();
        gateway.groups.retain(|group| group.id != id);
        if gateway.groups.len() == before {
            bail!("模型组不存在：{id}");
        }
        let reference = format!("{GROUP_MODEL_PREFIX}{id}");
        for group in &mut gateway.groups {
            group.members.retain(|member| member.model != reference);
        }
        Ok(())
    })?;
    Ok(gateway)
}

/// 绑定解析后的默认模型，UI 展示用。
pub fn resolved_default_model(gateway: &GatewaySettings, binding: &AgentBinding) -> String {
    route::binding_default_model(gateway, binding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{
        AggregateRelayMember, AggregateRelayProfile, AggregateRelayStrategy, RelaySessionProvider,
    };

    #[test]
    fn catalog_ui_fields_keep_fast_tiers_and_drop_bulky_fields() {
        let entry = json!({
            "slug": "gpt-6.1-sol",
            "display_name": "GPT-6.1 Sol",
            "base_instructions": "很长的系统指令",
            "service_tiers": [{ "id": "priority", "name": "Fast" }],
            "additional_speed_tiers": ["fast"],
        });
        let fields = catalog_ui_fields(&entry);
        assert_eq!(
            fields["service_tiers"][0]["id"], "priority",
            "Fast 靠它解锁"
        );
        assert_eq!(fields["additional_speed_tiers"][0], "fast");
        assert!(!fields.contains_key("base_instructions"));
    }

    fn direct_relay(id: &str, base_url: &str, model: &str) -> RelayProfile {
        RelayProfile {
            id: id.to_string(),
            name: id.to_string(),
            relay_mode: RelayMode::PureApi,
            base_url: base_url.to_string(),
            api_key: format!("sk-{id}"),
            model: model.to_string(),
            model_list: model.to_string(),
            ..RelayProfile::default()
        }
    }

    #[test]
    fn aggregate_import_keeps_going_when_a_member_fails() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let settings = BackendSettings {
            relay_profiles: vec![
                direct_relay("agg-a", "https://a.test/v1", "model-a"),
                // 指向网关本身的成员复制会失败。
                direct_relay("agg-bad", "http://127.0.0.1:1/gw/codex/v1", "model-bad"),
                direct_relay("agg-c", "https://c.test/v1", "model-c"),
            ],
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: "agg".to_string(),
                name: "混合".to_string(),
                session_provider: RelaySessionProvider::Custom,
                model_aggregation_enabled: false,
                strategy: AggregateRelayStrategy::Failover,
                members: ["agg-a", "agg-bad", "agg-c"]
                    .into_iter()
                    .map(|id| AggregateRelayMember {
                        relay_id: id.to_string(),
                        weight: 1,
                    })
                    .collect(),
                routes: Vec::new(),
                default_relay_id: String::new(),
                sync_default_member_login: false,
                auto_review_relay_id: String::new(),
            }],
            ..BackendSettings::default()
        };
        let outcome = import_codex_aggregate(&store, &settings, "agg", None).unwrap();
        assert_eq!(outcome.added, 2, "出错的成员之外照常导入");
        assert!(
            outcome.notes.iter().any(|note| note.contains("agg-bad")),
            "{:?}",
            outcome.notes
        );
        let gateway = store.load().unwrap();
        let group = gateway.group(outcome.group.as_deref().unwrap()).unwrap();
        assert_eq!(group.members.len(), 2);
        assert_eq!(gateway.providers.len(), 2);
    }

    #[test]
    fn deleting_a_provider_keeps_slashed_members_and_forgets_runtime_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let (_, openai) = add_provider(
            &store,
            direct_relay("x", "https://openai.test/v1", "gpt-4o"),
            "",
            None,
        )
        .unwrap();
        let mut router = direct_relay("y", "https://router.test/v1", "openai/gpt-5");
        router.name = "router".to_string();
        add_provider(&store, router, "", None).unwrap();
        assert_eq!(openai, "x");
        save_group(
            &store,
            &GroupInput {
                name: "mix".to_string(),
                members: ["x/gpt-4o", "x/*", "router/openai/gpt-5"]
                    .into_iter()
                    .map(|model| GroupMember {
                        model: model.to_string(),
                        weight: 1,
                    })
                    .collect(),
                ..GroupInput::default()
            },
        )
        .unwrap();
        crate::gateway::speed::record("x", "gpt-4o", std::time::Duration::from_millis(10));
        crate::gateway::quota::record(
            "x",
            crate::gateway::quota::ProviderQuota {
                windows: vec![crate::gateway::quota::QuotaWindow {
                    window_minutes: Some(300),
                    used_percent: 99.0,
                    resets_at_ms: None,
                }],
                updated_at_ms: 0,
            },
        );

        let gateway = delete_provider(&store, "x").unwrap();
        let members: Vec<&str> = gateway.groups[0]
            .members
            .iter()
            .map(|member| member.model.as_str())
            .collect();
        assert_eq!(members, ["router/openai/gpt-5"]);
        assert!(crate::gateway::quota::get("x").is_none(), "额度跟着清掉");
        assert!(crate::gateway::speed::latency("x", "gpt-4o").is_none());
    }

    #[test]
    fn deleting_a_provider_keeps_members_that_name_another_providers_model() {
        let temp = tempfile::tempdir().unwrap();
        let store = GatewayStore::at(temp.path().join("gateway.json"));
        let mut openai = direct_relay("openai-src", "https://openai.test/v1", "gpt-5");
        openai.name = "openai".to_string();
        add_provider(&store, openai, "", None).unwrap();
        let mut router = direct_relay("router-src", "https://router.test/v1", "openai/gpt-5");
        router.name = "router".to_string();
        add_provider(&store, router, "", None).unwrap();
        save_group(
            &store,
            &GroupInput {
                name: "slashed".to_string(),
                members: ["openai/gpt-5", "openai/gpt-4o", "openai/*"]
                    .into_iter()
                    .map(|model| GroupMember {
                        model: model.to_string(),
                        weight: 1,
                    })
                    .collect(),
                ..GroupInput::default()
            },
        )
        .unwrap();
        let gateway = delete_provider(&store, "openai").unwrap();
        let members: Vec<&str> = gateway.groups[0]
            .members
            .iter()
            .map(|member| member.model.as_str())
            .collect();
        assert_eq!(
            members,
            ["openai/gpt-5", "openai/*"],
            "OpenRouter 声明的 openai/gpt-5 和还能匹配到它的通配符留下"
        );
    }

    #[test]
    fn pretty_model_names_match_the_manager() {
        for (raw, pretty) in [
            ("gpt-6.1-sol", "GPT-6.1-Sol"),
            ("deepseek-v4-pro", "DeepSeek-V4-Pro"),
            ("claude-opus-5-5", "Claude-Opus-5-5"),
            ("kimi-k2.5", "Kimi-K2.5"),
            ("qwen3-235b-a22b", "Qwen3-235B-A22B"),
            ("o3-mini", "o3-Mini"),
            ("glm-5.3", "GLM-5.3"),
            ("anthropic/claude-sonnet-5", "anthropic/Claude-Sonnet-5"),
            ("MiniMax-M3", "MiniMax-M3"),
        ] {
            assert_eq!(pretty_model_name(raw), pretty, "{raw}");
        }
    }
}
