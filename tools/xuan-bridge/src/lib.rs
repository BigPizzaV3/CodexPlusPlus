use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

pub const BRIDGE_PROTOCOL_VERSION: &str = "1";
pub const BRIDGE_VERSION: &str = "0.1.0";
const SEARCH_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_RESULTS: usize = 2_000;
const MAX_PREVIEW_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
pub struct RpcRequest {
    pub id: Value,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<RpcError>,
}

#[derive(Debug, Default)]
pub struct BridgeState {
    searches: HashMap<String, SearchRecord>,
    next_search_id: u64,
}

#[derive(Debug, Clone)]
struct SearchRecord {
    result: Value,
    cancelled: bool,
}

impl BridgeState {
    pub fn new() -> Self {
        Self::default()
    }
}

pub fn handle_request(state: &mut BridgeState, request: RpcRequest) -> RpcResponse {
    let id = request.id.clone();
    match dispatch(state, &request.method, request.params) {
        Ok(result) => RpcResponse {
            id,
            result: Some(result),
            error: None,
        },
        Err(error) => RpcResponse {
            id,
            result: None,
            error: Some(error),
        },
    }
}

fn dispatch(state: &mut BridgeState, method: &str, params: Value) -> Result<Value, RpcError> {
    match method {
        "bridge.health" => Ok(health_response()),
        "bridge.paths" => Ok(paths_response()),
        "workspace.search.start" => start_search(state, &params),
        "workspace.search.poll" => poll_search(state, &params),
        "workspace.search.cancel" => cancel_search(state, &params),
        "workspace.search.preview" => preview_file(&params),
        "usage.query" => query_usage(&params),
        "polish.generate" => generate_polish(&params),
        "mobile.status" => forward_mobile("status", &params),
        "mobile.pair" => forward_mobile("pair", &params),
        "mobile.confirm" => forward_mobile("confirm", &params),
        "mobile.tasks" => forward_mobile("tasks", &params),
        "mobile.send_input" => forward_mobile("send-input", &params),
        "mobile.stop" => forward_mobile("stop", &params),
        _ => Err(RpcError {
            code: "method_not_found".into(),
            message: format!("unsupported bridge method: {method}"),
        }),
    }
}

fn health_response() -> Value {
    json!({
        "status": "ok",
        "bridgeVersion": BRIDGE_VERSION,
        "protocolVersion": BRIDGE_PROTOCOL_VERSION,
        "capabilities": {
            "workspaceSearch": true,
            "usage": true,
            "polish": true,
            "mobile": std::env::var("XUAN_MOBILE_BRIDGE_URL").is_ok(),
        },
    })
}

fn query_usage(params: &Value) -> Result<Value, RpcError> {
    let base_url = std::env::var("XUAN_USAGE_BASE_URL").unwrap_or_default();
    if base_url.trim().is_empty() {
        return Err(error(
            "configuration_error",
            "set XUAN_USAGE_BASE_URL or configure a host credential adapter",
        ));
    }
    let api_key = std::env::var("XUAN_USAGE_API_KEY").unwrap_or_default();
    let usage_path = params
        .get("usagePath")
        .and_then(Value::as_str)
        .unwrap_or("/v1/usage");
    let url = build_usage_url(
        &base_url,
        usage_path,
        params.get("startDate").and_then(Value::as_str),
        params.get("endDate").and_then(Value::as_str),
        params.get("timezone").and_then(Value::as_str),
    )?;
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| error("transport_error", "unable to initialize usage client"))?;
    let mut request = client.get(url).header("Accept", "application/json");
    if !api_key.trim().is_empty() {
        request = request
            .bearer_auth(api_key.trim())
            .header("x-api-key", api_key.trim());
    }
    let response = request
        .send()
        .map_err(|_| error("transport_error", "usage request failed or timed out"))?;
    let status = response.status();
    let body: Value = response
        .json()
        .map_err(|_| error("invalid_response", "usage endpoint did not return JSON"))?;
    if !status.is_success() {
        return Err(error(
            "remote_error",
            format!("usage endpoint returned HTTP {}", status.as_u16()),
        ));
    }
    Ok(json!({
        "status": "ok",
        "provider": "generic",
        "data": body,
    }))
}

fn build_usage_url(
    endpoint: &str,
    usage_path: &str,
    start_date: Option<&str>,
    end_date: Option<&str>,
    timezone: Option<&str>,
) -> Result<Url, RpcError> {
    let mut url = Url::parse(endpoint.trim())
        .map_err(|_| error("configuration_error", "usage base URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(
            "configuration_error",
            "usage URL must use HTTP or HTTPS",
        ));
    }
    let path = usage_path.trim();
    if path.contains("://") || path.contains(['?', '#']) {
        return Err(error(
            "invalid_request",
            "usagePath must be a relative path",
        ));
    }
    let path = format!("/{}", path.trim_start_matches('/'));
    let base_path = url.path().trim_end_matches('/');
    let target_path = if base_path.ends_with("/usage") && path == "/v1/usage" {
        base_path.to_string()
    } else if base_path.ends_with("/v1") && path == "/v1/usage" {
        format!("{base_path}/usage")
    } else {
        format!("{base_path}{path}")
    };
    url.set_path(&target_path);
    url.set_query(None);
    if let (Some(start), Some(end)) = (start_date, end_date) {
        if !start.trim().is_empty() && !end.trim().is_empty() {
            url.query_pairs_mut()
                .append_pair("start_date", start.trim())
                .append_pair("end_date", end.trim())
                .append_pair("days", "90")
                .append_pair("timezone", timezone.unwrap_or("Asia/Shanghai"));
        }
    }
    Ok(url)
}

fn generate_polish(params: &Value) -> Result<Value, RpcError> {
    let text = required_string(params, "text")?;
    if text.chars().count() > 20_000 {
        return Err(error(
            "invalid_request",
            "text exceeds the 20,000 character limit",
        ));
    }
    let base_url = std::env::var("XUAN_POLISH_BASE_URL").unwrap_or_default();
    if base_url.trim().is_empty() {
        return Err(error(
            "configuration_error",
            "set XUAN_POLISH_BASE_URL or configure a host credential adapter",
        ));
    }
    let api_key = std::env::var("XUAN_POLISH_API_KEY").unwrap_or_default();
    let model = params
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| {
            std::env::var("XUAN_POLISH_MODEL")
                .ok()
                .map(|value| value.trim().to_owned())
                .filter(|value| !value.is_empty())
        })
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let style = params
        .get("style")
        .and_then(Value::as_str)
        .unwrap_or("structured");
    let system = match style {
        "concise" => {
            "Rewrite the draft concisely. Preserve intent, facts, identifiers and constraints."
        }
        "coding" => {
            "Rewrite the coding request precisely. Preserve code, paths, commands and acceptance criteria."
        }
        _ => {
            "Rewrite the draft into a clear, structured prompt. Preserve intent, facts, identifiers and constraints."
        }
    };
    let url = chat_completions_url(&base_url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| error("transport_error", "unable to initialize polish client"))?;
    let body = json!({
        "model": model,
        "temperature": 0.3,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": text}
        ]
    });
    let mut request = client
        .post(url)
        .header("Accept", "application/json")
        .json(&body);
    if !api_key.trim().is_empty() {
        request = request.bearer_auth(api_key.trim());
    }
    let response = request
        .send()
        .map_err(|_| error("transport_error", "polish request failed or timed out"))?;
    let status = response.status();
    let payload: Value = response
        .json()
        .map_err(|_| error("invalid_response", "polish endpoint did not return JSON"))?;
    if !status.is_success() {
        return Err(error(
            "remote_error",
            format!("polish endpoint returned HTTP {}", status.as_u16()),
        ));
    }
    let text = payload
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| error("invalid_response", "polish response has no text"))?;
    Ok(json!({ "status": "ok", "model": model, "text": text }))
}

fn chat_completions_url(endpoint: &str) -> Result<Url, RpcError> {
    let mut url = Url::parse(endpoint.trim())
        .map_err(|_| error("configuration_error", "polish base URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(
            "configuration_error",
            "polish URL must use HTTP or HTTPS",
        ));
    }
    let path = url.path().trim_end_matches('/');
    if path.ends_with("/chat/completions") {
        return Ok(url);
    }
    let target = if path.ends_with("/v1") {
        format!("{path}/chat/completions")
    } else {
        format!("{path}/v1/chat/completions")
    };
    url.set_path(&target);
    url.set_query(None);
    Ok(url)
}

fn forward_mobile(operation: &str, params: &Value) -> Result<Value, RpcError> {
    let base = std::env::var("XUAN_MOBILE_BRIDGE_URL").unwrap_or_default();
    if base.trim().is_empty() {
        return Err(error(
            "capability_unavailable",
            "configure XUAN_MOBILE_BRIDGE_URL for xuan-plus-remote",
        ));
    }
    let mut url = Url::parse(base.trim())
        .map_err(|_| error("configuration_error", "mobile bridge URL is invalid"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(error(
            "configuration_error",
            "mobile bridge URL must use HTTP or HTTPS",
        ));
    }
    url.set_path(&format!(
        "{}/v1/mobile/{operation}",
        url.path().trim_end_matches('/')
    ));
    url.set_query(None);
    let client = Client::builder()
        .timeout(Duration::from_secs(15))
        .build()
        .map_err(|_| {
            error(
                "transport_error",
                "unable to initialize mobile bridge client",
            )
        })?;
    let token = std::env::var("XUAN_MOBILE_BRIDGE_TOKEN").unwrap_or_default();
    let mut request = if operation == "status" {
        client.get(url)
    } else {
        client.post(url).json(params)
    };
    if !token.trim().is_empty() {
        request = request.bearer_auth(token.trim());
    }
    let response = request.send().map_err(|_| {
        error(
            "transport_error",
            "mobile bridge request failed or timed out",
        )
    })?;
    let status = response.status();
    let body: Value = response
        .json()
        .map_err(|_| error("invalid_response", "mobile bridge did not return JSON"))?;
    if !status.is_success() {
        return Err(error(
            "remote_error",
            format!("mobile bridge returned HTTP {}", status.as_u16()),
        ));
    }
    Ok(body)
}

fn paths_response() -> Value {
    let root = config_root();
    json!({
        "status": "ok",
        "configPath": root.join("xuan-plugins.json"),
        "databasePath": root.join("xuan-bridge.sqlite"),
    })
}

fn start_search(state: &mut BridgeState, params: &Value) -> Result<Value, RpcError> {
    let root = required_string(params, "root")?;
    let query = required_string(params, "query")?;
    if query.trim().is_empty() {
        return Err(error("invalid_request", "query must not be empty"));
    }
    if query.chars().count() > 1_000 || query.contains('\0') {
        return Err(error(
            "invalid_request",
            "query is too long or contains NUL",
        ));
    }
    let root = canonical_workspace_root(&root)?;
    let max_results = params
        .get("maxResults")
        .and_then(Value::as_u64)
        .unwrap_or(2_000)
        .clamp(1, MAX_RESULTS as u64) as usize;
    let include = string_array(params.get("include"));
    let exclude = string_array(params.get("exclude"));
    let case_sensitive = params
        .get("caseSensitive")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let whole_word = params
        .get("wholeWord")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let regex = params
        .get("regex")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let result = execute_search(
        &root,
        &query,
        &include,
        &exclude,
        case_sensitive,
        whole_word,
        regex,
        max_results,
    )?;
    state.next_search_id = state.next_search_id.saturating_add(1);
    let search_id = format!("xuan-search-{}", state.next_search_id);
    state.searches.insert(
        search_id.clone(),
        SearchRecord {
            result: result.clone(),
            cancelled: false,
        },
    );
    Ok(json!({
        "status": "ok",
        "searchId": search_id,
        "state": "complete",
        "result": result,
    }))
}

fn poll_search(state: &BridgeState, params: &Value) -> Result<Value, RpcError> {
    let search_id = required_string(params, "searchId")?;
    let record = state
        .searches
        .get(&search_id)
        .ok_or_else(|| error("not_found", "search task does not exist or expired"))?;
    if record.cancelled {
        return Ok(json!({ "status": "cancelled", "searchId": search_id }));
    }
    Ok(json!({
        "status": "ok",
        "searchId": search_id,
        "state": "complete",
        "result": record.result,
    }))
}

fn cancel_search(state: &mut BridgeState, params: &Value) -> Result<Value, RpcError> {
    let search_id = required_string(params, "searchId")?;
    let record = state
        .searches
        .get_mut(&search_id)
        .ok_or_else(|| error("not_found", "search task does not exist or expired"))?;
    record.cancelled = true;
    Ok(json!({ "status": "ok", "searchId": search_id }))
}

fn preview_file(params: &Value) -> Result<Value, RpcError> {
    let root = canonical_workspace_root(&required_string(params, "root")?)?;
    let path = PathBuf::from(required_string(params, "path")?);
    let path = std::fs::canonicalize(&path)
        .map_err(|_| error("not_found", "search result file is unavailable"))?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(error("permission_denied", "file is outside the workspace"));
    }
    let line = params
        .get("line")
        .and_then(Value::as_u64)
        .unwrap_or(1)
        .max(1) as usize;
    let mut bytes = Vec::new();
    File::open(&path)
        .and_then(|file| file.take(MAX_PREVIEW_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|_| error("io_error", "unable to read preview file"))?;
    if bytes.len() as u64 > MAX_PREVIEW_BYTES {
        return Err(error("too_large", "file is larger than 1 MB"));
    }
    if bytes.contains(&0) {
        return Err(error("binary_file", "binary files cannot be previewed"));
    }
    let text = String::from_utf8_lossy(&bytes);
    let lines = text.lines().collect::<Vec<_>>();
    let start = line.saturating_sub(6);
    let end = (line + 5).min(lines.len());
    let preview = lines[start..end]
        .iter()
        .enumerate()
        .map(|(index, value)| json!({ "number": start + index + 1, "text": value }))
        .collect::<Vec<_>>();
    Ok(json!({
        "status": "ok",
        "path": path,
        "line": line,
        "startLine": start + 1,
        "lines": preview,
    }))
}

#[allow(clippy::too_many_arguments)]
fn execute_search(
    root: &Path,
    query: &str,
    include: &[String],
    exclude: &[String],
    case_sensitive: bool,
    whole_word: bool,
    regex: bool,
    max_results: usize,
) -> Result<Value, RpcError> {
    let mut command = Command::new("rg");
    command.args([
        "--json",
        "--line-number",
        "--column",
        "--with-filename",
        "--no-heading",
        "--color",
        "never",
        "--max-filesize",
        "5M",
    ]);
    if regex {
        command.args(["--engine", "auto"]);
    } else {
        command.arg("--fixed-strings");
    }
    command.arg(if case_sensitive {
        "--case-sensitive"
    } else {
        "--ignore-case"
    });
    if whole_word {
        command.arg("--word-regexp");
    }
    for pattern in include.iter().filter(|value| !value.is_empty()).take(32) {
        command.arg("--glob").arg(pattern);
    }
    for pattern in exclude.iter().filter(|value| !value.is_empty()).take(32) {
        command.arg("--glob").arg(format!("!{pattern}"));
    }
    command
        .arg("-e")
        .arg(query)
        .arg(".")
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|_| {
        error(
            "dependency_missing",
            "ripgrep (rg) is not available on PATH",
        )
    })?;
    let started = Instant::now();
    let timed_out = loop {
        if let Some(_status) = child
            .try_wait()
            .map_err(|_| error("io_error", "failed to poll ripgrep"))?
        {
            break false;
        }
        if started.elapsed() >= SEARCH_TIMEOUT {
            let _ = child.kill();
            break true;
        }
        thread::sleep(Duration::from_millis(25));
    };
    let output = child
        .wait_with_output()
        .map_err(|_| error("io_error", "failed to collect ripgrep output"))?;
    let mut results = Vec::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        if let Some(item) = parse_rg_match(line, root) {
            results.push(item);
            if results.len() >= max_results {
                break;
            }
        }
    }
    if !timed_out && !matches!(output.status.code(), Some(0 | 1)) {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(error(
            "search_failed",
            if detail.is_empty() {
                "ripgrep search failed".into()
            } else {
                detail
            },
        ));
    }
    if timed_out && results.is_empty() {
        return Err(error("timeout", "workspace search exceeded 15 seconds"));
    }
    Ok(json!({
        "root": root,
        "results": results,
        "truncated": timed_out || results.len() >= max_results,
        "elapsedMs": started.elapsed().as_millis() as u64,
    }))
}

fn parse_rg_match(line: &str, root: &Path) -> Option<Value> {
    let event: Value = serde_json::from_str(line).ok()?;
    if event.get("type")?.as_str()? != "match" {
        return None;
    }
    let data = event.get("data")?;
    let relative = data
        .get("path")?
        .get("text")
        .or_else(|| data.get("path")?.get("bytes"))?
        .as_str()?
        .trim_start_matches("./")
        .trim_start_matches(".\\");
    let line_number = data.get("line_number")?.as_u64()?;
    let text = data
        .get("lines")?
        .get("text")?
        .as_str()?
        .trim_end_matches(['\r', '\n']);
    let ranges = data
        .get("submatches")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(json!({
                "start": byte_to_char_index(text, item.get("start")?.as_u64()? as usize),
                "end": byte_to_char_index(text, item.get("end")?.as_u64()? as usize),
            }))
        })
        .collect::<Vec<_>>();
    let column = ranges
        .first()
        .and_then(|value| value.get("start"))
        .and_then(Value::as_u64)
        .unwrap_or(0)
        + 1;
    Some(json!({
        "path": root.join(relative),
        "relativePath": relative.replace('\\', "/"),
        "line": line_number,
        "column": column,
        "text": text,
        "ranges": ranges,
    }))
}

fn byte_to_char_index(text: &str, byte_index: usize) -> usize {
    text.char_indices()
        .take_while(|(index, _)| *index < byte_index)
        .count()
}

fn canonical_workspace_root(value: &str) -> Result<PathBuf, RpcError> {
    let path = Path::new(value.trim());
    if !path.is_absolute() {
        return Err(error("invalid_request", "workspace root must be absolute"));
    }
    let path = std::fs::canonicalize(path)
        .map_err(|_| error("not_found", "workspace root is unavailable"))?;
    if !path.is_dir() {
        return Err(error(
            "invalid_request",
            "workspace root is not a directory",
        ));
    }
    if let Some(allowed) = std::env::var_os("XUAN_WORKSPACE_ROOTS") {
        let allowed = std::env::split_paths(&allowed)
            .filter_map(|item| std::fs::canonicalize(item).ok())
            .collect::<Vec<_>>();
        if !allowed.is_empty() && !allowed.iter().any(|item| path.starts_with(item)) {
            return Err(error(
                "permission_denied",
                "workspace root is not allow-listed",
            ));
        }
    }
    Ok(path)
}

fn required_string(params: &Value, key: &str) -> Result<String, RpcError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| {
            error(
                "invalid_request",
                format!("missing string parameter: {key}"),
            )
        })
}

fn string_array(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty() && value.len() <= 300 && !value.contains('\0'))
        .map(ToOwned::to_owned)
        .take(32)
        .collect()
}

fn error(code: impl Into<String>, message: impl Into<String>) -> RpcError {
    RpcError {
        code: code.into(),
        message: message.into(),
    }
}

pub fn config_root() -> PathBuf {
    if let Some(root) = std::env::var_os("XUAN_HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(root);
    }
    #[cfg(windows)]
    {
        if let Some(root) = std::env::var_os("APPDATA").filter(|value| !value.is_empty()) {
            return PathBuf::from(root).join("XuanPlusPlus");
        }
        if let Some(root) = std::env::var_os("USERPROFILE").filter(|value| !value.is_empty()) {
            return PathBuf::from(root).join(".xuan-plus-plus");
        }
    }
    if let Some(root) = std::env::var_os("HOME").filter(|value| !value.is_empty()) {
        return PathBuf::from(root).join(".config/xuan-plus-plus");
    }
    PathBuf::from(".xuan-plus-plus")
}

#[derive(Debug, Serialize)]
struct MigratedConfig {
    schema_version: u32,
    migrated_at: String,
    source_settings: String,
    plugins: Value,
    mobile: Value,
}

pub fn migrate_legacy_settings(input: &Path, output_root: &Path) -> Result<Value, String> {
    let raw = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let source: Value = serde_json::from_str(&raw).map_err(|error| error.to_string())?;
    std::fs::create_dir_all(output_root).map_err(|error| error.to_string())?;
    let backup = output_root.join(format!("legacy-settings-backup-{}.json", unix_timestamp()));
    std::fs::copy(input, &backup).map_err(|error| error.to_string())?;
    let mut permissions = std::fs::metadata(&backup)
        .map_err(|error| error.to_string())?
        .permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&backup, permissions).map_err(|error| error.to_string())?;
    let state_backup_root = output_root.join("legacy-state");
    std::fs::create_dir_all(&state_backup_root).map_err(|error| error.to_string())?;
    let mut state_backups = Vec::new();
    if let Some(legacy_state_root) = input.parent() {
        for name in ["mobile-remote.sqlite", "skills.json", "latest-status.json"] {
            let source = legacy_state_root.join(name);
            if !source.is_file() {
                continue;
            }
            let destination = state_backup_root.join(name);
            std::fs::copy(&source, &destination).map_err(|error| error.to_string())?;
            let mut permissions = std::fs::metadata(&destination)
                .map_err(|error| error.to_string())?
                .permissions();
            permissions.set_readonly(true);
            std::fs::set_permissions(&destination, permissions)
                .map_err(|error| error.to_string())?;
            state_backups.push(destination);
        }
    }
    let config = MigratedConfig {
        schema_version: 1,
        migrated_at: format!("{}", unix_timestamp()),
        source_settings: input.to_string_lossy().to_string(),
        plugins: json!({
            "xuan-workspace-search": { "enabled": bool_field(&source, "codexAppWorkspaceSearchEnabled") },
            "xuan-usage": {
                "enabled": bool_field(&source, "codexAppRelayBalanceEnabled"),
                "provider": string_field(&source, "codexAppRelayBalanceProvider")
            },
            "xuan-polish": {
                "enabled": bool_field(&source, "codexAppPromptOptimizeEnabled"),
                "relayId": string_field(&source, "codexAppPromptOptimizeRelayId"),
                "protocol": string_field(&source, "codexAppPromptOptimizeProtocol"),
                "model": string_field(&source, "codexAppPromptOptimizeModel"),
                "style": string_field(&source, "codexAppPromptOptimizeStyle")
            }
        }),
        mobile: json!({
            "enabled": bool_field(&source, "mobileRemoteEnabled"),
            "autoSync": bool_field(&source, "mobileRemoteAutoSync")
        }),
    };
    let output = output_root.join("xuan-plugins.json");
    let encoded = serde_json::to_vec_pretty(&config).map_err(|error| error.to_string())?;
    std::fs::write(&output, encoded).map_err(|error| error.to_string())?;
    Ok(json!({
        "status": "ok",
        "configPath": output,
        "backupPath": backup,
        "stateBackups": state_backups,
        "schemaVersion": 1,
    }))
}

fn bool_field(source: &Value, key: &str) -> bool {
    source.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn string_field(source: &Value, key: &str) -> String {
    source
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub fn serve_json_lines<R: Read, W: Write>(reader: R, mut writer: W) -> Result<(), String> {
    let mut state = BridgeState::new();
    let reader = BufReader::new(reader);
    for line in BufRead::lines(reader) {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<RpcRequest>(&line) {
            Ok(request) => handle_request(&mut state, request),
            Err(error) => RpcResponse {
                id: Value::Null,
                result: None,
                error: Some(RpcError {
                    code: "invalid_json".into(),
                    message: error.to_string(),
                }),
            },
        };
        serde_json::to_writer(&mut writer, &response).map_err(|error| error.to_string())?;
        writer.write_all(b"\n").map_err(|error| error.to_string())?;
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub fn serve_http(addr: &str) -> Result<(), String> {
    let listener = TcpListener::bind(addr).map_err(|error| error.to_string())?;
    let state = MutexState::new();
    for stream in listener.incoming() {
        let stream = stream.map_err(|error| error.to_string())?;
        let state = state.clone();
        thread::spawn(move || {
            let _ = handle_http_connection(stream, state);
        });
    }
    Ok(())
}

#[derive(Clone)]
struct MutexState(std::sync::Arc<std::sync::Mutex<BridgeState>>);

impl MutexState {
    fn new() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(
            BridgeState::new(),
        )))
    }
}

fn handle_http_connection(mut stream: TcpStream, state: MutexState) -> Result<(), String> {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .map_err(|error| error.to_string())?;
    let mut request = Vec::new();
    let mut header_end = None;
    let mut chunk = [0_u8; 1024];
    while request.len() < 64 * 1024 {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
        if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = Some(index + 4);
            break;
        }
    }
    let Some(header_end) = header_end else {
        return write_http_error(&mut stream, 400, "invalid HTTP request");
    };
    let headers = String::from_utf8_lossy(&request[..header_end]).to_string();
    let mut lines = headers.lines();
    let request_line = lines.next().unwrap_or_default().to_string();
    let mut request_parts = request_line.split_whitespace();
    let http_method = request_parts.next().unwrap_or_default().to_string();
    let path = request_parts.next().unwrap_or_default().to_string();
    let content_length = lines
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        })
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
        .unwrap_or(0);
    if content_length > 2 * 1024 * 1024 {
        return write_http_error(&mut stream, 413, "request body is too large");
    }
    while request.len() < header_end + content_length {
        let read = stream.read(&mut chunk).map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        request.extend_from_slice(&chunk[..read]);
    }
    let (method, expected_method) = http_bridge_method(&http_method, &path)
        .ok_or_else(|| "unknown HTTP bridge path".to_string())?;
    if expected_method != http_method {
        return write_http_error(&mut stream, 405, "HTTP method is not allowed");
    }
    let params = if content_length == 0 {
        Value::Object(Default::default())
    } else {
        serde_json::from_slice(&request[header_end..header_end + content_length])
            .map_err(|_| "request body must be JSON".to_string())?
    };
    let response = {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "bridge state is poisoned".to_string())?;
        handle_request(
            &mut guard,
            RpcRequest {
                id: Value::String("http".into()),
                method: method.into(),
                params,
            },
        )
    };
    let status = if response.error.is_some() { 400 } else { 200 };
    let body = if let Some(result) = response.result {
        result
    } else {
        json!({ "status": "failed", "error": response.error })
    };
    let body = serde_json::to_vec(&body).map_err(|error| error.to_string())?;
    let header = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        if status == 200 { "OK" } else { "Bad Request" },
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| error.to_string())
}

fn write_http_error(stream: &mut TcpStream, status: u16, message: &str) -> Result<(), String> {
    let body = serde_json::to_vec(
        &json!({ "status": "failed", "error": { "code": "http_error", "message": message } }),
    )
    .map_err(|error| error.to_string())?;
    let reason = match status {
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Bad Request",
    };
    let header = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(&body))
        .map_err(|error| error.to_string())
}

fn http_bridge_method(method: &str, path: &str) -> Option<(&'static str, &'static str)> {
    let path = path.split('?').next().unwrap_or(path);
    match (method, path) {
        ("GET", "/v1/health") => Some(("bridge.health", "GET")),
        ("GET", "/v1/paths") => Some(("bridge.paths", "GET")),
        ("POST", "/v1/search/start") => Some(("workspace.search.start", "POST")),
        ("POST", "/v1/search/poll") => Some(("workspace.search.poll", "POST")),
        ("POST", "/v1/search/cancel") => Some(("workspace.search.cancel", "POST")),
        ("POST", "/v1/search/preview") => Some(("workspace.search.preview", "POST")),
        ("POST", "/v1/usage") => Some(("usage.query", "POST")),
        ("POST", "/v1/polish") => Some(("polish.generate", "POST")),
        ("GET", "/v1/mobile/status") => Some(("mobile.status", "GET")),
        ("POST", "/v1/mobile/pair") => Some(("mobile.pair", "POST")),
        ("POST", "/v1/mobile/confirm") => Some(("mobile.confirm", "POST")),
        ("POST", "/v1/mobile/tasks") => Some(("mobile.tasks", "POST")),
        ("POST", "/v1/mobile/send-input") => Some(("mobile.send_input", "POST")),
        ("POST", "/v1/mobile/stop") => Some(("mobile.stop", "POST")),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn health_advertises_versioned_capabilities() {
        let result = health_response();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["protocolVersion"], BRIDGE_PROTOCOL_VERSION);
        assert_eq!(result["capabilities"]["workspaceSearch"], true);
    }

    #[test]
    fn migration_creates_read_only_backup_and_namespaced_config() {
        let dir = std::env::temp_dir().join(format!(
            "xuan-bridge-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let input = dir.join("settings.json");
        fs::write(
            &input,
            r#"{"codexAppWorkspaceSearchEnabled":true,"codexAppPromptOptimizeModel":"gpt-test","codexAppPromptOptimizeApiKey":"do-not-copy"}"#,
        )
        .unwrap();
        let output = migrate_legacy_settings(&input, &dir).unwrap();
        let config = fs::read_to_string(output["configPath"].as_str().unwrap()).unwrap();
        assert!(config.contains("xuan-workspace-search"));
        assert!(config.contains("gpt-test"));
        assert!(!config.contains("do-not-copy"));
        let backup = output["backupPath"].as_str().unwrap();
        assert!(fs::metadata(backup).unwrap().permissions().readonly());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn workspace_search_returns_matches_and_preserves_result_shape() {
        let dir = std::env::temp_dir().join(format!(
            "xuan-bridge-search-test-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("sample.txt"), "alpha\nneedle here\nomega\n").unwrap();
        let mut state = BridgeState::new();
        let request = RpcRequest {
            id: json!(1),
            method: "workspace.search.start".into(),
            params: json!({
                "root": dir,
                "query": "needle",
                "maxResults": 10
            }),
        };
        let response = handle_request(&mut state, request);
        assert!(response.error.is_none());
        let result = response.result.unwrap();
        assert_eq!(result["status"], "ok");
        assert_eq!(result["state"], "complete");
        assert_eq!(result["result"]["results"][0]["line"], 2);
        assert_eq!(result["result"]["results"][0]["relativePath"], "sample.txt");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn usage_url_normalization_preserves_provider_base_paths() {
        let url = build_usage_url(
            "https://relay.example/v1",
            "/v1/usage",
            Some("2026-09-01"),
            Some("2026-09-11"),
            Some("Asia/Shanghai"),
        )
        .unwrap();
        assert_eq!(url.path(), "/v1/usage");
        assert!(url.as_str().contains("start_date=2026-09-01"));
        assert!(
            build_usage_url(
                "https://relay.example",
                "https://evil.example",
                None,
                None,
                None
            )
            .is_err()
        );
    }
}
