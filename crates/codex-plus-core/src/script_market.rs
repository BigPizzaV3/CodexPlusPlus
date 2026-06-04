use anyhow::Context;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::user_scripts::UserScriptManager;

pub const DEFAULT_MARKET_INDEX_URL: &str =
    "https://raw.githubusercontent.com/BigPizzaV3/CodexPlusPlusScriptMarket/main/index.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ScriptMarketManifest {
    pub version: u64,
    pub updated_at: Option<String>,
    pub scripts: Vec<MarketScript>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketScript {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub homepage: String,
    pub script_url: String,
    #[serde(default)]
    pub sha256: String,
}

pub fn parse_market_manifest(raw: Value) -> anyhow::Result<ScriptMarketManifest> {
    let version = raw.get("version").and_then(Value::as_u64).unwrap_or(1);
    let updated_at = raw
        .get("updated_at")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    let scripts = raw
        .get("scripts")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(parse_market_script)
        .collect();

    Ok(ScriptMarketManifest {
        version,
        updated_at,
        scripts,
    })
}

pub async fn fetch_market_manifest(url: &str) -> anyhow::Result<ScriptMarketManifest> {
    let raw = reqwest::get(url)
        .await
        .with_context(|| format!("failed to request script market index {url}"))?
        .error_for_status()
        .with_context(|| format!("script market index returned an error status {url}"))?
        .json::<Value>()
        .await
        .context("failed to decode script market index JSON")?;
    parse_market_manifest(raw)
}

pub async fn download_script(url: &str) -> anyhow::Result<Vec<u8>> {
    Ok(reqwest::get(url)
        .await
        .with_context(|| format!("failed to request script {url}"))?
        .error_for_status()
        .with_context(|| format!("script download returned an error status {url}"))?
        .bytes()
        .await
        .context("failed to read script download body")?
        .to_vec())
}

pub fn install_market_script_content(
    manager: &UserScriptManager,
    script: &MarketScript,
    content: &[u8],
) -> anyhow::Result<()> {
    // 先校验完整性再落盘:市场脚本是注入页面执行的 JS,内容哈希是其唯一防篡改手段。
    // 校验失败立即返回,不写文件、不记录安装,保留已有版本不变。
    verify_script_checksum(script, content)?;
    let path = manager.user_script_path_for_market_id(&script.id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create user script directory {}",
                parent.display()
            )
        })?;
    }
    crate::settings::atomic_write(&path, content)
        .with_context(|| format!("failed to write script {}", path.display()))?;
    manager.record_market_install(script)?;
    Ok(())
}

pub async fn install_market_script(
    manager: &UserScriptManager,
    script: &MarketScript,
) -> anyhow::Result<()> {
    let content = download_script(&script.script_url).await?;
    install_market_script_content(manager, script, &content)
}

/// 比对下载内容与清单声明的 sha256。
///
/// 清单未提供 sha256(字段为空)时跳过校验以兼容无哈希的旧清单;提供时按
/// 大小写不敏感的十六进制比对,不匹配即返回错误。
fn verify_script_checksum(script: &MarketScript, content: &[u8]) -> anyhow::Result<()> {
    use sha2::{Digest, Sha256};

    let expected = script.sha256.trim();
    if expected.is_empty() {
        return Ok(());
    }
    let actual = hex::encode(Sha256::digest(content));
    if !actual.eq_ignore_ascii_case(expected) {
        anyhow::bail!(
            "脚本 {} 的 sha256 校验失败:清单声明 {expected},实际下载内容为 {actual}",
            script.id
        );
    }
    Ok(())
}

fn parse_market_script(raw: Value) -> Option<MarketScript> {
    let id = required_string(&raw, "id")?;
    let name = required_string(&raw, "name")?;
    let version = required_string(&raw, "version")?;
    let script_url = required_string(&raw, "script_url")?;
    Some(MarketScript {
        id,
        name,
        description: optional_string(&raw, "description"),
        version,
        author: optional_string(&raw, "author"),
        tags: raw
            .get("tags")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(ToOwned::to_owned)
                    .collect()
            })
            .unwrap_or_default(),
        homepage: optional_string(&raw, "homepage"),
        script_url,
        sha256: optional_string(&raw, "sha256"),
    })
}

fn required_string(raw: &Value, key: &str) -> Option<String> {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn optional_string(raw: &Value, key: &str) -> String {
    raw.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_string()
}
