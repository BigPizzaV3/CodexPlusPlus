//! 按索引位置读取仍在七天内的原始工具输入和输出。
use super::execution_index::{DETAIL_RETENTION_MS, ExecutionLocator, ExecutionLocators};
use super::persistent_index::{decode_source, encode_identity};
use anyhow::{Context, ensure};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

const MAX_DETAIL_BYTES: u64 = 4 * 1024 * 1024;
pub(super) const MAX_DECOMPRESSED_BYTES: u64 = 32 * 1024 * 1024;
pub(super) const MAX_RECOVERY_ROWS: usize = 2048;
pub(super) const MAX_RECOVERY_OUTPUTS: usize = 64;

pub(super) struct Detail {
    pub(super) input: Value,
    pub(super) output: Vec<Value>,
    pub(super) partial: bool,
    pub(super) duration_ms: Option<i64>,
}

pub(super) fn recent(timestamp_ms: i64, now_ms: i64) -> bool {
    timestamp_ms <= now_ms && timestamp_ms > now_ms.saturating_sub(DETAIL_RETENTION_MS)
}

enum SourceReader {
    Plain(File),
    Compressed {
        reader: Box<dyn BufRead>,
        offset: u64,
    },
}

impl SourceReader {
    fn read_range(&mut self, start: u64, end: u64) -> anyhow::Result<Vec<u8>> {
        let length = end.checked_sub(start).context("invalid detail range")?;
        ensure!(
            length > 0 && length <= super::MAX_LINE as u64,
            "detail line exceeds limit"
        );
        let mut bytes = vec![0; usize::try_from(length)?];
        match self {
            Self::Plain(file) => {
                file.seek(SeekFrom::Start(start))?;
                file.read_exact(&mut bytes)?;
            }
            Self::Compressed { reader, offset } => {
                ensure!(
                    start >= *offset,
                    "compressed detail positions are out of order"
                );
                ensure!(
                    end <= MAX_DECOMPRESSED_BYTES,
                    "compressed detail exceeds read budget"
                );
                let skipped =
                    io::copy(&mut reader.by_ref().take(start - *offset), &mut io::sink())?;
                ensure!(
                    skipped == start - *offset,
                    "compressed detail source is incomplete"
                );
                reader.read_exact(&mut bytes)?;
                *offset = end;
            }
        }
        Ok(bytes)
    }

    fn read_line(&mut self, start: u64, limit: u64) -> anyhow::Result<Vec<u8>> {
        let mut bytes = Vec::new();
        match self {
            Self::Plain(file) => {
                file.seek(SeekFrom::Start(start))?;
                BufReader::new(file)
                    .take(limit)
                    .read_until(b'\n', &mut bytes)?;
            }
            Self::Compressed { reader, offset } => {
                ensure!(
                    start >= *offset,
                    "compressed detail positions are out of order"
                );
                ensure!(
                    start < MAX_DECOMPRESSED_BYTES,
                    "compressed detail exceeds read budget"
                );
                let skipped =
                    io::copy(&mut reader.by_ref().take(start - *offset), &mut io::sink())?;
                ensure!(
                    skipped == start - *offset,
                    "compressed detail source is incomplete"
                );
                let limit = limit.min(MAX_DECOMPRESSED_BYTES - start);
                reader.by_ref().take(limit).read_until(b'\n', &mut bytes)?;
                *offset = start + bytes.len() as u64;
            }
        }
        Ok(bytes)
    }
}

fn payload(
    bytes: &[u8],
    id: &str,
    locator: &ExecutionLocator,
    output: bool,
) -> anyhow::Result<Value> {
    let row: Value = serde_json::from_slice(bytes).context("invalid detail record")?;
    let item = &row["payload"];
    let timestamp_ms = row["timestamp"]
        .as_str()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
        .map(|value| value.timestamp_millis());
    let kind = item["type"].as_str();
    ensure!(
        row["type"] == "response_item"
            && timestamp_ms == Some(locator.timestamp_ms)
            && item["call_id"].as_str().or_else(|| item["id"].as_str()) == Some(id)
            && if output {
                matches!(
                    kind,
                    Some(
                        "function_call_output"
                            | "custom_tool_call_output"
                            | "local_shell_call_output"
                    )
                )
            } else {
                matches!(
                    kind,
                    Some("function_call" | "custom_tool_call" | "local_shell_call")
                )
            },
        "detail record identity changed"
    );
    Ok(item.clone())
}

fn shell_call(item: &Value) -> bool {
    item["type"] == "local_shell_call"
        || (!item["name"].as_str().is_some_and(|name| name.starts_with("mcp__"))
            && !item["namespace"]
                .as_str()
                .is_some_and(|namespace| namespace.starts_with("mcp__"))
            && item["name"].as_str().is_some_and(|name| {
                matches!(name.rsplit('.').next(), Some("exec_command" | "shell"))
            }))
}

/// 旧索引只保留行首时，按需从同一会话有界恢复，不写回索引。
fn recover(
    reader: &mut SourceReader,
    id: &str,
    call: &ExecutionLocator,
) -> anyhow::Result<(Value, Vec<(i64, Value)>, bool, bool)> {
    ensure!(
        !matches!(reader, SourceReader::Compressed { .. }) || call.start < MAX_DECOMPRESSED_BYTES,
        "compressed detail exceeds read budget"
    );
    let mut position = call.start;
    let mut used = 0u64;
    let mut input = Value::Null;
    let mut output = Vec::new();
    let mut complete = false;
    let mut shell = false;
    for row_index in 0..MAX_RECOVERY_ROWS {
        if used >= MAX_DETAIL_BYTES
            || matches!(reader, SourceReader::Compressed { .. })
                && position >= MAX_DECOMPRESSED_BYTES
        {
            break;
        }
        let limit = (super::MAX_LINE as u64 + 1).min(MAX_DETAIL_BYTES - used);
        let bytes = reader.read_line(position, limit)?;
        if bytes.is_empty() {
            ensure!(row_index > 0, "detail call is missing");
            complete = row_index > 0 && !output.is_empty();
            break;
        }
        let length = bytes.len() as u64;
        if bytes.last() != Some(&b'\n') || length > super::MAX_LINE as u64 {
            ensure!(row_index > 0, "detail call exceeds read budget");
            break;
        }
        let start = position;
        position += length;
        used += length;
        if row_index == 0 {
            let locator = ExecutionLocator {
                source: call.source.clone(),
                identity: call.identity.clone(),
                start,
                end: position,
                timestamp_ms: call.timestamp_ms,
                call_id: id.to_owned(),
            };
            let item = payload(&bytes, id, &locator, false)?;
            shell = shell_call(&item);
            input = item
                .get("arguments")
                .or_else(|| item.get("input"))
                .or_else(|| item.get("action"))
                .cloned()
                .unwrap_or(Value::Null);
            if let Value::String(text) = input {
                input = serde_json::from_str(&text).unwrap_or(Value::String(text));
            }
            continue;
        }
        let row: Value = match serde_json::from_slice(&bytes) {
            Ok(row) => row,
            Err(_) => break,
        };
        let item = &row["payload"];
        if row["type"] != "response_item"
            || item["call_id"].as_str().or_else(|| item["id"].as_str()) != Some(id)
        {
            continue;
        }
        let kind = item["type"].as_str();
        if matches!(
            kind,
            Some("function_call" | "custom_tool_call" | "local_shell_call")
        ) {
            break;
        }
        if !matches!(
            kind,
            Some("function_call_output" | "custom_tool_call_output" | "local_shell_call_output")
        ) {
            continue;
        }
        if output.len() == MAX_RECOVERY_OUTPUTS {
            break;
        }
        let Some(timestamp_ms) = row["timestamp"]
            .as_str()
            .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.timestamp_millis())
        else {
            break;
        };
        let value = item.get("output").cloned().unwrap_or(Value::Null);
        let receipt = super::exec_receipt(&value);
        output.push((timestamp_ms, value));
        if shell && matches!(receipt, Some((false, _))) {
            complete = true;
            break;
        }
    }
    if shell
        && !output
            .last()
            .is_some_and(|(_, value)| matches!(super::exec_receipt(value), Some((false, _))))
    {
        complete = false;
    }
    Ok((input, output, !complete, shell))
}

/// 返回 None 表示调用已经超过详情保留期；部分输出不可读时保留已验证的输入。
pub(super) fn read(
    home: &Path,
    id: &str,
    locators: &ExecutionLocators,
    now_ms: impl Fn() -> i64,
) -> anyhow::Result<Option<Detail>> {
    let call = &locators.call;
    let read_start_ms = now_ms();
    if !recent(call.timestamp_ms, read_start_ms) {
        return Ok(None);
    }
    ensure!(call.call_id == id, "detail call identity changed");
    let path = decode_source(&call.source)?;
    ensure!(
        (path.starts_with(home.join("sessions"))
            || path.starts_with(home.join("archived_sessions")))
            && super::log_file(&path)
            && fs::canonicalize(&path)? == path,
        "detail source is outside the session directory"
    );
    let file = File::open(&path)?;
    ensure!(
        encode_identity(codex_plus_core::codex_sqlite::opened_file_identity(&file)?)
            == call.identity,
        "detail source identity changed"
    );
    let mut reader = if path.extension().is_some_and(|ext| ext == "zst") {
        SourceReader::Compressed {
            reader: Box::new(BufReader::new(zstd::stream::read::Decoder::new(file)?)),
            offset: 0,
        }
    } else {
        SourceReader::Plain(file)
    };
    let (input, mut output, mut partial, shell) = if call.end == 0 {
        recover(&mut reader, id, call)?
    } else {
        let call_bytes = reader.read_range(call.start, call.end)?;
        let item = payload(&call_bytes, id, call, false)?;
        let shell = shell_call(&item);
        let input = item
            .get("arguments")
            .or_else(|| item.get("input"))
            .or_else(|| item.get("action"))
            .cloned()
            .unwrap_or(Value::Null);
        let input = match input {
            Value::String(text) => serde_json::from_str(&text).unwrap_or(Value::String(text)),
            value => value,
        };
        let mut used = call.end - call.start;
        let mut last_end = call.end;
        let mut output = Vec::new();
        let mut partial = locators.outputs_truncated;
        for locator in &locators.outputs {
            if !recent(locator.timestamp_ms, read_start_ms) {
                partial = true;
                continue;
            }
            let Some(length) = locator.end.checked_sub(locator.start) else {
                partial = true;
                continue;
            };
            if locator.source != call.source
                || locator.identity != call.identity
                || locator.start < last_end
                || length == 0
                || length > super::MAX_LINE as u64
                || used.saturating_add(length) > MAX_DETAIL_BYTES
            {
                partial = true;
                continue;
            }
            let bytes = match reader.read_range(locator.start, locator.end) {
                Ok(bytes) => bytes,
                Err(_) => {
                    partial = true;
                    break;
                }
            };
            last_end = locator.end;
            used += length;
            match payload(&bytes, &locator.call_id, locator, true) {
                Ok(item) => output.push((
                    locator.timestamp_ms,
                    item.get("output").cloned().unwrap_or(Value::Null),
                )),
                Err(_) => partial = true,
            }
        }
        (input, output, partial, shell)
    };
    let current = File::open(&path)?;
    ensure!(
        fs::canonicalize(&path)? == path
            && encode_identity(codex_plus_core::codex_sqlite::opened_file_identity(
                &current
            )?) == call.identity,
        "detail source changed during read"
    );
    let read_end_ms = now_ms();
    if !recent(call.timestamp_ms, read_end_ms) {
        return Ok(None);
    }
    let output_count = output.len();
    output.retain(|(timestamp_ms, _)| recent(*timestamp_ms, read_end_ms));
    partial |= output.len() != output_count;
    let duration_ms = output.last().and_then(|(timestamp_ms, value)| {
        let receipt = super::exec_receipt(value);
        if partial
            || *timestamp_ms < call.timestamp_ms
            || shell && !matches!(receipt, Some((false, _)))
        {
            None
        } else {
            Some(*timestamp_ms - call.timestamp_ms)
        }
    });
    partial |= output.is_empty()
        || shell
            && !output
                .last()
                .is_some_and(|(_, value)| matches!(super::exec_receipt(value), Some((false, _))));
    Ok(Some(Detail {
        input,
        output: output.into_iter().map(|(_, value)| value).collect(),
        partial,
        duration_ms,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::persistent_index::encode_source;
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    const NOW: i64 = 1_793_275_200_000;

    fn fixture(compressed: bool) -> (tempfile::TempDir, PathBuf, ExecutionLocators) {
        let root = tempfile::tempdir().unwrap();
        let home = fs::canonicalize(root.path()).unwrap();
        let sessions = home.join("sessions");
        fs::create_dir(&sessions).unwrap();
        let path = sessions.join(if compressed {
            "rollout-detail.jsonl.zst"
        } else {
            "rollout-detail.jsonl"
        });
        let timestamp = chrono::DateTime::from_timestamp_millis(NOW - 1_000)
            .unwrap()
            .to_rfc3339();
        let rows = [
            json!({"type":"response_item","timestamp":timestamp,"payload":{
                "type":"function_call","call_id":"one","name":"functions.exec_command",
                "arguments":"{\"cmd\":\"Get-Content C:\\\\work\\\\file.txt\"}"}}),
            json!({"type":"response_item","timestamp":timestamp,"payload":{
                "type":"function_call_output","call_id":"one","output":"first"}}),
            json!({"type":"response_item","timestamp":timestamp,"payload":{
                "type":"function_call_output","call_id":"one","output":{"content":"second"}}}),
        ];
        let mut bytes = Vec::new();
        let mut ranges = Vec::new();
        for row in rows {
            let start = bytes.len() as u64;
            bytes.extend_from_slice(row.to_string().as_bytes());
            bytes.push(b'\n');
            ranges.push((start, bytes.len() as u64));
        }
        let stored = if compressed {
            zstd::stream::encode_all(bytes.as_slice(), 1).unwrap()
        } else {
            bytes
        };
        fs::write(&path, stored).unwrap();
        let path = fs::canonicalize(path).unwrap();
        let identity = encode_identity(
            codex_plus_core::codex_sqlite::opened_file_identity(&File::open(&path).unwrap())
                .unwrap(),
        );
        let locator = |(start, end)| ExecutionLocator {
            source: encode_source(&path),
            identity: identity.clone(),
            call_id: "one".into(),
            start,
            end,
            timestamp_ms: NOW - 1_000,
        };
        let locators = ExecutionLocators {
            call: locator(ranges[0]),
            outputs: vec![locator(ranges[1]), locator(ranges[2])],
            outputs_truncated: false,
        };
        (root, home, locators)
    }

    fn rewrite_rows(locators: &mut ExecutionLocators, rows: &[Value]) {
        let path = decode_source(&locators.call.source).unwrap();
        let mut bytes = Vec::new();
        for (index, row) in rows.iter().enumerate() {
            let start = bytes.len() as u64;
            bytes.extend_from_slice(row.to_string().as_bytes());
            bytes.push(b'\n');
            let locator = if index == 0 {
                &mut locators.call
            } else {
                &mut locators.outputs[index - 1]
            };
            locator.start = start;
            locator.end = bytes.len() as u64;
            locator.timestamp_ms = chrono::DateTime::parse_from_rfc3339(
                row["timestamp"].as_str().unwrap(),
            )
            .unwrap()
            .timestamp_millis();
        }
        fs::write(path, bytes).unwrap();
    }

    fn fixture_rows(locators: &ExecutionLocators) -> Vec<Value> {
        let path = decode_source(&locators.call.source).unwrap();
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }

    fn timestamp(milliseconds: i64) -> String {
        chrono::DateTime::from_timestamp_millis(milliseconds)
            .unwrap()
            .to_rfc3339()
    }

    #[test]
    fn reads_original_paths_and_multiple_output_parts() {
        for compressed in [false, true] {
            let (_root, home, locators) = fixture(compressed);
            let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
            assert_eq!(detail.input["cmd"], "Get-Content C:\\work\\file.txt");
            assert_eq!(
                detail.output,
                vec![json!("first"), json!({"content":"second"})]
            );
            assert!(detail.partial);
            assert!(detail.duration_ms.is_none());
        }
    }

    #[test]
    fn restores_old_positions_on_demand_without_writing_the_index() {
        for compressed in [false, true] {
            let (_root, home, mut locators) = fixture(compressed);
            locators.call.end = 0;
            locators.outputs.clear();
            let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
            assert_eq!(detail.input["cmd"], "Get-Content C:\\work\\file.txt");
            assert_eq!(
                detail.output,
                vec![json!("first"), json!({"content":"second"})]
            );
            assert!(detail.partial);
            assert!(detail.duration_ms.is_none());
        }
    }

    #[test]
    fn recovered_running_process_is_partial_and_never_assigned_duration() {
        let (_root, home, mut locators) = fixture(false);
        let path = decode_source(&locators.call.source).unwrap();
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("first", "Process running with session ID 77")
            .replace(
                "{\"content\":\"second\"}",
                "\"Process running with session ID 77\"",
            );
        fs::write(&path, text).unwrap();
        locators.call.end = 0;
        locators.outputs.clear();
        let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
        assert!(detail.partial);
        assert!(detail.duration_ms.is_none());
    }

    #[test]
    fn duration_requires_a_terminal_receipt_for_shell() {
        let (_root, home, mut locators) = fixture(false);
        let path = decode_source(&locators.call.source).unwrap();
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("{\"content\":\"second\"}", "\"Process exited with code 0\"");
        fs::write(&path, &text).unwrap();
        locators.outputs[1].end = text.len() as u64;
        let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
        assert!(!detail.partial);
        assert_eq!(detail.duration_ms, Some(0));
    }

    #[test]
    fn mcp_shell_name_is_not_treated_as_a_command_executor() {
        assert!(!shell_call(&json!({"name":"mcp__server__shell"})));
        assert!(!shell_call(&json!({"name":"shell","namespace":"mcp__server"})));
        assert!(!shell_call(&json!({"name":"functions.powershell"})));
        assert!(shell_call(&json!({"name":"functions.shell"})));
        assert!(shell_call(&json!({"type":"local_shell_call"})));

        for recovered in [false, true] {
            let (_root, home, mut locators) = fixture(false);
            let mut rows = fixture_rows(&locators);
            rows[0]["payload"]["name"] = json!("mcp__server__shell");
            rows[1]["payload"]["output"] = json!("Process running with session ID 77");
            rewrite_rows(&mut locators, &rows);
            if recovered {
                locators.call.end = 0;
                locators.outputs.clear();
            }
            let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
            assert!(!detail.partial);
            assert_eq!(detail.duration_ms, Some(0));
            assert_eq!(detail.output.len(), 2);
        }
    }

    #[test]
    fn local_shell_action_and_both_output_forms_are_read() {
        for recovered in [false, true] {
            for output_kind in ["function_call_output", "local_shell_call_output"] {
                let (_root, home, mut locators) = fixture(false);
                let mut rows = fixture_rows(&locators);
                rows[0]["payload"] = json!({
                    "type":"local_shell_call","id":"one","status":"completed",
                    "action":{
                        "type":"exec","command":["Get-Content","C:\\work\\file.txt"],
                        "working_directory":"C:\\work","timeout_ms":1000,"env":{}
                    }
                });
                rows[2]["payload"]["type"] = json!(output_kind);
                rows[2]["payload"]["output"] = json!("Process exited with code 0");
                rewrite_rows(&mut locators, &rows);
                if recovered {
                    locators.call.end = 0;
                    locators.outputs.clear();
                }
                let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
                assert_eq!(detail.input["command"][1], "C:\\work\\file.txt");
                assert_eq!(detail.input["working_directory"], "C:\\work");
                assert_eq!(detail.output.len(), 2);
                assert!(!detail.partial);
                assert_eq!(detail.duration_ms, Some(0));
            }
        }
    }

    #[test]
    fn duration_uses_the_last_confirmed_terminal_output() {
        for recovered in [false, true] {
            let (_root, home, mut locators) = fixture(false);
            let mut rows = fixture_rows(&locators);
            rows[0]["timestamp"] = json!(timestamp(NOW - 10_000));
            rows[1]["timestamp"] = json!(timestamp(NOW - 9_000));
            rows[1]["payload"]["output"] = json!("Process running with session ID 77");
            rows[2]["timestamp"] = json!(timestamp(NOW - 3_000));
            rows[2]["payload"]["output"] = json!("Process exited with code 0");
            rewrite_rows(&mut locators, &rows);
            if recovered {
                locators.call.end = 0;
                locators.outputs.clear();
            }
            let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
            assert!(!detail.partial);
            assert_eq!(detail.duration_ms, Some(7_000));
        }
    }

    #[test]
    fn output_expiring_during_read_never_produces_duration() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        for recovered in [false, true] {
            let (_root, home, mut locators) = fixture(false);
            let mut rows = fixture_rows(&locators);
            rows[0]["timestamp"] = json!(timestamp(NOW - DETAIL_RETENTION_MS + 1_000));
            rows[1]["timestamp"] = json!(timestamp(NOW - DETAIL_RETENTION_MS + 100));
            rows[2]["timestamp"] = json!(timestamp(NOW - DETAIL_RETENTION_MS + 2_000));
            rows[2]["payload"]["output"] = json!("Process exited with code 0");
            rewrite_rows(&mut locators, &rows);
            if recovered {
                locators.call.end = 0;
                locators.outputs.clear();
            }
            let checks = AtomicUsize::new(0);
            let detail = read(&home, "one", &locators, || {
                if checks.fetch_add(1, Ordering::Relaxed) == 0 {
                    NOW
                } else {
                    NOW + 500
                }
            })
            .unwrap()
            .unwrap();
            assert!(detail.partial);
            assert!(detail.duration_ms.is_none());
            assert_eq!(detail.output, vec![json!("Process exited with code 0")]);
        }
    }

    #[test]
    fn recovery_output_limit_is_reported_as_partial() {
        let (_root, home, mut locators) = fixture(false);
        let path = decode_source(&locators.call.source).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let first_newline = text.find('\n').unwrap() + 1;
        let output = &text[first_newline..text[first_newline..].find('\n').unwrap() + first_newline + 1];
        let mut contents = text[..first_newline].to_owned();
        for _ in 0..MAX_RECOVERY_OUTPUTS + 1 {
            contents.push_str(output);
        }
        fs::write(path, contents).unwrap();
        locators.call.end = 0;
        locators.outputs.clear();
        let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
        assert_eq!(detail.output.len(), MAX_RECOVERY_OUTPUTS);
        assert!(detail.partial);
        assert!(detail.duration_ms.is_none());
    }

    #[test]
    fn skips_future_output_even_when_a_locator_is_supplied() {
        let (_root, home, mut locators) = fixture(false);
        let path = decode_source(&locators.call.source).unwrap();
        let mut text = fs::read_to_string(&path).unwrap();
        let original = chrono::DateTime::from_timestamp_millis(NOW - 1_000)
            .unwrap()
            .to_rfc3339();
        let future = chrono::DateTime::from_timestamp_millis(NOW + 1_000)
            .unwrap()
            .to_rfc3339();
        let first = text.find(&original).unwrap();
        let second =
            first + original.len() + text[first + original.len()..].find(&original).unwrap();
        text.replace_range(second..second + original.len(), &future);
        fs::write(path, text).unwrap();
        locators.outputs[0].timestamp_ms = NOW + 1_000;

        let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
        assert_eq!(detail.output, vec![json!({"content":"second"})]);
        assert!(detail.partial);
    }

    #[test]
    fn expires_at_exactly_seven_days() {
        let (_root, home, mut locators) = fixture(false);
        locators.call.timestamp_ms = NOW - DETAIL_RETENTION_MS;
        assert!(read(&home, "one", &locators, || NOW).unwrap().is_none());
    }

    #[test]
    fn expires_if_read_crosses_the_seven_day_boundary() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let (_root, home, locators) = fixture(false);
        let checks = AtomicUsize::new(0);
        let detail = read(&home, "one", &locators, || {
            if checks.fetch_add(1, Ordering::Relaxed) == 0 {
                NOW
            } else {
                NOW + DETAIL_RETENTION_MS - 1_000
            }
        })
        .unwrap();
        assert!(detail.is_none());
        assert_eq!(checks.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn refuses_wrong_call_or_replaced_source() {
        let (_root, home, mut locators) = fixture(false);
        assert!(read(&home, "other", &locators, || NOW).is_err());
        locators.call.identity = vec![0; 16];
        assert!(read(&home, "one", &locators, || NOW).is_err());
    }

    #[test]
    fn refuses_source_outside_the_session_directories() {
        let (_root, home, mut locators) = fixture(false);
        let path = home.join("rollout-outside.jsonl");
        fs::write(&path, b"{}\n").unwrap();
        locators.call.source = encode_source(&fs::canonicalize(path).unwrap());
        assert!(read(&home, "one", &locators, || NOW).is_err());
    }

    #[test]
    fn bad_output_and_oversized_compressed_skip_are_partial() {
        let (_root, home, mut locators) = fixture(true);
        locators.outputs[0].timestamp_ms -= 1;
        locators.outputs[1].start = MAX_DECOMPRESSED_BYTES;
        locators.outputs[1].end = MAX_DECOMPRESSED_BYTES + 1;
        let detail = read(&home, "one", &locators, || NOW).unwrap().unwrap();
        assert!(detail.partial);
        assert!(detail.output.is_empty());
    }

    #[test]
    fn missing_source_or_unreadable_recovery_call_is_an_error() {
        let (_root, home, mut locators) = fixture(false);
        locators.call.end = 0;
        locators.call.start = 10_000;
        assert!(read(&home, "one", &locators, || NOW).is_err());
        let path = decode_source(&locators.call.source).unwrap();
        fs::remove_file(path).unwrap();
        assert!(read(&home, "one", &locators, || NOW).is_err());

        let (_root, home, mut locators) = fixture(true);
        locators.call.start = MAX_DECOMPRESSED_BYTES;
        locators.call.end = 0;
        assert!(read(&home, "one", &locators, || NOW).is_err());
    }
}
