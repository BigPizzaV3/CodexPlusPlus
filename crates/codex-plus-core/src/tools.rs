//! 多工具适配层：CC Switch 式的「按工具切换」，每个工具在自己的命名空间里
//! 存放供应商 profile 与写入目标，互相不串。
//!
//! 现状：Codex 的历史配置仍以 `BackendSettings` 上的扁平字段为唯一事实来源，
//! 本模块负责把它镜像进 `settings.json` 的 `tools.codex` 分片，同时为后续工具
//! （Grok / Claude Code / …）预留同名分片。迁移是纯增量的 —— 扁平字段不删、
//! 不改名、继续序列化，所以老版本管理器读新文件照常工作。

use std::path::PathBuf;

use crate::settings::{
    AggregateRelayProfile, BackendSettings, RelayProfile, default_active_relay_id,
    default_relay_test_model,
};

/// 工具标识。`Unknown` 兜底任何本版本还不认识的值，保证前向兼容：
/// 新版本写进去的 `tools.claude` 被老版本读到也不会解析失败。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ToolId {
    Codex,
    Grok,
    Unknown(String),
}

impl Default for ToolId {
    fn default() -> Self {
        Self::Codex
    }
}

impl ToolId {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Codex => "codex",
            Self::Grok => "grok",
            Self::Unknown(value) => value.as_str(),
        }
    }

    pub fn parse(value: &str) -> Self {
        match value.trim() {
            "codex" => Self::Codex,
            "grok" => Self::Grok,
            other => Self::Unknown(other.to_string()),
        }
    }
}

impl serde::Serialize for ToolId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> serde::Deserialize<'de> for ToolId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(Self::parse(&value))
    }
}

/// 单个工具的配置分片：供应商列表 + 当前选中项 + 该工具自己的公共配置片段。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolConfig {
    #[serde(default)]
    pub relay_profiles: Vec<RelayProfile>,
    #[serde(default)]
    pub active_relay_id: String,
    #[serde(default)]
    pub aggregate_relay_profiles: Vec<AggregateRelayProfile>,
    #[serde(default)]
    pub active_aggregate_relay_id: String,
    #[serde(default)]
    pub relay_common_config_contents: String,
    #[serde(default)]
    pub relay_context_config_contents: String,
    #[serde(default)]
    pub relay_test_model: String,
}

impl Default for ToolConfig {
    fn default() -> Self {
        Self {
            relay_profiles: crate::settings::default_relay_profiles(),
            active_relay_id: default_active_relay_id(),
            aggregate_relay_profiles: Vec::new(),
            active_aggregate_relay_id: String::new(),
            relay_common_config_contents: String::new(),
            relay_context_config_contents: String::new(),
            relay_test_model: default_relay_test_model(),
        }
    }
}

/// 工具的静态元信息，供 UI 顶栏切换条渲染。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    pub id: ToolId,
    pub name: String,
    /// 该工具的配置根目录（Codex 是 CODEX_HOME，Grok 是 ~/.grok）。
    pub home_dir: String,
    /// 供应商配置写入能力是否已经接好。未接好的工具在 UI 上可见但不可切。
    pub switchable: bool,
}

pub const TOOL_CODEX_NAME: &str = "Codex";
pub const TOOL_GROK_NAME: &str = "Grok";

/// 已注册的工具，顺序即 UI 顶栏顺序。
pub const REGISTERED_TOOLS: &[ToolId] = &[ToolId::Codex, ToolId::Grok];

pub fn tool_display_name(id: &ToolId) -> &'static str {
    match id {
        ToolId::Codex => TOOL_CODEX_NAME,
        ToolId::Grok => TOOL_GROK_NAME,
        ToolId::Unknown(_) => "未知工具",
    }
}

/// 供应商 profile 的写入目标目录。
pub fn tool_home_dir(id: &ToolId) -> PathBuf {
    match id {
        ToolId::Codex => crate::relay_config::default_codex_home_dir(),
        ToolId::Grok => crate::grok_config::default_grok_home_dir(),
        // 未注册的工具没有已知 home，退回配置根目录，避免调用方拿到 panic。
        ToolId::Unknown(_) => crate::paths::default_app_state_dir(),
    }
}

/// 该工具的 profile 是否已经能真正写盘并切换。Grok 目前只有
/// `grok_config.rs` 的模型列表读写，还没接进 RelayProfile 体系。
pub fn tool_is_switchable(id: &ToolId) -> bool {
    matches!(id, ToolId::Codex)
}

pub fn tool_specs() -> Vec<ToolSpec> {
    REGISTERED_TOOLS
        .iter()
        .map(|id| ToolSpec {
            id: id.clone(),
            name: tool_display_name(id).to_string(),
            home_dir: tool_home_dir(id).to_string_lossy().to_string(),
            switchable: tool_is_switchable(id),
        })
        .collect()
}

impl BackendSettings {
    /// 读取某个工具的配置分片。Codex 直接由扁平字段组装，保证与所有历史
    /// 读写路径字节级一致；其它工具读自己的分片。
    pub fn tool_config(&self, id: &ToolId) -> ToolConfig {
        match id {
            ToolId::Codex => self.codex_tool_config(),
            other => self
                .tools
                .get(other)
                .cloned()
                .unwrap_or_else(ToolConfig::default),
        }
    }

    /// 写入某个工具的配置分片。Codex 写回扁平字段（保持既有行为不变），
    /// 其它工具写进 `tools` 分片。
    pub fn set_tool_config(&mut self, id: &ToolId, config: ToolConfig) {
        match id {
            ToolId::Codex => {
                self.relay_profiles = config.relay_profiles;
                self.active_relay_id = config.active_relay_id;
                self.aggregate_relay_profiles = config.aggregate_relay_profiles;
                self.active_aggregate_relay_id = config.active_aggregate_relay_id;
                self.relay_common_config_contents = config.relay_common_config_contents;
                self.relay_context_config_contents = config.relay_context_config_contents;
                self.relay_test_model = config.relay_test_model;
            }
            other => {
                self.tools.insert(other.clone(), config);
            }
        }
    }

    /// 由扁平字段组装 Codex 分片。
    fn codex_tool_config(&self) -> ToolConfig {
        ToolConfig {
            relay_profiles: self.relay_profiles.clone(),
            active_relay_id: self.active_relay_id.clone(),
            aggregate_relay_profiles: self.aggregate_relay_profiles.clone(),
            active_aggregate_relay_id: self.active_aggregate_relay_id.clone(),
            relay_common_config_contents: self.relay_common_config_contents.clone(),
            relay_context_config_contents: self.relay_context_config_contents.clone(),
            relay_test_model: self.relay_test_model.clone(),
        }
    }

    /// 把扁平字段镜像进 `tools.codex`，并保留其它工具分片不动。
    ///
    /// 每次 load / save / update 都会调用，所以两边不会漂移；这也让
    /// 「新版写入 → 老版读取」和「老版写入 → 新版读取」两个方向都成立。
    pub fn sync_tool_shards(&mut self) {
        let codex = self.codex_tool_config();
        self.tools.insert(ToolId::Codex, codex);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::RelayMode;

    #[test]
    fn tool_id_round_trips_through_json() {
        for id in [ToolId::Codex, ToolId::Grok] {
            let encoded = serde_json::to_string(&id).unwrap();
            assert_eq!(encoded, format!("\"{}\"", id.as_str()));
            let decoded: ToolId = serde_json::from_str(&encoded).unwrap();
            assert_eq!(decoded, id);
        }
    }

    #[test]
    fn unknown_tool_id_survives_round_trip() {
        let decoded: ToolId = serde_json::from_str("\"claude\"").unwrap();
        assert_eq!(decoded, ToolId::Unknown("claude".to_string()));
        // 关键：未知工具再序列化回去必须还是原字符串，不能退化成 null / 丢失。
        assert_eq!(serde_json::to_string(&decoded).unwrap(), "\"claude\"");
    }

    #[test]
    fn sync_mirrors_flat_fields_into_codex_shard() {
        let mut settings = BackendSettings::default();
        settings.active_relay_id = "relay-x".to_string();
        settings.relay_profiles[0].name = "测试供应商".to_string();
        settings.relay_profiles[0].relay_mode = RelayMode::PureApi;

        settings.sync_tool_shards();

        let shard = settings.tool_config(&ToolId::Codex);
        assert_eq!(shard.active_relay_id, "relay-x");
        assert_eq!(shard.relay_profiles[0].name, "测试供应商");
        assert_eq!(shard.relay_profiles[0].relay_mode, RelayMode::PureApi);
        assert_eq!(shard.relay_profiles, settings.relay_profiles);
    }

    #[test]
    fn sync_keeps_other_tool_shards_intact() {
        let mut settings = BackendSettings::default();
        let mut grok = ToolConfig {
            active_relay_id: "grok-relay".to_string(),
            ..ToolConfig::default()
        };
        grok.relay_profiles = vec![RelayProfile {
            id: "grok-relay".to_string(),
            name: "Grok 供应商".to_string(),
            ..RelayProfile::default()
        }];
        settings.tools.insert(ToolId::Grok, grok);

        settings.active_relay_id = "codex-relay".to_string();
        settings.sync_tool_shards();

        // 改 Codex 不能碰到 Grok —— 这是整个分区的存在意义。
        assert_eq!(
            settings.tool_config(&ToolId::Codex).active_relay_id,
            "codex-relay"
        );
        assert_eq!(
            settings.tool_config(&ToolId::Grok).active_relay_id,
            "grok-relay"
        );
    }

    #[test]
    fn set_tool_config_for_codex_writes_flat_fields() {
        let mut settings = BackendSettings::default();
        let mut config = settings.tool_config(&ToolId::Codex);
        config.active_relay_id = "switched".to_string();
        settings.set_tool_config(&ToolId::Codex, config);

        // 走扁平字段，老代码路径读到的就是新值。
        assert_eq!(settings.active_relay_id, "switched");
        settings.sync_tool_shards();
        assert_eq!(
            settings.tool_config(&ToolId::Codex).active_relay_id,
            "switched"
        );
    }

    #[test]
    fn missing_shard_falls_back_to_default() {
        let settings = BackendSettings::default();
        let grok = settings.tool_config(&ToolId::Grok);
        assert_eq!(grok.active_relay_id, default_active_relay_id());
        assert_eq!(
            grok.relay_profiles,
            crate::settings::default_relay_profiles()
        );
    }

    #[test]
    fn tool_specs_expose_registered_tools() {
        let specs = tool_specs();
        assert_eq!(specs.len(), REGISTERED_TOOLS.len());
        assert_eq!(specs[0].id, ToolId::Codex);
        assert_eq!(specs[0].name, TOOL_CODEX_NAME);
        assert!(specs[0].switchable);
        assert_eq!(specs[1].id, ToolId::Grok);
        // Grok 还没接入 profile 写盘，UI 上必须显示为不可切。
        assert!(!specs[1].switchable);
        assert!(specs.iter().all(|spec| !spec.home_dir.is_empty()));
    }
}
