// 「prompt 前缀稳定层」的分叉诊断探针（opt-in，随 stabilize_prompt_prefix 生效）。
//
// 背景（2026-10-11 凌晨取证）：codex 客户端重启后重放历史，system 区出现
// +347B 变化（sys 49927b -> 50274b），mlx-serve 前缀从 98.6% 深命中跌到
// 24576（仅 sys+tools 之前的浅层），重启首轮全历史冷算约 68 秒。
// 现有毒源清单（OPS_RUNBOOK §5）对不上号，需要 payload 级证据：
// 同一会话（prompt_cache_key）的「转换后 system 块序列 + tools 序列化」
// 指纹由本模块逐轮记录，字节变化即 dump 分叉点新旧对比，基线另存全文。
use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Mutex;

/// 与 compaction_tools 相同的会话缓存上限策略：超限整体重置，只退化为一次基线。
const CACHE_LIMIT: usize = 32;

/// dump 分叉点上下文半径（字节）。
const SNIP_RADIUS: usize = 240;

pub enum DivergenceKind {
    /// 首次见到该会话：基线已登记（全文另存 /tmp）。
    Baseline,
    /// system+tools 前缀字节与上一轮不同。
    Diverged,
}

pub struct DivergenceReport {
    pub kind: DivergenceKind,
    pub old_len: usize,
    pub new_len: usize,
    /// 首个差异字节偏移；基线轮为 0。
    pub first_diff: usize,
    pub old_snip: String,
    pub new_snip: String,
}

static CACHE: std::sync::OnceLock<Mutex<BTreeMap<String, String>>> = std::sync::OnceLock::new();

fn normalize_cache_key(key: &str) -> Option<String> {
    let trimmed = key.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 提取 chat message 的文本内容：字符串直接用；数组按 part 拼接 text。
fn message_text(message: &Value) -> Option<String> {
    match message.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(parts)) => {
            let mut joined = String::new();
            for part in parts {
                match part.get("text").and_then(Value::as_str) {
                    Some(text) => {
                        joined.push_str(text);
                        joined.push('\n');
                    }
                    // content part 形态不认识：宁可放弃本轮观察，也不要把
                    // 结构差异误报成分叉（基线照登，事件宁缺毋滥）。
                    None => return None,
                }
            }
            Some(joined)
        }
        _ => None,
    }
}

/// 会话前缀指纹 = 头部连续 system/developer 块内容 + 末尾分隔 + tools 序列化。
/// 只取「头部连续段」：历史消息里混入的 system 项属于消息区，由稳定层
/// 折叠逻辑负责，不归本探针（探针盯的是重启分叉最高发的 prompt 最前端）。
pub fn prompt_prefix_fingerprint(body: &Value) -> Option<String> {
    let mut fingerprint = String::new();
    let messages = body.get("messages").and_then(Value::as_array);
    for message in messages.unwrap_or(&Vec::new()) {
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        if role != "system" && role != "developer" {
            break;
        }
        fingerprint.push_str(&message_text(message)?);
        fingerprint.push('\n');
    }
    match body.get("tools") {
        Some(Value::Array(tools)) if !tools.is_empty() => {
            serde_json::to_string(&Value::Array(tools.clone())).ok().map(|tools| format!("{fingerprint}\u{1}{tools}"))
        }
        None | Some(Value::Null) => Some(fingerprint),
        _ => None,
    }
}

/// UTF-8 安全的分叉点上下文切片：按 first_diff 所在 char 边界向两侧扩半径。
fn snip_around(text: &str, first_diff: usize, radius: usize) -> String {
    let bytes = text.as_bytes();
    let mut start = first_diff.min(bytes.len());
    while start > 0 && !text.is_char_boundary(start) {
        start -= 1;
    }
    let mut end = (first_diff + radius).min(bytes.len());
    while end < bytes.len() && !text.is_char_boundary(end) {
        end += 1;
    }
    format!(
        "<<{}|{}>>",
        String::from_utf8_lossy(&text.as_bytes()[start..first_diff]),
        String::from_utf8_lossy(&text.as_bytes()[first_diff..end]),
    )
}

fn first_diff_offset(old: &str, new: &str) -> usize {
    old.bytes()
        .zip(new.bytes())
        .take_while(|(a, b)| a == b)
        .count()
}

/// 记录并对比当前轮前缀指纹。返回 None 表示本轮放弃观察（缺 key / 形态不认）。
pub fn observe(prompt_cache_key: Option<&str>, body: &Value) -> Option<DivergenceReport> {
    let key = prompt_cache_key.and_then(normalize_cache_key)?;
    let fingerprint = prompt_prefix_fingerprint(body)?;
    let mut cache = CACHE.get_or_init(Default::default).lock().ok()?;
    let previous = cache.insert(key.clone(), fingerprint.clone());
    let Some(previous) = previous else {
        drop(cache);
        dump_baseline_file(&key, &fingerprint);
        return Some(DivergenceReport {
            kind: DivergenceKind::Baseline,
            old_len: 0,
            new_len: fingerprint.len(),
            first_diff: 0,
            old_snip: String::new(),
            new_snip: String::new(),
        });
    };
    if previous == fingerprint {
        return None;
    }
    let first_diff = first_diff_offset(&previous, &fingerprint);
    Some(DivergenceReport {
        kind: DivergenceKind::Diverged,
        old_len: previous.len(),
        new_len: fingerprint.len(),
        first_diff,
        old_snip: snip_around(&previous, first_diff, SNIP_RADIUS),
        new_snip: snip_around(&fingerprint, first_diff, SNIP_RADIUS),
    })
}

/// 基线全文落盘（本机诊断文件；分叉 dump 会引用它做全文对照）。
fn dump_baseline_file(key: &str, fingerprint: &str) {
    let digest = format!("{:016x}", fnv1a64(key.as_bytes()));
    let path = format!("/tmp/prefix_baseline_{digest}.txt");
    let _ = std::fs::write(path, fingerprint);
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body(system: &str, tools: Value) -> Value {
        json!({
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": "hi"},
            ],
            "tools": tools,
        })
    }

    fn request(key: &str) -> Value {
        json!({"prompt_cache_key": key})
    }

    fn key_of(request: &Value) -> Option<&str> {
        request.get("prompt_cache_key").and_then(Value::as_str)
    }

    #[test]
    fn first_round_reports_baseline_and_stores_file() {
        let key = "thread-div-baseline-1";
        let report = observe(key_of(&request(key)), &body("SYS", json!([{"type": "function"}]))).expect("首轮必须登记");
        assert!(matches!(report.kind, DivergenceKind::Baseline));
        assert!(report.new_len > 0);
        let digest = format!("{:016x}", fnv1a64(key.as_bytes()));
        let path = format!("/tmp/prefix_baseline_{digest}.txt");
        assert!(std::path::Path::new(&path).exists(), "基线全文必须落盘供取证");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn identical_prefix_reports_nothing_but_changed_sys_diverges_with_offset() {
        let key = "thread-div-baseline-2";
        observe(key_of(&request(key)), &body("SYS-A", json!([{"type": "function"}]))).expect("基线");
        // 完全相同：无事件。
        assert!(observe(key_of(&request(key)), &body("SYS-A", json!([{"type": "function"}]))).is_none());
        // 纯追加：分叉点落在旧文本末尾。
        let report = observe(key_of(&request(key)), &body("SYS-A-ADDED", json!([{"type": "function"}]))).expect("必须有分叉事件");
        assert!(matches!(report.kind, DivergenceKind::Diverged));
        assert_eq!(report.first_diff, "SYS-A".len(), "分叉点=旧文本首个不同字节（原换行处）");
        // 更新后相同前缀再次静默。
        assert!(observe(key_of(&request(key)), &body("SYS-A-ADDED", json!([{"type": "function"}]))).is_none());
    }

    #[test]
    fn tools_change_also_diverges() {
        let key = "thread-div-baseline-3";
        observe(key_of(&request(key)), &body("SYS", json!([{"type": "function", "function": {"name": "a"}}]))).expect("基线");
        let report = observe(key_of(&request(key)), &body("SYS", json!([{"type": "function", "function": {"name": "b"}}]))).expect("tools 变化必须可见");
        assert!(matches!(report.kind, DivergenceKind::Diverged));
        assert!(report.first_diff > 4);
    }

    #[test]
    fn unknown_message_shape_abandons_observation() {
        let key = "thread-div-baseline-4";
        // system content 为未知形态（数字）：放弃观察而不是误报。
        let odd = json!({"messages": [{"role": "system", "content": 1}], "tools": []});
        assert!(observe(key_of(&request(key)), &odd).is_none());
        // Null tools 属可观察形态：首轮必须登记为基线（不依赖并行用例的全局计数）。
        let report = observe(key_of(&request("thread-div-baseline-5")), &body("SYS", Value::Null)).expect("Null tools 必须可观察为基线");
        assert!(matches!(report.kind, DivergenceKind::Baseline));
    }

    #[test]
    fn snip_keeps_utf8_boundaries() {
        let text = "中文前缀测试×30".repeat(30);
        // 故意切在多字节字符中间：snip 不得 panic。
        let raw = snip_around(&text, 7, 16);
        assert!(raw.starts_with("<<") && raw.ends_with(">>"));
    }
}
