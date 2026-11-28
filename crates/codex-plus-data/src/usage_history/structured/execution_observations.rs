use chrono::{DateTime, Local, NaiveDate};
use serde::Deserialize;
use serde_json::value::RawValue;
use std::borrow::Cow;

pub(super) const DETAIL_MS: i64 = 86_400_000;
pub(super) const LIST_MS: i64 = 7 * DETAIL_MS;

#[derive(Deserialize)]
pub(super) struct Item<'a> {
    #[serde(rename = "type")]
    pub(super) kind: &'a str,
    #[serde(borrow)]
    call_id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    id: Option<Cow<'a, str>>,
    #[serde(borrow)]
    name: Option<&'a RawValue>,
    #[serde(borrow)]
    namespace: Option<&'a RawValue>,
    #[serde(borrow)]
    arguments: Option<&'a RawValue>,
    #[serde(borrow)]
    output: Option<&'a RawValue>,
}

impl Item<'_> {
    pub(super) fn call_id(&self) -> Option<&str> {
        self.call_id.as_deref().or(self.id.as_deref())
    }
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
        let Some(call_id) = item.call_id().filter(|id| !skip && !id.is_empty()) else {
            return false;
        };
        let mut name = (!is_output).then(|| {
            item.name
                .and_then(|name| serde_json::from_str(name.get()).ok())
                .unwrap_or_else(|| {
                    if item.kind == "local_shell_call" {
                        "local_shell".into()
                    } else {
                        String::new()
                    }
                })
        });
        let mut namespace = (!is_output).then(|| {
            item.namespace
                .and_then(|ns| serde_json::from_str::<String>(ns.get()).ok())
                .unwrap_or_default()
        });
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
            item.arguments
                .and_then(|args| {
                    let raw = args.get();
                    if raw.starts_with('"') {
                        serde_json::from_str::<String>(raw)
                            .ok()
                            .and_then(|decoded| poll_session(&decoded))
                    } else {
                        poll_session(raw)
                    }
                })
                .map(|session| session.to_string())
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
    let text = super::oversized::raw_receipt(output?.get())?;
    let (running, id) = super::exec_receipt(&text)?;
    running.then_some(())?;
    id.parse::<u64>().ok().map(|id| id.to_string())
}

pub(super) fn poll_session(arguments: &str) -> Option<u64> {
    #[derive(Deserialize)]
    struct Poll {
        session_id: u64,
    }
    serde_json::from_str::<Poll>(arguments)
        .ok()
        .map(|poll| poll.session_id)
}

impl Observation {
    pub(super) fn heap_bytes(&self) -> usize {
        self.call_id.capacity()
            + self.name.as_ref().map_or(0, String::capacity)
            + self.namespace.as_ref().map_or(0, String::capacity)
            + self.process.as_ref().map_or(0, String::capacity)
    }
}
