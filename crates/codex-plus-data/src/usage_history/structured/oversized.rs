//! 从超长原生日志提取计量字段，正文中的同名字符串不参与分类。
use serde::de::{DeserializeSeed, IgnoredAny, MapAccess, SeqAccess, Visitor};
use serde_json::Value;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

pub(super) struct Spool {
    decompressed: Option<File>,
    pub(super) bytes: u64,
    pub(super) complete: bool,
}

pub(super) enum Projected {
    Line(Vec<u8>),
    Skipped {
        affects_usage: bool,
        ordinal: Option<u64>,
    },
}

impl Spool {
    pub(super) fn new(prefix: &[u8], compressed: bool) -> io::Result<Self> {
        let mut spool = Self {
            decompressed: compressed.then(tempfile::tempfile).transpose()?,
            bytes: 0,
            complete: false,
        };
        spool.extend(prefix)?;
        Ok(spool)
    }

    pub(super) fn extend(&mut self, bytes: &[u8]) -> io::Result<()> {
        if let Some(file) = &mut self.decompressed {
            file.write_all(bytes)?;
        }
        self.bytes += bytes.len() as u64;
        self.complete = bytes.last() == Some(&b'\n');
        Ok(())
    }

    pub(super) fn finish(
        &mut self,
        path: &Path,
        end: u64,
        identity: Option<(u64, u64)>,
        session: &str,
        turn: &str,
    ) -> io::Result<Projected> {
        let mut original;
        let file = if let Some(file) = &mut self.decompressed {
            file.rewind()?;
            file
        } else {
            original = File::open(path)?;
            if Some(codex_plus_core::codex_sqlite::opened_file_identity(
                &original,
            )?) != identity
                || std::fs::canonicalize(path)? != path
                || original.metadata()?.len() < end
            {
                return Err(io::Error::other(
                    "rollout changed while reading a large record",
                ));
            }
            original.seek(SeekFrom::Start(end - self.bytes))?;
            &mut original
        };
        let mut json = serde_json::Deserializer::from_reader(BufReader::new(file.take(self.bytes)));
        let mut record = match Columns::Envelope(session, turn)
            .deserialize(&mut json)
            .and_then(|value| {
                json.end()?;
                Ok(value)
            }) {
            Ok(record) => record,
            Err(error) if error.is_io() => return Err(error.into()),
            // 交给普通行的语法错误处理，保留同样的诊断和序号行为。
            Err(_) => return Ok(Projected::Line(b"!\n".to_vec())),
        };
        let terminal = record["type"] == "token_usage_record";
        let event = record["type"] == "event_msg";
        let bad_payload = terminal && record["payload"]["duplicate_terminal"] == true
            || event && record["payload"]["duplicate_event_type"] == true;
        if bad_payload || terminal || event && record["payload"]["type"] == "token_count" {
            if bad_payload {
                record["payload"] = Value::Bool(false);
            }
            let mut line = serde_json::to_vec(&record)?;
            line.push(b'\n');
            Ok(Projected::Line(line))
        } else {
            let affects_usage = match record["type"].as_str() {
                Some("event_msg") => record["payload"]["type"]
                    .as_str()
                    .is_none_or(relevant_event),
                Some("session_meta" | "turn_context") | None => true,
                Some(_) => false,
            };
            let ordinal = record["ordinal"].as_u64();
            Ok(Projected::Skipped {
                affects_usage,
                ordinal,
            })
        }
    }
}

#[derive(Clone, Copy)]
enum Columns<'a> {
    Envelope(&'a str, &'a str),
    Payload(&'a str, &'a str),
    Info,
    Counts,
    Kind,
    Timestamp,
    Id(&'a str),
    Response,
    Number,
}

impl<'de> DeserializeSeed<'de> for Columns<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Columns<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("a JSON object")
    }
    fn visit_unit<E>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_bool<E>(self, _: bool) -> Result<Value, E> {
        Ok(Value::Bool(false))
    }
    fn visit_i64<E>(self, n: i64) -> Result<Value, E> {
        Ok(if matches!(self, Self::Number) {
            Value::from(n)
        } else {
            Value::Bool(false)
        })
    }
    fn visit_u64<E>(self, n: u64) -> Result<Value, E> {
        Ok(if matches!(self, Self::Number) {
            Value::from(n)
        } else {
            Value::Bool(false)
        })
    }
    fn visit_f64<E>(self, _: f64) -> Result<Value, E> {
        Ok(Value::Bool(false))
    }
    fn visit_str<E>(self, s: &str) -> Result<Value, E> {
        Ok(match self {
            Self::Kind if s.starts_with("thread_settings") => Value::from("thread_settings"),
            Self::Kind
                if matches!(
                    s,
                    "event_msg" | "token_usage_record" | "session_meta" | "turn_context"
                ) || relevant_event(s) =>
            {
                Value::from(s)
            }
            Self::Kind => Value::from(""),
            Self::Timestamp => match chrono::DateTime::parse_from_rfc3339(s) {
                Ok(time) => Value::from(time.to_rfc3339()),
                Err(_) => Value::from(""),
            },
            Self::Id(expected) if s == expected => Value::from(s),
            Self::Response if super::terminal_receipts::valid_response_id(s) => Value::from(s),
            _ => Value::Bool(false),
        })
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        while seq.next_element::<IgnoredAny>()?.is_some() {}
        Ok(Value::Bool(false))
    }
    fn visit_map<M: MapAccess<'de>>(self, mut map: M) -> Result<Value, M::Error> {
        if !matches!(
            self,
            Self::Envelope(_, _) | Self::Payload(_, _) | Self::Info | Self::Counts
        ) {
            while map.next_entry::<IgnoredAny, IgnoredAny>()?.is_some() {}
            return Ok(Value::Bool(false));
        }
        let mut fields = serde_json::Map::new();
        while let Some(key) = map.next_key::<String>()? {
            let field = (self, key.as_str());
            let value = match field {
                (Self::Envelope(session, turn), "payload") => {
                    map.next_value_seed(Self::Payload(session, turn))?
                }
                (Self::Payload(_, _), "info") => map.next_value_seed(Self::Info)?,
                (Self::Payload(_, _), "usage" | "thread_token_usage")
                | (Self::Info, "total_token_usage" | "last_token_usage") => {
                    map.next_value_seed(Self::Counts)?
                }
                (Self::Envelope(_, _) | Self::Payload(_, _), "type") => {
                    map.next_value_seed(Self::Kind)?
                }
                (Self::Envelope(_, _), "timestamp") => map.next_value_seed(Self::Timestamp)?,
                (Self::Payload(session, _), "thread_id") => {
                    map.next_value_seed(Self::Id(session))?
                }
                (Self::Payload(_, turn), "turn_id") => map.next_value_seed(Self::Id(turn))?,
                (Self::Payload(_, _), "response_id") => map.next_value_seed(Self::Response)?,
                (Self::Envelope(_, _), "ordinal")
                | (
                    Self::Counts,
                    "input_tokens"
                    | "output_tokens"
                    | "cached_input_tokens"
                    | "cache_write_input_tokens"
                    | "reasoning_output_tokens"
                    | "total_tokens",
                ) => map.next_value_seed(Self::Number)?,
                _ => {
                    map.next_value::<IgnoredAny>()?;
                    continue;
                }
            };
            if fields.contains_key(&key) {
                match field {
                    (Self::Envelope(_, _), _) => {
                        return Err(serde::de::Error::custom("duplicate envelope field"));
                    }
                    (
                        Self::Payload(_, _),
                        "thread_id" | "turn_id" | "response_id" | "usage" | "thread_token_usage",
                    ) => {
                        fields.insert("duplicate_terminal".into(), Value::Bool(true));
                    }
                    (Self::Payload(_, _), "type") => {
                        fields.insert("duplicate_event_type".into(), Value::Bool(true));
                    }
                    _ => (),
                }
            }
            fields.insert(key, value);
        }
        Ok(Value::Object(fields))
    }
}

fn relevant_event(kind: &str) -> bool {
    kind.starts_with("thread_settings")
        || matches!(
            kind,
            "token_count"
                | "task_started"
                | "turn_started"
                | "task_complete"
                | "turn_completed"
                | "task_failed"
                | "turn_aborted"
                | "task_aborted"
                | "error"
                | "stream_error"
        )
}

struct UsagePrefix<'a> {
    payload: bool,
    result: &'a std::cell::Cell<Option<bool>>,
    may_meter: &'a std::cell::Cell<bool>,
    ordinal: &'a std::cell::Cell<Option<u64>>,
}
impl<'de> serde::de::DeserializeSeed<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn deserialize<D: serde::Deserializer<'de>>(self, deserializer: D) -> Result<bool, D::Error> {
        deserializer.deserialize_map(self)
    }
}
impl<'de> serde::de::Visitor<'de> for UsagePrefix<'_> {
    type Value = bool;
    fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("rollout event")
    }
    fn visit_map<M: serde::de::MapAccess<'de>>(self, mut map: M) -> Result<bool, M::Error> {
        let mut event_msg = false;
        while let Some(key) = map.next_key::<String>()? {
            if key == "type" {
                let kind = map.next_value::<String>()?;
                if self.payload {
                    let relevant = relevant_event(&kind);
                    self.may_meter.set(kind == "token_count");
                    self.result.set(Some(relevant));
                    return Ok(relevant);
                }
                match kind.as_str() {
                    "session_meta" | "turn_context" | "token_usage_record" => {
                        self.may_meter.set(kind == "token_usage_record");
                        self.result.set(Some(true));
                        return Ok(true);
                    }
                    "event_msg" => event_msg = true,
                    _ => {
                        self.may_meter.set(false);
                        self.result.set(Some(false));
                        if self.ordinal.get().is_some() {
                            return Ok(false);
                        }
                    }
                }
            } else if key == "payload" && event_msg {
                return map.next_value_seed(UsagePrefix {
                    payload: true,
                    result: self.result,
                    may_meter: self.may_meter,
                    ordinal: self.ordinal,
                });
            } else if !self.payload && key == "ordinal" {
                self.ordinal.set(
                    map.next_value::<Option<i64>>()?
                        .and_then(|n| u64::try_from(n).ok()),
                );
                if self.result.get() == Some(false) {
                    return Ok(false);
                }
            } else if key == "payload" && self.result.get() == Some(false) {
                return Ok(false);
            } else {
                map.next_value::<serde::de::IgnoredAny>()?;
            }
        }
        Ok(true)
    }
}
pub(super) fn inspect(prefix: &[u8]) -> (bool, Option<u64>, bool) {
    let result = std::cell::Cell::new(None);
    let ordinal = std::cell::Cell::new(None);
    let may_meter = std::cell::Cell::new(true);
    // 前缀可能没有闭合括号；已确认的类型独立保存，不依赖完整 JSON 解析成功。
    let _ = serde::Deserializer::deserialize_map(
        &mut serde_json::Deserializer::from_slice(prefix),
        UsagePrefix {
            payload: false,
            result: &result,
            ordinal: &ordinal,
            may_meter: &may_meter,
        },
    );
    (result.get().unwrap_or(true), ordinal.get(), may_meter.get())
}

#[cfg(test)]
mod tests {
    use super::super::{FileState, index_budget::Budget};
    use chrono::NaiveDate;
    use serde_json::json;

    #[test]
    fn malformed_and_duplicate_fields_match_short_record_diagnostics() -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let path = root.join("rollout.jsonl");
        let session = "00000000-0000-4000-8000-000000000001";
        let at = "2026-01-02T12:00:00Z";
        let counts = r#"{"input_tokens":10,"cached_input_tokens":0,"output_tokens":2}"#;
        let counter = format!(
            r#"{{"type":"event_msg","timestamp":"{at}","ordinal":2,"extra":"PAD","payload":{{"type":"token_count","info":{{"total_token_usage":{counts},"last_token_usage":null}}}}}}"#
        );
        let receipt = format!(
            r#"{{"type":"token_usage_record","timestamp":"{at}","ordinal":2,"extra":"PAD","payload":{{"thread_id":"{session}","turn_id":"a","response_id":"ID","usage":{counts},"thread_token_usage":{counts}}}}}"#
        );
        let prefix = format!(
            "{}\n{}\n",
            json!({"type":"session_meta","ordinal":0,"timestamp":at,"payload":{"id":session}}),
            json!({"type":"turn_context","ordinal":1,"timestamp":at,"payload":{"turn_id":"a"}})
        );
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let big = "x".repeat(1100 << 10);
        for row in [
            counter.replace(
                r#""input_tokens":10"#,
                r#""input_tokens":1,"input_tokens":10"#,
            ),
            counter.replace(r#""input_tokens":10"#, r#""input_tokens":[1,2]"#),
            counter.replace(
                r#""type":"token_count""#,
                r#""type":"token_count","type":"token_count""#,
            ),
            counter.replace(r#""ordinal":2"#, r#""ordinal":0,"ordinal":2"#),
            receipt.replace(r#""turn_id":"a""#, r#""turn_id":"a","turn_id":"a""#),
            receipt,
        ] {
            let parse = |padding: &str, id: &str| -> anyhow::Result<_> {
                std::fs::write(
                    &path,
                    format!(
                        "{prefix}{}\n",
                        row.replace("PAD", padding).replace("ID", id)
                    ),
                )?;
                let mut state = FileState::default();
                assert!(state.read_with_budget(
                    &path,
                    cutoff,
                    &mut Budget::new(std::iter::empty())
                )?);
                Ok((
                    state.issues,
                    state.invalid_counts,
                    state.malformed_token_counts,
                    state.rollout_out_of_order,
                    state.rollout_extent.map(|(_, ordinal, _)| ordinal),
                    state
                        .events
                        .into_iter()
                        .map(|e| (e.id, e.total, e.delta))
                        .collect::<Vec<_>>(),
                ))
            };
            assert_eq!(parse("", &"x".repeat(257))?, parse(&big, &big)?);
        }
        Ok(())
    }

    #[test]
    fn metered_fields_after_large_values_survive_plain_and_compressed_rounds() -> anyhow::Result<()>
    {
        let dir = tempfile::tempdir()?;
        let root = dir.path().canonicalize()?;
        let session = "00000000-0000-4000-8000-000000000001";
        let at = "2026-01-02T12:00:00Z";
        let big = "x".repeat(5 << 20);
        let usage = json!({"input_tokens":10,"cached_input_tokens":0,"output_tokens":2});
        let rows = [
            json!({"type":"session_meta","ordinal":0,"timestamp":at,"payload":{"id":session}}),
            json!({"type":"turn_context","ordinal":1,"timestamp":at,"payload":{"turn_id":"a"}}),
            json!({"large":big,"payload":{"extra":big,"response_id":"resp","thread_id":session,
                "turn_id":"a","usage":usage,"thread_token_usage":usage},"type":"token_usage_record","timestamp":at,"ordinal":2}),
            json!({"type":"event_msg","payload":{"large":big,"type":"token_count","info":{
                "last_token_usage":usage,"total_token_usage":usage}},"timestamp":"2026-01-02T12:00:01Z","ordinal":3}),
        ];
        let lines: Vec<_> = rows.iter().map(|value| format!("{value}\n")).collect();
        let first = (lines[0].len() + lines[1].len()) as u64;
        let second = first + lines[2].len() as u64;
        let end = second + lines[3].len() as u64;
        let text = lines.concat();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        for compressed in [false, true] {
            let path = root.join(if compressed {
                "rollout.jsonl.zst"
            } else {
                "rollout.jsonl"
            });
            if compressed {
                std::fs::write(
                    &path,
                    zstd::stream::encode_all(text.trim_end().as_bytes(), 1)?,
                )?;
            } else {
                std::fs::write(&path, text.as_bytes())?;
            }
            let mut state = FileState::default();
            for round in 0..8 {
                let mut budget = Budget::new(std::iter::empty());
                if state.read_with_budget(&path, cutoff, &mut budget)? {
                    assert!(round >= 3);
                    break;
                }
            }
            assert_eq!(state.events.len(), 2);
            assert_eq!(state.events[0].terminal_response.as_deref(), Some("resp"));
            assert!(state.events[1].terminal_response.is_none());
            assert_eq!(state.events[1].total, state.events[0].total);
            assert_eq!(state.events[0].rollout_position, Some((2, first, second)));
            assert_eq!(
                state.events[1].rollout_position,
                Some((3, second, end - u64::from(compressed)))
            );
            assert_eq!(state.issues, 0);
        }
        let path = root.join("rollout-malformed.jsonl");
        std::fs::write(
            &path,
            format!(
                "{}{}{}?\n{}",
                lines[0],
                lines[1],
                lines[2].trim_end(),
                lines[3]
            ),
        )?;
        let mut state = FileState::default();
        for _ in 0..8 {
            if state.read_with_budget(&path, cutoff, &mut Budget::new(std::iter::empty()))? {
                break;
            }
        }
        assert_eq!(state.issues, 1);
        assert_eq!(state.events.len(), 1);
        assert!(state.events[0].terminal_response.is_none());
        Ok(())
    }
}
