/**
 * @description 聚合供应商轮转选择器，负责按失败、对话、请求和权重策略选择已有中转配置。
 * @author Albert_Luo
 * @email 480199976@qq.com
 * @date 2026-05-27 00:00
 */
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::json;
use sha2::{Digest, Sha256};

use crate::diagnostic_log::append_diagnostic_log;
use crate::settings::{
    AggregateRelayProfile, AggregateRelayRoute, AggregateRelayStrategy, BackendSettings,
    RelayProfile,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectionError {
    NoActiveAggregate,
    SettingsSnapshotMismatch,
    EmptyAggregateMembers {
        aggregate_id: String,
    },
    UnknownMemberRelay {
        aggregate_id: String,
        relay_id: String,
    },
    InvalidMemberRelay {
        aggregate_id: String,
        relay_id: String,
    },
}

impl std::fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SelectionError::NoActiveAggregate => write!(formatter, "未找到当前聚合供应商"),
            SelectionError::SettingsSnapshotMismatch => write!(formatter, "设置与接纳快照不匹配"),
            SelectionError::EmptyAggregateMembers { aggregate_id } => {
                write!(formatter, "聚合供应商「{aggregate_id}」没有成员")
            }
            SelectionError::UnknownMemberRelay {
                aggregate_id,
                relay_id,
            } => write!(
                formatter,
                "聚合供应商「{aggregate_id}」引用了不存在的供应商「{relay_id}」"
            ),
            SelectionError::InvalidMemberRelay {
                aggregate_id,
                relay_id,
            } => write!(
                formatter,
                "聚合供应商「{aggregate_id}」成员「{relay_id}」缺少可用的 API 配置或官方登录凭据"
            ),
        }
    }
}

/// 聚合成员可用性：普通 API 成员要求 base_url 与认证齐备；
/// 官方登录成员要求是纯官方 profile 且快照里带 ChatGPT 登录凭据
///（token 刷新在请求路径处理）。
pub(crate) fn member_relay_ready(relay: &RelayProfile) -> bool {
    if relay.relay_mode == crate::settings::RelayMode::Aggregate {
        return false;
    }
    if crate::official_login::member_uses_official_login_transport(relay) {
        return true;
    }
    !relay.base_url.trim().is_empty() && (!relay.api_key.trim().is_empty() || relay.uses_no_auth())
}

impl std::error::Error for SelectionError {}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RotationContext {
    pub conversation_id: Option<String>,
    pub model: Option<String>,
}

impl RotationContext {
    pub fn for_conversation(conversation_id: impl Into<String>) -> Self {
        Self {
            conversation_id: Some(conversation_id.into()),
            model: None,
        }
    }
}

/// 路由匹配过程结果信息，供 protocol_proxy 记录 diagnostic_log
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteMatchInfo {
    /// 规则命中且目标成员有效（含目标 relayId 与规则 pattern）
    Matched { relay_id: String, pattern: String },
    /// 规则命中但目标成员无效（不存在或缺少 base_url/api_key），已跳过
    SkippedInvalidRelay { relay_id: String, pattern: String },
    /// 路由未命中（仅当 routes 非空且 model 存在时产生）
    Unmatched,
}

/// 通配符匹配：pattern 与 model 均 trim + 忽略大小写；`*` 匹配任意 0+ 字符；
/// 无 `*` 时精确匹配；含 `*` 时按 `*` 分段，各段在 model 中按序且不重叠出现
pub fn match_route_pattern(pattern: &str, model: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    let model = model.trim().to_lowercase();
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('*') {
        return pattern == model;
    }
    let segments: Vec<&str> = pattern.split('*').collect();
    let mut position = 0usize;
    for (index, segment) in segments.iter().enumerate() {
        if segment.is_empty() {
            continue;
        }
        if index == 0 {
            // 首段必须是前缀
            if !model[position..].starts_with(segment) {
                return false;
            }
            position += segment.len();
        } else if index == segments.len() - 1 {
            // 末段必须是后缀，且不能与已匹配部分重叠
            if !model.ends_with(segment) || model.len() - segment.len() < position {
                return false;
            }
            position = model.len();
        } else {
            // 中间段按序出现
            let Some(relative) = model[position..].find(segment) else {
                return false;
            };
            position += relative + segment.len();
        }
    }
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RotationEvent {
    Success,
    Failure,
}

#[derive(Debug, Clone)]
pub struct RelayRotationSelector {
    aggregate: AggregateRelayProfile,
    failover_index: usize,
    failover_version: u64,
    request_index: usize,
    weighted_index: usize,
    conversation_assignments: HashMap<String, String>,
}

/// 只可由 SettingsStore 持有 write_lock 的协调读闭包接纳。
/// revision 是内容身份，epoch 才代表接纳顺序。
#[derive(Debug, Clone)]
pub struct RelaySettingsSnapshot {
    revision: String,
    epoch: u64,
    settings_identity: String,
}

#[derive(Debug, Clone)]
pub struct RelayRotationSnapshot {
    revision: String,
    generation: u64,
    relay_id: String,
    failover_version: u64,
    failover_position: usize,
    feedback_used: Arc<AtomicBool>,
}

impl RelayRotationSnapshot {
    /// 每次真实上游 attempt 前调用；fallback 必须为自己的 relay 单独创建 token。
    /// clone 表示同一次 attempt，共享一次性反馈；本方法表示新的 attempt。
    /// 根选择后即使游标已经绕回，相同 relay 的新 attempt 也以此刻游标为准。
    pub fn for_attempt(&self, relay_id: &str) -> Self {
        let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
        let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let selector = (guard.generation == self.generation && guard.revision == self.revision)
            .then_some(guard.selector.as_ref())
            .flatten();
        Self::capture(&self.revision, self.generation, relay_id, selector)
    }

    fn capture(
        revision: &str,
        generation: u64,
        relay_id: &str,
        selector: Option<&RelayRotationSelector>,
    ) -> Self {
        Self {
            revision: revision.to_string(),
            generation,
            relay_id: relay_id.to_string(),
            failover_version: selector.map_or(0, |selector| selector.failover_version),
            failover_position: selector.map_or(0, |selector| selector.failover_index),
            feedback_used: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Debug, Default)]
struct GlobalSelectorState {
    selector: Option<RelayRotationSelector>,
    revision: String,
    generation: u64,
    authoritative_admission: bool,
    settings_identity: String,
    legacy_unversioned: bool,
}

static GLOBAL_SELECTOR: OnceLock<Mutex<GlobalSelectorState>> = OnceLock::new();

/// 所有会使用全局轮转状态的库内测试共用；guard 必须持有到测试结束。
#[cfg(test)]
pub(crate) fn rotation_test_guard() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    let guard = LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    *GLOBAL_SELECTOR
        .get_or_init(|| Mutex::new(GlobalSelectorState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = GlobalSelectorState::default();
    guard
}

impl RelayRotationSelector {
    pub fn from_settings(settings: &BackendSettings) -> Result<Self, SelectionError> {
        let aggregate = active_aggregate(settings)?.clone();
        ensure_aggregate_has_members(&aggregate)?;
        Ok(Self {
            aggregate,
            failover_index: 0,
            failover_version: 0,
            request_index: 0,
            weighted_index: 0,
            conversation_assignments: HashMap::new(),
        })
    }

    pub fn select(
        &mut self,
        settings: &BackendSettings,
        context: RotationContext,
    ) -> Result<RelayProfile, SelectionError> {
        self.select_with_outcomes(settings, context)
            .map(|(relay, _outcomes)| relay)
    }

    /// 选择 relay 的同时返回路由匹配过程结果，供调用方记录 diagnostic_log。
    /// 保证日志与实际选择来自同一次匹配，避免二次匹配带来的结果与日志不一致
    pub fn select_with_outcomes(
        &mut self,
        settings: &BackendSettings,
        context: RotationContext,
    ) -> Result<(RelayProfile, Vec<RouteMatchInfo>), SelectionError> {
        let (routed_relay_id, outcomes) =
            match_route_for_aggregate(settings, &self.aggregate, context.model.as_deref());
        let relay = if let Some(relay_id) = routed_relay_id {
            // 路由目标已由 relay_is_available 保证有效，直接取用
            relay_profile_by_id(settings, &relay_id).ok_or_else(|| {
                SelectionError::UnknownMemberRelay {
                    aggregate_id: self.aggregate.id.clone(),
                    relay_id,
                }
            })?
        } else {
            let relay_id = match self.aggregate.strategy {
                AggregateRelayStrategy::Failover => self.member_id_at(self.failover_index),
                AggregateRelayStrategy::ConversationRoundRobin => {
                    self.select_for_conversation(context.conversation_id)
                }
                AggregateRelayStrategy::RequestRoundRobin => self.select_next_request(),
                AggregateRelayStrategy::WeightedRoundRobin => self.select_next_weighted(),
            };
            selected_member_relay(settings, &self.aggregate, &relay_id)?
        };
        Ok((relay, outcomes))
    }

    pub fn peek(&self, settings: &BackendSettings) -> Result<RelayProfile, SelectionError> {
        let relay_id = match self.aggregate.strategy {
            AggregateRelayStrategy::Failover => self.member_id_at(self.failover_index),
            AggregateRelayStrategy::ConversationRoundRobin
            | AggregateRelayStrategy::RequestRoundRobin => self.member_id_at(self.request_index),
            AggregateRelayStrategy::WeightedRoundRobin => {
                let schedule = self.weighted_schedule();
                schedule[self.weighted_index % schedule.len()].clone()
            }
        };
        selected_member_relay(settings, &self.aggregate, &relay_id)
    }

    pub fn record_event(&mut self, event: RotationEvent) {
        if event == RotationEvent::Failure
            && self.aggregate.strategy == AggregateRelayStrategy::Failover
            && !self.aggregate.members.is_empty()
        {
            self.failover_index = (self.failover_index + 1) % self.aggregate.members.len();
            self.failover_version = self
                .failover_version
                .checked_add(1)
                .expect("cursor exhausted");
        }
    }

    fn select_for_conversation(&mut self, conversation_id: Option<String>) -> String {
        let Some(conversation_id) = conversation_id else {
            return self.select_next_request();
        };
        if let Some(relay_id) = self.conversation_assignments.get(&conversation_id) {
            return relay_id.clone();
        }

        let relay_id = self.select_next_request();
        self.conversation_assignments
            .insert(conversation_id, relay_id.clone());
        relay_id
    }

    fn select_next_request(&mut self) -> String {
        let relay_id = self.member_id_at(self.request_index);
        self.request_index = (self.request_index + 1) % self.aggregate.members.len();
        relay_id
    }

    fn select_next_weighted(&mut self) -> String {
        let schedule = self.weighted_schedule();
        let relay_id = schedule[self.weighted_index % schedule.len()].clone();
        self.weighted_index = (self.weighted_index + 1) % schedule.len();
        relay_id
    }

    fn weighted_schedule(&self) -> Vec<String> {
        self.aggregate
            .members
            .iter()
            .flat_map(|member| {
                std::iter::repeat_n(member.relay_id.clone(), member.weight.max(1) as usize)
            })
            .collect()
    }

    fn member_id_at(&self, index: usize) -> String {
        self.aggregate.members[index % self.aggregate.members.len()]
            .relay_id
            .clone()
    }

    fn reconcile(
        &mut self,
        settings: &BackendSettings,
        aggregate: &AggregateRelayProfile,
    ) -> Result<(), SelectionError> {
        ensure_aggregate_has_members(aggregate)?;
        if self.aggregate.id != aggregate.id
            || self.aggregate.strategy != aggregate.strategy
            || self.aggregate.model_aggregation_enabled != aggregate.model_aggregation_enabled
        {
            *self = Self::from_settings(settings)?;
            return Ok(());
        }

        let current_failover = self.member_id_at(self.failover_index);
        let next_request = self.member_id_at(self.request_index);
        let old_weighted_schedule = self.weighted_schedule();
        let next_weighted =
            old_weighted_schedule[self.weighted_index % old_weighted_schedule.len()].clone();
        let members_unchanged = self.aggregate.members == aggregate.members;

        self.aggregate = aggregate.clone();
        self.conversation_assignments.retain(|_, relay_id| {
            aggregate
                .members
                .iter()
                .any(|member| member.relay_id == *relay_id)
        });
        if members_unchanged {
            return Ok(());
        }

        self.failover_index = member_position(aggregate, &current_failover).unwrap_or(0);
        self.request_index = member_position(aggregate, &next_request).unwrap_or(0);
        let weighted_schedule = self.weighted_schedule();
        self.weighted_index = weighted_schedule
            .iter()
            .position(|relay_id| relay_id == &next_weighted)
            .unwrap_or(0);
        Ok(())
    }
}

fn member_position(aggregate: &AggregateRelayProfile, relay_id: &str) -> Option<usize> {
    aggregate
        .members
        .iter()
        .position(|member| member.relay_id == relay_id)
}

pub fn select_relay_for_request(
    settings: &BackendSettings,
    context: RotationContext,
) -> Result<RelayProfile, SelectionError> {
    select_relay_for_request_with_revision(settings, context, "").map(|(relay, _snapshot)| relay)
}

pub fn select_relay_for_request_with_revision(
    settings: &BackendSettings,
    context: RotationContext,
    revision: &str,
) -> Result<(RelayProfile, RelayRotationSnapshot), SelectionError> {
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot = legacy_snapshot(&mut guard, settings, revision);
    select_with_snapshot_locked(&mut guard, settings, context, &snapshot)
}

fn snapshot_revision(settings: &BackendSettings) -> String {
    if !settings.settings_revision.is_empty() {
        return settings.settings_revision.clone();
    }
    // 无持久化 revision 的内存配置仍以内容区分，避免凭据更新遗漏代际失效。
    settings_identity(settings)
}

fn settings_identity(settings: &BackendSettings) -> String {
    // RelayProfile 的持久化序列化跳过这三个派生字段，但请求直接使用它们。
    // 其余有效字段由 settings 序列化覆盖；这里只保存摘要，不输出凭据。
    let effective_profiles = settings
        .relay_profiles
        .iter()
        .map(|profile| (&profile.model, &profile.base_url, &profile.api_key))
        .collect::<Vec<_>>();
    let contents =
        serde_json::to_vec(&(settings, effective_profiles)).expect("BackendSettings serializes");
    format!("sha256:{:x}", Sha256::digest(contents))
}

/// 必须在 SettingsStore 的同一个 write_lock 协调读闭包中调用，
/// 不能把先前读取的 settings 延后送来接纳。锁序固定为 store -> GLOBAL_SELECTOR。
pub(crate) fn admit_settings_snapshot(settings: &BackendSettings) -> RelaySettingsSnapshot {
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    guard.authoritative_admission = true;
    admit_locked(&mut guard, settings, snapshot_revision(settings))
}

fn admit_locked(
    guard: &mut GlobalSelectorState,
    settings: &BackendSettings,
    revision: String,
) -> RelaySettingsSnapshot {
    let settings_identity = settings_identity(settings);
    let aggregate = settings.active_aggregate_relay_profile();
    if guard.generation == 0
        || guard.revision != revision
        || guard.settings_identity != settings_identity
    {
        guard.generation = guard
            .generation
            .checked_add(1)
            .expect("selector epoch exhausted");
        guard.revision = revision;
        guard.settings_identity = settings_identity;
        if let Some(aggregate) = aggregate {
            // 无效新配置也建立代际，旧请求不能继续控制之前的 selector。
            guard.selector = if let Some(mut selector) = guard.selector.take() {
                selector
                    .reconcile(settings, &aggregate)
                    .ok()
                    .map(|()| selector)
            } else {
                RelayRotationSelector::from_settings(settings).ok()
            };
        } else {
            guard.selector = None;
        }
    }
    RelaySettingsSnapshot {
        revision: guard.revision.clone(),
        epoch: guard.generation,
        settings_identity: guard.settings_identity.clone(),
    }
}

fn legacy_snapshot(
    guard: &mut GlobalSelectorState,
    settings: &BackendSettings,
    revision: &str,
) -> RelaySettingsSnapshot {
    if guard.authoritative_admission {
        let revision = if revision.is_empty() {
            snapshot_revision(settings)
        } else {
            revision.to_string()
        };
        return RelaySettingsSnapshot {
            epoch: if revision == guard.revision
                && settings_identity(settings) == guard.settings_identity
            {
                guard.generation
            } else {
                0
            },
            revision,
            settings_identity: settings_identity(settings),
        };
    }
    // 无 revision 的旧同步 API 仍视为调用方主动更新；生产异步路径必须使用 admission。
    if revision.is_empty() {
        guard.legacy_unversioned = true;
        return admit_locked(guard, settings, snapshot_revision(settings));
    }
    let same_revision = revision == guard.revision;
    let ordered = match (revision_parts(revision), revision_parts(&guard.revision)) {
        (Some(candidate), Some(current)) => candidate >= current,
        (Some(_), None) => guard.legacy_unversioned,
        _ => false,
    };
    if guard.generation == 0 || same_revision || ordered {
        guard.legacy_unversioned = false;
        return admit_locked(guard, settings, revision.to_string());
    }
    // 不推断陌生 SHA 的先后，也不保留可能无限增长的 SHA history。
    RelaySettingsSnapshot {
        revision: revision.to_string(),
        epoch: 0,
        settings_identity: settings_identity(settings),
    }
}

pub fn select_relay_for_request_with_snapshot(
    settings: &BackendSettings,
    context: RotationContext,
    snapshot: &RelaySettingsSnapshot,
) -> Result<(RelayProfile, RelayRotationSnapshot), SelectionError> {
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    select_with_snapshot_locked(&mut guard, settings, context, snapshot)
}

fn select_with_snapshot_locked(
    guard: &mut GlobalSelectorState,
    settings: &BackendSettings,
    context: RotationContext,
    snapshot: &RelaySettingsSnapshot,
) -> Result<(RelayProfile, RelayRotationSnapshot), SelectionError> {
    if snapshot.settings_identity != settings_identity(settings) {
        return Err(SelectionError::SettingsSnapshotMismatch);
    }
    let current = snapshot.epoch != 0
        && snapshot.epoch == guard.generation
        && snapshot.revision == guard.revision
        && snapshot.settings_identity == guard.settings_identity;
    if settings.active_aggregate_relay_profile().is_none() {
        let relay = settings.active_relay_profile();
        let attempt = RelayRotationSnapshot::capture(&snapshot.revision, 0, &relay.id, None);
        return Ok((relay, attempt));
    }
    if !current {
        let model = context.model.clone();
        let (relay, outcomes) = RelayRotationSelector::from_settings(settings)?
            .select_with_outcomes(settings, context)?;
        log_route_outcomes(model.as_deref(), &outcomes);
        let attempt = RelayRotationSnapshot::capture(&snapshot.revision, 0, &relay.id, None);
        return Ok((relay, attempt));
    }
    let selector =
        guard
            .selector
            .as_mut()
            .ok_or_else(|| SelectionError::EmptyAggregateMembers {
                aggregate_id: settings
                    .active_aggregate_relay_profile()
                    .expect("aggregate")
                    .id,
            })?;
    let model = context.model.clone();
    let (relay, outcomes) = selector.select_with_outcomes(settings, context)?;
    let attempt = RelayRotationSnapshot::capture(
        &snapshot.revision,
        snapshot.epoch,
        &relay.id,
        Some(selector),
    );
    log_route_outcomes(model.as_deref(), &outcomes);
    Ok((relay, attempt))
}

/// 将路由匹配过程结果写入 diagnostic_log，事件名与字段与旧 protocol_proxy 侧实现完全一致；
/// 仅在聚合选择路径被调用（无活动聚合时由调用方直接返回单 relay，不产生路由日志）
fn log_route_outcomes(model: Option<&str>, outcomes: &[RouteMatchInfo]) {
    for outcome in outcomes {
        let (event, detail) = match outcome {
            RouteMatchInfo::Matched { relay_id, pattern } => (
                "protocol_proxy.route_matched",
                json!({
                    "relayId": relay_id,
                    "model": model,
                    "rule": pattern,
                }),
            ),
            RouteMatchInfo::SkippedInvalidRelay { relay_id, pattern } => (
                "protocol_proxy.route_skipped_invalid_relay",
                json!({
                    "relayId": relay_id,
                    "model": model,
                    "rule": pattern,
                }),
            ),
            RouteMatchInfo::Unmatched => (
                "protocol_proxy.route_unmatched",
                json!({
                    "model": model,
                }),
            ),
        };
        let _ = append_diagnostic_log(event, detail);
    }
}

pub fn select_relay_for_probe(settings: &BackendSettings) -> Result<RelayProfile, SelectionError> {
    select_relay_for_probe_with_revision(settings, "")
}

pub fn select_relay_for_probe_with_revision(
    settings: &BackendSettings,
    revision: &str,
) -> Result<RelayProfile, SelectionError> {
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let snapshot = legacy_snapshot(&mut guard, settings, revision);
    probe_with_snapshot_locked(&guard, settings, &snapshot)
}

pub fn select_relay_for_probe_with_snapshot(
    settings: &BackendSettings,
    snapshot: &RelaySettingsSnapshot,
) -> Result<RelayProfile, SelectionError> {
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    probe_with_snapshot_locked(&guard, settings, snapshot)
}

fn probe_with_snapshot_locked(
    guard: &GlobalSelectorState,
    settings: &BackendSettings,
    snapshot: &RelaySettingsSnapshot,
) -> Result<RelayProfile, SelectionError> {
    if snapshot.settings_identity != settings_identity(settings) {
        return Err(SelectionError::SettingsSnapshotMismatch);
    }
    if settings.active_aggregate_relay_profile().is_none() {
        return Ok(settings.active_relay_profile());
    }
    if snapshot.epoch != 0
        && snapshot.epoch == guard.generation
        && snapshot.revision == guard.revision
        && snapshot.settings_identity == guard.settings_identity
        && let Some(selector) = &guard.selector
    {
        return selector.peek(settings);
    }
    RelayRotationSelector::from_settings(settings)?.peek(settings)
}

pub fn fallback_relays_after(
    settings: &BackendSettings,
    relay_id: &str,
) -> Result<Vec<RelayProfile>, SelectionError> {
    let Some(active_aggregate) = settings.active_aggregate_relay_profile() else {
        return Ok(Vec::new());
    };
    ensure_aggregate_has_members(&active_aggregate)?;
    let start_index = active_aggregate
        .members
        .iter()
        .position(|member| member.relay_id == relay_id)
        .map(|index| index + 1)
        .unwrap_or(0);
    let fallbacks = (0..active_aggregate.members.len().saturating_sub(1))
        .map(|offset| {
            let index = (start_index + offset) % active_aggregate.members.len();
            &active_aggregate.members[index]
        })
        .filter(|member| relay_is_available(settings, &active_aggregate, &member.relay_id))
        .filter_map(|member| relay_profile_by_id(settings, &member.relay_id))
        .collect::<Vec<_>>();
    Ok(fallbacks)
}

pub fn record_relay_request_event(settings: &BackendSettings, event: RotationEvent) {
    if settings.active_aggregate_relay_profile().is_none() {
        return;
    }
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    // 旧接口没有 attempt 身份，仅供同步兼容；生产反馈必须用 snapshot 接口。
    if guard.authoritative_admission || guard.revision != snapshot_revision(settings) {
        return;
    }
    if let Some(selector) = guard.selector.as_mut()
        && settings
            .active_aggregate_relay_profile()
            .is_some_and(|aggregate| selector.aggregate == aggregate)
    {
        selector.record_event(event);
    }
}

pub fn record_relay_request_event_for_snapshot(
    snapshot: &RelayRotationSnapshot,
    event: RotationEvent,
) {
    if snapshot.feedback_used.swap(true, Ordering::AcqRel) || snapshot.generation == 0 {
        return;
    }
    let lock = GLOBAL_SELECTOR.get_or_init(|| Mutex::new(GlobalSelectorState::default()));
    let mut guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if guard.generation == snapshot.generation
        && guard.revision == snapshot.revision
        && let Some(selector) = guard.selector.as_mut()
        && selector.failover_version == snapshot.failover_version
        && selector.failover_index == snapshot.failover_position
        && selector.member_id_at(selector.failover_index) == snapshot.relay_id
    {
        selector.record_event(event);
    }
}

pub fn record_relay_request_failure_for_snapshot(snapshot: &RelayRotationSnapshot) {
    record_relay_request_event_for_snapshot(snapshot, RotationEvent::Failure);
}

pub fn record_relay_request_failure(settings: &BackendSettings) {
    record_relay_request_event(settings, RotationEvent::Failure);
}

fn active_aggregate(settings: &BackendSettings) -> Result<&AggregateRelayProfile, SelectionError> {
    let active_id = settings
        .active_aggregate_relay_profile()
        .map(|aggregate| aggregate.id)
        .ok_or(SelectionError::NoActiveAggregate)?;

    settings
        .aggregate_relay_profiles
        .iter()
        .find(|aggregate| aggregate.id == active_id)
        .ok_or(SelectionError::NoActiveAggregate)
}

fn ensure_aggregate_has_members(aggregate: &AggregateRelayProfile) -> Result<(), SelectionError> {
    if aggregate.members.is_empty() {
        return Err(SelectionError::EmptyAggregateMembers {
            aggregate_id: aggregate.id.clone(),
        });
    }
    Ok(())
}

/// 最终选中的普通策略成员严格校验：relay 必须存在且具备可用凭据
fn selected_member_relay(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
    relay_id: &str,
) -> Result<RelayProfile, SelectionError> {
    let relay = relay_profile_by_id(settings, relay_id).ok_or_else(|| {
        SelectionError::UnknownMemberRelay {
            aggregate_id: aggregate.id.clone(),
            relay_id: relay_id.to_string(),
        }
    })?;
    if !member_relay_ready(&relay) {
        return Err(SelectionError::InvalidMemberRelay {
            aggregate_id: aggregate.id.clone(),
            relay_id: relay_id.to_string(),
        });
    }
    Ok(relay)
}

/// 按 model 执行聚合路由规则匹配，返回命中的成员 relayId 与过程结果信息
fn match_route_for_aggregate(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
    model: Option<&str>,
) -> (Option<String>, Vec<RouteMatchInfo>) {
    let mut outcomes = Vec::new();
    if aggregate.routes.is_empty() {
        return (None, outcomes);
    }
    let Some(model) = model.map(str::trim).filter(|value| !value.is_empty()) else {
        return (None, outcomes);
    };
    let mut routes: Vec<&AggregateRelayRoute> = aggregate.routes.iter().collect();
    // 按 priority 降序；稳定排序保证同 priority 保持数组顺序
    routes.sort_by(|left, right| right.priority.cmp(&left.priority));
    for route in routes {
        if route.pattern.trim().is_empty() {
            continue;
        }
        if !match_route_pattern(&route.pattern, model) {
            continue;
        }
        if relay_is_available(settings, aggregate, &route.relay_id) {
            outcomes.push(RouteMatchInfo::Matched {
                relay_id: route.relay_id.clone(),
                pattern: route.pattern.clone(),
            });
            return (Some(route.relay_id.clone()), outcomes);
        }
        outcomes.push(RouteMatchInfo::SkippedInvalidRelay {
            relay_id: route.relay_id.clone(),
            pattern: route.pattern.clone(),
        });
    }
    outcomes.push(RouteMatchInfo::Unmatched);
    (None, outcomes)
}

/// 返回当前活动聚合的路由匹配过程结果。
/// 已不再被生产代码调用：路由日志已并入 select_with_outcomes（与实际选择同一次匹配），
/// 本函数仅保留供测试直接断言路由过程结果；无活动聚合或 model 缺失时为空
#[allow(dead_code)]
pub fn route_match_outcomes(
    settings: &BackendSettings,
    model: Option<&str>,
) -> Vec<RouteMatchInfo> {
    let Some(aggregate) = settings.active_aggregate_relay_profile() else {
        return Vec::new();
    };
    match_route_for_aggregate(settings, &aggregate, model).1
}

/// 路由目标有效性：属于聚合成员且 relay 存在且具备可用凭据。
/// 除与 validate_aggregate_members 同标准外，额外要求目标必须在 aggregate.members 内，
/// 避免路由把请求导向聚合外部的独立 relay
fn relay_is_available(
    settings: &BackendSettings,
    aggregate: &AggregateRelayProfile,
    relay_id: &str,
) -> bool {
    if !aggregate
        .members
        .iter()
        .any(|member| member.relay_id == relay_id)
    {
        return false;
    }
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == relay_id)
        .map(member_relay_ready)
        .unwrap_or(false)
}

fn revision_parts(revision: &str) -> Option<(u128, u64)> {
    let (modified, length) = revision.split_once(':').unwrap_or((revision, "0"));
    Some((modified.parse().ok()?, length.parse().ok()?))
}

fn relay_profile_by_id(settings: &BackendSettings, relay_id: &str) -> Option<RelayProfile> {
    settings
        .relay_profiles
        .iter()
        .find(|profile| profile.id == relay_id)
        .cloned()
}

#[cfg(test)]
mod admission_tests {
    use super::*;
    use crate::settings::{AggregateRelayMember, RelayMode};

    fn settings(strategy: AggregateRelayStrategy) -> BackendSettings {
        let mut settings = BackendSettings {
            relay_profiles: ["a", "b", "c"]
                .into_iter()
                .map(|id| RelayProfile {
                    id: id.into(),
                    base_url: format!("https://{id}.example/v1"),
                    api_key: format!("test-{id}"),
                    auth_contents: json!({"OPENAI_API_KEY": format!("test-{id}")}).to_string(),
                    ..RelayProfile::default()
                })
                .chain(std::iter::once(RelayProfile {
                    id: "aggregate".into(),
                    relay_mode: RelayMode::Aggregate,
                    ..RelayProfile::default()
                }))
                .collect(),
            aggregate_relay_profiles: vec![AggregateRelayProfile {
                id: "aggregate".into(),
                name: "test aggregate".into(),
                session_provider: crate::settings::RelaySessionProvider::Custom,
                model_aggregation_enabled: false,
                strategy,
                members: ["a", "b", "c"]
                    .into_iter()
                    .map(|id| AggregateRelayMember {
                        relay_id: id.into(),
                        weight: 1,
                    })
                    .collect(),
                routes: Vec::new(),
                default_relay_id: String::new(),
                sync_default_member_login: false,
            }],
            active_relay_id: "aggregate".into(),
            ..BackendSettings::default()
        };
        stamp(&mut settings);
        settings
    }

    // 与生产 revision 一样，对实际序列化内容求 SHA，绝不使用时间戳替身。
    fn stamp(settings: &mut BackendSettings) {
        settings.settings_revision.clear();
        settings.settings_revision = format!(
            "sha256:{:x}",
            Sha256::digest(serde_json::to_vec(settings).unwrap())
        );
    }

    fn set_persisted_key(settings: &mut BackendSettings, index: usize, key: &str) {
        let profile = &mut settings.relay_profiles[index];
        profile.api_key = key.into();
        profile.auth_contents = json!({"OPENAI_API_KEY": key}).to_string();
    }

    fn select(
        settings: &BackendSettings,
        snapshot: &RelaySettingsSnapshot,
    ) -> (RelayProfile, RelayRotationSnapshot) {
        select_relay_for_request_with_snapshot(settings, RotationContext::default(), snapshot)
            .unwrap()
    }

    #[test]
    fn sha_old_event_and_selection_cannot_change_current_generation() {
        let _guard = rotation_test_guard();
        let old = settings(AggregateRelayStrategy::Failover);
        let old_snapshot = admit_settings_snapshot(&old);
        let (_, old_attempt) = select(&old, &old_snapshot);
        record_relay_request_failure_for_snapshot(&old_attempt.for_attempt("a"));
        let mut current = old.clone();
        set_persisted_key(&mut current, 0, "replacement-test-key");
        stamp(&mut current);
        let current_snapshot = admit_settings_snapshot(&current);
        assert_ne!(old_snapshot.revision, current_snapshot.revision);
        assert!(current_snapshot.epoch > old_snapshot.epoch);
        assert_eq!(select(&current, &current_snapshot).0.id, "b");

        record_relay_request_failure_for_snapshot(&old_attempt);
        let (late, late_attempt) = select(&old, &old_snapshot);
        assert_eq!(late.id, "a");
        record_relay_request_failure_for_snapshot(&late_attempt);
        assert_eq!(
            select_relay_for_probe_with_snapshot(&old, &old_snapshot)
                .unwrap()
                .id,
            "a"
        );
        assert_eq!(select(&current, &current_snapshot).0.id, "b");
        // 旧兼容接口也不能绕过生产 admission，包括数字版本、空版本和非聚合反馈。
        for revision in ["999999:1", old.settings_revision.as_str(), ""] {
            let (_, late_attempt) =
                select_relay_for_request_with_revision(&old, RotationContext::default(), revision)
                    .unwrap();
            record_relay_request_failure_for_snapshot(&late_attempt);
        }
        assert_eq!(select(&current, &current_snapshot).0.id, "b");
    }

    #[test]
    fn sha_regular_admission_and_late_regular_selection_have_ordered_epochs() {
        let _guard = rotation_test_guard();
        let mut regular = settings(AggregateRelayStrategy::Failover);
        regular.active_relay_id = "c".into();
        stamp(&mut regular);
        let old_regular_snapshot = admit_settings_snapshot(&regular);
        let aggregate = settings(AggregateRelayStrategy::Failover);
        let aggregate_snapshot = admit_settings_snapshot(&aggregate);
        let (_, attempt) = select(&aggregate, &aggregate_snapshot);
        record_relay_request_failure_for_snapshot(&attempt);
        assert_eq!(select(&regular, &old_regular_snapshot).0.id, "c");
        assert_eq!(
            select_relay_for_probe_with_snapshot(&regular, &old_regular_snapshot)
                .unwrap()
                .id,
            "c"
        );
        record_relay_request_failure(&regular);
        assert_eq!(select(&aggregate, &aggregate_snapshot).0.id, "b");

        let new_regular_snapshot = admit_settings_snapshot(&regular);
        assert!(new_regular_snapshot.epoch > aggregate_snapshot.epoch);
        assert!(aggregate_snapshot.epoch > old_regular_snapshot.epoch);
        // 聚合旧选择不能在新非聚合 admission 后重建全局 selector。
        let (_, late_attempt) = select(&aggregate, &aggregate_snapshot);
        record_relay_request_failure_for_snapshot(&late_attempt.for_attempt("a"));
        assert!(
            GLOBAL_SELECTOR
                .get()
                .unwrap()
                .lock()
                .unwrap()
                .selector
                .is_none()
        );
        let returned = admit_settings_snapshot(&aggregate);
        assert!(returned.epoch > new_regular_snapshot.epoch);
        assert_eq!(select(&aggregate, &returned).0.id, "a");
    }

    #[test]
    fn sha_credentials_preserve_conversation_binding_and_request_cursor() {
        let _guard = rotation_test_guard();
        let mut settings = settings(AggregateRelayStrategy::ConversationRoundRobin);
        let old = admit_settings_snapshot(&settings);
        for (chat, expected) in [("first", "a"), ("second", "b")] {
            assert_eq!(
                select_relay_for_request_with_snapshot(
                    &settings,
                    RotationContext::for_conversation(chat),
                    &old,
                )
                .unwrap()
                .0
                .id,
                expected
            );
        }
        set_persisted_key(&mut settings, 1, "new-test-key");
        stamp(&mut settings);
        let new = admit_settings_snapshot(&settings);
        assert!(new.epoch > old.epoch);
        assert_eq!(admit_settings_snapshot(&settings).epoch, new.epoch);
        assert_eq!(
            select_relay_for_request_with_snapshot(
                &settings,
                RotationContext::for_conversation("second"),
                &new,
            )
            .unwrap()
            .0
            .id,
            "b"
        );
        assert_eq!(
            select_relay_for_request_with_snapshot(
                &settings,
                RotationContext::for_conversation("third"),
                &new,
            )
            .unwrap()
            .0
            .id,
            "c"
        );
    }

    #[test]
    fn sha_strategy_and_aggregate_id_changes_reset_selector() {
        let _guard = rotation_test_guard();
        let mut settings = settings(AggregateRelayStrategy::ConversationRoundRobin);
        let original = admit_settings_snapshot(&settings);
        assert_eq!(select(&settings, &original).0.id, "a");
        assert_eq!(select(&settings, &original).0.id, "b");
        settings.aggregate_relay_profiles[0].strategy = AggregateRelayStrategy::RequestRoundRobin;
        stamp(&mut settings);
        let strategy = admit_settings_snapshot(&settings);
        assert_eq!(select(&settings, &strategy).0.id, "a");
        assert_eq!(select(&settings, &strategy).0.id, "b");
        settings.aggregate_relay_profiles[0].id = "replacement".into();
        settings.relay_profiles.last_mut().unwrap().id = "replacement".into();
        settings.active_relay_id = "replacement".into();
        stamp(&mut settings);
        let identity = admit_settings_snapshot(&settings);
        assert_eq!(select(&settings, &identity).0.id, "a");
    }

    #[test]
    fn model_aggregation_mode_change_resets_pool_after_strict_member_updates() {
        let _guard = rotation_test_guard();
        let mut settings = settings(AggregateRelayStrategy::Failover);
        settings.aggregate_relay_profiles[0].model_aggregation_enabled = true;
        stamp(&mut settings);
        admit_settings_snapshot(&settings);

        let first_member = settings.aggregate_relay_profiles[0].members.remove(0);
        stamp(&mut settings);
        let removed = admit_settings_snapshot(&settings);
        assert_eq!(
            select_relay_for_probe_with_snapshot(&settings, &removed)
                .unwrap()
                .id,
            "b"
        );
        settings.aggregate_relay_profiles[0]
            .members
            .insert(0, first_member);
        stamp(&mut settings);
        let restored = admit_settings_snapshot(&settings);
        assert_eq!(
            select_relay_for_probe_with_snapshot(&settings, &restored)
                .unwrap()
                .id,
            "b"
        );

        settings.aggregate_relay_profiles[0].name = "renamed aggregate".into();
        set_persisted_key(&mut settings, 1, "updated-test-key");
        stamp(&mut settings);
        let same_mode = admit_settings_snapshot(&settings);
        assert_eq!(
            select_relay_for_probe_with_snapshot(&settings, &same_mode)
                .unwrap()
                .id,
            "b"
        );

        settings.aggregate_relay_profiles[0].model_aggregation_enabled = false;
        stamp(&mut settings);
        let pool = admit_settings_snapshot(&settings);
        let (first, attempt) = select(&settings, &pool);
        assert_eq!(first.id, "a");
        record_relay_request_failure_for_snapshot(&attempt);
        assert_eq!(select(&settings, &pool).0.id, "b");

        settings.aggregate_relay_profiles[0].model_aggregation_enabled = true;
        stamp(&mut settings);
        let strict = admit_settings_snapshot(&settings);
        assert_eq!(
            select_relay_for_probe_with_snapshot(&settings, &strict)
                .unwrap()
                .id,
            "a"
        );
    }

    #[test]
    fn sha_probe_does_not_advance_and_old_request_cannot_consume_current_cursor() {
        let _guard = rotation_test_guard();
        let old = settings(AggregateRelayStrategy::RequestRoundRobin);
        let old_snapshot = admit_settings_snapshot(&old);
        assert_eq!(select(&old, &old_snapshot).0.id, "a");
        let mut current = old.clone();
        set_persisted_key(&mut current, 0, "new-test-key");
        stamp(&mut current);
        let current_snapshot = admit_settings_snapshot(&current);
        for _ in 0..2 {
            assert_eq!(
                select_relay_for_probe_with_snapshot(&current, &current_snapshot)
                    .unwrap()
                    .id,
                "b"
            );
            assert_eq!(select(&old, &old_snapshot).0.id, "a");
        }
        assert_eq!(select(&current, &current_snapshot).0.id, "b");
    }

    #[test]
    fn sha_reverting_content_does_not_revalidate_old_attempts() {
        let _guard = rotation_test_guard();
        let original = settings(AggregateRelayStrategy::Failover);
        let original_snapshot = admit_settings_snapshot(&original);
        let (_, delayed) = select(&original, &original_snapshot);
        let mut changed = original.clone();
        set_persisted_key(&mut changed, 0, "new-test-key");
        stamp(&mut changed);
        let changed_snapshot = admit_settings_snapshot(&changed);
        let returned = admit_settings_snapshot(&original);
        assert_eq!(returned.revision, original_snapshot.revision);
        assert!(returned.epoch > changed_snapshot.epoch);
        record_relay_request_failure_for_snapshot(&delayed.for_attempt("a"));
        assert_eq!(select(&original, &returned).0.id, "a");
    }

    #[test]
    fn first_legacy_sha_initializes_but_unknown_sha_cannot_replace_it() {
        let _guard = rotation_test_guard();
        let settings = settings(AggregateRelayStrategy::Failover);
        let (_, attempt) = select_relay_for_request_with_revision(
            &settings,
            RotationContext::default(),
            &settings.settings_revision,
        )
        .unwrap();
        assert_ne!(attempt.generation, 0);
        record_relay_request_failure_for_snapshot(&attempt);
        let mut unknown = settings.clone();
        set_persisted_key(&mut unknown, 0, "another-test-key");
        stamp(&mut unknown);
        let (_, isolated) = select_relay_for_request_with_revision(
            &unknown,
            RotationContext::default(),
            &unknown.settings_revision,
        )
        .unwrap();
        assert_eq!(isolated.generation, 0);
        record_relay_request_failure_for_snapshot(&isolated);
        assert_eq!(
            select_relay_for_probe_with_revision(&settings, &settings.settings_revision,)
                .unwrap()
                .id,
            "b"
        );
    }

    #[test]
    fn request_and_probe_reject_settings_paired_with_another_token() {
        let _guard = rotation_test_guard();
        let original = settings(AggregateRelayStrategy::RequestRoundRobin);
        let snapshot = admit_settings_snapshot(&original);
        for field in ["api_key", "base_url", "model", "active_relay_id"] {
            let mut mismatched = original.clone();
            // 连 revision 和持久化字段都保留原值，仍须检测到派生字段错配。
            match field {
                "api_key" => mismatched.relay_profiles[0].api_key = "wrong-test-key".into(),
                "base_url" => {
                    mismatched.relay_profiles[0].base_url = "https://other.example/v1".into()
                }
                "model" => mismatched.relay_profiles[0].model = "different-model".into(),
                "active_relay_id" => mismatched.active_relay_id = "b".into(),
                _ => unreachable!(),
            }
            assert!(
                matches!(
                    select_relay_for_request_with_snapshot(
                        &mismatched,
                        RotationContext::default(),
                        &snapshot,
                    ),
                    Err(SelectionError::SettingsSnapshotMismatch)
                ),
                "request accepted mismatched {field}"
            );
            assert!(
                matches!(
                    select_relay_for_probe_with_snapshot(&mismatched, &snapshot),
                    Err(SelectionError::SettingsSnapshotMismatch)
                ),
                "probe accepted mismatched {field}"
            );
        }
        assert_eq!(select(&original, &snapshot).0.id, "a");
    }

    #[test]
    fn unversioned_explicit_settings_can_update_before_authoritative_admission() {
        let _guard = rotation_test_guard();
        let mut first = settings(AggregateRelayStrategy::Failover);
        first.settings_revision.clear();
        let (_, attempt) =
            select_relay_for_request_with_revision(&first, RotationContext::default(), "").unwrap();
        record_relay_request_failure_for_snapshot(&attempt);
        let mut second = first.clone();
        second.relay_profiles[0].api_key = "new-test-key".into();
        let (relay, attempt) =
            select_relay_for_request_with_revision(&second, RotationContext::default(), "")
                .unwrap();
        assert_eq!(relay.id, "b");
        record_relay_request_failure_for_snapshot(&attempt);
        assert_eq!(select_relay_for_probe(&second).unwrap().id, "c");
        let admitted = admit_settings_snapshot(&second);
        let (_, isolated) =
            select_relay_for_request_with_revision(&first, RotationContext::default(), "").unwrap();
        record_relay_request_failure_for_snapshot(&isolated);
        assert_eq!(select(&second, &admitted).0.id, "c");
    }
}
