//! 上游的响应速度：每个（供应商, 模型）最近几次成功请求从发出到收到响应头的耗时，做指数平均；
//! 另记上次成功以来连续失败的次数，「速度优先」排序时给失败过的上游加罚时。
//!
//! 只存在内存，重启清零；模型组「速度优先」按它排先后，界面在「路由」页的组成员和供应商池的「按模型」里显示。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use serde::Serialize;

/// 新样本的权重：越大越跟得上最近的变化。
const ALPHA: f64 = 0.3;
/// 每次连续失败在排序时折算的额外耗时：失败过的排在测过、能用的成员后面，
/// 「没测过先试一次」也只试这一次。成功一次就清零。
const FAILURE_PENALTY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeedView {
    pub relay_id: String,
    pub model: String,
    /// 平均首包耗时（毫秒）。
    pub ms: u64,
    pub samples: u32,
}

#[derive(Debug, Clone, Copy, Default)]
struct Stat {
    /// 成功请求首包耗时的指数平均（毫秒），`samples` 为 0 时无意义。
    ms: f64,
    samples: u32,
    /// 上次成功以来连续失败了几次。
    failures: u32,
}

fn table() -> &'static Mutex<HashMap<(String, String), Stat>> {
    static TABLE: OnceLock<Mutex<HashMap<(String, String), Stat>>> = OnceLock::new();
    TABLE.get_or_init(Mutex::default)
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<(String, String), Stat>> {
    table()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn key(relay_id: &str, model: &str) -> (String, String) {
    (relay_id.to_string(), model.to_ascii_lowercase())
}

/// 记一次成功请求的首包耗时。
pub fn record(relay_id: &str, model: &str, elapsed: Duration) {
    let ms = elapsed.as_secs_f64() * 1000.0;
    let mut guard = lock();
    let entry = guard.entry(key(relay_id, model)).or_default();
    entry.ms = if entry.samples == 0 {
        ms
    } else {
        entry.ms * (1.0 - ALPHA) + ms * ALPHA
    };
    entry.samples = entry.samples.saturating_add(1);
    entry.failures = 0;
}

/// 记一次失败（上游报错、换下一个候选、连接失败等）：这个上游算「试过了」，
/// 「速度优先」排序时排到测过、能用的成员后面。
pub fn record_failure(relay_id: &str, model: &str) {
    let mut guard = lock();
    let entry = guard.entry(key(relay_id, model)).or_default();
    entry.failures = entry.failures.saturating_add(1);
}

/// 平均首包耗时；还没成功测过返回 None。
pub fn latency(relay_id: &str, model: &str) -> Option<Duration> {
    lock()
        .get(&key(relay_id, model))
        .filter(|stat| stat.samples > 0)
        .map(|stat| Duration::from_secs_f64(stat.ms / 1000.0))
}

/// 「速度优先」排序用的耗时：没试过返回 None（先试一次）；失败过的按连续失败次数加罚时。
pub fn rank(relay_id: &str, model: &str) -> Option<Duration> {
    let stat = *lock().get(&key(relay_id, model))?;
    let measured = if stat.samples > 0 {
        Duration::from_secs_f64(stat.ms / 1000.0)
    } else {
        Duration::ZERO
    };
    Some(measured.saturating_add(FAILURE_PENALTY.saturating_mul(stat.failures)))
}

/// 忘掉一个供应商的全部速度记录（删除供应商时用；id 可能被新供应商复用）。
pub fn forget(relay_id: &str) {
    lock().retain(|(relay, _), _| relay != relay_id);
}

pub fn snapshot() -> Vec<SpeedView> {
    let guard = lock();
    let mut views: Vec<SpeedView> = guard
        .iter()
        // 只失败过、没成功测过的不显示耗时。
        .filter(|(_, stat)| stat.samples > 0)
        .map(|((relay_id, model), stat)| SpeedView {
            relay_id: relay_id.clone(),
            model: model.clone(),
            ms: stat.ms.round() as u64,
            samples: stat.samples,
        })
        .collect();
    views
        .sort_by(|left, right| (&left.relay_id, &left.model).cmp(&(&right.relay_id, &right.model)));
    views
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn averages_recent_samples_and_ignores_model_case() {
        record("speed-test", "Model-A", Duration::from_millis(1000));
        assert_eq!(
            latency("speed-test", "model-a"),
            Some(Duration::from_millis(1000))
        );
        record("speed-test", "model-a", Duration::from_millis(2000));
        let ms = latency("speed-test", "model-a").unwrap().as_millis();
        assert_eq!(ms, 1300, "0.7 × 1000 + 0.3 × 2000");
        assert_eq!(latency("speed-test", "other"), None);
        let view = snapshot()
            .into_iter()
            .find(|view| view.relay_id == "speed-test")
            .unwrap();
        assert_eq!((view.ms, view.samples), (1300, 2));
    }

    #[test]
    fn failures_count_as_tried_and_rank_after_working_members() {
        assert_eq!(rank("speed-fail", "m"), None, "没试过");
        record_failure("speed-fail", "m");
        assert_eq!(latency("speed-fail", "m"), None, "失败不算测出耗时");
        assert!(rank("speed-fail", "m").unwrap() >= FAILURE_PENALTY);
        assert!(snapshot().iter().all(|view| view.relay_id != "speed-fail"));
        record("speed-fail", "m", Duration::from_millis(500));
        assert_eq!(
            rank("speed-fail", "m"),
            Some(Duration::from_millis(500)),
            "成功后罚时清零"
        );
        forget("speed-fail");
        assert_eq!(rank("speed-fail", "m"), None);
    }
}
