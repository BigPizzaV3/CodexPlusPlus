use chrono::{DateTime, Local, NaiveDate};
use serde::Deserialize;
use serde_json::value::RawValue;

pub(super) const DETAIL_MS: i64 = 86_400_000;

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
        self.rows.push(Observation {
            call_id: call_id.to_owned(),
            timestamp_ms,
            position,
            end: recent.then_some(end),
            is_output,
            name: (!is_output).then(|| {
                bounded(item.name).unwrap_or_else(|| {
                    if item.kind == "local_shell_call" {
                        "local_shell".into()
                    } else {
                        String::new()
                    }
                })
            }),
            namespace: (!is_output).then(|| bounded(item.namespace).unwrap_or_default()),
        });
        true
    }
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
    }
}
