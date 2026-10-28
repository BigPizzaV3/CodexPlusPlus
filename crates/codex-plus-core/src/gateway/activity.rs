//! 网关的路由动态：每个 Agent 最近一次请求是怎么走的、请求计数，以及正在请求哪些上游。
//!
//! 只放内存（常驻 helper 在 manager 进程内），供「路由」页实时展示；重启后清零。

use std::collections::{BTreeMap, HashMap};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime};

use serde::Serialize;

use crate::gateway::route::{GatewayCandidate, RoutePlan};

/// 一次尝试：发给哪个供应商的哪个 Key，结果如何。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteStep {
    pub relay_id: String,
    pub relay_name: String,
    pub key_id: String,
    pub model: String,
    /// `answered` / `failed` / `transport`
    pub outcome: String,
    /// 失败分类（同 Key 健康状态里的 `lastFailure`）。
    pub failure: Option<String>,
    pub status: Option<u16>,
    pub elapsed_ms: u64,
}

/// 路由计划里的一个成员（按尝试顺序，同一供应商的多个 Key 合并成一条）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedMember {
    pub relay_id: String,
    pub relay_name: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteRecord {
    pub requested_model: String,
    pub model: String,
    pub group_id: Option<String>,
    pub planned: Vec<PlannedMember>,
    pub steps: Vec<RouteStep>,
    /// `answered` / `failed` / `cooling`
    pub result: String,
    pub cooling_ms: Option<u64>,
    pub finished_at_ms: u64,
    pub elapsed_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentActivity {
    pub agent: String,
    pub requests: u64,
    /// 换过上游才成功的请求数。
    pub rerouted: u64,
    /// Agent 最终收到错误的请求数。
    pub errors: u64,
    /// 正在请求的供应商 id（可能重复：并发请求）。
    pub in_flight: Vec<String>,
    pub last: Option<RouteRecord>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivitySnapshot {
    pub agents: Vec<AgentActivity>,
}

#[derive(Default)]
struct ActivityState {
    agents: BTreeMap<String, AgentActivity>,
    in_flight: HashMap<(String, String), u32>,
}

fn lock() -> std::sync::MutexGuard<'static, ActivityState> {
    static STATE: OnceLock<Mutex<ActivityState>> = OnceLock::new();
    STATE
        .get_or_init(|| Mutex::new(ActivityState::default()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|value| value.as_millis() as u64)
        .unwrap_or_default()
}

/// 正在请求某个上游；drop 时（整个响应转发完或放弃该候选）撤销。
pub struct InFlight {
    agent: String,
    relay_id: String,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let mut state = lock();
        let key = (self.agent.clone(), self.relay_id.clone());
        if let Some(count) = state.in_flight.get_mut(&key) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                state.in_flight.remove(&key);
            }
        }
    }
}

/// 记录一次网关请求的路由过程，结束时调用 [`RouteRecorder::finish`]。
pub struct RouteRecorder {
    agent: String,
    started: Instant,
    record: RouteRecord,
}

impl RouteRecorder {
    pub fn new(plan: &RoutePlan) -> Self {
        let mut planned: Vec<PlannedMember> = Vec::new();
        for candidate in &plan.candidates {
            if planned
                .last()
                .is_some_and(|member| member.relay_id == candidate.relay.id)
            {
                continue;
            }
            planned.push(PlannedMember {
                relay_id: candidate.relay.id.clone(),
                relay_name: candidate.relay.name.clone(),
                model: candidate.upstream_model.clone(),
            });
        }
        Self {
            agent: plan.agent.as_str().to_string(),
            started: Instant::now(),
            record: RouteRecord {
                requested_model: plan.source_model.clone(),
                model: plan.model.clone(),
                group_id: plan.group_id.clone(),
                planned,
                steps: Vec::new(),
                result: "failed".to_string(),
                cooling_ms: None,
                finished_at_ms: 0,
                elapsed_ms: 0,
            },
        }
    }

    /// 开始尝试一个候选。
    pub fn begin(&self, candidate: &GatewayCandidate) -> InFlight {
        let mut state = lock();
        *state
            .in_flight
            .entry((self.agent.clone(), candidate.relay.id.clone()))
            .or_default() += 1;
        InFlight {
            agent: self.agent.clone(),
            relay_id: candidate.relay.id.clone(),
        }
    }

    pub fn step(
        &mut self,
        candidate: &GatewayCandidate,
        outcome: &str,
        failure: Option<&str>,
        status: Option<u16>,
    ) {
        self.record.steps.push(RouteStep {
            relay_id: candidate.relay.id.clone(),
            relay_name: candidate.relay.name.clone(),
            key_id: candidate.key.key_id.clone(),
            model: candidate.upstream_model.clone(),
            outcome: outcome.to_string(),
            failure: failure.map(str::to_string),
            status,
            elapsed_ms: self.started.elapsed().as_millis() as u64,
        });
    }

    pub fn cooling(&mut self, wait_ms: u64) {
        self.record.cooling_ms = Some(wait_ms);
    }

    /// 写入该 Agent 的最近一次记录并累加计数。`answered` 为 false 表示 Agent 收到了错误。
    pub fn finish(mut self, answered: bool) {
        self.record.result = if answered {
            "answered"
        } else if self.record.cooling_ms.is_some() {
            "cooling"
        } else {
            "failed"
        }
        .to_string();
        self.record.finished_at_ms = now_unix_ms();
        self.record.elapsed_ms = self.started.elapsed().as_millis() as u64;
        let rerouted = answered && self.record.steps.len() > 1;
        let mut state = lock();
        let entry = state
            .agents
            .entry(self.agent.clone())
            .or_insert_with(|| AgentActivity {
                agent: self.agent.clone(),
                ..AgentActivity::default()
            });
        entry.requests += 1;
        if rerouted {
            entry.rerouted += 1;
        }
        if !answered {
            entry.errors += 1;
        }
        entry.last = Some(self.record);
    }
}

pub fn snapshot() -> ActivitySnapshot {
    let state = lock();
    let mut agents = state.agents.clone();
    for ((agent, relay_id), count) in &state.in_flight {
        let entry = agents
            .entry(agent.clone())
            .or_insert_with(|| AgentActivity {
                agent: agent.clone(),
                ..AgentActivity::default()
            });
        for _ in 0..*count {
            entry.in_flight.push(relay_id.clone());
        }
    }
    ActivitySnapshot {
        agents: agents.into_values().collect(),
    }
}

/// 清空计数与最近记录（不影响正在进行的请求）。
pub fn reset() {
    lock().agents.clear();
}
