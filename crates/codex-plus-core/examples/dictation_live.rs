//! Groq 真实服务联调：读取用户经管理器保存的听写配置，不写 settings。
//!
//! 测试音频必须是调用方新生成的非敏感 WAV；程序不会生成音频或启动原生 app。
//! 输出只含识别文字、字节数、耗时和已去敏失败原因。密钥和 helper token 仅在进程内使用。

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use base64::Engine;
use codex_plus_core::launcher::{DefaultLaunchHooks, LaunchHooks};
use codex_plus_core::models::{DeleteResult, ExportResult, SessionRef};
use codex_plus_core::routes::{
    BridgeContext, BridgeDataService, BridgeSettingsService, CoreRuntimeService,
    handle_bridge_request,
};
use codex_plus_core::settings::{BackendSettings, DictationSettings, SettingsStore};
use codex_plus_core::status::StatusStore;
use serde_json::{Value, json};

// 探针固定使用首次校验过的听写快照；并发修改管理器设置不能切换这次 ASR 请求。
struct ReadOnlyDictationSettings {
    dictation: DictationSettings,
}

#[async_trait::async_trait]
impl BridgeSettingsService for ReadOnlyDictationSettings {
    async fn get_settings(&self) -> Result<BackendSettings> {
        Ok(BackendSettings {
            dictation: self.dictation.clone(),
            ..BackendSettings::default()
        })
    }

    async fn set_settings(&self, _payload: Value) -> Result<BackendSettings> {
        bail!("live probe settings are read-only")
    }
}

struct NoProbeData;

#[async_trait::async_trait]
impl BridgeDataService for NoProbeData {
    async fn delete(&self, _session: SessionRef) -> Result<DeleteResult> {
        bail!("data operations are unavailable in the live probe")
    }
    async fn undo(&self, _undo_token: String) -> Result<DeleteResult> {
        bail!("data operations are unavailable in the live probe")
    }
    async fn export_markdown(&self, _session: SessionRef) -> Result<ExportResult> {
        bail!("data operations are unavailable in the live probe")
    }
    async fn thread_usage_history(&self, _session: SessionRef) -> Result<Value> {
        bail!("data operations are unavailable in the live probe")
    }
    async fn find_archived_thread_by_title(&self, _title: String) -> Result<Option<SessionRef>> {
        bail!("data operations are unavailable in the live probe")
    }
}

struct Options {
    debug_port: u16,
    helper_port: u16,
    settings_path: PathBuf,
    audio_file: PathBuf,
    diagnostic_directory: PathBuf,
    language: String,
    duration_seconds: u64,
    stop_file: PathBuf,
}

impl Options {
    fn parse() -> Result<Option<Self>> {
        let mut args = std::env::args().skip(1);
        let mut debug_port = None;
        let mut helper_port = None;
        let mut settings_path = None;
        let mut audio_file = None;
        let mut diagnostic_directory = None;
        let mut language = None;
        let mut duration_seconds = None;
        let mut stop_file = None;
        while let Some(flag) = args.next() {
            if matches!(flag.as_str(), "--help" | "-h") {
                println!(
                    "dictation_live --debug-port N --helper-port N \
                     --settings-path EXISTING_ABS_PATH --audio-file NEW_TEST_WAV_ABS_PATH \
                     --diagnostic-directory NEW_TEMP_ABS_PATH --language en \
                     --duration-seconds 1..3600 --stop-file ABS_PATH_IN_DIAGNOSTIC_DIRECTORY\n\
                     Only loads existing settings; does not save settings or launch the app.\n\
                     The diagnostic directory must be new. Creating the stop file ends the run."
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
                "--audio-file" => audio_file = Some(PathBuf::from(value)),
                "--diagnostic-directory" => diagnostic_directory = Some(PathBuf::from(value)),
                "--language" => language = Some(value),
                "--duration-seconds" => duration_seconds = Some(value.parse::<u64>()?),
                "--stop-file" => stop_file = Some(PathBuf::from(value)),
                _ => bail!("unknown argument {flag}"),
            }
        }
        let options = Self {
            debug_port: debug_port.context("--debug-port is required")?,
            helper_port: helper_port.context("--helper-port is required")?,
            settings_path: settings_path.context("--settings-path is required")?,
            audio_file: audio_file.context("--audio-file is required")?,
            diagnostic_directory: diagnostic_directory
                .context("--diagnostic-directory is required")?,
            language: language.context("--language is required")?,
            duration_seconds: duration_seconds.context("--duration-seconds is required")?,
            stop_file: stop_file.context("--stop-file is required")?,
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
        ensure!(
            options.language.len() == 2
                && options
                    .language
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase()),
            "--language must be a two-letter lowercase ISO-639-1 code"
        );
        let bind = std::env::var("CODEX_PLUS_HELPER_BIND").unwrap_or_default();
        ensure!(
            matches!(bind.trim(), "" | "127.0.0.1"),
            "live helper must bind to 127.0.0.1"
        );
        Ok(Some(options))
    }
}

fn read_test_wav(path: &Path) -> Result<Vec<u8>> {
    ensure!(path.is_absolute(), "audio path must be absolute");
    ensure!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("wav")),
        "test audio must have a .wav extension"
    );
    let metadata = std::fs::symlink_metadata(path)?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "test audio must be a regular file, not a symlink"
    );
    let limit = codex_plus_core::dictation::MAX_AUDIO_BODY_BYTES;
    ensure!(metadata.len() <= limit as u64, "test audio exceeds 25MiB");
    let mut audio = Vec::new();
    std::fs::File::open(path)?
        .take(limit as u64 + 1)
        .read_to_end(&mut audio)?;
    ensure!(audio.len() <= limit, "test audio exceeds 25MiB");
    ensure!(
        audio.len() > 44 && &audio[..4] == b"RIFF" && &audio[8..12] == b"WAVE",
        "test audio must contain WAV audio data"
    );
    Ok(audio)
}

fn create_diagnostic_directory(path: &Path) -> Result<PathBuf> {
    ensure!(path.is_absolute(), "diagnostic directory must be absolute");
    let parent = path
        .parent()
        .context("diagnostic directory has no parent")?
        .canonicalize()?;
    let mut temporary_roots = vec![std::env::temp_dir().canonicalize()?];
    #[cfg(unix)]
    if let Ok(path) = Path::new("/tmp").canonicalize() {
        temporary_roots.push(path);
    }
    ensure!(
        temporary_roots.iter().any(|root| parent.starts_with(root)),
        "diagnostic directory must be inside a temporary directory"
    );
    // create_dir 拒绝使用已有目录，日志不会覆盖之前的联调文件。
    let directory = parent.join(
        path.file_name()
            .context("diagnostic directory has no name")?,
    );
    std::fs::create_dir(&directory)?;
    directory.canonicalize().map_err(Into::into)
}

async fn transcribe_test_audio(
    debug_port: u16,
    audio: &[u8],
    language: &str,
    timeout_seconds: u64,
    dictation: DictationSettings,
) -> Result<()> {
    let ctx = BridgeContext::new(
        Arc::new(ReadOnlyDictationSettings { dictation }),
        Arc::new(CoreRuntimeService::new(debug_port, StatusStore::default())),
        Arc::new(NoProbeData),
    );
    let status = handle_bridge_request(ctx.clone(), "/dictation/status", json!({})).await;
    let token = status
        .get("helperToken")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .context("dictation bridge did not supply a capability token")?;
    let request_id = uuid::Uuid::new_v4().to_string();
    let started = Instant::now();
    let response = tokio::time::timeout(
        Duration::from_secs(timeout_seconds.saturating_add(5)),
        handle_bridge_request(
            ctx.clone(),
            "/dictation/transcribe",
            json!({
                "requestId": request_id,
                "helperToken": token,
                "audioBase64": base64::engine::general_purpose::STANDARD.encode(audio),
                "mimeType": "audio/wav",
                "filename": "dictation-live-test.wav",
                "language": language
            }),
        ),
    )
    .await;
    let response = match response {
        Ok(response) => response,
        Err(_) => {
            let _ = handle_bridge_request(
                ctx,
                "/dictation/cancel",
                json!({"requestId": request_id, "helperToken": token}),
            )
            .await;
            bail!("test transcription timed out");
        }
    };
    if response.get("status").and_then(Value::as_str) == Some("failed") {
        // 此路由的 message 已由后端去敏；不能打印完整 response 或配置。
        let message = response
            .get("message")
            .and_then(Value::as_str)
            .filter(|message| !message.is_empty())
            .unwrap_or("语音转写失败");
        bail!("{message}");
    }
    let text = response
        .get("text")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .context("test transcription did not return nonempty text")?;
    println!("transcript: {text}");
    println!(
        "audio_bytes={}, elapsed_ms={}",
        audio.len(),
        started.elapsed().as_millis()
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
    ensure!(
        std::fs::metadata(&options.settings_path)?.is_file(),
        "settings must be an existing file"
    );
    let settings_path = options.settings_path.canonicalize()?;
    let settings = SettingsStore::new(settings_path.clone())
        .load()
        .map_err(|_| anyhow::anyhow!("saved settings could not be loaded"))?;
    let service = reqwest::Url::parse(&settings.dictation.base_url)
        .map_err(|_| anyhow::anyhow!("dictation service URL is invalid"))?;
    ensure!(
        service.as_str().trim_end_matches('/') == "https://api.groq.com/openai/v1",
        "live test requires the official Groq transcription service"
    );
    let status = codex_plus_core::dictation::public_status(&settings.dictation);
    ensure!(
        status.get("enabled").and_then(Value::as_bool) == Some(true)
            && status.get("configured").and_then(Value::as_bool) == Some(true),
        "save an enabled, configured dictation service in the manager first"
    );
    let dictation_snapshot = settings.dictation.clone();
    let timeout_seconds = dictation_snapshot.timeout_seconds;
    drop(settings);
    let audio = read_test_wav(&options.audio_file)?;
    let diagnostic_directory = create_diagnostic_directory(&options.diagnostic_directory)?;
    ensure!(
        options.stop_file.is_absolute(),
        "stop file must be absolute"
    );
    ensure!(
        options
            .stop_file
            .parent()
            .context("stop file has no parent")?
            .canonicalize()?
            == diagnostic_directory,
        "stop file must be inside the diagnostic directory"
    );
    let stop_file = diagnostic_directory.join(
        options
            .stop_file
            .file_name()
            .context("stop file has no name")?,
    );
    let log_path = diagnostic_directory.join("dictation-live.log");
    ensure!(
        stop_file != log_path,
        "stop marker must differ from log file"
    );
    ensure!(
        std::fs::symlink_metadata(&stop_file)
            .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound),
        "stop marker must not already exist"
    );
    let mut log_options = std::fs::OpenOptions::new();
    log_options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        log_options.mode(0o600);
    }
    drop(log_options.open(&log_path)?);
    // 仅更改当前进程的读取路径，不创建、保存或更新 settings。
    codex_plus_core::paths::set_settings_path_for_tests(Some(settings_path.clone()));
    ensure!(
        codex_plus_core::paths::default_settings_path() == settings_path,
        "settings read path was not installed"
    );
    codex_plus_core::diagnostic_log::set_diagnostic_log_path_for_tests(Some(log_path));

    let hooks = DefaultLaunchHooks::default();
    hooks.start_helper(options.helper_port).await?;
    let result = async {
        transcribe_test_audio(
            options.debug_port,
            &audio,
            &options.language,
            timeout_seconds,
            dictation_snapshot,
        )
        .await?;
        tokio::time::timeout(
            Duration::from_secs(60),
            hooks.inject(options.debug_port, options.helper_port),
        )
        .await
        .context("CDP injection did not complete within 60 seconds")??;
        println!("ready_duration_seconds={}", options.duration_seconds);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(options.duration_seconds);
        while !stop_file.try_exists()? && tokio::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        Ok(())
    }
    .await;
    hooks.shutdown_helper(options.helper_port).await;
    result
}
