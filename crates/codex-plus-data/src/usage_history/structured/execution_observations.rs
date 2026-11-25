use chrono::{DateTime, Local, NaiveDate};
use serde::Deserialize;
use serde_json::value::RawValue;

pub(super) const DETAIL_MS: i64 = 86_400_000;
pub(super) const LIST_MS: i64 = 7 * DETAIL_MS;

#[derive(Deserialize)]
pub(super) struct Item<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'a str,
    pub(super) call_id: Option<&'a str>,
    id: Option<&'a str>,
    #[serde(borrow)]
    name: Option<&'a RawValue>,
    #[serde(borrow)]
    namespace: Option<&'a RawValue>,
    #[serde(borrow)]
    arguments: Option<&'a RawValue>,
    #[serde(borrow)]
    output: Option<&'a RawValue>,
}

#[derive(Default)]
pub(super) struct Batch {
    pub(super) now_ms: i64,
    pub(super) rows: Vec<Observation>,
}

pub(super) struct Observation {
    pub(super) call_id: String,
    pub(super) timestamp_ms: i64,
    pub(super) position: u64,
    pub(super) end: Option<u64>,
    pub(super) is_output: bool,
    pub(super) name: Option<String>,
    pub(super) namespace: Option<String>,
    pub(super) process: Option<String>,
}

impl Batch {
    pub(super) fn capture(
        &mut self,
        item: &Item<'_>,
        timestamp: Option<&str>,
        position: u64,
        end: u64,
        cutoff: NaiveDate,
        skip: bool,
    ) -> bool {
        let is_output = match item.kind {
            "function_call" | "custom_tool_call" | "local_shell_call" => false,
            "function_call_output" | "custom_tool_call_output" | "local_shell_call_output" => true,
            _ => return true,
        };
        let Some(at) = timestamp.and_then(|at| DateTime::parse_from_rfc3339(at).ok()) else {
            return false;
        };
        if at.with_timezone(&Local).date_naive() < cutoff {
            return true;
        }
        let timestamp_ms = at.timestamp_millis();
        let recent = timestamp_ms <= self.now_ms && timestamp_ms > self.now_ms - DETAIL_MS;
        if is_output && !recent {
            return true;
        }
        let Some(call_id) = item
            .call_id
            .or(item.id)
            .filter(|id| !skip && !id.is_empty() && id.len() <= 300)
        else {
            return false;
        };
        let mut name = (!is_output).then(|| {
            bounded(item.name).unwrap_or_else(|| {
                if item.kind == "local_shell_call" {
                    "local_shell".into()
                } else {
                    String::new()
                }
            })
        });
        let mut namespace = (!is_output).then(|| bounded(item.namespace).unwrap_or_default());
        let poll = name
            .as_deref()
            .is_some_and(|name| name.rsplit('.').next() == Some("write_stdin"));
        if !is_output
            && timestamp_ms <= self.now_ms - LIST_MS
            && !poll
            && !name.as_deref().unwrap().starts_with("mcp__")
            && !namespace.as_deref().unwrap().starts_with("mcp__")
        {
            name = None;
            namespace = None;
        }
        let process = if !recent {
            None
        } else if is_output {
            running_process(item.output)
        } else if poll {
            #[derive(Deserialize)]
            struct Poll {
                session_id: u64,
            }
            item.arguments
                .filter(|args| args.get().len() <= 4096)
                .and_then(|args| {
                    let raw = args.get();
                    if raw.starts_with('"') {
                        serde_json::from_str::<String>(raw)
                            .ok()
                            .and_then(|decoded| serde_json::from_str::<Poll>(&decoded).ok())
                    } else {
                        serde_json::from_str::<Poll>(raw).ok()
                    }
                })
                .map(|poll| poll.session_id.to_string())
        } else {
            None
        };
        self.rows.push(Observation {
            call_id: call_id.to_owned(),
            timestamp_ms,
            position,
            end: recent.then_some(end),
            is_output,
            name,
            namespace,
            process,
        });
        true
    }
}

fn running_process(output: Option<&RawValue>) -> Option<String> {
    let raw = output?.get();
    if !raw.starts_with('"') {
        return None;
    }
    // 只解码执行器回执头，不复制整段输出。截断时保留完整的 JSON 转义行。
    let text: String = if raw.len() <= 4096 {
        serde_json::from_str(raw).ok()?
    } else {
        let bytes = &raw.as_bytes()[..4096];
        let mut end = None;
        let mut offset = 1;
        while offset + 1 < bytes.len() {
            if bytes[offset] != b'\\' {
                offset += 1;
                continue;
            }
            offset += match bytes[offset + 1] {
                b'u' if offset + 6 <= bytes.len() => {
                    if bytes[offset + 2..offset + 6].eq_ignore_ascii_case(b"000a")
                        || bytes[offset + 2..offset + 6].eq_ignore_ascii_case(b"000d")
                    {
                        end = Some(offset + 6);
                    }
                    6
                }
                b'n' | b'r' => {
                    end = Some(offset + 2);
                    2
                }
                _ => 2,
            };
        }
        serde_json::from_str(&format!("{}\"", raw.get(..end?)?)).ok()?
    };
    let (running, id) = super::exec_receipt(&serde_json::Value::String(text))?;
    if !running {
        return None;
    }
    id.parse::<u64>().ok().map(|id| id.to_string())
}

fn bounded(raw: Option<&RawValue>) -> Option<String> {
    let raw = raw?.get();
    if raw.len() > 1802 {
        return None;
    }
    let mut text: String = serde_json::from_str(raw).ok()?;
    text.truncate(text.floor_char_boundary(300));
    text.shrink_to_fit();
    Some(text)
}

impl Observation {
    pub(super) fn heap_bytes(&self) -> usize {
        self.call_id.capacity()
            + self.name.as_ref().map_or(0, String::capacity)
            + self.namespace.as_ref().map_or(0, String::capacity)
            + self.process.as_ref().map_or(0, String::capacity)
    }
}
