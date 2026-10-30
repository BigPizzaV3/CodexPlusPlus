//! 精选清单维护：本机 gh 账号对市场仓库有写权限时（维护者），在管理器里审核社区拓展并写进精选清单，
//! 以及审核精选拓展的新版本。审核的就是将要锁定的那个提交：社区安装的 prepare 把它下载、校验、暂存，
//! 这里在暂存的包上列文件、扫密钥，通过后把暂存元数据里的 repo / tag / sha 写进清单。
//!
//! 清单写入只改 `packages` 段和 `updated_at`，其余内容（老的单文件脚本、下架名单）按原文保留，
//! 每次写入是市场仓库里的一个提交，提交说明写明仓库与 sha。

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, bail};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::extension_discovery::CommunityEntry;
use crate::extension_publish::{Publisher, package_files, scan_secrets};
use crate::script_market::OFFICIAL_OWNERS;
use crate::user_scripts::EXTENSION_MANIFEST_FILE;

/// 精选清单所在的仓库、文件与分支。
pub const MARKET_REPO: &str = "BigPizzaV3/CodexPlusPlusScriptMarket";
pub const MARKET_INDEX_PATH: &str = "index.json";
pub const MARKET_BRANCH: &str = "main";

/// 差异里单个文件、整体的补丁上限，超出截断（界面只用来过目，完整差异去 GitHub 看）。
const PATCH_FILE_LIMIT: usize = 24 * 1024;
const PATCH_TOTAL_LIMIT: usize = 256 * 1024;

/// 精选清单 `packages` 里的一条。字段顺序即写进清单的顺序；不认识的字段原样保留。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CuratedEntry {
    #[serde(default = "package_kind")]
    pub kind: String,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub author: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub homepage: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub icon: String,
    #[serde(default)]
    pub updated_at: String,
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub sha: String,
    #[serde(default)]
    pub official: bool,
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

fn package_kind() -> String {
    "package".into()
}

/// 仓库能不能标「官方」：所有者在 [`OFFICIAL_OWNERS`] 里。
pub fn can_be_official(repo: &str) -> bool {
    let owner = repo.split('/').next().unwrap_or_default();
    OFFICIAL_OWNERS.iter().any(|official| official.eq_ignore_ascii_case(owner))
}

/// 由暂存元数据、包清单和社区缓存条目拼出精选条目。名称、描述、图标优先用社区条目（图标已是完整地址）。
/// 审核通过只算「精选」；`official` 要维护者明确勾选，且仓库属于官方账号才生效。
pub fn curated_entry(meta: &Value, manifest: &Value, community: Option<&CommunityEntry>, today: &str, official: bool) -> anyhow::Result<CuratedEntry> {
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::trim).filter(|text| !text.is_empty()).map(str::to_string);
    let repo = text(meta, "repo").context("暂存元数据缺少 repo")?;
    let id = text(meta, "id").context("暂存元数据缺少 id")?;
    let sha = text(meta, "sha").context("暂存元数据缺少 sha")?.to_lowercase();
    if sha.len() != 40 || !sha.chars().all(|ch| ch.is_ascii_hexdigit()) {
        bail!("提交 sha 无效：{sha}");
    }
    let owner = repo.split('/').next().unwrap_or_default();
    let tags = manifest
        .get("keywords")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .or_else(|| community.map(|entry| entry.keywords.clone()))
        .unwrap_or_default();
    Ok(CuratedEntry {
        kind: package_kind(),
        name: text(manifest, "name").or_else(|| community.map(|entry| entry.name.clone())).unwrap_or_else(|| id.clone()),
        description: text(manifest, "description").or_else(|| community.map(|entry| entry.description.clone())).unwrap_or_default(),
        version: text(meta, "version").or_else(|| text(manifest, "version")).unwrap_or_default(),
        author: text(manifest, "author").unwrap_or_else(|| owner.to_string()),
        tags,
        homepage: text(manifest, "homepage").unwrap_or_else(|| format!("https://github.com/{repo}")),
        icon: community.map(|entry| entry.icon.clone()).unwrap_or_default(),
        updated_at: today.to_string(),
        tag: text(meta, "tag"),
        official: official && can_be_official(&repo),
        id,
        repo,
        sha,
        extra: BTreeMap::new(),
    })
}

/// 把条目写进清单文本：同仓库的条目替换，没有就追加。返回新文本与是否替换了旧条目。
/// 同 id 已被另一个仓库占用时拒绝。只改 `packages` 段与 `updated_at`，其余按原文保留。
pub fn upsert_package(index_text: &str, entry: &CuratedEntry, now: &str) -> anyhow::Result<(String, bool)> {
    let raw: Value = serde_json::from_str(index_text).context("精选清单不是合法 JSON")?;
    if !raw.is_object() {
        bail!("精选清单顶层不是对象");
    }
    // 混在 scripts 里的拓展包条目（kind: "package"）这里不改写，撞上了就请人工处理。
    let mixed = raw.get("scripts").and_then(Value::as_array).into_iter().flatten().filter(|item| item.get("kind").and_then(Value::as_str) == Some("package"));
    for item in mixed {
        let repo = item.get("repo").and_then(Value::as_str).unwrap_or_default();
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default();
        if repo.eq_ignore_ascii_case(&entry.repo) || id == entry.id {
            bail!("{id}（{repo}）写在清单的 scripts 里，请先手动把它移到 packages");
        }
    }
    let mut packages: Vec<CuratedEntry> = match raw.get("packages") {
        Some(value) => serde_json::from_value(value.clone()).context("清单的 packages 格式不对")?,
        None => Vec::new(),
    };
    if let Some(taken) = packages.iter().find(|item| item.id == entry.id && !item.repo.eq_ignore_ascii_case(&entry.repo)) {
        bail!("精选里已有 id 为 {} 的拓展（{}），不能再收同 id 的 {}", entry.id, taken.repo, entry.repo);
    }
    let replaced = match packages.iter_mut().find(|item| item.repo.eq_ignore_ascii_case(&entry.repo)) {
        Some(existing) => {
            // 保留维护者在清单里手加的字段。
            let extra = std::mem::take(&mut existing.extra);
            *existing = CuratedEntry { extra, ..entry.clone() };
            true
        }
        None => {
            packages.push(entry.clone());
            false
        }
    };
    let array = indent_continuation(&serde_json::to_string_pretty(&packages)?, "  ");
    let mut text = match top_level_value_span(index_text, "packages") {
        Some((start, end)) => format!("{}{}{}", &index_text[..start], array, &index_text[end..]),
        None => insert_top_level(index_text, "packages", &array)?,
    };
    if let Some((start, end)) = top_level_value_span(&text, "updated_at") {
        text = format!("{}{}{}", &text[..start], serde_json::to_string(now)?, &text[end..]);
    }
    // 写完再解析一遍，确保没把清单写坏。
    serde_json::from_str::<Value>(&text).context("写入后的清单不是合法 JSON")?;
    Ok((text, replaced))
}

/// 多行文本除第一行外每行加前缀（把数组嵌进顶层对象时对齐缩进）。
fn indent_continuation(text: &str, prefix: &str) -> String {
    text.lines().enumerate().map(|(index, line)| if index == 0 { line.to_string() } else { format!("{prefix}{line}") }).collect::<Vec<_>>().join("\n")
}

/// 在顶层对象末尾加一个键。
fn insert_top_level(text: &str, key: &str, value: &str) -> anyhow::Result<String> {
    let close = text.rfind('}').context("精选清单缺少结尾的 }")?;
    let body = text[..close].trim_end();
    let separator = if body.ends_with('{') { "" } else { "," };
    Ok(format!("{body}{separator}\n  {}: {value}\n{}", serde_json::to_string(key)?, &text[close..]))
}

/// 顶层对象里某个键的值在原文里的字节区间。
fn top_level_value_span(text: &str, key: &str) -> Option<(usize, usize)> {
    let bytes = text.as_bytes();
    let mut index = skip_whitespace(bytes, 0);
    if bytes.get(index) != Some(&b'{') {
        return None;
    }
    index += 1;
    loop {
        index = skip_whitespace(bytes, index);
        match bytes.get(index)? {
            b'}' => return None,
            b',' => {
                index += 1;
                continue;
            }
            b'"' => {}
            _ => return None,
        }
        let key_end = string_end(bytes, index)?;
        let name: String = serde_json::from_str(&text[index..key_end]).ok()?;
        index = skip_whitespace(bytes, key_end);
        if bytes.get(index) != Some(&b':') {
            return None;
        }
        let start = skip_whitespace(bytes, index + 1);
        let end = value_end(bytes, start)?;
        if name == key {
            return Some((start, end));
        }
        index = end;
    }
}

fn skip_whitespace(bytes: &[u8], mut index: usize) -> usize {
    while bytes.get(index).is_some_and(|byte| byte.is_ascii_whitespace()) {
        index += 1;
    }
    index
}

/// `start` 处字符串的结束位置（右引号之后）。
fn string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut index = start + 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => index += 2,
            b'"' => return Some(index + 1),
            _ => index += 1,
        }
    }
    None
}

/// `start` 处 JSON 值的结束位置。
fn value_end(bytes: &[u8], start: usize) -> Option<usize> {
    match bytes.get(start)? {
        b'"' => string_end(bytes, start),
        b'{' | b'[' => {
            let mut depth = 0usize;
            let mut index = start;
            while index < bytes.len() {
                match bytes[index] {
                    b'"' => {
                        index = string_end(bytes, index)?;
                        continue;
                    }
                    b'{' | b'[' => depth += 1,
                    b'}' | b']' => {
                        depth -= 1;
                        if depth == 0 {
                            return Some(index + 1);
                        }
                    }
                    _ => {}
                }
                index += 1;
            }
            None
        }
        _ => {
            let mut index = start;
            while index < bytes.len() && !matches!(bytes[index], b',' | b'}' | b']') && !bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            Some(index)
        }
    }
}

/// 暂存包的审核信息：文件列表、密钥扫描。权限已在 prepare 的结果里。
pub fn inspect_staged(dir: &Path) -> Value {
    let mut problems = Vec::new();
    let files = package_files(dir, &mut problems);
    let secrets = scan_secrets(dir, &files);
    json!({
        "files": files.iter().map(|(path, bytes)| json!({ "path": path, "bytes": bytes })).collect::<Vec<_>>(),
        "bytes": files.iter().map(|(_, bytes)| bytes).sum::<u64>(),
        "secrets": secrets,
        "problems": problems,
    })
}

/// 读暂存包的清单。
pub fn staged_manifest(dir: &Path) -> anyhow::Result<Value> {
    Ok(serde_json::from_slice(&std::fs::read(dir.join(EXTENSION_MANIFEST_FILE))?)?)
}

/// 两次提交间的差异（GitHub compare 接口的结果），补丁按上限截断。
pub fn trim_compare(raw: &Value) -> Value {
    let mut total = 0usize;
    let files = raw
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|file| {
            let patch = file.get("patch").and_then(Value::as_str).unwrap_or_default();
            let budget = PATCH_FILE_LIMIT.min(PATCH_TOTAL_LIMIT.saturating_sub(total));
            let cut = if patch.len() > budget { floor_char_boundary(patch, budget) } else { patch.len() };
            total += cut;
            json!({
                "filename": file.get("filename").cloned().unwrap_or(Value::Null),
                "status": file.get("status").cloned().unwrap_or(Value::Null),
                "additions": file.get("additions").cloned().unwrap_or(json!(0)),
                "deletions": file.get("deletions").cloned().unwrap_or(json!(0)),
                "patch": &patch[..cut],
                "truncated": cut < patch.len() || (patch.is_empty() && file.get("patch").is_none() && file.get("changes").and_then(Value::as_u64).unwrap_or(0) > 0),
            })
        })
        .collect::<Vec<_>>();
    json!({
        "commits": raw.get("total_commits").cloned().unwrap_or(json!(0)),
        "url": raw.get("html_url").cloned().unwrap_or(Value::Null),
        "files": files,
    })
}

/// Unix 秒 → `2026-10-30T08:00:00.000Z`（清单 `updated_at` 的格式）。
pub fn utc_iso(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rest = secs % 86_400;
    // 天数 → 公历日期（Howard Hinnant 的 civil_from_days）。
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}.000Z", rest / 3_600, rest % 3_600 / 60, rest % 60)
}

pub fn now_iso() -> String {
    utc_iso(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|time| time.as_secs()).unwrap_or(0))
}

fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// 用本机 gh 读写市场仓库。
pub struct Curator {
    pub publisher: Publisher,
    pub market_repo: String,
}

impl Curator {
    pub fn new(publisher: Publisher) -> Self {
        Self { publisher, market_repo: MARKET_REPO.into() }
    }

    /// 当前 gh 账号与是否对市场仓库有写权限。
    pub fn status(&self) -> anyhow::Result<Value> {
        let account = self.publisher.account()?;
        let push = self.publisher.gh(&["api", &format!("repos/{}", self.market_repo), "--jq", ".permissions.push"])?;
        Ok(json!({ "account": account, "maintainer": push.trim() == "true", "marketRepo": self.market_repo }))
    }

    pub fn require_maintainer(&self) -> anyhow::Result<String> {
        let status = self.status()?;
        if status["maintainer"] != true {
            bail!("gh 账号 {} 对 {} 没有写权限", status["account"].as_str().unwrap_or(""), self.market_repo);
        }
        Ok(status["account"].as_str().unwrap_or_default().to_string())
    }

    /// gh 的令牌，给社区栏的 GitHub 请求用（限额高）。只在内存里用，不落盘。
    pub fn token(&self) -> Option<String> {
        self.publisher.gh(&["auth", "token"]).ok().map(|token| token.trim().to_string()).filter(|token| !token.is_empty())
    }

    /// 读清单：原文与文件 sha（写回时做并发校验）。
    pub fn read_index(&self) -> anyhow::Result<(String, String)> {
        let raw = self.publisher.gh(&["api", &format!("repos/{}/contents/{MARKET_INDEX_PATH}?ref={MARKET_BRANCH}", self.market_repo)])?;
        let value: Value = serde_json::from_str(&raw).context("读精选清单失败")?;
        let sha = value.get("sha").and_then(Value::as_str).context("精选清单缺少 sha")?.to_string();
        let encoded = value.get("content").and_then(Value::as_str).context("精选清单缺少内容")?.replace(['\n', '\r'], "");
        let bytes = base64::engine::general_purpose::STANDARD.decode(encoded).context("精选清单内容解码失败")?;
        Ok((String::from_utf8(bytes).context("精选清单不是 UTF-8")?, sha))
    }

    /// 写清单。`base_sha` 是读到时的文件 sha，期间被别人改过时 GitHub 会拒绝。返回提交地址。
    pub fn write_index(&self, text: &str, base_sha: &str, message: &str) -> anyhow::Result<String> {
        let content = base64::engine::general_purpose::STANDARD.encode(text.as_bytes());
        let raw = self.publisher.gh(&[
            "api",
            "-X",
            "PUT",
            &format!("repos/{}/contents/{MARKET_INDEX_PATH}", self.market_repo),
            "-f",
            &format!("message={message}"),
            "-f",
            &format!("content={content}"),
            "-f",
            &format!("sha={base_sha}"),
            "-f",
            &format!("branch={MARKET_BRANCH}"),
        ])?;
        let value: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
        Ok(value.pointer("/commit/html_url").and_then(Value::as_str).unwrap_or_default().to_string())
    }

    /// 把条目写进清单并提交。返回提交地址、是否更新了旧条目、写入后的清单原文。
    pub fn publish_entry(&self, entry: &CuratedEntry, now: &str) -> anyhow::Result<Value> {
        self.require_maintainer()?;
        let (text, base_sha) = self.read_index()?;
        let (next, replaced) = upsert_package(&text, entry, now)?;
        let short = &entry.sha[..7.min(entry.sha.len())];
        let version = entry.tag.clone().unwrap_or_else(|| entry.version.clone());
        let message = if replaced {
            format!("curate: 更新 {} 到 {version}（{}@{short}）", entry.id, entry.repo)
        } else {
            format!("curate: 收录 {} {version}（{}@{short}）", entry.id, entry.repo)
        };
        let commit = self.write_index(&next, &base_sha, &message)?;
        Ok(json!({ "commitUrl": commit, "replaced": replaced, "index": next }))
    }

    /// 两次提交间的差异。
    pub fn compare(&self, repo: &str, base: &str, head: &str) -> anyhow::Result<Value> {
        let raw = self.publisher.gh(&["api", &format!("repos/{repo}/compare/{base}...{head}")])?;
        Ok(trim_compare(&serde_json::from_str(&raw).context("读提交差异失败")?))
    }
}
