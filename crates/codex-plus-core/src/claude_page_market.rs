//! Claude 页面扩展市场：和 ClaudeMom 读同一份市场目录——BigPizzaV3/claude-mom-extensions 的 dist 分支上的
//! market.json（官方审核的和打了 topic 的社区扩展）。按条目里固定的提交逐个下载清单列出的文件，
//! 再交给 claude-local 的 `extensions::import` 校验安装。GitHub 读不到时走 jsDelivr。
//! 规则与 ClaudeMom 的 market.rs 一致；这边的区别只是请求走 Codex++ 的代理设置。

use std::path::Path;
use std::time::Duration;

use anyhow::{Context, Result, bail, ensure};
use claude_local::extensions::{Imported, parse_manifest, valid_id};
use serde::{Deserialize, Serialize};

const MARKET_REPO: &str = "BigPizzaV3/claude-mom-extensions";
/// 一个扩展最多下载这么多字节（和导入扩展包的上限一致）。
const MAX_BYTES: usize = 20 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(8);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MarketEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// `official`（官方仓库里审核过的）或 `community`（作者自己的仓库，未审核）。
    pub source: String,
    /// `owner/name`。
    pub repo: String,
    /// 扩展在仓库里的文件夹；社区扩展在仓库根上，为空。
    #[serde(default)]
    pub path: String,
    /// 收录时的提交：装的就是这一版。
    pub commit: String,
    pub url: String,
    #[serde(default)]
    pub stars: Option<u64>,
}

fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let segment = |part: Option<&str>| {
        part.is_some_and(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
    };
    segment(parts.next()) && segment(parts.next()) && parts.next().is_none()
}

/// 仓库里的文件夹：空，或几段由字母数字、`_`、`-`、`.` 组成且不以 `.` 开头的路径。
fn valid_path(path: &str) -> bool {
    path.is_empty()
        || path.split('/').all(|segment| {
            segment.bytes().next().is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && segment.bytes().all(|b| b.is_ascii_alphanumeric() || b"_.-".contains(&b))
        })
}

impl MarketEntry {
    /// 拼进下载地址的字段都要合规：目录被改过或下载地址被劫持时，也出不了这个仓库的这个提交。
    fn valid(&self) -> bool {
        valid_id(&self.id)
            && matches!(self.source.as_str(), "official" | "community")
            && valid_repo(&self.repo)
            && valid_path(&self.path)
            && self.commit.len() == 40
            && self.commit.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    }

    /// 仓库里一个文件的下载地址：先 GitHub，再 jsDelivr。
    fn file_urls(&self, file: &str) -> [String; 2] {
        let relative = if self.path.is_empty() { file.to_string() } else { format!("{}/{file}", self.path) };
        [
            format!("https://raw.githubusercontent.com/{}/{}/{relative}", self.repo, self.commit),
            format!("https://cdn.jsdelivr.net/gh/{}@{}/{relative}", self.repo, self.commit),
        ]
    }
}

/// 依次试各个地址，返回第一个成功的内容；都失败时把每个的原因合在一起。
async fn get(urls: &[String]) -> Result<Vec<u8>> {
    let mut errors = Vec::new();
    for url in urls {
        let attempt = async {
            let client = crate::http_client::proxied_client_for_url_with_timeouts(
                "CodexPlusPlus/claude-page-market",
                url,
                CONNECT_TIMEOUT,
                TOTAL_TIMEOUT,
            )?;
            let response = client.get(url).send().await?;
            let status = response.status();
            ensure!(status.is_success(), "{status}");
            let body = response.bytes().await?;
            ensure!(body.len() <= MAX_BYTES, "文件太大");
            anyhow::Ok(body.to_vec())
        };
        match attempt.await {
            Ok(body) => return Ok(body),
            Err(error) => errors.push(format!("{url}：{error:#}")),
        }
    }
    bail!("{}", errors.join("；"))
}

fn parse_index(body: &[u8]) -> Result<Vec<MarketEntry>> {
    #[derive(Deserialize)]
    struct Index {
        extensions: Vec<serde_json::Value>,
    }
    let index: Index = serde_json::from_slice(body).context("扩展市场的目录格式不对")?;
    Ok(index
        .extensions
        .into_iter()
        .filter_map(|value| serde_json::from_value::<MarketEntry>(value).ok())
        .filter(MarketEntry::valid)
        .collect())
}

/// 市场里的扩展：官方在前，社区按星数（目录里已排好）。不合规的条目直接丢掉。
pub async fn fetch() -> Result<Vec<MarketEntry>> {
    let urls = [
        format!("https://raw.githubusercontent.com/{MARKET_REPO}/dist/market.json"),
        format!("https://cdn.jsdelivr.net/gh/{MARKET_REPO}@dist/market.json"),
    ];
    parse_index(&get(&urls).await.context("读不到扩展市场")?)
}

/// 把条目对应的扩展下载到 `to`：先清单（id、版本要和目录里的一致），再清单列出的文件。
async fn download(entry: &MarketEntry, to: &Path) -> Result<()> {
    ensure!(entry.valid(), "市场条目不合规");
    let raw = get(&entry.file_urls("manifest.json")).await.context("下载 manifest.json 失败")?;
    let manifest = parse_manifest(&raw, Some(&entry.id))?;
    ensure!(
        manifest.version == entry.version,
        "仓库里的版本（{}）和市场目录里的（{}）对不上",
        manifest.version,
        entry.version
    );
    std::fs::write(to.join("manifest.json"), &raw)?;
    let mut total = raw.len();
    for file in manifest.scripts.iter().chain(&manifest.styles) {
        let body = get(&entry.file_urls(file)).await.with_context(|| format!("下载 {file} 失败"))?;
        total += body.len();
        ensure!(total <= MAX_BYTES, "扩展太大：最多 {} MB", MAX_BYTES / 1024 / 1024);
        let target = to.join(file);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(target, body)?;
    }
    Ok(())
}

/// 下载并安装（同 id 的直接换成这一版）。下载到临时文件夹，装完删掉。
pub async fn install(entry: &MarketEntry) -> Result<Imported> {
    let staging = std::env::temp_dir().join(format!("codex-plus-claude-ext-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&staging)?;
    let result = async {
        download(entry, &staging).await?;
        let dir = staging.clone();
        tokio::task::spawn_blocking(move || crate::claude_local_backend::import_page_extension(&dir)).await?
    }
    .await;
    let _ = std::fs::remove_dir_all(&staging);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> MarketEntry {
        MarketEntry {
            id: "wide-chat".into(),
            name: "加宽对话区".into(),
            version: "1.0.0".into(),
            author: String::new(),
            description: String::new(),
            source: "official".into(),
            repo: "BigPizzaV3/claude-mom-extensions".into(),
            path: "extensions/wide-chat".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            url: String::new(),
            stars: None,
        }
    }

    #[test]
    fn entries_must_pin_a_repo_and_commit_that_stay_inside_github() {
        assert!(entry().valid());
        assert_eq!(entry().file_urls("css/a.css"), [
            "https://raw.githubusercontent.com/BigPizzaV3/claude-mom-extensions/0123456789abcdef0123456789abcdef01234567/extensions/wide-chat/css/a.css",
            "https://cdn.jsdelivr.net/gh/BigPizzaV3/claude-mom-extensions@0123456789abcdef0123456789abcdef01234567/extensions/wide-chat/css/a.css",
        ]);
        let community = MarketEntry { source: "community".into(), repo: "a/b".into(), path: String::new(), ..entry() };
        assert_eq!(community.file_urls("x.js")[0], "https://raw.githubusercontent.com/a/b/0123456789abcdef0123456789abcdef01234567/x.js");

        let bad = [
            MarketEntry { id: "Wide".into(), ..entry() },
            MarketEntry { source: "other".into(), ..entry() },
            MarketEntry { repo: "a/../b".into(), ..entry() },
            MarketEntry { repo: "a".into(), ..entry() },
            MarketEntry { repo: "a/b?x=1".into(), ..entry() },
            MarketEntry { path: "../x".into(), ..entry() },
            MarketEntry { path: "a//b".into(), ..entry() },
            MarketEntry { path: "/abs".into(), ..entry() },
            MarketEntry { commit: "main".into(), ..entry() },
            MarketEntry { commit: "0123456789ABCDEF0123456789abcdef01234567".into(), ..entry() },
        ];
        for item in bad {
            assert!(!item.valid(), "{item:?}");
        }
    }

    #[test]
    fn index_drops_entries_that_do_not_validate() {
        let good = serde_json::to_value(entry()).unwrap();
        let mut bad = good.clone();
        bad["commit"] = "main".into();
        let body = serde_json::json!({ "version": 1, "extensions": [good, bad, { "id": 1 }] }).to_string();
        let entries = parse_index(body.as_bytes()).unwrap();
        assert_eq!(entries, [entry()]);
        assert!(parse_index(b"[]").is_err());
    }
}
