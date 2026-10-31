use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[cfg(any(target_os = "macos", test))]
pub mod macos;

pub const DEFAULT_REPOSITORY: &str = "BigPizzaV3/CodexPlusPlus";
pub const DEFAULT_LATEST_JSON_URL: &str =
    "https://github.com/BigPizzaV3/CodexPlusPlus/releases/latest/download/latest.json";
const UPDATE_CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const UPDATE_DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub body: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateCheck {
    pub current_version: String,
    pub latest_version: Option<String>,
    pub release_summary: String,
    pub asset_name: Option<String>,
    pub asset_url: Option<String>,
    pub update_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct UpdateInstall {
    pub release: Release,
    pub installer_path: PathBuf,
    pub launched: bool,
}

/// 更新过程中可被前端消费的真实阶段。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateStage {
    Downloading,
    Writing,
    Launching,
}

/// 传给 UI 的进度事件；percent 仅在下载阶段且响应带 Content-Length 时存在。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub stage: UpdateStage,
    pub downloaded_bytes: u64,
    pub total_bytes: Option<u64>,
    pub percent: Option<f64>,
    pub elapsed_ms: u64,
}

pub fn parse_version_tag(value: &str) -> anyhow::Result<Vec<u64>> {
    let normalized = value.trim().trim_start_matches(['v', 'V']);
    let mut digits = String::new();
    for ch in normalized.chars() {
        if ch.is_ascii_digit() || ch == '.' {
            digits.push(ch);
        } else {
            break;
        }
    }
    if digits.is_empty() {
        anyhow::bail!("Invalid version tag: {value}");
    }
    digits
        .split('.')
        .map(|part| part.parse::<u64>().map_err(Into::into))
        .collect()
}

pub fn is_newer_version(candidate: &str, current: &str) -> anyhow::Result<bool> {
    let mut left = parse_version_tag(candidate)?;
    let mut right = parse_version_tag(current)?;
    let len = left.len().max(right.len());
    left.resize(len, 0);
    right.resize(len, 0);
    Ok(left > right)
}

pub fn release_from_github_payload(payload: &Value) -> anyhow::Result<Release> {
    let version = payload
        .get("tag_name")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("release payload missing tag_name"))?
        .to_string();
    let assets = payload
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            Some((
                asset.get("name")?.as_str()?.to_string(),
                asset.get("browser_download_url")?.as_str()?.to_string(),
            ))
        })
        .collect::<Vec<_>>();
    let selected = select_update_asset(&assets);
    Ok(Release {
        version,
        url: payload
            .get("html_url")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: payload
            .get("body")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        asset_name: selected.as_ref().map(|asset| asset.name.clone()),
        asset_url: selected.map(|asset| asset.browser_download_url),
    })
}

pub fn release_from_latest_json_payload(payload: &Value) -> anyhow::Result<Release> {
    let version = payload
        .get("version")
        .or_else(|| payload.get("tag_name"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow::anyhow!("latest.json missing version"))?
        .to_string();
    let assets = payload
        .get("assets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|asset| {
            let name = asset.get("name")?.as_str()?.to_string();
            let url = asset
                .get("url")
                .or_else(|| asset.get("browser_download_url"))?
                .as_str()?
                .to_string();
            Some((name, url))
        })
        .collect::<Vec<_>>();
    let selected = select_update_asset(&assets);
    Ok(Release {
        version,
        url: payload
            .get("url")
            .or_else(|| payload.get("html_url"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        body: payload
            .get("body")
            .or_else(|| payload.get("release_summary"))
            .or_else(|| payload.get("notes"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        asset_name: selected.as_ref().map(|asset| asset.name.clone()),
        asset_url: selected.map(|asset| asset.browser_download_url),
    })
}

pub fn select_update_asset(assets: &[(String, String)]) -> Option<ReleaseAsset> {
    let named = assets
        .iter()
        .filter(|(name, url)| !name.trim().is_empty() && !url.trim().is_empty());
    let mut best: Option<(u8, &str, &str)> = None;
    for (name, url) in named {
        let rank = platform_asset_rank(&name.to_ascii_lowercase());
        if rank >= 2 {
            continue;
        }
        if best.map_or(true, |(r, _, _)| rank < r) {
            best = Some((rank, name.as_str(), url.as_str()));
        }
    }
    best.map(|(_, name, url)| ReleaseAsset {
        name: name.to_string(),
        browser_download_url: url.to_string(),
    })
}

pub async fn fetch_latest_release(latest_json_url: &str) -> anyhow::Result<Release> {
    let client = update_http_client()?;
    let payload = client
        .get(latest_json_url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await?
        .error_for_status()?
        .json::<Value>()
        .await?;
    release_from_latest_json_payload(&payload)
}

pub async fn check_for_update(current_version: &str) -> anyhow::Result<UpdateCheck> {
    let release = fetch_latest_release(DEFAULT_LATEST_JSON_URL).await?;
    let update_available = is_newer_version(&release.version, current_version)?;
    Ok(UpdateCheck {
        current_version: current_version.to_string(),
        latest_version: Some(release.version),
        release_summary: release.body,
        asset_name: release.asset_name,
        asset_url: release.asset_url,
        update_available,
    })
}

pub async fn perform_update(
    release: &Release,
    download_dir: &Path,
) -> anyhow::Result<UpdateInstall> {
    perform_update_with_progress(release, download_dir, |_| {}).await
}

pub async fn perform_update_with_progress<F>(
    release: &Release,
    download_dir: &Path,
    mut on_progress: F,
) -> anyhow::Result<UpdateInstall>
where
    F: FnMut(UpdateProgress) + Send,
{
    let (installer_path, downloaded_bytes, total_bytes) =
        download_release_asset_with_progress(release, download_dir, &mut on_progress).await?;

    let launch_started = Instant::now();
    on_progress(UpdateProgress {
        stage: UpdateStage::Launching,
        downloaded_bytes,
        total_bytes,
        percent: None,
        elapsed_ms: 0,
    });
    #[cfg(target_os = "macos")]
    let launch_result = {
        let path = installer_path.clone();
        let version = release.version.clone();
        tokio::task::spawn_blocking(move || macos::launch_update(&path, &version))
            .await
            .map_err(|error| anyhow::anyhow!("准备 macOS 自动更新失败：{error}"))?
    };
    #[cfg(not(target_os = "macos"))]
    let launch_result = launch_installer(&installer_path);
    if let Err(error) = launch_result {
        let _ = crate::diagnostic_log::append_diagnostic_log(
            "update.launch.failed",
            json!({
                "version": release.version,
                "assetName": release.asset_name,
                "installerPath": installer_path.to_string_lossy(),
                "elapsedMs": launch_started.elapsed().as_millis() as u64,
                "error": error.to_string()
            }),
        );
        return Err(error);
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "update.launch.completed",
        json!({
            "version": release.version,
            "assetName": release.asset_name,
            "installerPath": installer_path.to_string_lossy(),
            "elapsedMs": launch_started.elapsed().as_millis() as u64
        }),
    );
    Ok(UpdateInstall {
        release: release.clone(),
        installer_path,
        launched: true,
    })
}

async fn download_release_asset_with_progress<F>(
    release: &Release,
    download_dir: &Path,
    on_progress: &mut F,
) -> anyhow::Result<(PathBuf, u64, Option<u64>)>
where
    F: FnMut(UpdateProgress),
{
    let url = release
        .asset_url
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("没有可下载的 Release asset"))?;
    let download_started = Instant::now();
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "update.perform.start",
        json!({
            "version": release.version,
            "assetName": release.asset_name,
            "assetUrl": url,
            "downloadTimeoutSeconds": UPDATE_DOWNLOAD_TIMEOUT.as_secs()
        }),
    );
    let response = match update_http_client()?.get(url).send().await {
        Ok(response) => response,
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "update.download.failed",
                json!({
                    "version": release.version,
                    "assetName": release.asset_name,
                    "elapsedMs": download_started.elapsed().as_millis() as u64,
                    "error": error.to_string()
                }),
            );
            return Err(anyhow::anyhow!("下载安装包失败：{error}"));
        }
    };
    let response = match response.error_for_status() {
        Ok(response) => response,
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "update.download.bad_status",
                json!({
                    "version": release.version,
                    "assetName": release.asset_name,
                    "elapsedMs": download_started.elapsed().as_millis() as u64,
                    "error": error.to_string()
                }),
            );
            return Err(anyhow::anyhow!("下载安装包失败：{error}"));
        }
    };

    // 用流式读取替代 response.bytes()，让 UI 能拿到真实下载字节数；
    // 仍先把完整安装包收集到内存，写盘行为与旧实现保持一致。
    let total_bytes = response.content_length();
    on_progress(UpdateProgress {
        stage: UpdateStage::Downloading,
        downloaded_bytes: 0,
        total_bytes,
        percent: total_bytes.filter(|total| *total > 0).map(|_| 0.0),
        elapsed_ms: 0,
    });
    let mut bytes = Vec::new();
    let mut downloaded_bytes = 0_u64;
    let mut last_percent_bucket = -1_i32;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = crate::diagnostic_log::append_diagnostic_log(
                    "update.download.body_failed",
                    json!({
                        "version": release.version,
                        "assetName": release.asset_name,
                        "downloadedBytes": downloaded_bytes,
                        "elapsedMs": download_started.elapsed().as_millis() as u64,
                        "error": error.to_string()
                    }),
                );
                return Err(anyhow::anyhow!("读取安装包失败：{error}"));
            }
        };
        downloaded_bytes += chunk.len() as u64;
        bytes.extend_from_slice(&chunk);
        let percent = total_bytes
            .filter(|total| *total > 0)
            .map(|total| (downloaded_bytes as f64 / total as f64 * 100.0).clamp(0.0, 100.0));
        let percent_bucket = percent.map(|value| value.floor() as i32).unwrap_or(-1);
        let finished_known_total = total_bytes.is_some_and(|total| downloaded_bytes >= total);
        if percent.is_none() || percent_bucket > last_percent_bucket || finished_known_total {
            last_percent_bucket = percent_bucket;
            on_progress(UpdateProgress {
                stage: UpdateStage::Downloading,
                downloaded_bytes,
                total_bytes,
                percent,
                elapsed_ms: download_started.elapsed().as_millis() as u64,
            });
        }
    }
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "update.download.completed",
        json!({
            "version": release.version,
            "assetName": release.asset_name,
            "bytes": bytes.len(),
            "totalBytes": total_bytes,
            "elapsedMs": download_started.elapsed().as_millis() as u64
        }),
    );

    on_progress(UpdateProgress {
        stage: UpdateStage::Writing,
        downloaded_bytes,
        total_bytes,
        percent: None,
        elapsed_ms: 0,
    });
    let write_started = Instant::now();
    let installer_path = match download_asset_to(release, &bytes, download_dir) {
        Ok(path) => path,
        Err(error) => {
            let _ = crate::diagnostic_log::append_diagnostic_log(
                "update.write.failed",
                json!({
                    "version": release.version,
                    "assetName": release.asset_name,
                    "downloadDir": download_dir.to_string_lossy(),
                    "bytes": bytes.len(),
                    "elapsedMs": write_started.elapsed().as_millis() as u64,
                    "error": error.to_string()
                }),
            );
            return Err(error);
        }
    };
    let _ = crate::diagnostic_log::append_diagnostic_log(
        "update.write.completed",
        json!({
            "version": release.version,
            "assetName": release.asset_name,
            "installerPath": installer_path.to_string_lossy(),
            "bytes": bytes.len(),
            "elapsedMs": write_started.elapsed().as_millis() as u64
        }),
    );
    Ok((installer_path, downloaded_bytes, total_bytes))
}

fn update_http_client() -> anyhow::Result<reqwest::Client> {
    // 更新与安装包下载同样要走系统代理：在需要代理才能访问 GitHub 的网络里，
    // 直连会一直超时，用户看到的是「检查更新成功但下载/安装失败」（#1329）。
    // 复用统一构造器，保留原有连接/总超时语义。
    crate::http_client::proxied_client_with_timeouts(
        &format!("Codex++/{}", crate::version::VERSION),
        UPDATE_CONNECT_TIMEOUT,
        UPDATE_DOWNLOAD_TIMEOUT,
    )
}

pub fn download_asset_to(
    release: &Release,
    bytes: &[u8],
    download_dir: &Path,
) -> anyhow::Result<PathBuf> {
    let name = release
        .asset_name
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("没有可下载的 Release asset"))?;
    let safe = safe_asset_name(name)?;
    std::fs::create_dir_all(download_dir)?;
    let path = download_dir.join(safe);
    std::fs::write(&path, bytes)?;
    // 历史安装包会一直留在 updates/ 里占磁盘（#2544）：每次下载成功后只保留
    // 最近 `UPDATE_KEEP_INSTALLERS` 个安装包（含本次），其余删除。清理失败
    // 不影响本次更新。
    prune_old_installers(download_dir, &path);
    Ok(path)
}

const UPDATE_KEEP_INSTALLERS: usize = 2;

fn prune_old_installers(download_dir: &Path, current: &Path) {
    let Ok(entries) = std::fs::read_dir(download_dir) else {
        return;
    };
    let mut installers: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path != current && is_installer_asset_path(path))
        .filter_map(|path| {
            let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    if installers.is_empty() {
        return;
    }
    installers.sort_by(|left, right| right.0.cmp(&left.0));
    for (_, stale) in installers
        .into_iter()
        .skip(UPDATE_KEEP_INSTALLERS.saturating_sub(1))
    {
        let _ = std::fs::remove_file(stale);
    }
}

fn is_installer_asset_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "exe" | "msi" | "dmg" | "pkg" | "appimage" | "deb" | "rpm"
            )
        })
}

pub fn safe_asset_name(name: &str) -> anyhow::Result<String> {
    if name.trim().is_empty() {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    let path = Path::new(name);
    if path.components().count() != 1 {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("非法 Release asset 文件名: {name}"))?;
    if file_name == "." || file_name == ".." {
        anyhow::bail!("非法 Release asset 文件名: {name}");
    }
    Ok(file_name.to_string())
}

fn platform_asset_rank(name: &str) -> u8 {
    // 0 = exact match (current OS + native arch)
    // 1 = same OS, other arch (acceptable fallback, e.g. x86_64 on arm64 or vice versa)
    // 2 = wrong platform
    if cfg!(target_os = "macos") {
        if !is_macos_installer_asset(name) {
            return 2;
        }
        if is_macos_native_arch_asset(name) {
            return 0;
        }
        return 1;
    }
    if cfg!(windows) && is_windows_installer_asset(name) {
        return 0;
    }
    if cfg!(target_os = "linux") {
        if !is_linux_installer_asset(name) {
            return 2;
        }
        if is_linux_native_arch_asset(name) {
            return 0;
        }
        return 1;
    }
    2
}

fn is_macos_native_arch_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let native_arch_token = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return true, // unknown arch — accept anything
    };
    // Modern filename shape: `...-macos-x64.dmg` or `...-macos-arm64.dmg`
    if lower.contains(&format!("-{native_arch_token}.")) {
        return true;
    }
    // Old filename shape: `CodexPlusPlus_1.0.9_x64.dmg`
    if lower.contains(&format!("_{native_arch_token}.")) {
        return true;
    }
    // Newer but alternative shape: `..._x64.dmg` (no `macos-` token)
    let other_token = if native_arch_token == "x64" {
        "arm64"
    } else {
        "x64"
    };
    if lower.contains(&format!("_{other_token}.")) || lower.contains(&format!("-{other_token}.")) {
        return false;
    }
    // No arch token at all — assume it matches the current arch.
    true
}

fn is_linux_installer_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.contains("codex")
        && lower.contains("plus")
        && (lower.ends_with(".deb")
            || lower.ends_with(".appimage")
            || lower.ends_with(".tar.gz"))
}

fn is_linux_native_arch_asset(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let native_tokens: &[&str] = match std::env::consts::ARCH {
        "x86_64" => &["amd64", "x86_64", "x64"],
        "aarch64" => &["arm64", "aarch64"],
        _ => return true,
    };
    native_tokens.iter().any(|token| lower.contains(token))
}

fn is_windows_installer_asset(name: &str) -> bool {
    name.contains("codex")
        && name.contains("plus")
        && (name.ends_with(".msi")
            || name.ends_with("-setup.exe")
            || name.ends_with("_setup.exe")
            || name.ends_with("setup.exe")
            || name.ends_with("installer.exe"))
}

fn is_macos_installer_asset(name: &str) -> bool {
    // Loose shape check; arch preference is handled by platform_asset_rank
    // via is_macos_native_arch_asset.
    name.contains("codex") && name.contains("plus") && name.ends_with(".dmg")
}

pub fn launch_installer(path: &Path) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        std::process::Command::new(path)
            .creation_flags(crate::windows_integration::CREATE_NO_WINDOW)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("启动安装包失败：{error}"))
    }

    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("打开 DMG 失败：{error}"))
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map(|_| ())
            .map_err(|error| anyhow::anyhow!("打开安装包失败：{error}"))
    }

    #[cfg(all(not(windows), not(target_os = "macos"), not(target_os = "linux")))]
    {
        let _ = path;
        anyhow::bail!("当前平台不支持启动安装包")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    #[test]
    fn prune_old_installers_keeps_only_recent_installers() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("CodexPlusPlus-9.9.9.exe");
        std::fs::write(&current, b"current").unwrap();
        for (index, name) in [
            "old-1.exe",
            "old-2.dmg",
            "old-3.exe",
            "notes.txt",
        ]
        .iter()
        .enumerate()
        {
            let path = directory.path().join(name);
            std::fs::write(&path, b"x").unwrap();
            let stamp = std::time::SystemTime::now()
                - std::time::Duration::from_secs(100 - index as u64);
            let file = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.set_modified(stamp).unwrap();
        }
        prune_old_installers(directory.path(), &current);
        assert!(current.exists(), "本次下载的安装包必须保留");
        let remaining: Vec<String> = std::fs::read_dir(directory.path())
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .collect();
        // 时间戳：old-3 最新，old-2 次之，old-1 最旧。只保留当前 + 最新的 1 个。
        assert!(
            remaining.iter().any(|name| name == "old-3.exe"),
            "最新的历史安装包应保留：{remaining:?}"
        );
        assert!(
            !remaining.iter().any(|name| name == "old-1.exe" || name == "old-2.dmg"),
            "更旧的安装包应被清理：{remaining:?}"
        );
        assert!(
            remaining.iter().any(|name| name == "notes.txt"),
            "非安装包文件不得删除：{remaining:?}"
        );
    }

    #[tokio::test]
    async fn download_stage_progress_reports_bytes_then_writing() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = b"fake-installer-payload-for-test".to_vec();
        let body_for_server = body.clone();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body_for_server.len()
            );
            stream.write_all(header.as_bytes()).unwrap();
            let split = body_for_server.len() / 2;
            stream.write_all(&body_for_server[..split]).unwrap();
            stream.flush().unwrap();
            std::thread::sleep(Duration::from_millis(5));
            stream.write_all(&body_for_server[split..]).unwrap();
        });

        let release = Release {
            version: "v9.9.9".to_string(),
            url: String::new(),
            body: String::new(),
            asset_name: Some("CodexPlusPlus-9.9.9-windows-x64-setup.exe".to_string()),
            asset_url: Some(format!("http://{address}/installer.exe")),
        };
        let directory = tempfile::tempdir().unwrap();
        let observed = Arc::new(Mutex::new(Vec::<UpdateProgress>::new()));
        let observed_for_callback = observed.clone();
        let (written_path, downloaded, total) = download_release_asset_with_progress(
            &release,
            directory.path(),
            &mut |progress| observed_for_callback.lock().unwrap().push(progress),
        )
        .await
        .unwrap();
        server.join().unwrap();

        assert_eq!(downloaded, body.len() as u64);
        assert_eq!(total, Some(body.len() as u64));
        assert_eq!(std::fs::read(&written_path).unwrap(), body);
        let observed = observed.lock().unwrap();
        assert_eq!(observed.first().map(|progress| progress.stage), Some(UpdateStage::Downloading));
        assert!(observed
            .iter()
            .any(|progress| progress.stage == UpdateStage::Downloading && progress.downloaded_bytes > 0));
        assert_eq!(observed.last().map(|progress| progress.stage), Some(UpdateStage::Writing));
    }
}
