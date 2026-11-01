use std::path::PathBuf;

use anyhow::{Context, bail};
use serde_json::{Value, json};

use super::app_server::{AppServerConfig, CodexAppServer};

pub async fn read_projects(server: &mut CodexAppServer) -> anyhow::Result<Vec<Value>> {
    let mut projects = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let response = server
            .api_request("project/list", json!({"cursor":cursor,"limit":100}))
            .await?;
        projects.extend(
            response["data"]
                .as_array()
                .context("项目列表响应缺少 data")?
                .iter()
                .cloned(),
        );
        let next = response["nextCursor"].clone();
        if next.is_null() {
            break;
        }
        if next == cursor || projects.len() > 10_000 {
            bail!("项目列表分页异常");
        }
        cursor = next;
    }
    Ok(projects)
}

pub async fn list_projects(executable: String, work_dir: PathBuf) -> anyhow::Result<Value> {
    let mut server = CodexAppServer::start(AppServerConfig {
        executable,
        work_dir,
        model: String::new(),
        reasoning_effort: String::new(),
        sandbox: "read-only".into(),
    })
    .await?;
    let result = async {
        let projects = read_projects(&mut server).await?;
        Ok(json!({"projects": projects}))
    }
    .await;
    server.close().await;
    result
}

pub async fn resolve_project(
    server: &mut CodexAppServer,
    selected_id: &str,
) -> anyhow::Result<Option<Value>> {
    if selected_id.is_empty() {
        return Ok(None);
    }
    let result = server
        .api_request("project/read", json!({"projectId":selected_id}))
        .await?;
    let project = result.get("project").context("所选工作项目不可用")?;
    if project["id"].as_str() != Some(selected_id) {
        bail!("工作项目 ID 不匹配");
    }
    Ok(Some(project.clone()))
}
