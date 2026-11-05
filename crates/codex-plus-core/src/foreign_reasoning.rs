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
//!
//! 另一类同源的拒收（#2917）：DeepSeek / GLM 等经 Codex++ 转换回来的 reasoning 项带着明文
//! `content`（`reasoning_text` 片段）和一个占位的 `encrypted_content`。同一对话换到 OpenAI 风格的
//! Responses 上游后，它要求 reasoning 的 `content` 必须为空，整条请求 400：
//! 「Invalid 'input[26].content': array too long. Expected an array with maximum length 0 …」
//! （`array_above_max_length`）。只有上游明确这样报、且点名的 `input[N]` 确实是带明文的推理项时，
//! 才把所有推理项的 `content` 清空（`summary`、`encrypted_content` 保留）后重发一次，
//! 并按「上游（[`memory_scope`]）+ 模型」记住，同一对话后面的请求发往这一家这个模型时提前清空。

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

/// 请求日志里这次重试的原因：上游不收 reasoning 项的明文 `content`（#2917）。
pub const CONTENT_RETRY_REASON: &str = "reasoning_content_not_allowed";

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

/// 被拒的记忆：值是最近一次用到的时刻，按时间过期、按数量淘汰最久没用到的。
/// 加密项按「作用域 + 内容摘要」记；推理明文被拒（#2917）按「作用域 + 模型」记。
struct Memory<K = MemoryKey> {
    entries: HashMap<K, Instant>,
}

impl<K> Default for Memory<K> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }
}

impl<K: Eq + Hash + Clone> Memory<K> {
    fn forget_expired(&mut self, now: Instant) {
        self.entries
            .retain(|_, last_used| now.saturating_duration_since(*last_used) < MEMORY_TTL);
    }

    fn remember(&mut self, key: K, now: Instant) {
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
    fn touch(&mut self, key: &K, now: Instant) -> bool {
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

/// 推理明文被拒的记忆键：作用域（见 [`memory_scope`]）+ 发出去的模型名。
type ContentMemoryKey = (String, String);

fn content_memory() -> &'static Mutex<Memory<ContentMemoryKey>> {
    static MEMORY: OnceLock<Mutex<Memory<ContentMemoryKey>>> = OnceLock::new();
    MEMORY.get_or_init(|| Mutex::new(Memory::default()))
}

/// 推理项带着非空的明文 `content` 数组。
fn has_plain_reasoning_content(item: &Value) -> bool {
    item.get("type").and_then(Value::as_str) == Some("reasoning")
        && item
            .get("content")
            .and_then(Value::as_array)
            .is_some_and(|parts| !parts.is_empty())
}

/// 请求里有没有带明文 `content` 的推理项：没有时上游的错误与 #2917 无关，不必预读错误体。
pub fn has_reasoning_content(body: &Value) -> bool {
    input_items(body).iter().any(has_plain_reasoning_content)
}

/// 报错里点名的 `input[N].content` 的所有 N（只认正好指到 `content` 本身的，
/// `input[N].content[0].text` 这类指向内部字段的不算）。
fn named_content_indexes(text: &str) -> Vec<usize> {
    const LEAD: &str = "input[";
    const TAIL: &str = "].content";
    let mut indexes = Vec::new();
    for start in positions(text, LEAD) {
        let rest = &text[start + LEAD.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 || !rest[digits..].starts_with(TAIL) {
            continue;
        }
        let mut after = rest[digits + TAIL.len()..].chars();
        let continues_path = match after.next() {
            None => false,
            Some('[') => true,
            Some('.') => after
                .next()
                .is_some_and(|next| next.is_ascii_alphanumeric() || next == '_'),
            Some(next) => next.is_ascii_alphanumeric() || next == '_',
        };
        if continues_path {
            continue;
        }
        if let Ok(index) = rest[..digits].parse() {
            indexes.push(index);
        }
    }
    indexes
}

/// 报错文字是不是「推理项的 `content` 必须为空」（#2917）。大小写不敏感，要同时满足：
/// - 提到 `array_above_max_length` 或「expected an array with maximum length 0」；
/// - 点名了 `input[N].content`，且 `body`（这次实际发出的请求）里 `input[N]` 是带明文 `content` 的推理项。
pub fn mentions_reasoning_content_not_allowed(text: &str, body: &Value) -> bool {
    let text = text.to_ascii_lowercase();
    if !text.contains("array_above_max_length")
        && !text.contains("expected an array with maximum length 0")
    {
        return false;
    }
    let items = input_items(body);
    named_content_indexes(&text)
        .into_iter()
        .any(|index| items.get(index).is_some_and(has_plain_reasoning_content))
}

/// 上游的 4xx 错误体是不是「推理项的 `content` 必须为空」；5xx 一律不算。
pub fn is_reasoning_content_rejection(status: u16, error_body: &[u8], body: &Value) -> bool {
    (400..500).contains(&status)
        && mentions_reasoning_content_not_allowed(&String::from_utf8_lossy(error_body), body)
}

/// 流里的 `response.failed` / `error` 事件是不是「推理项的 `content` 必须为空」。
/// 错误码与 message 可能分开放（码里是 `array_above_max_length`，message 里点名位置），合起来看。
pub fn is_reasoning_content_stream_failure(
    code: Option<&str>,
    message: Option<&str>,
    body: &Value,
) -> bool {
    let text = [code, message]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ");
    mentions_reasoning_content_not_allowed(&text, body)
}

/// 清空所有推理项的明文 `content`（`summary`、`encrypted_content` 等其余字段不动），返回清空了几项。
fn empty_reasoning_content(body: &mut Value) -> usize {
    let Some(items) = body.get_mut("input").and_then(Value::as_array_mut) else {
        return 0;
    };
    let mut emptied = 0;
    for item in items
        .iter_mut()
        .filter(|item| has_plain_reasoning_content(item))
    {
        item["content"] = Value::Array(Vec::new());
        emptied += 1;
    }
    emptied
}

fn content_memory_key(scope: &str, body: &Value) -> ContentMemoryKey {
    let model = body
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    (scope.to_string(), model.to_string())
}

/// 上游拒收推理明文后清空所有推理项的 `content`，并记住 `scope`（见 [`memory_scope`]）的这个模型不收。
/// 返回清空了几项；0 表示请求里没有可清的，错误照常交回。
pub fn strip_rejected_reasoning_content(scope: &str, body: &mut Value) -> usize {
    strip_rejected_reasoning_content_at(content_memory(), scope, body, Instant::now())
}

fn strip_rejected_reasoning_content_at(
    memory: &Mutex<Memory<ContentMemoryKey>>,
    scope: &str,
    body: &mut Value,
    now: Instant,
) -> usize {
    let emptied = empty_reasoning_content(body);
    if emptied > 0 {
        let mut memory = memory.lock().unwrap_or_else(|error| error.into_inner());
        memory.forget_expired(now);
        memory.remember(content_memory_key(scope, body), now);
    }
    emptied
}

/// `scope`（见 [`memory_scope`]）的这个模型之前拒收过推理明文：提前清空，返回清空了几项。
pub fn strip_remembered_reasoning_content(scope: &str, body: &mut Value) -> usize {
    strip_remembered_reasoning_content_at(content_memory(), scope, body, Instant::now())
}

fn strip_remembered_reasoning_content_at(
    memory: &Mutex<Memory<ContentMemoryKey>>,
    scope: &str,
    body: &mut Value,
    now: Instant,
) -> usize {
    if !has_reasoning_content(body) {
        return 0;
    }
    let mut memory = memory.lock().unwrap_or_else(|error| error.into_inner());
    if memory.entries.is_empty() {
        return 0;
    }
    memory.forget_expired(now);
    if !memory.touch(&content_memory_key(scope, body), now) {
        return 0;
    }
    drop(memory);
    empty_reasoning_content(body)
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

    /// DeepSeek / GLM 经 Codex++ 转换回来的推理项：明文 content + 占位 encrypted_content。
    fn plain_reasoning(text: &str) -> Value {
        json!({"type": "reasoning", "summary": [{"type": "summary_text", "text": "s"}],
               "content": [{"type": "reasoning_text", "text": text}],
               "encrypted_content": "placeholder"})
    }

    /// 第 2 项是带明文的推理项。
    fn plain_reasoning_body() -> Value {
        json!({"model": "gpt-5.5", "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
            {"type": "reasoning", "summary": [], "content": [], "encrypted_content": "gAAA"},
            plain_reasoning("deepseek thinking"),
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]},
        ]})
    }

    const OPENAI_CONTENT_REJECTION: &str = r#"{"error":{"message":"[ArrayParam] [input[2].content] [array_above_max_length] Invalid 'input[2].content': array too long. Expected an array with maximum length 0, but got an array with length 1 instead.","type":"invalid_request_error","param":"input[2].content","code":"array_above_max_length"}}"#;

    #[test]
    fn recognizes_reasoning_content_rejection_naming_a_plain_reasoning_item() {
        let body = plain_reasoning_body();
        assert!(has_reasoning_content(&body));
        assert!(is_reasoning_content_rejection(
            400,
            OPENAI_CONTENT_REJECTION.as_bytes(),
            &body
        ));
        assert!(is_reasoning_content_rejection(
            422,
            OPENAI_CONTENT_REJECTION.as_bytes(),
            &body
        ));
        assert!(!is_reasoning_content_rejection(
            500,
            OPENAI_CONTENT_REJECTION.as_bytes(),
            &body
        ));
        // 只有其中一种说法也认；大小写不敏感。
        for text in [
            "Invalid 'input[2].content': array too long. Expected an array with maximum length 0, but got an array with length 1 instead.",
            "INVALID 'INPUT[2].CONTENT': ARRAY_ABOVE_MAX_LENGTH",
            "Nebula upstream error: [input[2].content] [array_above_max_length]",
        ] {
            assert!(
                mentions_reasoning_content_not_allowed(text, &body),
                "{text}"
            );
        }
        // 流内失败：码与 message 分开放也认。
        assert!(is_reasoning_content_stream_failure(
            Some("array_above_max_length"),
            Some("Invalid 'input[2].content': array too long."),
            &body
        ));
    }

    #[test]
    fn does_not_mistake_other_array_errors_for_reasoning_content() {
        let body = plain_reasoning_body();
        for text in [
            // 点名的不是推理项 / 是已经为空的推理项 / 越界。
            "Invalid 'input[0].content': array too long. Expected an array with maximum length 0",
            "[input[1].content] [array_above_max_length]",
            "[input[9].content] [array_above_max_length]",
            // 指向内部字段。
            "[input[2].content[0].text] [array_above_max_length]",
            "[input[2].contents] [array_above_max_length]",
            // 没说数组超长。
            "Invalid 'input[2].content': expected a string",
            "Invalid type for 'input[2].content'",
            // 别的数组超长。
            "Invalid 'tools': array too long. Expected an array with maximum length 128 (array_above_max_length)",
            "The encrypted content for item rs_1 could not be verified.",
            "",
        ] {
            assert!(
                !mentions_reasoning_content_not_allowed(text, &body),
                "{text}"
            );
            assert!(
                !is_reasoning_content_rejection(400, text.as_bytes(), &body),
                "{text}"
            );
        }
        // 请求里没有带明文的推理项：同样的报错不算。
        let mut emptied = body.clone();
        empty_reasoning_content(&mut emptied);
        assert!(!has_reasoning_content(&emptied));
        assert!(!is_reasoning_content_rejection(
            400,
            OPENAI_CONTENT_REJECTION.as_bytes(),
            &emptied
        ));
        assert!(!is_reasoning_content_stream_failure(None, None, &body));
        assert!(!mentions_unverifiable_encrypted_content(
            OPENAI_CONTENT_REJECTION
        ));
    }

    #[test]
    fn empties_only_non_empty_reasoning_content_and_keeps_other_fields() {
        let memory = Mutex::new(Memory::default());
        let now = Instant::now();
        let mut body = json!({"model": "gpt-5.5", "input": [
            {"type": "message", "role": "user", "content": [{"type": "input_text", "text": "hi"}]},
            plain_reasoning("first"),
            {"type": "reasoning", "summary": [], "content": [], "encrypted_content": "gAAA"},
            {"type": "reasoning", "summary": [], "encrypted_content": "gBBB"},
            plain_reasoning("second"),
            {"type": "message", "role": "assistant", "content": [{"type": "output_text", "text": "ok"}]},
        ]});
        let mut expected = body.clone();
        expected["input"][1]["content"] = json!([]);
        expected["input"][4]["content"] = json!([]);

        assert_eq!(
            strip_rejected_reasoning_content_at(&memory, "r", &mut body, now),
            2
        );
        assert_eq!(body, expected, "消息、summary、encrypted_content 原样保留");
        assert_eq!(body["input"][1]["summary"][0]["text"], "s");
        assert_eq!(body["input"][1]["encrypted_content"], "placeholder");
        // 已经没有可清的：返回 0，调用方把错误照常交回。
        assert_eq!(
            strip_rejected_reasoning_content_at(&memory, "r", &mut body, now),
            0
        );
    }

    #[test]
    fn remembered_reasoning_content_rejection_is_scoped_by_upstream_and_model() {
        let memory = Mutex::new(Memory::default());
        let now = Instant::now();
        let mut body = plain_reasoning_body();
        strip_rejected_reasoning_content_at(&memory, "nebula#a", &mut body, now);

        // 同一上游同一模型：提前清空。
        let mut next = plain_reasoning_body();
        assert_eq!(
            strip_remembered_reasoning_content_at(&memory, "nebula#a", &mut next, now),
            1
        );
        assert!(!has_reasoning_content(&next));
        // 别的上游、或同一上游换成别的模型（例如回到 DeepSeek，它需要回传思考内容）：原样保留。
        let mut elsewhere = plain_reasoning_body();
        assert_eq!(
            strip_remembered_reasoning_content_at(&memory, "other", &mut elsewhere, now),
            0
        );
        let mut other_model = plain_reasoning_body();
        other_model["model"] = json!("deepseek-v4-pro");
        assert_eq!(
            strip_remembered_reasoning_content_at(&memory, "nebula#a", &mut other_model, now),
            0
        );
        assert_eq!(
            other_model["input"][2]["content"][0]["text"],
            "deepseek thinking"
        );
        // 一整天没用到就忘掉。
        let mut expired = plain_reasoning_body();
        assert_eq!(
            strip_remembered_reasoning_content_at(
                &memory,
                "nebula#a",
                &mut expired,
                now + MEMORY_TTL + MEMORY_TTL
            ),
            0
        );
    }
}
