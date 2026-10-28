//! 原生模块补丁通道（原生 Fast、原生输入预测）。仅修改本机应用资产，不暂停 JS，也不改写安装包。
//! 多个改写器可以修改同一个块；每个改写器由各自的设置开关控制，结构不符时只放弃该改写器。

use super::{
    BridgeGeneration, CdpSession, bridge_generation_is_current, connect_cdp_websocket,
    next_message_id, runtime_evaluate_params_return_by_value, wait_for_stale_bridge_generation,
};
use anyhow::{Context, bail};
use base64::Engine;
use serde_json::{Value, json};
use std::time::Duration;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

type Session = CdpSession<WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>>;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);
const CLEANUP_TIMEOUT: Duration = Duration::from_secs(3);
const CLEANUP_COMMAND_TIMEOUT: Duration = Duration::from_millis(750);
const SOURCE_LIMIT: usize = 24 * 1024 * 1024;
const COMPILED_MARKER: &str = "\n;globalThis.__codexPlusNativeFastCompiled='v1';\n";
const MODULES_LIMIT: usize = 48 * 1024 * 1024;
/// 入口块：执行完成标记、启动重载探针都以它为准。
const ENTRY_PREFIX: &str = "app-initial-";
const FAST_PREFIXES: &[&str] = &[ENTRY_PREFIX];
const PREDICTIONS_PREFIXES: &[&str] = &[
    ENTRY_PREFIX,
    "composer-content-",
    "initialize-app-host-services-",
    "general-settings-",
];

#[derive(Default)]
struct InterceptorState {
    script_id: Option<String>,
    fetch_attempted: bool,
}

#[derive(Clone, Copy)]
struct ModuleSettings {
    fast: crate::native_fast::RuntimeSettings,
    predictions: crate::native_predictions::RuntimeSettings,
}

impl ModuleSettings {
    fn load(settings: &crate::settings::BackendSettings) -> Self {
        Self {
            fast: crate::native_fast::runtime_settings(settings),
            predictions: crate::native_predictions::runtime_settings(settings),
        }
    }

    /// 只拦截开启的改写器需要的块；仅开 Fast 时保持原来只拦截 app-initial。
    fn prefixes(&self) -> Vec<&'static str> {
        let mut prefixes = Vec::new();
        let groups = [
            (
                self.fast.enhancements_enabled && self.fast.codex_app_native_fast_unlock,
                FAST_PREFIXES,
            ),
            (
                self.predictions.enhancements_enabled
                    && self.predictions.codex_app_native_predictions,
                PREDICTIONS_PREFIXES,
            ),
        ];
        for (enabled, group) in groups {
            if !enabled {
                continue;
            }
            for prefix in group {
                if !prefixes.contains(prefix) {
                    prefixes.push(*prefix);
                }
            }
        }
        prefixes
    }
}

fn bootstrap_script(settings: ModuleSettings) -> String {
    // 只发送功能开关和本机 API 身份投影，不能序列化含供应商凭据的后端 settings。
    format!(
        "{}\n;window.__codexPlusNativeFastUnlock.runtime.update({});\n{}\n;window.__codexPlusNativePredictions.runtime.update({});\nwindow.__codexPlusNativeFastPatchState='armed';\nwindow.__codexPlusNativeFastAssetPatchReady=true;",
        crate::assets::native_fast_unlock_script(),
        json!(settings.fast),
        crate::assets::native_predictions_script(),
        json!(settings.predictions),
    )
}

pub(super) async fn install(
    websocket_url: &str,
    generation: BridgeGeneration,
) -> anyhow::Result<()> {
    let settings = crate::settings::SettingsStore::default().load()?;
    if !crate::native_predictions::native_modules_requested(&settings) {
        return Ok(());
    }
    let socket = connect_cdp_websocket(websocket_url).await?;
    let mut session = CdpSession::new(socket);
    let mut state = InterceptorState::default();
    let module_settings = ModuleSettings::load(&settings);
    let prefixes = module_settings.prefixes();
    let bootstrap = bootstrap_script(module_settings);
    let initialized = async {
        let location = evaluate(&mut session, "location.protocol").await?;
        if location
            .pointer("/result/result/value")
            .and_then(Value::as_str)
            == Some("app:")
        {
            initialize_module(&mut session, &mut state, &bootstrap, &generation).await
        } else {
            initialize(&mut session, &mut state, &bootstrap, &prefixes).await
        }
    }
    .await;
    if let Err(error) = initialized {
        if bridge_generation_is_current(&generation) {
            publish_patch_state(&mut session, "failed").await;
        }
        cleanup(&mut session, &generation, &state).await;
        if !bridge_generation_is_current(&generation) {
            return Ok(());
        }
        return Err(error);
    }
    if !bridge_generation_is_current(&generation) {
        cleanup(&mut session, &generation, &state).await;
        return Ok(());
    }
    diagnostic("bridge.native_fast_interceptor_ready", json!({}));
    tokio::spawn(async move {
        let result = watch(&mut session, &generation, &prefixes).await;
        cleanup(&mut session, &generation, &state).await;
        if result.is_err() {
            // 不把 Runtime 异常或响应正文写入诊断。
            diagnostic(
                "bridge.native_fast_interceptor_stopped",
                json!({ "reason": "transport_failed" }),
            );
        }
    });
    Ok(())
}

// Electron 的 app:// 自定义协议不经过 Chromium Fetch 拦截器。
// 在新文档的 meta CSP 解析前预载同源模块的补丁副本，原始模块图通过 import map 复用它。
// 不修改 CSP，也不在后台注入时刷新页面；真正应用仍由显式启动流程负责。
async fn initialize_module(
    session: &mut Session,
    state: &mut InterceptorState,
    bootstrap: &str,
    generation: &BridgeGeneration,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        bridge_generation_is_current(generation),
        "native Fast generation expired"
    );
    command(session, "Runtime.enable", json!({})).await?;
    command(session, "Page.enable", json!({})).await?;
    anyhow::ensure!(
        bridge_generation_is_current(generation),
        "native Fast generation expired"
    );
    let current_bootstrap =
        bootstrap.replace("window.__codexPlusNativeFastAssetPatchReady=true;", "");
    evaluate(session, &current_bootstrap).await?;
    // 发现、改写与跨块完整性校验都在补丁脚本里完成；这里只校验地址与体积后登记新文档脚本。
    let prepared = evaluate_with_timeout(
        session,
        "window.__codexPlusNativeFastUnlock.prepareDocumentModules().then(r=>({ok:r.ok===true,modules:r.ok===true?r.modules:[],diagnostics:r.diagnostics??null}))",
        Duration::from_secs(30),
    )
    .await?;
    let value = prepared
        .pointer("/result/result/value")
        .context("native module preparation returned no value")?;
    diagnostic(
        "bridge.native_module_patch_prepared",
        json!({
            "ok": value["ok"] == true,
            "reason": value.pointer("/diagnostics/reason").cloned().unwrap_or(Value::Null),
            "rewriters": value.pointer("/diagnostics/rewriters").cloned().unwrap_or(Value::Null),
        }),
    );
    anyhow::ensure!(value["ok"] == true, "当前 Codex 资产不支持原生模块补丁");
    let modules = value["modules"]
        .as_array()
        .context("native module list is missing")?;
    anyhow::ensure!(!modules.is_empty(), "native module list is empty");
    let mut total = 0usize;
    let mut has_entry = false;
    for module in modules {
        let url = module["url"]
            .as_str()
            .context("native module URL is missing")?;
        anyhow::ensure!(
            is_local_asset(url, PREDICTIONS_PREFIXES),
            "native module must be a local asset"
        );
        has_entry |= is_local_asset(url, FAST_PREFIXES);
        let source = module["patchedSource"]
            .as_str()
            .context("native module source is missing")?;
        anyhow::ensure!(source.len() <= SOURCE_LIMIT, "native module is too large");
        total += source.len();
    }
    anyhow::ensure!(has_entry, "native module entry is missing");
    anyhow::ensure!(total <= MODULES_LIMIT, "native modules are too large");
    anyhow::ensure!(
        bridge_generation_is_current(generation),
        "native Fast generation expired"
    );
    let rewriters = value
        .pointer("/diagnostics/rewriters")
        .cloned()
        .unwrap_or_else(|| json!({}));
    let script = format!(
        "{bootstrap}\n;window.__codexPlusNativeFastUnlock.installModules({},{});",
        Value::Array(modules.clone()),
        json!({ "rewriters": rewriters })
    );
    let reply = command(
        session,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({"source":script}),
    )
    .await?;
    state.script_id = reply
        .pointer("/result/identifier")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    anyhow::ensure!(
        state.script_id.is_some(),
        "native Fast module bootstrap was not registered"
    );
    anyhow::ensure!(
        bridge_generation_is_current(generation),
        "native Fast generation expired"
    );
    evaluate(session, "window.__codexPlusNativeFastAssetPatchReady=true").await?;
    diagnostic(
        "bridge.native_fast_module_ready",
        json!({"transport":"import_map"}),
    );
    Ok(())
}

async fn initialize(
    session: &mut Session,
    state: &mut InterceptorState,
    bootstrap: &str,
    prefixes: &[&str],
) -> anyhow::Result<()> {
    command(session, "Runtime.enable", json!({})).await?;
    // Page 域未 enable 时 Chromium 不保证执行 new-document 脚本。
    command(session, "Page.enable", json!({})).await?;
    let reply = command(
        session,
        "Page.addScriptToEvaluateOnNewDocument",
        json!({ "source": bootstrap }),
    )
    .await?;
    state.script_id = reply
        .pointer("/result/identifier")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
        .map(str::to_string);
    anyhow::ensure!(
        state.script_id.is_some(),
        "native Fast bootstrap was not registered"
    );
    // 当前文档先安装 transformer；ready 只能在 Fetch.enable 确认成功后发布。
    let current_bootstrap =
        bootstrap.replace("window.__codexPlusNativeFastAssetPatchReady=true;", "");
    evaluate(session, &current_bootstrap).await?;
    let patterns: Vec<Value> = prefixes
        .iter()
        .map(
            |prefix| json!({ "urlPattern": format!("*{prefix}*.js*"), "requestStage": "Response" }),
        )
        .collect();
    anyhow::ensure!(!patterns.is_empty(), "no native module patch is enabled");
    state.fetch_attempted = true;
    command(session, "Fetch.enable", json!({ "patterns": patterns })).await?;
    evaluate(session, "window.__codexPlusNativeFastAssetPatchReady=true").await?;
    Ok(())
}

async fn command(session: &mut Session, method: &str, params: Value) -> anyhow::Result<Value> {
    session
        .send_command_with_timeout(next_message_id(), method, params, COMMAND_TIMEOUT)
        .await
}

async fn evaluate(session: &mut Session, expression: &str) -> anyhow::Result<Value> {
    evaluate_with_timeout(session, expression, COMMAND_TIMEOUT).await
}

async fn evaluate_with_timeout(
    session: &mut Session,
    expression: &str,
    timeout: Duration,
) -> anyhow::Result<Value> {
    let mut params = runtime_evaluate_params_return_by_value(expression);
    params["awaitPromise"] = json!(true);
    let reply = session
        .send_command_with_timeout(next_message_id(), "Runtime.evaluate", params, timeout)
        .await?;
    anyhow::ensure!(
        reply.pointer("/result/exceptionDetails").is_none(),
        "native Fast evaluation failed"
    );
    Ok(reply)
}

async fn watch(
    session: &mut Session,
    generation: &BridgeGeneration,
    prefixes: &[&str],
) -> anyhow::Result<()> {
    loop {
        if !bridge_generation_is_current(generation) {
            return Ok(());
        }
        // read_socket_message 是唯一入队点，包括其它命令等待回包时收到的事件。
        // 不直接处理 next_message 的返回值，避免同一个 paused request 被消费两次。
        if let Some(request) = session.fetch_requests.pop_front() {
            handle_paused(session, &request, prefixes).await?;
            continue;
        }
        tokio::select! {
            result = session.next_message() => {
                if result?.is_none() { return Ok(()); }
            }
            _ = wait_for_stale_bridge_generation(generation) => return Ok(()),
        }
    }
}

fn is_local_asset(url: &str, prefixes: &[&str]) -> bool {
    let Ok(url) = reqwest::Url::parse(url) else {
        return false;
    };
    let local_origin = match url.scheme() {
        "app" => url.host_str() == Some("-"),
        "file" => url
            .host_str()
            .is_none_or(|host| host.is_empty() || host == "localhost"),
        _ => false,
    };
    let Some((prefix, filename)) = url.path().rsplit_once('/') else {
        return false;
    };
    local_origin
        && prefix.ends_with("/assets")
        && prefixes.iter().any(|module| {
            filename
                .strip_prefix(module)
                .and_then(|rest| rest.strip_suffix(".js"))
                .is_some_and(|hash| {
                    !hash.is_empty()
                        && hash
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                })
        })
}

fn javascript_response(params: &Value) -> bool {
    params.get("responseStatusCode").and_then(Value::as_u64) == Some(200)
        && params.get("responseErrorReason").is_none()
        && params
            .get("responseHeaders")
            .and_then(Value::as_array)
            .is_some_and(|headers| {
                headers.iter().any(|header| {
                    header["name"]
                        .as_str()
                        .is_some_and(|name| name.eq_ignore_ascii_case("content-type"))
                        && header["value"].as_str().is_some_and(|value| {
                            matches!(
                                value
                                    .split(';')
                                    .next()
                                    .unwrap_or("")
                                    .trim()
                                    .to_ascii_lowercase()
                                    .as_str(),
                                "application/javascript"
                                    | "text/javascript"
                                    | "application/ecmascript"
                                    | "text/ecmascript"
                                    | "application/x-javascript"
                            )
                        })
                })
            })
}

fn fulfillment_headers(params: &Value) -> Vec<Value> {
    params["responseHeaders"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|header| {
            header["name"].as_str().is_some_and(|name| {
                !name.eq_ignore_ascii_case("content-length")
                    && !name.eq_ignore_ascii_case("content-encoding")
            })
        })
        .cloned()
        .collect()
}

async fn handle_paused(
    session: &mut Session,
    event: &Value,
    prefixes: &[&str],
) -> anyhow::Result<()> {
    let params = event
        .get("params")
        .context("native Fast response event has no params")?;
    let request_id = params["requestId"]
        .as_str()
        .context("native Fast response event has no request id")?;
    let url = params
        .pointer("/request/url")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !is_local_asset(url, prefixes) {
        return release_request(session, request_id).await;
    }
    // 只有入口块的失败代表整个通道不可用；同前缀的其它懒加载块不含目标结构是正常情况。
    let entry = is_local_asset(url, FAST_PREFIXES);
    if !javascript_response(params) {
        if entry {
            publish_patch_state(session, "unsupported").await;
        }
        return release_request(session, request_id).await;
    }
    let patched = patch_response(session, request_id, url, entry).await;
    match patched {
        Ok(Some(source)) => {
            let mut fulfill = json!({
                "requestId": request_id,
                "responseCode": 200,
                "responseHeaders": fulfillment_headers(params),
                "body": base64::engine::general_purpose::STANDARD.encode(source),
            });
            if let Some(phrase) = params.get("responseStatusText").and_then(Value::as_str) {
                fulfill["responsePhrase"] = json!(phrase);
            }
            if command(session, "Fetch.fulfillRequest", fulfill)
                .await
                .is_ok()
            {
                if entry {
                    publish_patch_state(session, "patched").await;
                }
                diagnostic(
                    "bridge.native_fast_asset_patched",
                    json!({ "version": "v1" }),
                );
                return Ok(());
            }
            diagnostic(
                "bridge.native_fast_patch_skipped",
                json!({ "reason": "fulfill_failed" }),
            );
            publish_patch_state(session, "failed").await;
        }
        Ok(None) => {
            if entry {
                publish_patch_state(session, "unsupported").await;
                diagnostic(
                    "bridge.native_fast_patch_skipped",
                    json!({ "reason": "unsupported_asset" }),
                );
            }
        }
        Err(_) => {
            publish_patch_state(session, "failed").await;
            diagnostic(
                "bridge.native_fast_patch_skipped",
                json!({ "reason": "transform_failed" }),
            );
        }
    }
    // 任意取正文/变换/回填失败都释放原响应；不能把应用挂在暂停的网络请求上。
    release_request(session, request_id).await
}

async fn publish_patch_state(session: &mut Session, state: &str) {
    let expression = format!("window.__codexPlusNativeFastPatchState={}", json!(state));
    let _ = evaluate(session, &expression).await;
}

async fn patch_response(
    session: &mut Session,
    request_id: &str,
    url: &str,
    entry: bool,
) -> anyhow::Result<Option<Vec<u8>>> {
    let reply = command(
        session,
        "Fetch.getResponseBody",
        json!({ "requestId": request_id }),
    )
    .await?;
    let body = reply
        .pointer("/result/body")
        .and_then(Value::as_str)
        .context("asset response has no body")?;
    anyhow::ensure!(
        body.len() <= SOURCE_LIMIT * 4 / 3 + 4,
        "asset response is too large"
    );
    let source = if reply
        .pointer("/result/base64Encoded")
        .and_then(Value::as_bool)
        == Some(true)
    {
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(body)?)?
    } else {
        body.to_string()
    };
    anyhow::ensure!(source.len() <= SOURCE_LIMIT, "asset source is too large");
    let expression = format!(
        "(async()=>{{const result=await window.__codexPlusNativeFastUnlock?.transformModule?.({},{});return result?.ok===true&&typeof result.patchedSource==='string'?{{ok:true,patchedSource:result.patchedSource}}:{{ok:false,diagnostics:result?.diagnostics??{{reason:'transformer-unavailable'}}}}}})()",
        serde_json::to_string(&source)?,
        serde_json::to_string(url)?,
    );
    let reply = evaluate(session, &expression).await?;
    let value = reply.pointer("/result/result/value");
    if value
        .and_then(|value| value.get("ok"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        #[cfg(test)]
        eprintln!(
            "native Fast transform declined: {:?}",
            value.and_then(|value| value.get("diagnostics"))
        );
        return Ok(None);
    }
    let patched = value
        .and_then(|value| value.get("patchedSource"))
        .and_then(Value::as_str)
        .context("asset transformer did not return source")?;
    anyhow::ensure!(patched.len() <= SOURCE_LIMIT, "patched asset is too large");
    let mut patched = patched.as_bytes().to_vec();
    if entry {
        patched.extend_from_slice(COMPILED_MARKER.as_bytes());
    }
    Ok(Some(patched))
}

async fn release_request(session: &mut Session, request_id: &str) -> anyhow::Result<()> {
    for method in ["Fetch.continueResponse", "Fetch.continueRequest"] {
        if session
            .send_command_with_timeout(
                next_message_id(),
                method,
                json!({ "requestId": request_id }),
                CLEANUP_COMMAND_TIMEOUT,
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
    }
    // 返回错误让监督循环退出，Fetch.disable 与关闭此独立连接兜底释放请求。
    bail!("native Fast could not release asset response")
}

async fn cleanup(session: &mut Session, generation: &BridgeGeneration, state: &InterceptorState) {
    let _ = tokio::time::timeout(CLEANUP_TIMEOUT, async {
        if state.fetch_attempted {
            let _ = session
                .send_command_with_timeout(
                    next_message_id(),
                    "Fetch.disable",
                    json!({}),
                    CLEANUP_COMMAND_TIMEOUT,
                )
                .await;
        }
        if let Some(script_id) = &state.script_id {
            let _ = session
                .send_command_with_timeout(
                    next_message_id(),
                    "Page.removeScriptToEvaluateOnNewDocument",
                    json!({ "identifier": script_id }),
                    CLEANUP_COMMAND_TIMEOUT,
                )
                .await;
        }
        if bridge_generation_is_current(generation) {
            let _ = session
                .send_command_with_timeout(
                    next_message_id(),
                    "Runtime.evaluate",
                    runtime_evaluate_params_return_by_value(
                        "window.__codexPlusNativeFastAssetPatchReady=false",
                    ),
                    CLEANUP_COMMAND_TIMEOUT,
                )
                .await;
        }
    })
    .await;
    let _ = tokio::time::timeout(CLEANUP_COMMAND_TIMEOUT, session.close()).await;
}

fn diagnostic(event: &str, details: Value) {
    let _ = crate::diagnostic_log::append_diagnostic_log(event, details);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bridge::{
        next_bridge_generation, publish_bridge_generation, release_bridge_generation,
    };
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    fn module_settings(fast: bool, local_api: bool, predictions: bool) -> ModuleSettings {
        ModuleSettings {
            fast: crate::native_fast::RuntimeSettings {
                enhancements_enabled: true,
                codex_app_native_fast_unlock: fast,
                native_fast_local_api_configured: local_api,
            },
            predictions: crate::native_predictions::RuntimeSettings {
                enhancements_enabled: true,
                codex_app_native_predictions: predictions,
                native_predictions_local_api_configured: local_api,
            },
        }
    }

    #[tokio::test]
    async fn app_modules_register_preload_without_fetch_interception_or_csp_bypass() {
        for (prepared, superseded) in [(true, false), (false, false), (true, true)] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let generation = next_bridge_generation(&format!("native-fast-module-{address}"));
            assert!(publish_bridge_generation(&generation));
            let server_generation = generation.clone();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let mut commands = Vec::new();
                while let Some(Ok(message)) = socket.next().await {
                    let Message::Text(message) = message else {
                        continue;
                    };
                    let request: Value = serde_json::from_str(&message).unwrap();
                    let result = match request["method"].as_str().unwrap() {
                        "Runtime.enable" | "Page.enable" => json!({}),
                        "Runtime.evaluate" => {
                            let expression = request["params"]["expression"].as_str().unwrap();
                            if expression.contains("prepareDocumentModules()") {
                                if superseded {
                                    release_bridge_generation(&server_generation);
                                }
                                json!({"result":{"value":if prepared {
                                    json!({"ok":true,"modules":[
                                        {"url":"app://-/assets/app-initial-test.js","patchedSource":"patched-module"},
                                        {"url":"app://-/assets/composer-content-test.js","patchedSource":"patched-lazy"}
                                    ],"diagnostics":{"reason":"modules-prepared","rewriters":{"fast":"patched","predictions":"patched"}}})
                                } else {json!({"ok":false,"modules":[],"diagnostics":{"reason":"unsupported","rewriters":{"predictions":"composer-content:incomplete-structure"}}})}}})
                            } else {
                                json!({"result":{"value":true}})
                            }
                        }
                        "Page.addScriptToEvaluateOnNewDocument" => {
                            json!({"identifier":"module-script"})
                        }
                        method => panic!("unexpected module transport command {method}"),
                    };
                    socket
                        .send(Message::Text(
                            json!({"id":request["id"],"result":result})
                                .to_string()
                                .into(),
                        ))
                        .await
                        .unwrap();
                    commands.push(request);
                }
                commands
            });
            let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/module"))
                .await
                .unwrap();
            let mut session = CdpSession::new(socket);
            let mut state = InterceptorState::default();
            let result = initialize_module(
                &mut session,
                &mut state,
                &bootstrap_script(module_settings(true, false, true)),
                &generation,
            )
            .await;
            assert_eq!(result.is_ok(), prepared && !superseded);
            assert_eq!(state.script_id.is_some(), prepared && !superseded);
            assert!(!state.fetch_attempted);
            session.close().await;
            release_bridge_generation(&generation);
            let commands = server.await.unwrap();
            let registration = commands
                .iter()
                .find(|c| c["method"] == "Page.addScriptToEvaluateOnNewDocument");
            assert_eq!(registration.is_some(), prepared && !superseded);
            if let Some(registration) = registration {
                let source = registration["params"]["source"].as_str().unwrap();
                assert!(source.contains("installModules([{"));
                assert!(source.contains("\"patchedSource\":\"patched-module\""));
                assert!(source.contains("\"url\":\"app://-/assets/composer-content-test.js\""));
                assert!(
                    source.contains(
                        "\"rewriters\":{\"fast\":\"patched\",\"predictions\":\"patched\"}"
                    )
                );
                assert!(source.contains("\"codexAppNativeFastUnlock\":true"));
                assert!(source.contains("\"codexAppNativePredictions\":true"));
            }
        }
    }

    #[derive(Clone, Copy)]
    enum Case {
        Patched,
        LazyUnmatched,
        Unsupported,
        BodyError,
        Remote,
        WrongMime,
        FulfillError,
        Interleaved,
    }

    fn paused(id: &str, url: &str, mime: &str) -> Value {
        json!({ "method": "Fetch.requestPaused", "params": {
            "requestId": id, "request": { "url": url }, "responseStatusCode": 200,
            "responseStatusText": "OK", "responseHeaders": [
                { "name": "Content-Type", "value": mime },
                { "name": "Content-Length", "value": "1234" },
                { "name": "content-encoding", "value": "gzip" },
                { "name": "Content-Security-Policy", "value": "script-src 'self'" },
            ]
        } })
    }

    async fn run_case(case: Case) -> Vec<Value> {
        run_case_with(case, module_settings(true, false, false)).await
    }

    async fn run_case_with(case: Case, settings: ModuleSettings) -> Vec<Value> {
        let prefixes = settings.prefixes();
        let expected_patterns: Vec<String> = prefixes
            .iter()
            .map(|prefix| format!("*{prefix}*.js*"))
            .collect();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let generation = next_bridge_generation(&format!("native-fast-{address}"));
        assert!(publish_bridge_generation(&generation));
        let server_generation = generation.clone();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
            let mut commands = Vec::new();
            let mut completed = 0;
            let mut interleaved = false;
            while let Some(Ok(Message::Text(text))) = socket.next().await {
                let command: Value = serde_json::from_str(&text).unwrap();
                let method = command["method"].as_str().unwrap();
                let mut result = json!({});
                let mut error = None;
                match method {
                    "Page.addScriptToEvaluateOnNewDocument" => {
                        result = json!({ "identifier": "native-fast-bootstrap" })
                    }
                    "Fetch.enable" => {
                        assert_eq!(command["params"]["patterns"][0]["requestStage"], "Response");
                        let patterns: Vec<&str> = command["params"]["patterns"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .map(|pattern| pattern["urlPattern"].as_str().unwrap())
                            .collect();
                        assert_eq!(patterns, expected_patterns);
                        assert!(
                            command["params"]["patterns"][0]
                                .get("resourceType")
                                .is_none()
                        );
                        let url = if matches!(case, Case::Remote) {
                            "https://example.com/assets/app-initial-a.js"
                        } else if matches!(case, Case::LazyUnmatched) {
                            "app://-/assets/general-settings-other.js"
                        } else {
                            "app://-/assets/app-initial-a.js"
                        };
                        let mime = if matches!(case, Case::WrongMime) {
                            "text/html"
                        } else {
                            "text/javascript; charset=utf-8"
                        };
                        // 在 enable 回包前到达，验证 wait_for_id 不能丢失暂停请求。
                        socket
                            .send(Message::Text(paused("first", url, mime).to_string().into()))
                            .await
                            .unwrap();
                    }
                    "Fetch.getResponseBody" => {
                        if matches!(case, Case::BodyError) {
                            error = Some(json!({ "code": -32000, "message": "body unavailable" }));
                        } else {
                            result = json!({ "body": base64::engine::general_purpose::STANDARD.encode("original-source"), "base64Encoded": true });
                        }
                        if matches!(case, Case::Interleaved) && !interleaved {
                            interleaved = true;
                            socket
                                .send(Message::Text(
                                    paused(
                                        "second",
                                        "app://-/assets/app-initial-a.js",
                                        "application/javascript",
                                    )
                                    .to_string()
                                    .into(),
                                ))
                                .await
                                .unwrap();
                        }
                    }
                    "Runtime.evaluate" => {
                        if command["params"]["expression"]
                            .as_str()
                            .unwrap()
                            .contains("?.transformModule?.(")
                        {
                            assert_eq!(command["params"]["awaitPromise"], true);
                            assert_eq!(command["params"]["returnByValue"], true);
                            result = json!({ "result": { "value": if matches!(case, Case::Unsupported | Case::LazyUnmatched) { json!({ "ok": false }) } else { json!({ "ok": true, "patchedSource": "patched-source" }) } } });
                        }
                    }
                    "Fetch.fulfillRequest" => {
                        let source = String::from_utf8(
                            base64::engine::general_purpose::STANDARD
                                .decode(command["params"]["body"].as_str().unwrap())
                                .unwrap(),
                        )
                        .unwrap();
                        assert_eq!(source, format!("patched-source{COMPILED_MARKER}"));
                        assert_eq!(command["params"]["responseCode"], 200);
                        assert_eq!(command["params"]["responsePhrase"], "OK");
                        let headers = command["params"]["responseHeaders"].as_array().unwrap();
                        assert_eq!(headers.len(), 2);
                        assert!(
                            headers
                                .iter()
                                .any(|header| header["name"] == "Content-Security-Policy")
                        );
                        if matches!(case, Case::FulfillError) {
                            error =
                                Some(json!({ "code": -32000, "message": "fulfill unavailable" }));
                        } else {
                            completed += 1;
                        }
                    }
                    "Fetch.continueResponse" => {
                        if matches!(case, Case::BodyError) {
                            error = Some(
                                json!({ "code": -32601, "message": "continueResponse unavailable" }),
                            );
                        } else {
                            completed += 1;
                        }
                    }
                    "Fetch.continueRequest" => completed += 1,
                    "Runtime.enable"
                    | "Page.enable"
                    | "Fetch.disable"
                    | "Page.removeScriptToEvaluateOnNewDocument" => {}
                    other => panic!("unexpected command {other}"),
                }
                let reply = if let Some(error) = error {
                    json!({ "id": command["id"], "error": error })
                } else {
                    json!({ "id": command["id"], "result": result })
                };
                socket
                    .send(Message::Text(reply.to_string().into()))
                    .await
                    .unwrap();
                commands.push(command);
                let expected = if matches!(case, Case::Interleaved) {
                    2
                } else {
                    1
                };
                if completed == expected {
                    release_bridge_generation(&server_generation);
                }
            }
            commands
        });
        let socket = connect_cdp_websocket(&format!("ws://{address}/devtools/page/native-fast"))
            .await
            .unwrap();
        let mut session = CdpSession::new(socket);
        let mut state = InterceptorState::default();
        tokio::time::timeout(Duration::from_secs(5), async {
            initialize(
                &mut session,
                &mut state,
                &bootstrap_script(settings),
                &prefixes,
            )
            .await
            .unwrap();
            watch(&mut session, &generation, &prefixes).await.unwrap();
            cleanup(&mut session, &generation, &state).await;
        })
        .await
        .unwrap();
        let commands = server.await.unwrap();
        assert!(
            commands
                .iter()
                .any(|command| command["method"] == "Fetch.disable")
        );
        assert!(
            commands
                .iter()
                .any(|command| command["method"] == "Page.removeScriptToEvaluateOnNewDocument")
        );
        assert!(
            !commands
                .iter()
                .any(|command| command["method"].as_str().unwrap().starts_with("Debugger."))
        );
        commands
    }

    #[test]
    fn asset_scope_is_local_and_bootstrap_contains_only_feature_settings() {
        assert!(is_local_asset(
            "app://-/assets/app-initial-a.js?x=1",
            FAST_PREFIXES
        ));
        assert!(is_local_asset(
            "file:///Applications/ChatGPT.app/Contents/Resources/assets/app-initial-a.js",
            FAST_PREFIXES
        ));
        for url in [
            "app://-/assets/composer-content-a1.js",
            "app://-/assets/initialize-app-host-services-b2.js",
            "app://-/assets/general-settings-c3.js",
        ] {
            assert!(is_local_asset(url, PREDICTIONS_PREFIXES), "{url}");
            assert!(!is_local_asset(url, FAST_PREFIXES), "{url}");
        }
        for url in [
            "app://-/assets/composer-content-.js",
            "app://-/assets/composer-content-a/b.js",
            "https://example.com/assets/composer-content-a.js",
            "app://-/assets/general-settings-a.css",
        ] {
            assert!(!is_local_asset(url, PREDICTIONS_PREFIXES), "{url}");
        }
        assert_eq!(
            module_settings(true, false, false).prefixes(),
            FAST_PREFIXES
        );
        assert_eq!(
            module_settings(false, false, true).prefixes(),
            PREDICTIONS_PREFIXES
        );
        assert_eq!(
            module_settings(true, false, true).prefixes(),
            PREDICTIONS_PREFIXES
        );
        assert!(module_settings(false, false, false).prefixes().is_empty());
        for url in [
            "https://example.com/assets/app-initial-a.js",
            "file://remote/assets/app-initial-a.js",
            "http://127.0.0.1/assets/app-initial-a.js",
            "app://-/assets/app-shared-a.js",
            "app://-/assets/app-initial-a.js.json",
        ] {
            assert!(!is_local_asset(url, PREDICTIONS_PREFIXES), "{url}");
        }
        let bootstrap = bootstrap_script(module_settings(false, true, true));
        let (_, update) = bootstrap
            .rsplit_once("\n;window.__codexPlusNativeFastUnlock.runtime.update(")
            .unwrap();
        let (settings, _) = update.split_once(");\n").unwrap();
        // 核对完整字段和值，不依赖 serde_json 的对象字段序列化顺序。
        assert_eq!(
            serde_json::from_str::<Value>(settings).unwrap(),
            json!({
                "codexAppNativeFastUnlock": false,
                "enhancementsEnabled": true,
                "nativeFastLocalApiConfigured": true,
            })
        );
        let (_, update) = bootstrap
            .rsplit_once("\n;window.__codexPlusNativePredictions.runtime.update(")
            .unwrap();
        let (settings, _) = update.split_once(");\n").unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(settings).unwrap(),
            json!({
                "codexAppNativePredictions": true,
                "enhancementsEnabled": true,
                "nativePredictionsLocalApiConfigured": true,
            })
        );
        assert!(bootstrap.contains("__codexPlusNativeFastPatchState='armed'"));
    }

    #[tokio::test]
    async fn queued_pause_is_patched_and_stale_generation_cleans_up() {
        let commands = run_case(Case::Patched).await;
        assert_eq!(
            commands
                .iter()
                .filter(|command| command["method"] == "Fetch.fulfillRequest")
                .count(),
            1
        );
        assert!(commands.iter().any(|command| {
            command["params"]["expression"]
                .as_str()
                .is_some_and(|expression| {
                    expression == "window.__codexPlusNativeFastPatchState=\"patched\""
                })
        }));
    }

    #[tokio::test]
    async fn predictions_intercept_lazy_chunks_and_unmatched_variants_are_released_quietly() {
        let commands = run_case_with(Case::Patched, module_settings(false, false, true)).await;
        assert_eq!(
            commands
                .iter()
                .filter(|command| command["method"] == "Fetch.fulfillRequest")
                .count(),
            1
        );
        let commands =
            run_case_with(Case::LazyUnmatched, module_settings(false, false, true)).await;
        assert!(
            commands
                .iter()
                .any(|command| command["method"] == "Fetch.continueResponse")
        );
        // 同前缀的其它懒加载块不含目标结构是正常情况，不能把整个通道标成 unsupported。
        assert!(!commands.iter().any(|command| {
            command["params"]["expression"]
                .as_str()
                .is_some_and(|expression| {
                    expression.starts_with("window.__codexPlusNativeFastPatchState=")
                })
        }));
    }

    #[tokio::test]
    async fn pauses_received_while_waiting_for_body_are_not_lost_or_duplicated() {
        let commands = run_case(Case::Interleaved).await;
        assert_eq!(
            commands
                .iter()
                .filter(|command| command["method"] == "Fetch.fulfillRequest")
                .count(),
            2
        );
    }

    #[tokio::test]
    async fn mismatched_assets_and_transform_or_transport_failures_release_response() {
        for case in [
            Case::Unsupported,
            Case::BodyError,
            Case::Remote,
            Case::WrongMime,
            Case::FulfillError,
        ] {
            let commands = run_case(case).await;
            assert!(
                commands
                    .iter()
                    .any(|command| command["method"] == "Fetch.continueResponse")
            );
            if matches!(case, Case::Remote | Case::WrongMime) {
                assert!(
                    !commands
                        .iter()
                        .any(|command| command["method"] == "Fetch.getResponseBody")
                );
            }
            if matches!(case, Case::BodyError) {
                assert!(
                    commands
                        .iter()
                        .any(|command| command["method"] == "Fetch.continueRequest")
                );
            }
            let states: Vec<&str> = commands
                .iter()
                .filter_map(|command| command["params"]["expression"].as_str())
                .filter(|expression| {
                    expression.starts_with("window.__codexPlusNativeFastPatchState=")
                })
                .collect();
            if matches!(case, Case::Remote) {
                assert!(states.is_empty());
            } else {
                let expected = if matches!(case, Case::Unsupported | Case::WrongMime) {
                    "unsupported"
                } else {
                    "failed"
                };
                assert!(states.contains(
                    &format!("window.__codexPlusNativeFastPatchState={}", json!(expected)).as_str()
                ));
            }
        }
    }

    /// 仅对调用者提供的独立测试 Chromium 页面运行，不能指向正在使用的 Codex 页面。
    /// FILE 是加载 assets/app-initial-*.js 的本地 HTML，需提供 Mxo/S$s 的最小依赖。
    #[tokio::test]
    #[ignore = "requires an isolated Chromium page and local native Fast fixture"]
    async fn real_chromium_file_response_is_patched_and_opt_out_restores_gates() {
        let (Ok(websocket_url), Ok(file)) = (
            std::env::var("CODEX_PLUS_NATIVE_FAST_TEST_WS"),
            std::env::var("CODEX_PLUS_NATIVE_FAST_TEST_FILE"),
        ) else {
            return;
        };
        let file_url = reqwest::Url::from_file_path(file).expect("absolute fixture HTML path");
        let generation = next_bridge_generation(&format!("native-fast-smoke-{websocket_url}"));
        assert!(publish_bridge_generation(&generation));
        let socket = connect_cdp_websocket(&websocket_url).await.unwrap();
        let mut interceptor = CdpSession::new(socket);
        let mut state = InterceptorState::default();
        initialize(
            &mut interceptor,
            &mut state,
            &bootstrap_script(module_settings(true, false, false)),
            FAST_PREFIXES,
        )
        .await
        .unwrap();
        let watching_generation = generation.clone();
        let watcher = tokio::spawn(async move {
            let result = watch(&mut interceptor, &watching_generation, FAST_PREFIXES).await;
            cleanup(&mut interceptor, &watching_generation, &state).await;
            result
        });
        let result: anyhow::Result<()> = async {
            let socket = connect_cdp_websocket(&websocket_url).await?;
            let mut control = CdpSession::new(socket);
            let navigation = command(
                &mut control,
                "Page.navigate",
                json!({ "url": file_url.as_str() }),
            )
            .await?;
            anyhow::ensure!(navigation.pointer("/result/errorText").is_none(), "fixture navigation failed");
            tokio::time::timeout(Duration::from_secs(12), async {
                loop {
                    if let Ok(reply) = evaluate(
                        &mut control,
                        "globalThis.__codexPlusNativeFastCompiled==='v1'&&typeof Mxo==='function'&&typeof S$s==='function'",
                    ).await {
                        if reply.pointer("/result/result/value").and_then(Value::as_bool) == Some(true) {
                            return Ok::<(), anyhow::Error>(());
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
            }).await.context("compiled marker was not observed")??;
            let enabled = evaluate(
                &mut control,
                "(async()=>Mxo().serviceTierAccess?.fast===true&&(await S$s(globalThis.proofScope,'local'))?.fast===true)()",
            ).await?;
            anyhow::ensure!(enabled.pointer("/result/result/value").and_then(Value::as_bool) == Some(true), "API key gates did not open");
            let disabled = evaluate(
                &mut control,
                "(async()=>{window.__codexPlusNativeFastUnlock.runtime.update({enhancementsEnabled:true,codexAppNativeFastUnlock:false});return Mxo().serviceTierAccess===null&&await S$s(globalThis.proofScope,'local')===null})()",
            ).await?;
            anyhow::ensure!(disabled.pointer("/result/result/value").and_then(Value::as_bool) == Some(true), "opt-out did not restore original API key gates");
            control.close().await;
            Ok(())
        }.await;
        release_bridge_generation(&generation);
        let watch_result = tokio::time::timeout(Duration::from_secs(6), watcher)
            .await
            .unwrap()
            .unwrap();
        watch_result.unwrap();
        result.unwrap();
    }
}
