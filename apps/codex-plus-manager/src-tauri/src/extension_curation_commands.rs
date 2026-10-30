//! 管理器的精选维护入口：本机 gh 账号对市场仓库有写权限时，审核社区拓展并收录进精选、审核精选拓展的新版本。
//! 审核走社区安装的 prepare（下载并暂存将要锁定的提交），收录时只认暂存元数据里的 repo / tag / sha。

use anyhow::Context;
use codex_plus_core::extension_curation::{self, Curator};
use codex_plus_core::extension_discovery::{self, CommunityStore, Endpoints};
use codex_plus_core::extension_publish::Publisher;
use codex_plus_core::script_market;
use codex_plus_core::user_scripts::default_user_scripts_config_dir;
use serde_json::{Value, json};

#[tauri::command]
pub async fn extension_curation_request(action: String, payload: Option<Value>) -> Result<Value, String> {
    handle(&action, payload.unwrap_or(Value::Null))
        .await
        .map_err(|error| format!("{error:#}"))
}

fn curator() -> anyhow::Result<Curator> {
    Ok(Curator::new(Publisher::new(default_user_scripts_config_dir())?))
}

/// gh / git 是阻塞的外部命令，放到阻塞线程里跑。
async fn blocking<T: Send + 'static>(work: impl FnOnce(Curator) -> anyhow::Result<T> + Send + 'static) -> anyhow::Result<T> {
    tauri::async_runtime::spawn_blocking(move || work(curator()?)).await?
}

/// 维护者身份确认后，带上 gh 令牌的 GitHub 地址（限额高）。
async fn maintainer_endpoints() -> anyhow::Result<Endpoints> {
    let token = blocking(|curator| {
        curator.require_maintainer()?;
        Ok(curator.token())
    })
    .await?;
    Ok(Endpoints::github().with_token(token))
}

async fn handle(action: &str, payload: Value) -> anyhow::Result<Value> {
    let store = CommunityStore::new(default_user_scripts_config_dir());
    match action {
        // 没装 gh、没登录、没有写权限都只是「不是维护者」，界面不显示维护入口。
        "status" => Ok(blocking(|curator| curator.status())
            .await
            .unwrap_or_else(|error| json!({ "maintainer": false, "message": format!("{error:#}") }))),
        // 审核：下载并暂存仓库当前的最新版本，列出文件、权限、密钥扫描；给了 baseSha 再附上与它的差异。
        "review" => {
            let repo = text(&payload, "repo")?.to_string();
            let endpoints = maintainer_endpoints().await?;
            let manager = crate::commands::default_user_script_manager();
            let mut prepared = store.prepare(&endpoints, &manager, &repo).await?;
            let token = prepared["token"].as_str().unwrap_or_default().to_string();
            let result = async {
                let (dir, meta) = store.staged(&token)?;
                let manifest = extension_curation::staged_manifest(&dir)?;
                let entry = extension_curation::curated_entry(&meta, &manifest, store.cached_entry(&repo).as_ref(), &extension_curation::now_iso(), false)?;
                prepared["canBeOfficial"] = json!(extension_curation::can_be_official(&repo));
                prepared["inspection"] = extension_curation::inspect_staged(&dir);
                prepared["entry"] = serde_json::to_value(&entry)?;
                let current = script_market::cached_market_manifest(false)
                    .await
                    .ok()
                    .and_then(|market| market.packages.into_iter().find(|package| package.repo.eq_ignore_ascii_case(&repo)));
                prepared["current"] = match &current {
                    Some(package) => json!({ "tag": package.tag, "sha": package.sha, "version": package.version, "official": package.is_official() }),
                    None => Value::Null,
                };
                if let Some(base) = current.map(|package| package.sha).filter(|sha| !sha.eq_ignore_ascii_case(&entry.sha)) {
                    let head = entry.sha.clone();
                    let repo = repo.clone();
                    prepared["diff"] = blocking(move |curator| curator.compare(&repo, &base, &head)).await?;
                }
                anyhow::Ok(())
            }
            .await;
            if let Err(error) = result {
                store.discard(&token);
                return Err(error);
            }
            Ok(prepared)
        }
        // 收录 / 更新：只收暂存令牌，条目由后端按暂存元数据重新拼，不信任前端传来的 sha。
        "approve" => {
            let token = text(&payload, "token")?.to_string();
            let (dir, meta) = store.staged(&token)?;
            let manifest = extension_curation::staged_manifest(&dir)?;
            let repo = meta.get("repo").and_then(Value::as_str).context("暂存元数据缺少 repo")?.to_string();
            // 「官方」由维护者在审核框里勾选；仓库不属于官方账号时 curated_entry 会忽略它。
            let official = payload.get("official").and_then(Value::as_bool).unwrap_or(false);
            let entry = extension_curation::curated_entry(&meta, &manifest, store.cached_entry(&repo).as_ref(), &extension_curation::now_iso(), official)?;
            let mut result = blocking(move |curator| curator.publish_entry(&entry, &extension_curation::now_iso())).await?;
            // 写进去的清单立刻生效，不等 CDN。
            if let Some(index) = result.get("index").and_then(Value::as_str) {
                if let Ok(parsed) = serde_json::from_str::<Value>(index).map_err(anyhow::Error::from).and_then(script_market::parse_market_manifest) {
                    script_market::remember_published_manifest(parsed);
                }
            }
            result.as_object_mut().map(|object| object.remove("index"));
            store.discard(&token);
            Ok(result)
        }
        "discard" => {
            store.discard(text(&payload, "token")?);
            Ok(json!({}))
        }
        // 精选拓展里作者发了新版本的：仓库最新版本的提交与清单锁定的不同。
        "updates" => {
            let endpoints = maintainer_endpoints().await?;
            let market = script_market::cached_market_manifest(false).await?;
            let mut updates = Vec::new();
            let mut problems = Vec::new();
            for package in &market.packages {
                let branch = store.cached_entry(&package.repo).map(|entry| entry.default_branch).unwrap_or_else(|| "main".into());
                match extension_discovery::latest_version(&endpoints, &package.repo, &package.id, &branch).await {
                    Ok(latest) if !latest.sha.eq_ignore_ascii_case(&package.sha) => updates.push(json!({
                        "id": package.id,
                        "repo": package.repo,
                        "currentTag": package.tag,
                        "currentSha": package.sha,
                        "latestTag": latest.tag,
                        "latestSha": latest.sha,
                    })),
                    Ok(_) => {}
                    Err(error) => problems.push(format!("{}：{error:#}", package.repo)),
                }
            }
            Ok(json!({ "updates": updates, "problems": problems }))
        }
        other => anyhow::bail!("未知操作 {other}"),
    }
}

fn text<'a>(payload: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .with_context(|| format!("缺少参数 {key}"))
}
