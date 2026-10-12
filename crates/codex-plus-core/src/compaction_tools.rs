// compaction 请求的 tools 回填缓存（「prompt 前缀稳定层」的组成部分，opt-in 同开关）。
//
// 背景（取证定案，见 COMPACT_PREFILL.md）：codex 客户端的 v2 压缩请求自带
// `tools: []`；而上游 chat 模板把 tools 渲染进 prompt 前部，且 relay 转换层
// 折叠历史 tool 项时依赖 tools schema——tools 由整块变空会让全历史消息重构
// （实测 231→388 msgs）、token 前缀在历史首个 tool 项处分叉，压缩轮整段
// ~15 万 token 全量冷算（约 4 分钟）。
//
// 方案：普通请求把 tools 序列化缓存下来（key = prompt_cache_key，codex 客户端
// 里等于线程 id，跨轮稳定）；压缩请求回填同一 Value——序列化逐字节一致，
// 历史折叠形态不变，token 前缀纯追加；摘要阶段禁工具调用交给 tool_choice=none
// （rewrite_request_for_compaction 已设置，不入 prompt 文本，不影响前缀）。
// 无缓存（如 relay 重启后首次即压缩）时保持旧行为（tools 置空）。
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};

/// 上限 32 会话；超限整体重置（工具定义体量小，重置只会退回旧行为一次）。
const CACHE_LIMIT: usize = 32;

static CACHE: OnceLock<Mutex<BTreeMap<String, Value>>> = OnceLock::new();

fn cache_key(body: &Value) -> Option<String> {
    body.get("prompt_cache_key")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

/// 普通请求（非压缩轮）记录 tools 序列化，供后续压缩轮回填。
pub fn remember_tools(request: &Value) {
    let Some(key) = cache_key(request) else {
        return;
    };
    let Some(tools) = request
        .get("tools")
        .filter(|tools| tools.is_array())
        .cloned()
    else {
        return;
    };
    let Ok(mut cache) = CACHE.get_or_init(Default::default).lock() else {
        return;
    };
    if cache.len() >= CACHE_LIMIT && !cache.contains_key(&key) {
        cache.clear();
    }
    cache.insert(key, tools);
}

/// 压缩请求取回该会话最近一次普通请求的 tools；无缓存返回 None（回退旧行为）。
pub fn preserved_tools(request: &Value) -> Option<Value> {
    let key = cache_key(request)?;
    let cache = CACHE.get_or_init(Default::default).lock().ok()?;
    cache.get(&key).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remember_then_preserved_round_trips_exact_bytes() {
        let request = json!({
            "prompt_cache_key": "thread-unit-1",
            "tools": [{"type": "function", "name": "exec_command", "parameters": {"type": "object"}}],
        });
        remember_tools(&request);
        let preserved = preserved_tools(&request).expect("回填必须命中");
        assert_eq!(
            serde_json::to_string(&preserved).unwrap(),
            serde_json::to_string(&request["tools"]).unwrap(),
            "回填序列化必须与普通请求逐字节一致"
        );
    }

    #[test]
    fn remember_ignores_missing_or_non_array_tools() {
        let request = json!({"prompt_cache_key": "thread-unit-2"});
        remember_tools(&request);
        assert!(preserved_tools(&request).is_none());
        let request2 = json!({"prompt_cache_key": "thread-unit-2", "tools": {"weird": true}});
        remember_tools(&request2);
        assert!(preserved_tools(&request2).is_none());
    }

    #[test]
    fn preserved_requires_known_key() {
        let compaction = json!({"prompt_cache_key": "thread-unit-never-seen", "tools": []});
        assert!(preserved_tools(&compaction).is_none());
    }

    #[test]
    fn later_normal_request_overwrites_cached_tools() {
        let first = json!({
            "prompt_cache_key": "thread-unit-3",
            "tools": [{"type": "function", "name": "old_tool"}],
        });
        remember_tools(&first);
        let second = json!({
            "prompt_cache_key": "thread-unit-3",
            "tools": [{"type": "function", "name": "new_tool"}],
        });
        remember_tools(&second);
        let preserved = preserved_tools(&second).expect("回填必须命中最新一轮");
        assert_eq!(preserved[0]["name"], json!("new_tool"));
    }
}
