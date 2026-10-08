//! Responses SSE adapter for the flat-function compatibility retry.
//!
//! The upstream function form is deliberately kept private until its complete
//! JSON arguments have passed `decode_exec_input`.  In particular, argument
//! deltas are never forwarded as executable custom-tool input.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use crate::responses_exec::{adapt_response, decode_exec_input};

const MAX_CALL_ARGUMENTS: usize = 1024 * 1024;
const MAX_PENDING_ARGUMENTS: usize = 4 * 1024 * 1024;
const MAX_CALLS: usize = 64;
const MAX_ITEM_IDENTITIES: usize = 1024;

#[derive(Debug, Clone)]
struct ExecCall {
    function_item_id: String,
    custom_item_id: String,
    output_index: Option<Value>,
    item: Value,
    arguments: String,
    input: Option<String>,
    completion_emitted: bool,
    done_emitted: bool,
    added: bool,
}

impl ExecCall {
    fn custom_item(&self, input: &str) -> Value {
        custom_exec_item(&self.item, &self.custom_item_id, input)
    }
}

/// Converts a Responses SSE stream containing `function` `exec` calls back to
/// Codex's custom `exec` stream.
///
/// The adapter is request-scoped.  It does not execute programs or inspect
/// unrelated tools.
pub(crate) struct ExecSseAdapter {
    frame_buffer: Vec<u8>,
    calls: HashMap<String, ExecCall>,
    item_to_call: HashMap<String, String>,
    non_exec_items: HashSet<String>,
    non_exec_calls: HashSet<String>,
    non_exec_item_calls: HashMap<String, String>,
    seen_custom_ids: HashSet<String>,
    pending_argument_bytes: usize,
    retained_metadata_bytes: usize,
    next_sequence: u64,
    terminal_success: bool,
    done_marker_seen: bool,
    failed: bool,
    failure_emitted: bool,
    finished: bool,
    fatal: Option<String>,
}

impl Default for ExecSseAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExecSseAdapter {
    pub(crate) fn new() -> Self {
        Self {
            frame_buffer: Vec::new(),
            calls: HashMap::new(),
            item_to_call: HashMap::new(),
            non_exec_items: HashSet::new(),
            non_exec_calls: HashSet::new(),
            non_exec_item_calls: HashMap::new(),
            seen_custom_ids: HashSet::new(),
            pending_argument_bytes: 0,
            retained_metadata_bytes: 0,
            next_sequence: 0,
            terminal_success: false,
            done_marker_seen: false,
            failed: false,
            failure_emitted: false,
            finished: false,
            fatal: None,
        }
    }

    /// Appends an arbitrary network chunk and returns complete transformed SSE
    /// frames.  Partial UTF-8 and partial SSE frames remain buffered.
    pub(crate) fn push_bytes(&mut self, bytes: &[u8]) -> Result<Vec<u8>> {
        if self.finished || self.failed || self.fatal.is_some() || self.terminal_success {
            return Ok(Vec::new());
        }
        if self
            .frame_buffer
            .len()
            .saturating_add(bytes.len())
            .saturating_add(self.pending_argument_bytes)
            .saturating_add(self.retained_metadata_bytes)
            > MAX_PENDING_ARGUMENTS
        {
            return Err(self.reject("Responses SSE pending frame exceeds 4 MiB"));
        }
        self.frame_buffer.extend_from_slice(bytes);
        let mut output = Vec::new();
        while let Some(block) = take_frame(&mut self.frame_buffer) {
            if block.iter().all(|byte| byte.is_ascii_whitespace()) {
                continue;
            }
            let block = std::str::from_utf8(&block)
                .map_err(|_| self.reject("invalid UTF-8 in Responses SSE frame"))?;
            self.handle_frame(block, &mut output)?;
            if self.failed || self.terminal_success {
                self.frame_buffer.clear();
                break;
            }
        }
        Ok(output)
    }

    /// Completes the stream.  A successful `response.completed` is mandatory;
    /// a truncated or otherwise incomplete stream is converted to one failed
    /// terminal event.
    pub(crate) fn finish(&mut self) -> Result<Vec<u8>> {
        if self.finished {
            return Ok(Vec::new());
        }
        self.finished = true;
        if self.failed {
            return Ok(Vec::new());
        }
        if !self.frame_buffer.is_empty() {
            let message = if std::str::from_utf8(&self.frame_buffer).is_err() {
                "invalid UTF-8 in truncated Responses SSE frame"
            } else {
                "truncated Responses SSE frame"
            };
            return Ok(self.fail_with_message(message));
        }
        if let Some(message) = self.fatal.take() {
            return Ok(self.fail_with_message(&message));
        }
        if !self.terminal_success {
            return Ok(self.fail_with_message(
                "Responses stream ended without a successful response.completed",
            ));
        }
        if self.calls.values().any(|call| call.input.is_none()) {
            return Ok(
                self.fail_with_message("Responses stream ended with incomplete exec arguments")
            );
        }
        if self.done_marker_seen {
            return Ok(Vec::new());
        }
        self.done_marker_seen = true;
        Ok(b"data: [DONE]\n\n".to_vec())
    }

    /// Emits one sanitized failure terminal event.  Calling this repeatedly is
    /// idempotent and never duplicates the client-visible terminal event.
    pub(crate) fn fail(&mut self) -> Vec<u8> {
        self.fail_with_message("Responses exec protocol adaptation failed")
    }

    fn fail_with_message(&mut self, message: &str) -> Vec<u8> {
        if self.failure_emitted || self.terminal_success {
            return Vec::new();
        }
        self.failed = true;
        self.failure_emitted = true;
        self.fatal = None;

        let mut output = Vec::new();
        let event = json!({
            "type": "response.failed",
            "response": {
                "object": "response",
                "status": "failed",
                "output": [],
                "error": {
                    "code": "responses_exec_adaptation_failed",
                    "message": message
                }
            }
        });
        self.emit_value(&mut output, "response.failed", event);
        output.extend_from_slice(b"data: [DONE]\n\n");
        output
    }

    pub(crate) fn is_complete(&self) -> bool {
        self.terminal_success || self.failed || self.fatal.is_some()
    }

    pub(crate) fn succeeded(&self) -> bool {
        self.terminal_success
            && !self.failed
            && self.fatal.is_none()
            && self.frame_buffer.is_empty()
            && self.calls.values().all(|call| call.input.is_some())
    }

    fn reject(&mut self, message: &str) -> anyhow::Error {
        self.fatal = Some(message.to_string());
        anyhow!(message.to_string())
    }

    fn handle_frame(&mut self, block: &str, output: &mut Vec<u8>) -> Result<()> {
        let mut event_name = None;
        let mut data = Vec::new();
        for line in block.lines() {
            if let Some(value) = sse_field(line, "event") {
                event_name = Some(value.trim().to_string());
            } else if let Some(value) = sse_field(line, "data") {
                data.push(value.to_string());
            }
        }
        if data.is_empty() {
            output.extend_from_slice(block.as_bytes());
            output.extend_from_slice(b"\n\n");
            return Ok(());
        }
        let payload = data.join("\n");
        if payload.trim() == "[DONE]" {
            if !self.terminal_success {
                return Err(self.reject("Responses SSE ended before response.completed"));
            }
            self.done_marker_seen = true;
            output.extend_from_slice(b"data: [DONE]\n\n");
            return Ok(());
        }
        if self.terminal_success {
            return Err(self.reject("Responses SSE contains events after its terminal response"));
        }

        let event: Value = serde_json::from_str(&payload)
            .map_err(|_| self.reject("invalid JSON in Responses SSE frame"))?;
        let event_type = event
            .get("type")
            .and_then(Value::as_str)
            .or(event_name.as_deref())
            .ok_or_else(|| self.reject("Responses SSE event has no type"))?
            .to_string();

        let output_start = output.len();
        let result = match event_type.as_str() {
            "response.output_item.added" => self.handle_item_added(event, output),
            "response.function_call_arguments.delta" => self.handle_arguments_delta(event, output),
            "response.function_call_arguments.done" => self.handle_arguments_done(event, output),
            "response.output_item.done" => self.handle_item_done(event, output),
            "response.completed" => self.handle_completed(event, output),
            "response.failed" | "response.incomplete" | "error" => {
                output
                    .extend_from_slice(&self.fail_with_message("upstream Responses stream failed"));
                Ok(())
            }
            _ => {
                if event_type.contains("function_call_arguments")
                    || event_type.contains("custom_tool_call_input")
                {
                    if event.get("item_id").is_some() || event.get("call_id").is_some() {
                        match self.resolve_identity(&event) {
                            Ok((_, true)) => {}
                            Ok((_, false)) | Err(_) => {
                                return Err(
                                    self.reject("unknown or unsafe exec argument event type")
                                );
                            }
                        }
                    }
                }
                self.emit_value(output, &event_type, event);
                Ok(())
            }
        };
        if result.is_ok() && output.len() > output_start {
            let metadata = block
                .lines()
                .filter(|line| {
                    sse_field(line, "event").is_none() && sse_field(line, "data").is_none()
                })
                .map(|line| format!("{line}\n"))
                .collect::<String>();
            output.splice(output_start..output_start, metadata.bytes());
        }
        result
    }

    fn handle_item_added(&mut self, mut event: Value, output: &mut Vec<u8>) -> Result<()> {
        let item = event
            .get("item")
            .cloned()
            .ok_or_else(|| self.reject("response.output_item.added has no item"))?;
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        let item_id = item.get("id").and_then(Value::as_str).unwrap_or_default();

        if item_type != "function_call" || name != "exec" {
            if item_type == "custom_tool_call" && name == "exec" {
                return Err(self.reject("unexpected native custom exec in adapted stream"));
            }
            let track_item = !item_id.is_empty()
                && (item_type != "function_call" || !name.is_empty())
                && !self.non_exec_items.contains(item_id);
            let call_id = item.get("call_id").and_then(Value::as_str);
            if track_item {
                if self.item_to_call.contains_key(item_id) || self.seen_custom_ids.contains(item_id)
                {
                    return Err(self.reject("exec and unrelated item id collision"));
                }
                if self.non_exec_items.len() + self.item_to_call.len() >= MAX_ITEM_IDENTITIES {
                    return Err(self.reject("too many Responses item identities"));
                }
            }
            if let Some(call_id) = call_id {
                if self.calls.contains_key(call_id) || self.non_exec_calls.contains(call_id) {
                    return Err(self.reject("duplicate tool call_id"));
                }
                if self.non_exec_calls.len() + self.calls.len() >= MAX_ITEM_IDENTITIES {
                    return Err(self.reject("too many Responses call identities"));
                }
            }
            let metadata_bytes = if track_item { item_id.len() + 64 } else { 0 }
                + call_id.map_or(0, |call_id| {
                    call_id.len()
                        + 64
                        + if item_id.is_empty() {
                            0
                        } else {
                            item_id.len() + call_id.len() + 96
                        }
                });
            self.check_state_budget(metadata_bytes)?;
            self.retained_metadata_bytes += metadata_bytes;
            if track_item {
                self.non_exec_items.insert(item_id.to_string());
            }
            if let Some(call_id) = call_id {
                self.non_exec_calls.insert(call_id.to_string());
                if !item_id.is_empty() {
                    self.non_exec_item_calls
                        .insert(item_id.to_string(), call_id.to_string());
                }
            }
            self.emit_value(output, "response.output_item.added", event);
            return Ok(());
        }
        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| self.reject("exec function_call has no call_id"))?
            .to_string();
        let item_id = if item_id.is_empty() {
            format!("fc_{call_id}")
        } else {
            item_id.to_string()
        };
        self.register_call(
            call_id.clone(),
            item_id.clone(),
            item,
            event.get("output_index"),
            true,
        )?;
        let call = self.calls.get(&call_id).expect("registered exec call");
        let transformed = call.custom_item("");
        event["item"] = transformed;
        self.emit_value(output, "response.output_item.added", event);
        Ok(())
    }

    fn handle_arguments_delta(&mut self, event: Value, output: &mut Vec<u8>) -> Result<()> {
        let identity = self.resolve_identity(&event)?;
        if identity.1 {
            self.emit_value(output, "response.function_call_arguments.delta", event);
            return Ok(());
        }
        let call_id = identity
            .0
            .ok_or_else(|| self.reject("exec argument delta has no call identity"))?;
        let delta = event
            .get("delta")
            .and_then(Value::as_str)
            .ok_or_else(|| self.reject("exec argument delta is not a string"))?;
        self.append_arguments(&call_id, delta)?;
        Ok(())
    }

    fn handle_arguments_done(&mut self, event: Value, output: &mut Vec<u8>) -> Result<()> {
        let identity = self.resolve_identity(&event)?;
        if identity.1 {
            self.emit_value(output, "response.function_call_arguments.done", event);
            return Ok(());
        }
        let call_id = identity
            .0
            .ok_or_else(|| self.reject("exec argument completion has no call identity"))?;
        let candidate = event
            .get("arguments")
            .and_then(Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                event
                    .get("item")
                    .and_then(|item| item.get("arguments"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            });
        self.complete_call(&call_id, candidate.as_deref(), true, None, output)
    }

    fn handle_item_done(&mut self, mut event: Value, output: &mut Vec<u8>) -> Result<()> {
        let item = event.get("item").cloned().unwrap_or(Value::Null);
        let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
        if item_type != "function_call" || name != "exec" {
            if item_type == "custom_tool_call" && name == "exec" {
                return Err(self.reject("unexpected native custom exec in adapted stream"));
            }
            self.emit_value(output, "response.output_item.done", event);
            return Ok(());
        }

        let call_id = item
            .get("call_id")
            .and_then(Value::as_str)
            .or_else(|| event.get("call_id").and_then(Value::as_str))
            .filter(|value| !value.is_empty())
            .ok_or_else(|| self.reject("exec output_item.done has no call_id"))?
            .to_string();
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .unwrap_or("fc_unknown")
            .to_string();
        if !self.calls.contains_key(&call_id) {
            self.register_call(
                call_id.clone(),
                item_id,
                item.clone(),
                event.get("output_index"),
                false,
            )?;
        } else {
            self.ensure_call_item(&call_id, &item_id)?;
            self.replace_call_item(&call_id, item.clone())?;
        }
        let candidate = item
            .get("arguments")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.complete_call(
            &call_id,
            candidate.as_deref(),
            true,
            Some(&mut event),
            output,
        )
    }

    fn handle_completed(&mut self, mut event: Value, output: &mut Vec<u8>) -> Result<()> {
        if !event.get("response").is_some_and(Value::is_object)
            || !event
                .pointer("/response/output")
                .is_some_and(Value::is_array)
        {
            return Err(self.reject("terminal Responses event lacks a response output array"));
        }
        let status = event
            .pointer("/response/status")
            .and_then(Value::as_str)
            .unwrap_or_default();
        if status != "completed" {
            output.extend_from_slice(
                &self.fail_with_message("upstream Responses response was not completed"),
            );
            return Ok(());
        }

        let response_output = event
            .pointer("/response/output")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let adapted_response =
            adapt_response(event.get("response").cloned().unwrap_or(Value::Null))
                .map_err(|_| self.reject("invalid terminal exec response"))?;
        let mut final_calls = HashSet::new();
        for item in &response_output {
            if item["type"] == "function_call" && item["name"] == "exec" {
                let Some(call_id) = item.get("call_id").and_then(Value::as_str) else {
                    return Err(self.reject("terminal exec call has no call_id"));
                };
                if !final_calls.insert(call_id.to_string()) {
                    return Err(self.reject("duplicate exec call_id in terminal response"));
                }
            }
        }
        if self
            .calls
            .keys()
            .any(|call_id| !final_calls.contains(call_id))
        {
            return Err(self.reject("terminal response omits a previously streamed exec call"));
        }
        final_calls.clear();
        for (index, item) in response_output.into_iter().enumerate() {
            let item_type = item.get("type").and_then(Value::as_str).unwrap_or_default();
            let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
            if item_type != "function_call" || name != "exec" {
                if item_type == "custom_tool_call" && name == "exec" {
                    return Err(self.reject("unexpected native custom exec in adapted response"));
                }
                continue;
            }
            let call_id = item
                .get("call_id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| self.reject("terminal exec function_call has no call_id"))?
                .to_string();
            if !final_calls.insert(call_id.clone()) {
                return Err(self.reject("duplicate exec call_id in terminal response"));
            }
            let item_id = item
                .get("id")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
                .unwrap_or("fc_unknown")
                .to_string();
            if !self.calls.contains_key(&call_id) {
                self.register_call(
                    call_id.clone(),
                    item_id,
                    item.clone(),
                    Some(&json!(index)),
                    false,
                )?;
            } else {
                self.ensure_call_item(&call_id, &item_id)?;
            }
            let candidate = item.get("arguments").and_then(Value::as_str);
            self.complete_call(&call_id, candidate, true, None, output)?;
        }
        event["response"] = adapted_response;
        if self.calls.values().any(|call| call.input.is_none()) {
            return Err(self.reject("terminal response omitted complete exec arguments"));
        }
        self.terminal_success = true;
        self.emit_value(output, "response.completed", event);
        Ok(())
    }

    fn register_call(
        &mut self,
        call_id: String,
        item_id: String,
        mut item: Value,
        output_index: Option<&Value>,
        added: bool,
    ) -> Result<()> {
        if self.calls.len() >= MAX_CALLS {
            return Err(self.reject("too many in-flight exec calls"));
        }
        if self.calls.contains_key(&call_id) {
            return Err(self.reject("duplicate exec call_id"));
        }
        if self.non_exec_calls.contains(&call_id) {
            return Err(self.reject("exec call_id collides with another tool"));
        }
        if self.item_to_call.contains_key(&item_id) {
            return Err(self.reject("duplicate exec function item id"));
        }
        let custom_item_id = custom_item_id(&item_id)?;
        if self.non_exec_items.contains(&item_id) || self.non_exec_items.contains(&custom_item_id) {
            return Err(self.reject("exec and unrelated item id collision"));
        }
        if self.seen_custom_ids.contains(&custom_item_id) {
            return Err(self.reject("custom exec item id collision"));
        }
        if self.non_exec_items.len() + self.item_to_call.len() >= MAX_ITEM_IDENTITIES {
            return Err(self.reject("too many Responses item identities"));
        }
        let initial_arguments = item
            .get("arguments")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        if initial_arguments.len() > MAX_CALL_ARGUMENTS {
            return Err(self.reject("initial exec arguments exceed buffer limit"));
        }
        item.as_object_mut()
            .expect("exec item is an object")
            .remove("arguments");
        // Account for every retained copy, including identity maps and opaque metadata.
        let metadata_bytes = retained_value_bytes(&item)
            + output_index.map_or(0, retained_value_bytes)
            + 2 * (call_id.len() + item_id.len() + custom_item_id.len())
            + std::mem::size_of::<ExecCall>()
            + 256;
        self.check_state_budget(metadata_bytes + initial_arguments.capacity())?;
        self.retained_metadata_bytes += metadata_bytes;
        self.pending_argument_bytes += initial_arguments.capacity();
        self.seen_custom_ids.insert(custom_item_id.clone());
        self.item_to_call.insert(item_id.clone(), call_id.clone());
        self.calls.insert(
            call_id.clone(),
            ExecCall {
                function_item_id: item_id,
                custom_item_id,
                output_index: output_index.cloned(),
                item,
                arguments: initial_arguments,
                input: None,
                completion_emitted: false,
                done_emitted: false,
                added,
            },
        );
        Ok(())
    }

    fn check_state_budget(&mut self, additional: usize) -> Result<()> {
        if self
            .frame_buffer
            .len()
            .saturating_add(self.pending_argument_bytes)
            .saturating_add(self.retained_metadata_bytes)
            .saturating_add(additional)
            > MAX_PENDING_ARGUMENTS
        {
            return Err(self.reject("pending Responses state exceeds 4 MiB"));
        }
        Ok(())
    }

    fn replace_call_item(&mut self, call_id: &str, mut item: Value) -> Result<()> {
        item.as_object_mut()
            .expect("exec item is an object")
            .remove("arguments");
        let previous =
            retained_value_bytes(&self.calls.get(call_id).expect("exec call exists").item);
        let replacement = retained_value_bytes(&item);
        if replacement > previous {
            self.check_state_budget(replacement - previous)?;
        }
        self.retained_metadata_bytes = self
            .retained_metadata_bytes
            .saturating_sub(previous)
            .saturating_add(replacement);
        self.calls.get_mut(call_id).expect("exec call exists").item = item;
        Ok(())
    }

    fn ensure_call_item(&mut self, call_id: &str, item_id: &str) -> Result<()> {
        let Some(call) = self.calls.get(call_id) else {
            return Err(self.reject("unknown exec call"));
        };
        if call.function_item_id != item_id {
            return Err(self.reject("exec call_id references conflicting item ids"));
        }
        Ok(())
    }

    fn resolve_identity(&mut self, event: &Value) -> Result<(Option<String>, bool)> {
        let item_id = event.get("item_id").and_then(Value::as_str);
        let event_call_id = event.get("call_id").and_then(Value::as_str);
        if let Some(item_id) = item_id {
            if self.non_exec_items.contains(item_id) {
                if let Some(call_id) = event_call_id {
                    if self.non_exec_item_calls.get(item_id).map(String::as_str) != Some(call_id) {
                        return Err(self.reject("conflicting nonexec argument event identity"));
                    }
                }
                return Ok((None, true));
            }
            if let Some(call_id) = self.item_to_call.get(item_id) {
                if let Some(event_call_id) = event_call_id {
                    if event_call_id != call_id {
                        return Err(self.reject("conflicting exec call identity"));
                    }
                }
                return Ok((Some(call_id.clone()), false));
            }
            return Err(self.reject("unknown exec argument event item_id"));
        }
        if let Some(call_id) = event_call_id {
            if self.calls.contains_key(call_id) {
                if item_id.is_some() {
                    return Err(self.reject("exec argument event references conflicting item ids"));
                }
                return Ok((Some(call_id.to_string()), false));
            }
            if self.non_exec_calls.contains(call_id) {
                return Ok((None, true));
            }
        }
        Err(self.reject("unknown exec argument event identity"))
    }

    fn append_arguments(&mut self, call_id: &str, delta: &str) -> Result<()> {
        let delta_len = delta.len();
        let Some(call) = self.calls.get(call_id) else {
            return Err(self.reject("exec argument delta references an unknown call"));
        };
        if call.arguments.len().saturating_add(delta_len) > MAX_CALL_ARGUMENTS {
            return Err(self.reject("exec argument buffer exceeds 1 MiB"));
        }
        self.check_state_budget(delta_len)?;
        let call = self.calls.get(call_id).expect("exec call exists");
        if call.input.is_some() {
            return Err(self.reject("exec arguments arrived after completion"));
        }
        let Some(call) = self.calls.get_mut(call_id) else {
            return Err(self.reject("exec argument delta references an unknown call"));
        };
        let previous_capacity = call.arguments.capacity();
        call.arguments.reserve_exact(delta_len);
        call.arguments.push_str(delta);
        self.pending_argument_bytes = self
            .pending_argument_bytes
            .saturating_sub(previous_capacity)
            .saturating_add(call.arguments.capacity());
        self.check_state_budget(0)?;
        Ok(())
    }

    fn complete_call(
        &mut self,
        call_id: &str,
        candidate: Option<&str>,
        emit_done: bool,
        source_event: Option<&mut Value>,
        output: &mut Vec<u8>,
    ) -> Result<()> {
        let (args, already_input, custom_item_id, output_index) = {
            let Some(call) = self.calls.get(call_id) else {
                return Err(self.reject("unknown exec call"));
            };
            (
                candidate
                    .map(str::to_string)
                    .unwrap_or_else(|| call.arguments.clone()),
                call.input.clone(),
                call.custom_item_id.clone(),
                call.output_index.clone(),
            )
        };

        if args.len() > MAX_CALL_ARGUMENTS {
            return Err(self.reject("exec arguments exceed 1 MiB"));
        }
        let input = decode_exec_input(&args)
            .map_err(|_| self.reject("invalid or incomplete exec arguments"))?;
        let retained_len = self
            .calls
            .get(call_id)
            .expect("exec call exists")
            .arguments
            .capacity();
        if already_input.is_none()
            && self
                .pending_argument_bytes
                .saturating_sub(retained_len)
                .saturating_add(input.capacity())
                .saturating_add(self.retained_metadata_bytes)
                .saturating_add(self.frame_buffer.len())
                > MAX_PENDING_ARGUMENTS
        {
            return Err(self.reject("pending exec state exceeds 4 MiB"));
        }
        let buffered = &self.calls.get(call_id).expect("exec call exists").arguments;
        if already_input.is_none() && !buffered.is_empty() {
            let buffered_input = decode_exec_input(buffered)
                .map_err(|_| self.reject("incomplete exec argument delta sequence"))?;
            if buffered_input != input {
                return Err(self.reject("exec completion conflicts with argument delta sequence"));
            }
        }
        let mut emit_delta = false;
        if let Some(previous) = already_input.as_deref() {
            if previous != input {
                return Err(self.reject("conflicting exec completions"));
            }
        } else {
            let call = self.calls.get_mut(call_id).expect("exec call exists");
            self.pending_argument_bytes = self
                .pending_argument_bytes
                .saturating_sub(call.arguments.capacity());
            call.arguments = String::new();
            self.pending_argument_bytes += input.capacity();
            call.input = Some(input.clone());
            if !call.completion_emitted {
                call.completion_emitted = true;
                emit_delta = true;
            }
        }
        let added_event = {
            let call = self.calls.get_mut(call_id).expect("exec call exists");
            if !call.added {
                call.added = true;
                Some(json!({
                    "type": "response.output_item.added",
                    "output_index": call.output_index.clone().unwrap_or(Value::Null),
                    "item": call.custom_item("")
                }))
            } else {
                None
            }
        };
        if let Some(added_event) = added_event {
            self.emit_value(output, "response.output_item.added", added_event);
        }
        if emit_delta {
            let mut delta = json!({
                "type": "response.custom_tool_call_input.delta",
                "item_id": custom_item_id,
                "delta": input
            });
            if let Some(index) = output_index.clone() {
                delta["output_index"] = index;
            }
            self.emit_value(output, "response.custom_tool_call_input.delta", delta);
        }

        if emit_done {
            let (already_done, custom, call_output_index) = {
                let call = self.calls.get_mut(call_id).expect("exec call exists");
                (
                    call.done_emitted,
                    call.custom_item(call.input.as_deref().unwrap_or_default()),
                    call.output_index.clone(),
                )
            };
            if !already_done {
                let mut done = source_event.cloned().unwrap_or_else(|| {
                    json!({
                        "type": "response.output_item.done",
                        "output_index": call_output_index.clone().unwrap_or(Value::Null),
                        "item": custom
                    })
                });
                done["type"] = json!("response.output_item.done");
                done["item"] = custom;
                if let Some(index) = call_output_index {
                    done["output_index"] = index;
                }
                self.emit_value(output, "response.output_item.done", done);
                if let Some(call) = self.calls.get_mut(call_id) {
                    call.done_emitted = true;
                }
            }
        }
        Ok(())
    }

    fn emit_value(&mut self, output: &mut Vec<u8>, event_type: &str, mut value: Value) {
        if let Some(item_id) = value.get("item_id").and_then(Value::as_str) {
            if let Some(call_id) = self.item_to_call.get(item_id) {
                if let Some(call) = self.calls.get(call_id) {
                    value["item_id"] = json!(call.custom_item_id);
                }
            }
        }
        if let Some(object) = value.as_object_mut() {
            object.insert(
                "sequence_number".to_string(),
                Value::Number(self.next_sequence.into()),
            );
        }
        self.next_sequence = self.next_sequence.saturating_add(1);
        let encoded = serde_json::to_string(&value).unwrap_or_else(|_| "{}".to_string());
        output.extend_from_slice(format!("event: {event_type}\ndata: {encoded}\n\n").as_bytes());
    }
}

// Includes backing string/array storage and a conservative map-node allowance.
fn retained_value_bytes(value: &Value) -> usize {
    match value {
        Value::String(value) => value.capacity(),
        Value::Array(values) => {
            values.capacity() * std::mem::size_of::<Value>()
                + values.iter().map(retained_value_bytes).sum::<usize>()
        }
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| key.capacity() + 128 + retained_value_bytes(value))
            .sum(),
        _ => 0,
    }
}

fn custom_item_id(function_item_id: &str) -> Result<String> {
    let suffix = ["ctco_", "ctc_", "fco_", "fc_", "item_", "resp_", "cp_"]
        .into_iter()
        .find_map(|prefix| function_item_id.strip_prefix(prefix))
        .unwrap_or(function_item_id);
    if suffix.is_empty() {
        bail!("exec item has an empty id suffix");
    }
    Ok(format!("ctc_{suffix}"))
}

fn custom_exec_item(item: &Value, custom_id: &str, input: &str) -> Value {
    let mut item = item.clone();
    if let Some(object) = item.as_object_mut() {
        object.insert("type".to_string(), json!("custom_tool_call"));
        object.insert("id".to_string(), json!(custom_id));
        object.insert("name".to_string(), json!("exec"));
        object.insert("input".to_string(), json!(input));
        object.remove("arguments");
        object.remove("_responses_exec_done_emitted");
    }
    item
}

fn take_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let lf = find_delimiter(buffer, b"\n\n");
    let crlf = find_delimiter(buffer, b"\r\n\r\n");
    let (index, delimiter_len) = match (lf, crlf) {
        (Some(left), Some(right)) if left <= right => (left, 2),
        (Some(_), Some(right)) => (right, 4),
        (Some(index), None) => (index, 2),
        (None, Some(index)) => (index, 4),
        (None, None) => return None,
    };
    let frame = buffer[..index].to_vec();
    buffer.drain(..index + delimiter_len);
    Some(frame)
}

fn find_delimiter(buffer: &[u8], delimiter: &[u8]) -> Option<usize> {
    buffer
        .windows(delimiter.len())
        .position(|window| window == delimiter)
}

fn sse_field<'a>(line: &'a str, field: &str) -> Option<&'a str> {
    let (name, value) = line.split_once(':')?;
    if name == field {
        Some(value.strip_prefix(' ').unwrap_or(value))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(event: &str, value: Value) -> Vec<u8> {
        format!(
            "event: {event}\ndata: {}\n\n",
            serde_json::to_string(&value).unwrap()
        )
        .into_bytes()
    }

    fn added(call: &str) -> Value {
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {
                "type": "function_call",
                "id": format!("fc_{call}"),
                "call_id": call,
                "name": "exec",
                "arguments": ""
            }
        })
    }

    fn completed(call: &str, arguments: &str) -> Value {
        json!({
            "type": "response.completed",
            "response": {
                "status": "completed",
                "output": [{
                    "type": "function_call",
                    "id": format!("fc_{call}"),
                    "call_id": call,
                    "name": "exec",
                    "arguments": arguments
                }]
            }
        })
    }

    #[test]
    fn fragmented_utf8_and_crlf_are_assembled() {
        let mut adapter = ExecSseAdapter::new();
        let mut bytes = frame("response.output_item.added", added("a"));
        bytes.extend(frame(
            "response.completed",
            completed("a", r#"{"input":"const s = \"雪\";"}"#),
        ));
        let bytes = String::from_utf8(bytes)
            .unwrap()
            .replace('\n', "\r\n")
            .into_bytes();
        let split = bytes
            .windows("雪".len())
            .position(|window| window == "雪".as_bytes())
            .unwrap()
            + 1;
        let first = adapter.push_bytes(&bytes[..split]).unwrap();
        assert!(!first.is_empty());
        let second = adapter.push_bytes(&bytes[split..]).unwrap();
        let text = String::from_utf8([first, second].concat()).unwrap();
        assert!(text.contains("custom_tool_call"));
        assert!(text.contains("response.custom_tool_call_input.delta"));
        assert!(adapter.finish().unwrap().starts_with(b"data: [DONE]"));
        assert!(adapter.succeeded());
    }

    #[test]
    fn escaped_input_is_decoded_once_and_interleaving_is_preserved() {
        let mut adapter = ExecSseAdapter::new();
        let mut bytes = frame("response.output_item.added", added("a"));
        bytes.extend(frame(
            "response.output_item.added",
            json!({
                "type":"response.output_item.added",
                "output_index":1,
                "item":{"type":"message","id":"msg_1","role":"assistant"}
            }),
        ));
        bytes.extend(frame(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"{\"input\":\"a\\\\b"}),
        ));
        bytes.extend(frame(
            "response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":"\\n\"}"}),
        ));
        bytes.extend(frame(
            "response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","item_id":"fc_a","arguments":"{\"input\":\"a\\\\b\\n\"}"}),
        ));
        bytes.extend(frame(
            "response.completed",
            completed("a", "{\"input\":\"a\\\\b\\n\"}"),
        ));
        let text = String::from_utf8(adapter.push_bytes(&bytes).unwrap()).unwrap();
        assert!(text.contains("\"input\":\"a\\\\b\\n\""));
        assert!(text.contains("\"type\":\"message\""));
    }

    #[test]
    fn malformed_or_truncated_stream_never_emits_raw_input() {
        let mut adapter = ExecSseAdapter::new();
        let _ = adapter.push_bytes(&frame("response.output_item.added", added("a")));
        let malformed = frame(
            "response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","item_id":"fc_a","arguments":"{\"input\":\"x\""}),
        );
        assert!(adapter.push_bytes(&malformed).is_err());
        let failed = adapter.fail();
        assert!(
            String::from_utf8(failed)
                .unwrap()
                .contains("response.failed")
        );
        assert!(adapter.is_complete());
        assert!(!adapter.succeeded());
    }

    #[test]
    fn duplicate_completion_with_same_input_is_suppressed() {
        let mut adapter = ExecSseAdapter::new();
        let mut bytes = frame("response.output_item.added", added("a"));
        bytes.extend(frame(
            "response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","item_id":"fc_a","arguments":"{\"input\":\"x\"}"}),
        ));
        bytes.extend(frame(
            "response.output_item.done",
            json!({"type":"response.output_item.done","output_index":0,"item":{
                "type":"function_call","id":"fc_a","call_id":"a","name":"exec","arguments":"{\"input\":\"x\"}"
            }}),
        ));
        let output = String::from_utf8(adapter.push_bytes(&bytes).unwrap()).unwrap();
        assert_eq!(
            output
                .matches("event: response.custom_tool_call_input.delta")
                .count(),
            1
        );
        assert_eq!(
            output.matches("event: response.output_item.done").count(),
            1
        );
    }

    fn events(bytes: &[u8]) -> Vec<Value> {
        std::str::from_utf8(bytes)
            .unwrap()
            .lines()
            .filter_map(|line| line.strip_prefix("data: "))
            .filter(|data| *data != "[DONE]")
            .map(|data| serde_json::from_str(data).unwrap())
            .collect()
    }

    #[test]
    fn unrelated_function_deltas_and_interleaved_calls_keep_fields_and_indices() {
        let mut adapter = ExecSseAdapter::new();
        adapter
            .push_bytes(&frame("response.output_item.added", added("a")))
            .unwrap();
        let mut lookup = added("lookup");
        lookup["item"]["name"] = json!("lookup");
        lookup["output_index"] = json!(7);
        adapter
            .push_bytes(&frame("response.output_item.added", lookup))
            .unwrap();
        let unrelated = json!({"type":"response.function_call_arguments.delta",
            "item_id":"fc_lookup", "output_index":7, "delta":"{\"query\":", "vendor":"opaque"});
        let output = adapter
            .push_bytes(&frame(
                "response.function_call_arguments.delta",
                unrelated.clone(),
            ))
            .unwrap();
        let parsed = events(&output);
        assert_eq!(parsed[0]["delta"], unrelated["delta"]);
        assert_eq!(parsed[0]["vendor"], "opaque");
        assert_eq!(parsed[0]["item_id"], "fc_lookup");
        assert_eq!(parsed[0]["output_index"], 7);
        let mut second = added("b");
        second["output_index"] = json!(3);
        adapter
            .push_bytes(&frame("response.output_item.added", second))
            .unwrap();
        let output = adapter.push_bytes(&frame("response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","item_id":"fc_b","output_index":3,"arguments":"{\"input\":\"second\"}"}))).unwrap();
        let parsed = events(&output);
        assert_eq!(parsed[0]["item_id"], "ctc_b");
        assert_eq!(parsed[0]["output_index"], 3);
        assert_eq!(parsed[0]["delta"], "second");
        assert_eq!(parsed[1]["item"]["call_id"], "b");
        assert_eq!(parsed[1]["item"]["input"], "second");
        assert!(
            parsed
                .windows(2)
                .all(|pair| pair[0]["sequence_number"].as_u64()
                    < pair[1]["sequence_number"].as_u64())
        );
    }

    #[test]
    fn malformed_completions_fail_closed_at_every_completion_surface() {
        for arguments in [
            r#"{"input":"safe","input":"bad"}"#,
            r#"{"input":"safe","extra":1}"#,
            r#"{"input":1}"#,
            r#"{"input":"truncated"#,
        ] {
            for source in ["arguments", "item", "terminal"] {
                let mut adapter = ExecSseAdapter::new();
                adapter
                    .push_bytes(&frame("response.output_item.added", added("a")))
                    .unwrap();
                let (event_type, event) = match source {
                    "arguments" => (
                        "response.function_call_arguments.done",
                        json!({
                            "type":"response.function_call_arguments.done","item_id":"fc_a","arguments":arguments
                        }),
                    ),
                    "item" => (
                        "response.output_item.done",
                        json!({
                            "type":"response.output_item.done","item":{
                                "type":"function_call","name":"exec","id":"fc_a","call_id":"a","arguments":arguments
                            }
                        }),
                    ),
                    _ => ("response.completed", completed("a", arguments)),
                };
                assert!(
                    adapter.push_bytes(&frame(event_type, event)).is_err(),
                    "{source}: {arguments}"
                );
                let failure = adapter.fail();
                let parsed = events(&failure);
                assert_eq!(parsed.len(), 1);
                assert_eq!(parsed[0]["type"], "response.failed");
                assert_eq!(parsed[0]["response"]["output"], json!([]));
                assert!(adapter.fail().is_empty());
                assert!(!adapter.succeeded());
            }
        }
    }

    #[test]
    fn truncation_bad_utf8_and_unsuccessful_terminal_cannot_succeed() {
        for tail in [
            b"data: {\"type\":".as_slice(),
            b"data: \xf0\x9f".as_slice(),
            b"".as_slice(),
        ] {
            let mut adapter = ExecSseAdapter::new();
            adapter
                .push_bytes(&frame("response.output_item.added", added("a")))
                .unwrap();
            adapter.push_bytes(tail).unwrap();
            let failure = adapter.finish().unwrap();
            assert_eq!(events(&failure)[0]["type"], "response.failed");
            assert!(!adapter.succeeded());
            assert!(adapter.finish().unwrap().is_empty());
        }
        let mut adapter = ExecSseAdapter::new();
        assert!(adapter.push_bytes(b"data: \xff\n\n").is_err());
        let mut adapter = ExecSseAdapter::new();
        let failure = adapter
            .push_bytes(&frame(
                "response.completed",
                json!({"type":"response.completed","response":{"status":"incomplete","output":[]}}),
            ))
            .unwrap();
        assert_eq!(events(&failure)[0]["type"], "response.failed");
        assert!(!adapter.succeeded());
    }

    #[test]
    fn conflicting_completions_and_limits_are_rejected() {
        let mut adapter = ExecSseAdapter::new();
        adapter
            .push_bytes(&frame("response.output_item.added", added("a")))
            .unwrap();
        adapter.push_bytes(&frame("response.function_call_arguments.done",
            json!({"type":"response.function_call_arguments.done","item_id":"fc_a","arguments":"{\"input\":\"first\"}"}))).unwrap();
        assert!(
            adapter
                .push_bytes(&frame(
                    "response.completed",
                    completed("a", r#"{"input":"other"}"#)
                ))
                .is_err()
        );
        let mut adapter = ExecSseAdapter::new();
        adapter
            .push_bytes(&frame("response.output_item.added", added("a")))
            .unwrap();
        assert!(
            adapter
                .push_bytes(&frame("response.output_item.added", added("a")))
                .is_err()
        );
        let mut adapter = ExecSseAdapter::new();
        adapter
            .push_bytes(&frame("response.output_item.added", added("a")))
            .unwrap();
        let oversized = "x".repeat(MAX_CALL_ARGUMENTS + 1);
        assert!(adapter.push_bytes(&frame("response.function_call_arguments.delta",
            json!({"type":"response.function_call_arguments.delta","item_id":"fc_a","delta":oversized}))).is_err());
        let mut adapter = ExecSseAdapter::new();
        for index in 0..MAX_CALLS {
            adapter
                .push_bytes(&frame(
                    "response.output_item.added",
                    added(&index.to_string()),
                ))
                .unwrap();
        }
        assert!(
            adapter
                .push_bytes(&frame("response.output_item.added", added("overflow")))
                .is_err()
        );
    }

    #[test]
    fn terminal_only_exec_is_emitted_as_one_validated_custom_call() {
        let mut adapter = ExecSseAdapter::new();
        let output = adapter
            .push_bytes(&frame(
                "response.completed",
                completed("a", r#"{"input":""}"#),
            ))
            .unwrap();
        let parsed = events(&output);
        assert_eq!(
            parsed
                .iter()
                .map(|event| event["type"].as_str().unwrap())
                .collect::<Vec<_>>(),
            [
                "response.output_item.added",
                "response.custom_tool_call_input.delta",
                "response.output_item.done",
                "response.completed"
            ]
        );
        assert_eq!(parsed[1]["delta"], "");
        assert_eq!(parsed[2]["item"]["id"], "ctc_a");
        assert_eq!(parsed[3]["response"]["output"][0]["input"], "");
        adapter.finish().unwrap();
        assert!(adapter.succeeded());
    }

    #[test]
    fn both_item_and_call_identity_must_match_the_same_call() {
        for (item_id, call_id) in [("fc_unknown", "a"), ("fc_b", "a"), ("fc_lookup", "a")] {
            let mut adapter = ExecSseAdapter::new();
            adapter
                .push_bytes(&frame("response.output_item.added", added("a")))
                .unwrap();
            adapter
                .push_bytes(&frame("response.output_item.added", added("b")))
                .unwrap();
            let mut lookup = added("lookup");
            lookup["item"]["name"] = json!("lookup");
            adapter
                .push_bytes(&frame("response.output_item.added", lookup))
                .unwrap();
            assert!(adapter.push_bytes(&frame("response.function_call_arguments.delta",
                json!({"type":"response.function_call_arguments.delta","item_id":item_id,"call_id":call_id,"delta":"unsafe"}))).is_err());
        }
    }

    #[test]
    fn accepted_terminal_is_idempotent_across_chunks_and_never_emits_failure() {
        let mut adapter = ExecSseAdapter::new();
        let first = adapter
            .push_bytes(&frame(
                "response.completed",
                completed("a", r#"{"input":"ok"}"#),
            ))
            .unwrap();
        assert_eq!(
            events(&first)
                .iter()
                .filter(|event| event["type"] == "response.completed")
                .count(),
            1
        );
        for trailing in [
            frame(
                "response.completed",
                completed("a", r#"{"input":"different"}"#),
            ),
            b"data: [DONE]\n\ndata: [DONE]\n\n".to_vec(),
            b"data: \xff\n\n".to_vec(),
            b"data: {".to_vec(),
        ] {
            assert!(adapter.push_bytes(&trailing).unwrap().is_empty());
        }
        assert!(adapter.fail().is_empty());
        assert_eq!(adapter.finish().unwrap(), b"data: [DONE]\n\n");
        assert!(adapter.finish().unwrap().is_empty());
        assert!(adapter.succeeded());
    }

    #[test]
    fn terminal_requires_a_full_snapshot_containing_every_streamed_exec_call() {
        for terminal in [
            json!({"type":"response.completed", "status":"completed"}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
        ] {
            let mut adapter = ExecSseAdapter::new();
            adapter
                .push_bytes(&frame("response.output_item.added", added("a")))
                .unwrap();
            adapter.push_bytes(&frame("response.function_call_arguments.done",
                json!({"type":"response.function_call_arguments.done","item_id":"fc_a","arguments":"{\"input\":\"ok\"}"}))).unwrap();
            assert!(
                adapter
                    .push_bytes(&frame("response.completed", terminal))
                    .is_err()
            );
            assert!(!adapter.succeeded());
        }
    }

    #[test]
    fn retained_metadata_and_identity_registries_share_the_state_budget() {
        let mut adapter = ExecSseAdapter::new();
        let mut first = added("metadata-a");
        first["item"]["metadata"] = json!("x".repeat(3 * 1024 * 1024));
        adapter
            .push_bytes(&frame("response.output_item.added", first))
            .unwrap();
        let mut second = added("metadata-b");
        second["item"]["metadata"] = json!("x".repeat(3 * 1024 * 1024));
        assert!(
            adapter
                .push_bytes(&frame("response.output_item.added", second))
                .is_err()
        );

        let mut adapter = ExecSseAdapter::new();
        for index in 0..1024 {
            adapter
                .push_bytes(&frame(
                    "response.output_item.added",
                    json!({
                        "type":"response.output_item.added",
                        "item":{"type":"message","id":format!("msg_{index}"),"role":"assistant"}
                    }),
                ))
                .unwrap();
        }
        assert!(
            adapter
                .push_bytes(&frame(
                    "response.output_item.added",
                    json!({
                        "type":"response.output_item.added",
                        "item":{"type":"message","id":"msg_overflow","role":"assistant"}
                    })
                ))
                .is_err()
        );
    }

    #[test]
    fn replacing_completed_item_metadata_does_not_leak_budget() {
        let mut adapter = ExecSseAdapter::new();
        adapter
            .push_bytes(&frame("response.output_item.added", added("a")))
            .unwrap();
        let mut item = completed("a", r#"{"input":"ok"}"#)["response"]["output"][0].clone();
        item["metadata"] = json!("x".repeat(1024 * 1024));
        let event = frame(
            "response.output_item.done",
            json!({
                "type":"response.output_item.done", "item":item
            }),
        );
        for _ in 0..6 {
            adapter.push_bytes(&event).unwrap();
        }
        assert!(!adapter.failed);
    }
}
