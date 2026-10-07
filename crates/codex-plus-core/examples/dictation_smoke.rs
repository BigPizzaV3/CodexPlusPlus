//! 隔离的真实 renderer 听写 smoke：只启动 helper 并注入已启动的 CDP 页面。
//!
//! 原生 app 和本地 mock ASR 由测试调用方启动。本程序不启动/退出 app，
//! 不同步供应商，不处理凭据，不覆盖已有 settings 文件，不改系统环境变量。
//! 默认注入后运行 300 秒；创建 stop marker 可提前正常关闭 helper。

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use codex_plus_core::launcher::{DefaultLaunchHooks, LaunchHooks};
use codex_plus_core::routes::{BridgeContext, CoreRuntimeService, handle_bridge_request};
use codex_plus_core::settings::{BackendSettings, DictationSettings};
use codex_plus_core::status::StatusStore;

struct Options {
    debug_port: u16,
    helper_port: u16,
    settings_path: PathBuf,
    asr_base_url: String,
    duration_seconds: u64,
    stop_file: Option<PathBuf>,
}

impl Options {
    fn parse() -> Result<Option<Self>> {
        let mut args = std::env::args().skip(1);
        let mut debug_port = None;
        let mut helper_port = None;
        let mut settings_path = None;
        let mut asr_base_url = None;
        let mut duration_seconds = 300;
        let mut stop_file = None;
        while let Some(flag) = args.next() {
            if matches!(flag.as_str(), "--help" | "-h") {
                println!(
                    "dictation_smoke --debug-port N --helper-port N --settings-path ABS_PATH \
                     --asr-base-url http://127.0.0.1:N/v1 \
                     [--duration-seconds 1..3600] [--stop-file ABS_PATH]\n\
                     The settings file and stop marker must not already exist."
                );
                return Ok(None);
            }
            let value = args
                .next()
                .with_context(|| format!("missing value for {flag}"))?;
            match flag.as_str() {
                "--debug-port" => debug_port = Some(value.parse::<u16>()?),
                "--helper-port" => helper_port = Some(value.parse::<u16>()?),
                "--settings-path" => settings_path = Some(PathBuf::from(value)),
                "--asr-base-url" => asr_base_url = Some(value),
                "--duration-seconds" => duration_seconds = value.parse::<u64>()?,
                "--stop-file" => stop_file = Some(PathBuf::from(value)),
                _ => bail!("unknown argument {flag}"),
            }
        }
        let options = Self {
            debug_port: debug_port.context("--debug-port is required")?,
            helper_port: helper_port.context("--helper-port is required")?,
            settings_path: settings_path.context("--settings-path is required")?,
            asr_base_url: asr_base_url.context("--asr-base-url is required")?,
            duration_seconds,
            stop_file,
        };
        ensure!(
            options.debug_port != 0
                && options.helper_port != 0
                && options.debug_port != options.helper_port,
            "debug/helper ports must be nonzero and different"
        );
        ensure!(
            (1..=3600).contains(&options.duration_seconds),
            "--duration-seconds must be in 1..3600"
        );
        let asr = reqwest::Url::parse(&options.asr_base_url)?;
        ensure!(
            matches!(asr.scheme(), "http" | "https")
                && matches!(
                    asr.host_str(),
                    Some("127.0.0.1" | "localhost" | "::1" | "[::1]")
                )
                && asr.username().is_empty()
                && asr.password().is_none()
                && asr.query().is_none()
                && asr.fragment().is_none(),
            "smoke ASR must be a loopback HTTP(S) URL without credentials/query/fragment"
        );
        let bind = std::env::var("CODEX_PLUS_HELPER_BIND").unwrap_or_default();
        ensure!(
            matches!(bind.trim(), "" | "127.0.0.1"),
            "smoke helper must bind to 127.0.0.1, matching the renderer helper URL"
        );
        Ok(Some(options))
    }
}

fn new_fixture_path(path: &Path, directory: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "fixture paths must be absolute");
    let parent = path
        .parent()
        .context("fixture path has no parent directory")?
        .canonicalize()?;
    ensure!(
        parent == directory,
        "fixture must be in the settings directory"
    );
    let path = parent.join(path.file_name().context("fixture path has no file name")?);
    match std::fs::symlink_metadata(&path) {
        Ok(_) => bail!("refusing to overwrite existing fixture {}", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(error) => Err(error.into()),
    }
}

fn tiny_wav() -> Vec<u8> {
    // 8 帧 8kHz 单声道 16bit 静音，合法的 60 字节 WAV。
    let mut audio = Vec::with_capacity(60);
    audio.extend_from_slice(b"RIFF");
    audio.extend_from_slice(&52_u32.to_le_bytes());
    audio.extend_from_slice(b"WAVEfmt ");
    audio.extend_from_slice(&16_u32.to_le_bytes());
    audio.extend_from_slice(&1_u16.to_le_bytes());
    audio.extend_from_slice(&1_u16.to_le_bytes());
    audio.extend_from_slice(&8000_u32.to_le_bytes());
    audio.extend_from_slice(&16000_u32.to_le_bytes());
    audio.extend_from_slice(&2_u16.to_le_bytes());
    audio.extend_from_slice(&16_u16.to_le_bytes());
    audio.extend_from_slice(b"data");
    audio.extend_from_slice(&16_u32.to_le_bytes());
    audio.extend_from_slice(&[0; 16]);
    audio
}

async fn helper_http_self_check(debug_port: u16, helper_port: u16) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(15))
        .build()?;
    let helper_base = format!("http://127.0.0.1:{helper_port}");
    let response = client
        .get(format!("{helper_base}/dictation/status"))
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "HTTP status self-check failed"
    );
    let public_status: serde_json::Value = response.json().await?;
    ensure!(
        public_status.is_object()
            && public_status.get("helperToken").is_none()
            && public_status.get("apiKey").is_none(),
        "HTTP status must not expose dictation credentials or capability token"
    );
    ensure!(
        public_status
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            && public_status
                .get("configured")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
        "isolated dictation fixture must be enabled and configured"
    );
    let denied = client
        .post(format!("{helper_base}/dictation/transcribe"))
        .body(Vec::<u8>::new())
        .send()
        .await?;
    ensure!(
        denied.status() == reqwest::StatusCode::FORBIDDEN,
        "transcription without a capability token must return 403"
    );

    // 直接走 renderer 使用的 privileged route；token 只留在内存且不打印。
    let ctx = BridgeContext::core(Arc::new(CoreRuntimeService::new(
        debug_port,
        StatusStore::default(),
    )));
    let bridge_status =
        handle_bridge_request(ctx, "/dictation/status", serde_json::json!({})).await;
    let token = bridge_status
        .get("helperToken")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .context("privileged dictation status did not supply a capability token")?;
    let audio = tiny_wav();
    let boundary = "cpp-dictation-smoke-fixture";
    let mut multipart = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"smoke.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .into_bytes();
    multipart.extend_from_slice(&audio);
    multipart.extend_from_slice(
        format!(
            "\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\nzh\r\n--{boundary}--\r\n"
        )
        .as_bytes(),
    );
    let response = client
        .post(format!("{helper_base}/dictation/transcribe"))
        .header("X-Codex-Plus-Dictation-Token", token)
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("multipart/form-data; boundary={boundary}"),
        )
        .body(multipart)
        .send()
        .await?;
    ensure!(
        response.status().is_success(),
        "authorized transcription self-check returned HTTP {}",
        response.status()
    );
    let result: serde_json::Value = response.json().await?;
    let text = result
        .get("text")
        .and_then(serde_json::Value::as_str)
        .context("authorized transcription self-check did not return text")?;
    ensure!(
        text == "本地语音链路测试成功",
        "mock transcription text did not match"
    );
    println!(
        "dictation HTTP self-check passed: public_status_safe=true, unauthorized_status=403, audio_bytes={}, language=zh, model=mock-whisper, text_chars={}",
        audio.len(),
        text.chars().count()
    );
    Ok(())
}

async fn bridge_dictation_self_check(debug_port: u16) -> Result<()> {
    let ctx = BridgeContext::core(Arc::new(CoreRuntimeService::new(
        debug_port,
        StatusStore::default(),
    )));
    let status =
        handle_bridge_request(ctx.clone(), "/dictation/status", serde_json::json!({})).await;
    let token = status
        .get("helperToken")
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.is_empty())
        .context("bridge dictation status did not supply a capability token")?;
    let audio = tiny_wav();
    let request_id = uuid::Uuid::new_v4().to_string();
    let response = handle_bridge_request(
        ctx.clone(),
        "/dictation/transcribe",
        serde_json::json!({
            "requestId": request_id,
            "helperToken": token,
            "audioBase64": base64::engine::general_purpose::STANDARD.encode(&audio),
            "mimeType": "audio/wav",
            "filename": "smoke.wav",
            "language": "zh"
        }),
    )
    .await;
    let text = response
        .get("text")
        .and_then(serde_json::Value::as_str)
        .context("bridge transcription self-check did not return text")?;
    ensure!(
        text == "本地语音链路测试成功",
        "bridge mock transcription text did not match"
    );
    let cancelled = handle_bridge_request(
        ctx,
        "/dictation/cancel",
        serde_json::json!({"requestId": request_id, "helperToken": token}),
    )
    .await;
    ensure!(
        cancelled.get("status").and_then(serde_json::Value::as_str) == Some("ok")
            && cancelled
                .get("cancelled")
                .and_then(serde_json::Value::as_bool)
                == Some(false),
        "cancelling a completed bridge transcription must be a safe no-op"
    );
    println!(
        "dictation bridge self-check passed: audio_bytes={}, language=zh, model=mock-whisper, text_chars={}, completed_cancelled=false",
        audio.len(),
        text.chars().count()
    );
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let Some(options) = Options::parse()? else {
        return Ok(());
    };
    ensure!(
        options.settings_path.is_absolute(),
        "settings path must be absolute"
    );
    let directory = options
        .settings_path
        .parent()
        .context("settings path has no parent")?;
    std::fs::create_dir_all(directory)?;
    let directory = directory.canonicalize()?;
    let settings_path = new_fixture_path(&options.settings_path, &directory)?;
    let stop_file = new_fixture_path(
        &options
            .stop_file
            .unwrap_or_else(|| directory.join("dictation-smoke.stop")),
        &directory,
    )?;
    ensure!(
        stop_file != settings_path,
        "stop marker must differ from settings path"
    );
    // 不恢复 settings 路径：bridge 后台任务持续到退出，设置始终读取独立 fixture。
    codex_plus_core::paths::set_settings_path_for_tests(Some(settings_path.clone()));
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(
        directory.join("dictation-smoke.log"),
    ));
    let settings = BackendSettings {
        provider_sync_enabled: false,
        relay_profiles_enabled: false,
        relay_profiles: Vec::new(),
        codex_app_plugin_marketplace_unlock: false,
        codex_app_model_whitelist_unlock: false,
        dictation: DictationSettings {
            enabled: true,
            base_url: options.asr_base_url,
            api_key: "fake-asr-key".to_string(),
            api_key_env: String::new(),
            model: "mock-whisper".to_string(),
            language: String::new(),
            timeout_seconds: 30,
        },
        ..BackendSettings::default()
    };
    // 用 create_new 原子拒绝覆盖，避免通用 SettingsStore 保存保护读取旧配置。
    let mut fixture_options = std::fs::OpenOptions::new();
    fixture_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fixture_options.mode(0o600);
    }
    let mut fixture = fixture_options.open(&settings_path)?;
    fixture.write_all(&serde_json::to_vec_pretty(&settings)?)?;
    fixture.sync_all()?;

    let hooks = DefaultLaunchHooks::default();
    hooks.start_helper(options.helper_port).await?;
    let result = async {
        helper_http_self_check(options.debug_port, options.helper_port).await?;
        bridge_dictation_self_check(options.debug_port).await?;
        // inject 注册 CoreRuntime bridge；与 helper 同进程共享听写 capability token。
        tokio::time::timeout(
            Duration::from_secs(60),
            hooks.inject(options.debug_port, options.helper_port),
        )
        .await
        .context("CDP injection did not complete within 60 seconds")??;
        println!(
            "dictation smoke ready: debug={}, helper={}, settings={}, duration={}s, stop_file={}",
            options.debug_port,
            options.helper_port,
            settings_path.display(),
            options.duration_seconds,
            stop_file.display()
        );
        let deadline = tokio::time::Instant::now() + Duration::from_secs(options.duration_seconds);
        loop {
            if stop_file.try_exists()? {
                println!("dictation smoke stopping: marker found");
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                println!("dictation smoke stopping: duration elapsed");
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }
    .await;
    hooks.shutdown_helper(options.helper_port).await;
    result
}
