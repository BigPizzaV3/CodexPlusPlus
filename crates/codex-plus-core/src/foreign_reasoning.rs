//! 别家签发的加密内容（#2777）。
//!
//! Codex 每轮都把历史里的 reasoning 项、以及自己压缩上下文得到的 compaction / compaction_summary 项
//! （都带 `encrypted_content`）原样回传。同一个对话先用中转、再换到官方账号（或反过来、或换到 xAI）时，
//! 这些内容是上一家签发的，新上游校验不过就整条请求失败：
//! 「The encrypted content for item rs_… could not be verified」「Could not decrypt the provided encrypted_content」。
//! 有的上游直接回 4xx，有的先回 200、再在流里发 `response.failed` / `error`。
//!
//! 处理方式偏保守：只有上游明确报了这类错误，才分批去掉请求里的加密项重发（每个请求最多
//! [`MAX_RETRIES`] 次），并按「上游（供应商 + 实际使用的 Key / 账号，见 [`memory_scope`]）+ 内容摘要」
//! 记住被拒的项，同一对话后面的请求发往这一家时提前去掉，不再每轮先失败一次。没被拒过的请求一律不动。
//!
//! 分批的顺序：先去 reasoning（只是模型的思考过程，去掉后上游照常按消息与工具历史继续），
//! 还被拒才去 compaction 项（它代替了压缩前的整段历史，去掉会丢掉那段上下文，代价大，留到最后）。
//! 只有两种情况会去 compaction：报错点名了某个 compaction 项的 id；或者请求里已经没有带加密内容的
//! 推理项（推理已在前一次重发或按记忆提前去掉，或本来就没有）却仍以同类报错被拒——这时被拒的只可能是
//! 压缩结果，不去掉对话就无法继续。去掉 compaction 会丢掉压缩前的对话内容，调用方要在请求日志里
//! 用 [`COMPACTION_DROPPED_REASON`] 标出来并记诊断日志，不能悄悄发生。

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

/// 记住的被拒项上限（按上游 + 内容摘要），超出先忘最久没用到的。
const MEMORY_LIMIT: usize = 4096;

/// 被拒项多久没再用到就忘掉：对话早就结束，或者用户已经换回签发它的那一家。
const MEMORY_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// 每个请求因为加密内容被拒最多重发几次（每次去掉一批）。
pub const MAX_RETRIES: u32 = 2;

/// 请求日志里这次重试的原因。
pub const RETRY_REASON: &str = "invalid_encrypted_content";

/// 请求日志里的标记：这次请求去掉了别家签发的压缩上下文（compaction / compaction_summary），
/// 较早的对话内容上游已经看不到。它只是说明，不算一次重试。
pub const COMPACTION_DROPPED_REASON: &str = "foreign_compaction_dropped";

/// 识别报错措辞时，两个关键短语之间最多隔多少个字符（例如中间夹着项 id、"provided" 等）。
const PHRASE_GAP_LIMIT: usize = 160;

type MemoryKey = (String, u64);

/// 一次去掉了哪些加密项：推理项与压缩项各几个。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Stripped {
    pub reasoning: usize,
    pub compaction: usize,
}

impl Stripped {
    pub fn total(&self) -> usize {
        self.reasoning + self.compaction
    }

    fn count(&mut self, batch: Batch) {
        match batch {
            Batch::Reasoning => self.reasoning += 1,
            Batch::Compaction => self.compaction += 1,
        }
    }
}

/// 被拒记忆的作用域：供应商 id，加上这次实际使用的 Key / 账号标识（只存摘要，不存原文）。
///
/// 一个供应商配了多个 Key 轮换、或中转背后是账号池时，A 签发的内容被 B 拒过，
/// 不代表 A 也会拒；按 Key 分开记，轮回 A 时不会被提前去掉。拿不到标识时退回只按供应商记。
pub fn memory_scope(relay_id: &str, credential: Option<&str>) -> String {
    match credential.map(str::trim).filter(|value| !value.is_empty()) {
        Some(credential) => format!("{relay_id}#{:016x}", digest(credential)),
        None => relay_id.to_string(),
    }
}

/// 加密项分批：先去代价小的推理，再去代价大的压缩结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Batch {
    Reasoning,
    Compaction,
}

const BATCH_ORDER: [Batch; 2] = [Batch::Reasoning, Batch::Compaction];

/// 被拒项的记忆：值是最近一次用到的时刻，按时间过期、按数量淘汰最久没用到的。
#[derive(Default)]
struct Memory {
    entries: HashMap<MemoryKey, Instant>,
}

impl Memory {
    fn forget_expired(&mut self, now: Instant) {
        self.entries
            .retain(|_, last_used| now.saturating_duration_since(*last_used) < MEMORY_TTL);
    }

    fn remember(&mut self, key: MemoryKey, now: Instant) {
        self.entries.insert(key, now);
        while self.entries.len() > MEMORY_LIMIT {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, last_used)| **last_used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
    }

    /// 命中时顺带刷新「最近用到」，正在进行的对话不会因为过期又先失败一次。
    fn touch(&mut self, key: &MemoryKey, now: Instant) -> bool {
        match self.entries.get_mut(key) {
            Some(last_used) => {
                *last_used = now;
                true
            }
            None => false,
        }
    }
}

fn memory() -> &'static Mutex<Memory> {
    static MEMORY: OnceLock<Mutex<Memory>> = OnceLock::new();
    MEMORY.get_or_init(|| Mutex::new(Memory::default()))
}

/// 带不透明加密内容的项返回它属于哪一批与内容；其它项（含空内容）返回 `None`。
///
/// Codex++ 自己的本地压缩摘要在进入代理时已经展开成普通 user 消息（见
/// `protocol_proxy::expand_local_compaction_history`），到这里不会再是 compaction 项。
fn encrypted_item(item: &Value) -> Option<(Batch, &str)> {
    let batch = match item.get("type").and_then(Value::as_str)? {
        "reasoning" => Batch::Reasoning,
        "compaction" | "compaction_summary" => Batch::Compaction,
        _ => return None,
    };
    let content = item
        .get("encrypted_content")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())?;
    Some((batch, content))
}

/// 按内容摘要识别同一个项：经网关发往 ChatGPT 后端时项 id 会被去掉，不能按 id 记。
fn digest(content: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    hasher.finish()
}

fn input_items(body: &Value) -> &[Value] {
    body.get("input")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

/// 请求里有没有带加密内容的项：没有时上游的错误与这里无关，不必预读错误体。
pub fn has_encrypted_items(body: &Value) -> bool {
    input_items(body)
        .iter()
        .any(|item| encrypted_item(item).is_some())
}

/// `needle` 在 `haystack` 里的所有起始下标。
fn positions(haystack: &str, needle: &str) -> Vec<usize> {
    haystack
        .match_indices(needle)
        .map(|(index, _)| index)
        .collect()
}

/// `first` 里任一短语之后、相隔不超过 [`PHRASE_GAP_LIMIT`] 个字符出现 `second` 里任一短语。
fn phrase_followed_by(text: &str, first: &[&str], second: &[&str]) -> bool {
    first.iter().any(|lead| {
        positions(text, lead).into_iter().any(|start| {
            let after = start + lead.len();
            let window: String = text[after..].chars().take(PHRASE_GAP_LIMIT).collect();
            second.iter().any(|tail| window.contains(tail))
        })
    })
}

/// 报错文字是不是「加密内容校验不过 / 解不开」。大小写不敏感，能认出：
/// - OpenAI 的错误码 `invalid_encrypted_content`；
/// - 「encrypted content / encrypted_content … could not be verified / decrypted」；
/// - 「could not decrypt / verify … encrypted content / encrypted_content」（xAI / Grok 的说法，
///   也包括被包成 `Grok (SuperGrok): …` 的样子）。
pub fn mentions_unverifiable_encrypted_content(text: &str) -> bool {
    const SUBJECT: &[&str] = &["encrypted content", "encrypted_content"];
    let text = text.to_ascii_lowercase();
    text.contains("invalid_encrypted_content")
        || phrase_followed_by(
            &text,
            SUBJECT,
            &["could not be verified", "could not be decrypted"],
        )
        || phrase_followed_by(&text, &["could not decrypt", "could not verify"], SUBJECT)
}

/// 上游的 4xx 错误体是不是「加密内容校验不过」。各家拒收用的状态码不一（400、422 …），
/// 这里只看是不是客户端错误，具体以错误体措辞为准；5xx 一律不算。
pub fn is_unverifiable_reasoning_rejection(status: u16, body: &[u8]) -> bool {
    (400..500).contains(&status)
        && mentions_unverifiable_encrypted_content(&String::from_utf8_lossy(body))
}

/// 流里的 `response.failed` / `error` 事件是不是「加密内容校验不过」。
pub fn is_unverifiable_stream_failure(code: Option<&str>, message: Option<&str>) -> bool {
    [code, message]
        .into_iter()
        .flatten()
        .any(mentions_unverifiable_encrypted_content)
}

/// 这次该去掉哪一批：报错点名了某个加密项的 id 就去它那一批，否则按 [`BATCH_ORDER`] 取第一批非空的。
///
/// 没点名时，只要请求里还有加密推理项就先去推理；compaction 只在推理已经不在请求里时才轮到
/// （见模块说明）。
fn rejected_batch(items: &[Value], error_text: &str) -> Option<Batch> {
    let error_text = error_text.to_ascii_lowercase();
    let named = items.iter().find_map(|item| {
        let (batch, _) = encrypted_item(item)?;
        let id = item.get("id").and_then(Value::as_str)?.trim();
        (!id.is_empty() && error_text.contains(&id.to_ascii_lowercase())).then_some(batch)
    });
    named.or_else(|| {
        BATCH_ORDER.into_iter().find(|batch| {
            items
                .iter()
                .any(|item| encrypted_item(item).is_some_and(|(found, _)| found == *batch))
        })
    })
}

/// 上游拒收后去掉一批加密项（见 [`rejected_batch`]），并记住它们被 `scope`（见 [`memory_scope`]）
/// 拒收过。`error_text` 是上游的报错（错误体或流内失败事件的 message），用来认出点名的项。
pub fn strip_rejected_batch(scope: &str, body: &mut Value, error_text: &str) -> Stripped {
    strip_rejected_batch_at(memory(), scope, body, error_text, Instant::now())
}

fn strip_rejected_batch_at(
    memory: &Mutex<Memory>,
    scope: &str,
    body: &mut Value,
    error_text: &str,
    now: Instant,
) -> Stripped {
    let mut stripped = Stripped::default();
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return stripped;
    };
    let Some(batch) = rejected_batch(items, error_text) else {
        return stripped;
    };
    let mut rejected = Vec::new();
    items.retain(|item| match encrypted_item(item) {
        Some((found, content)) if found == batch => {
            rejected.push(digest(content));
            stripped.count(found);
            false
        }
        _ => true,
    });
    let mut memory = memory.lock().unwrap_or_else(|error| error.into_inner());
    memory.forget_expired(now);
    for hash in &rejected {
        memory.remember((scope.to_string(), *hash), now);
    }
    stripped
}

/// 去掉 `scope`（见 [`memory_scope`]）之前拒收过的加密项（其余项保留）。
pub fn strip_remembered(scope: &str, body: &mut Value) -> Stripped {
    strip_remembered_at(memory(), scope, body, Instant::now())
}

fn strip_remembered_at(
    memory: &Mutex<Memory>,
    scope: &str,
    body: &mut Value,
    now: Instant,
) -> Stripped {
    let mut stripped = Stripped::default();
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return stripped;
    };
    let mut memory = memory.lock().unwrap_or_else(|error| error.into_inner());
    if memory.entries.is_empty() {
        return stripped;
    }
    memory.forget_expired(now);
    items.retain(|item| match encrypted_item(item) {
        Some((batch, content)) if memory.touch(&(scope.to_string(), digest(content)), now) => {
            stripped.count(batch);
            false
        }
        _ => true,
    });
    stripped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn recognizes_openai_and_wrapped_rejections_on_4xx_only() {
        let openai = br#"{"error":{"message":"The encrypted content for item rs_1 could not be verified. Reason: Encrypted content item_id did not match the target item id.","type":"invalid_request_error","param":null,"code":"invalid_encrypted_content"}}"#;
        assert!(is_unverifiable_reasoning_rejection(400, openai));
        assert!(is_unverifiable_reasoning_rejection(422, openai));
        let wrapped = br#"{"error":{"message":"upstream: The encrypted content gAAA could not be decrypted or parsed."}}"#;
        assert!(is_unverifiable_reasoning_rejection(400, wrapped));
        assert!(!is_unverifiable_reasoning_rejection(500, openai));
        assert!(!is_unverifiable_reasoning_rejection(200, openai));
        assert!(!is_unverifiable_reasoning_rejection(
            400,
            br#"{"error":{"message":"Unsupported parameter: reasoning.effort"}}"#
        ));
    }

    #[test]
    fn recognizes_xai_and_grok_wordings() {
        for text in [
            "Could not decrypt the provided encrypted_content. Ensure the value is the unmodified encrypted_content from a previous response.",
            "Grok (SuperGrok): Could not decrypt the provided encrypted_content.",
            r#"{"code":"Client specified an invalid argument","error":"Could not decrypt the provided encrypted_content."}"#,
            "COULD NOT VERIFY THE ENCRYPTED CONTENT OF ITEM rs_9",
            "The encrypted_content for item cmp_1 could not be verified",
            "invalid_encrypted_content",
        ] {
            assert!(mentions_unverifiable_encrypted_content(text), "{text}");
            assert!(
                is_unverifiable_reasoning_rejection(400, text.as_bytes()),
                "{text}"
            );
        }
        assert!(is_unverifiable_stream_failure(
            None,
            Some("Grok (SuperGrok): Could not decrypt the provided encrypted_content.")
        ));
        assert!(is_unverifiable_stream_failure(
            Some("invalid_encrypted_content"),
            None
        ));
    }

    #[test]
    fn does_not_mistake_other_errors() {
        for text in [
            "Invalid value for reasoning.effort",
            "model not found",
            "could not parse the JSON body",
            "The model `gpt-x` does not exist or you do not have access to it.",
            "Our servers are currently overloaded. Please try again later.",
            "encrypted content is not supported for this model",
            "could not decrypt your request: TLS handshake failed",
            "",
        ] {
            assert!(!mentions_unverifiable_encrypted_content(text), "{text}");
            assert!(
                !is_unverifiable_reasoning_rejection(400, text.as_bytes()),
                "{text}"
            );
        }
        // 两个短语隔得太远（拼在一起的无关文字）不算。
        let far = format!(
            "encrypted content{}could not be verified",
            " filler".repeat(40)
        );
        assert!(!mentions_unverifiable_encrypted_content(&far));
        assert!(!is_unverifiable_stream_failure(
            Some("server_is_overloaded"),
            Some("Our servers are currently overloaded.")
        ));
        assert!(!is_unverifiable_stream_failure(None, None));
    }

    #[test]
    fn remembered_items_are_stripped_only_for_the_rejecting_relay() {
        let relay = "foreign-reasoning-unit";
        let mut body = json!({"input": [
            {"type": "message", "role": "user", "content": "hi"},
            {"type": "reasoning", "id": "rs_old", "summary": [], "encrypted_content": "unit-old"},
            {"type": "reasoning", "summary": [{"type": "summary_text", "text": "plain"}]},
        ]});
        assert!(has_encrypted_items(&body));
        assert_eq!(strip_rejected_batch(relay, &mut body, "").total(), 1);
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
        assert!(!has_encrypted_items(&body));

        let mut next = json!({"input": [
            {"type": "reasoning", "id": "rs_old", "summary": [], "encrypted_content": "unit-old"},
            {"type": "reasoning", "id": "rs_new", "summary": [], "encrypted_content": "unit-new"},
        ]});
        let mut elsewhere = next.clone();
        assert_eq!(strip_remembered(relay, &mut next).total(), 1);
        assert_eq!(next["input"][0]["id"], "rs_new");
        // 另一家没拒收过：原样保留。
        assert_eq!(strip_remembered("another-relay", &mut elsewhere).total(), 0);
        assert_eq!(elsewhere["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn batches_strip_reasoning_first_then_compaction() {
        let memory = Mutex::new(Memory::default());
        let now = Instant::now();
        let mut body = json!({"input": [
            {"type": "compaction", "id": "cmp_1", "encrypted_content": "unit-cmp"},
            {"type": "compaction_summary", "encrypted_content": "unit-cmp-summary"},
            {"type": "compaction", "encrypted_content": ""},
            {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "unit-rs"},
            {"type": "message", "role": "user", "content": "hi"},
        ]});
        // 没点名：先去推理。
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, "could not be verified", now),
            Stripped {
                reasoning: 1,
                compaction: 0
            }
        );
        assert_eq!(body["input"].as_array().unwrap().len(), 4);
        // 再被拒：去压缩项（空内容的不算加密项，保留）。
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, "", now),
            Stripped {
                reasoning: 0,
                compaction: 2
            }
        );
        assert_eq!(
            body["input"],
            json!([
                {"type": "compaction", "encrypted_content": ""},
                {"type": "message", "role": "user", "content": "hi"},
            ])
        );
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, "", now).total(),
            0
        );
        assert_eq!(memory.lock().unwrap().entries.len(), 3);
    }

    #[test]
    fn named_item_selects_its_batch_first() {
        let memory = Mutex::new(Memory::default());
        let mut body = json!({"input": [
            {"type": "reasoning", "id": "rs_1", "summary": [], "encrypted_content": "unit-rs"},
            {"type": "compaction", "id": "cmp_77", "encrypted_content": "unit-cmp"},
        ]});
        let error = "The encrypted content for item CMP_77 could not be verified.";
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, error, Instant::now()),
            Stripped {
                reasoning: 0,
                compaction: 1
            }
        );
        assert_eq!(body["input"][0]["id"], "rs_1", "推理项不被连带去掉");
    }

    #[test]
    fn compaction_is_kept_while_encrypted_reasoning_remains() {
        let memory = Mutex::new(Memory::default());
        let now = Instant::now();
        let compaction =
            json!({"type": "compaction", "id": "cmp_9", "encrypted_content": "unit-cmp-9"});
        // 点名的是推理项：只去推理，压缩结果不动。
        let mut body = json!({"input": [
            compaction.clone(),
            {"type": "reasoning", "id": "rs_9", "summary": [], "encrypted_content": "unit-rs-9"},
        ]});
        let error = "The encrypted content for item rs_9 could not be verified.";
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, error, now),
            Stripped {
                reasoning: 1,
                compaction: 0
            }
        );
        assert_eq!(body["input"], json!([compaction.clone()]));
        // 推理已不在请求里、仍以同类报错被拒：这时才去压缩结果。
        assert_eq!(
            strip_rejected_batch_at(&memory, "r", &mut body, error, now),
            Stripped {
                reasoning: 0,
                compaction: 1
            }
        );
        // 下一轮按记忆提前去掉时，也能分清去掉的是压缩结果。
        let mut next = json!({"input": [compaction]});
        assert_eq!(
            strip_remembered_at(&memory, "r", &mut next, now),
            Stripped {
                reasoning: 0,
                compaction: 1
            }
        );
    }

    #[test]
    fn memory_is_scoped_per_key_of_the_same_relay() {
        let memory = Mutex::new(Memory::default());
        let now = Instant::now();
        let key_a = memory_scope("pool", Some("sk-key-a"));
        let key_b = memory_scope("pool", Some("sk-key-b"));
        assert_ne!(key_a, key_b);
        assert!(!key_a.contains("sk-key-a"), "不存 Key 原文");
        assert_eq!(
            memory_scope("pool", Some("  ")),
            "pool",
            "拿不到标识时按供应商记"
        );
        assert_eq!(memory_scope("pool", None), "pool");

        let item =
            json!({"type": "reasoning", "summary": [], "encrypted_content": "unit-signed-by-a"});
        let mut body = json!({"input": [item.clone()]});
        assert_eq!(
            strip_rejected_batch_at(&memory, &key_b, &mut body, "", now).total(),
            1
        );
        // Key B 拒收过，发给 Key A 时原样保留；再发给 Key B 时提前去掉。
        let mut to_a = json!({"input": [item.clone()]});
        assert_eq!(
            strip_remembered_at(&memory, &key_a, &mut to_a, now).total(),
            0
        );
        assert_eq!(to_a["input"].as_array().unwrap().len(), 1);
        let mut to_b = json!({"input": [item]});
        assert_eq!(
            strip_remembered_at(&memory, &key_b, &mut to_b, now).total(),
            1
        );
    }

    #[test]
    fn memory_expires_after_ttl_without_use_and_refreshes_on_use() {
        let memory = Mutex::new(Memory::default());
        let start = Instant::now();
        let item = json!({"type": "reasoning", "summary": [], "encrypted_content": "unit-ttl"});
        let mut body = json!({"input": [item.clone()]});
        strip_rejected_batch_at(&memory, "r", &mut body, "", start);

        // 快到期时用到一次：刷新时间。
        let used = start + MEMORY_TTL - Duration::from_secs(60);
        let mut body = json!({"input": [item.clone()]});
        assert_eq!(
            strip_remembered_at(&memory, "r", &mut body, used).total(),
            1
        );

        // 从刷新时刻起还没到期：仍然记得。
        let later = used + MEMORY_TTL - Duration::from_secs(60);
        let mut body = json!({"input": [item.clone()]});
        assert_eq!(
            strip_remembered_at(&memory, "r", &mut body, later).total(),
            1
        );

        // 之后整整一天没用到：忘掉，请求原样发出。
        let expired = later + MEMORY_TTL;
        let mut body = json!({"input": [item.clone()]});
        assert_eq!(
            strip_remembered_at(&memory, "r", &mut body, expired).total(),
            0
        );
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert!(memory.lock().unwrap().entries.is_empty());
    }

    #[test]
    fn memory_limit_evicts_least_recently_used() {
        let mut memory = Memory::default();
        let start = Instant::now();
        for index in 0..MEMORY_LIMIT {
            memory.remember(
                ("r".to_string(), index as u64),
                start + Duration::from_millis(index as u64),
            );
        }
        // 最早记住的那项刚用过，不该被淘汰；被淘汰的是第二早的。
        let late = start + Duration::from_secs(3600);
        assert!(memory.touch(&("r".to_string(), 0), late));
        memory.remember(("r".to_string(), u64::MAX), late);
        assert_eq!(memory.entries.len(), MEMORY_LIMIT);
        assert!(memory.entries.contains_key(&("r".to_string(), 0)));
        assert!(!memory.entries.contains_key(&("r".to_string(), 1)));
    }

    #[test]
    fn memory_is_safe_under_concurrent_use() {
        let relay = "foreign-reasoning-concurrent";
        let handles: Vec<_> = (0..8)
            .map(|thread| {
                std::thread::spawn(move || {
                    for index in 0..50 {
                        let content = format!("unit-concurrent-{thread}-{index}");
                        let item = json!({"type": "reasoning", "encrypted_content": content});
                        let mut body = json!({"input": [item.clone()]});
                        assert_eq!(strip_rejected_batch(relay, &mut body, "").total(), 1);
                        let mut again = json!({"input": [item]});
                        assert_eq!(strip_remembered(relay, &mut again).total(), 1);
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
    }
}
