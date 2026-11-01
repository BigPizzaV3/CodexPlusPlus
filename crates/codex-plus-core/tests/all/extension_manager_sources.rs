use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use codex_plus_core::agent_collaboration::{CollaborationConfig, Store, TaskRecord};
use codex_plus_core::plugin_market::{self, MARKETPLACE_NAME};
use rusqlite::{Connection, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

fn assert_keys(value: &Value, expected: &[&str]) {
    let mut actual: Vec<_> = value
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    actual.sort_unstable();
    let mut expected = expected.to_vec();
    expected.sort_unstable();
    assert_eq!(actual, expected);
}

// 同时检查文件内容与目录集合，防止“只读”查询偷偷创建数据库、缓存、WAL 或锁文件。
fn disk_snapshot(root: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    fn walk(root: &Path, path: &Path, result: &mut BTreeMap<PathBuf, Option<Vec<u8>>>) {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let kind = entry.file_type().unwrap();
            if kind.is_dir() {
                result.insert(path.strip_prefix(root).unwrap().to_path_buf(), None);
                walk(root, &path, result);
            } else if kind.is_file() {
                result.insert(
                    path.strip_prefix(root).unwrap().to_path_buf(),
                    Some(fs::read(&path).unwrap()),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}

fn plugin_id(source: &str, id: &str, name: &str) -> String {
    let hash = format!("{:x}", Sha256::digest(format!("{source}:{id}").as_bytes()));
    format!("{name}@{MARKETPLACE_NAME}-{}", &hash[..12])
}

fn write_installed(home: &Path) {
    let market = home.join(".tmp").join(MARKETPLACE_NAME);
    fs::create_dir_all(&market).unwrap();
    fs::write(
        market.join("installed.json"),
        serde_json::to_vec(&json!({
            "plugins": {
                "public:alpha": {
                    "id": "alpha", "name": "alpha-plugin", "version": "1.2.3",
                    "source": "public", "tree": "a".repeat(40),
                    "repository": "https://secret-repository.invalid/private",
                    "path": "/secret/package/path", "auth": "SECRET_INSTALL_AUTH"
                },
                "full:beta": {
                    "id": "beta", "name": "beta-plugin", "version": "2.0.0",
                    "source": "full", "tree": "b".repeat(40)
                }
            }
        }))
        .unwrap(),
    )
    .unwrap();
    // 摘要不能读取索引、包文件或 manifest，也不能为了修复索引触发下载。
    fs::write(market.join("index-public.json"), b"invalid index").unwrap();
}

#[test]
fn installed_summary_is_local_allowlisted_and_respects_enabled_config() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("home");
    write_installed(&home);
    let alpha_id = plugin_id("public", "alpha", "alpha-plugin");
    let beta_id = plugin_id("full", "beta", "beta-plugin");
    fs::write(
        home.join("config.toml"),
        format!(
            "api_key = \"SECRET_CONFIG_AUTH\"\n\
             [marketplaces.unmanaged]\nsource = \"/secret/marketplace/path\"\n\
             [plugins.\"{alpha_id}\"]\nenabled = false\n\
             [plugins.\"unmanaged@external\"]\nenabled = true\n"
        ),
    )
    .unwrap();
    let before = disk_snapshot(temp.path());
    let summary = plugin_market::installed_summary(&home).unwrap();
    assert_keys(&summary, &["plugins"]);
    let plugins = summary["plugins"].as_array().unwrap();
    assert_eq!(plugins.len(), 2);
    for plugin in plugins {
        assert_keys(plugin, &["id", "name", "version", "source", "enabled"]);
    }
    let alpha = plugins
        .iter()
        .find(|plugin| plugin["id"] == alpha_id)
        .unwrap();
    assert_eq!(
        alpha,
        &json!({
            "id": alpha_id, "name": "alpha-plugin", "version": "1.2.3",
            "source": "public", "enabled": false
        })
    );
    let beta = plugins
        .iter()
        .find(|plugin| plugin["id"] == beta_id)
        .unwrap();
    assert_eq!(
        beta,
        &json!({
            "id": beta_id, "name": "beta-plugin", "version": "2.0.0",
            "source": "full", "enabled": true
        })
    );
    let encoded = summary.to_string();
    for sensitive in [
        "SECRET_INSTALL_AUTH",
        "SECRET_CONFIG_AUTH",
        "secret-repository",
        "/secret/",
        "unmanaged",
        &"a".repeat(40),
        &"b".repeat(40),
    ] {
        assert!(
            !encoded.contains(sensitive),
            "unexpected value: {sensitive}"
        );
    }
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[test]
fn installed_summary_missing_home_or_state_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path().join("missing").join("home");
    assert_eq!(
        plugin_market::installed_summary(&home).unwrap(),
        json!({"plugins": []})
    );
    assert!(disk_snapshot(temp.path()).is_empty());
    fs::create_dir_all(&home).unwrap();
    fs::write(
        home.join("config.toml"),
        "[plugins.\"external@market\"]\nenabled = true\n",
    )
    .unwrap();
    let before = disk_snapshot(temp.path());
    assert_eq!(
        plugin_market::installed_summary(&home).unwrap(),
        json!({"plugins": []})
    );
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[cfg(unix)]
#[test]
fn installed_summary_rejects_symlinked_state_and_config() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let outside = temp.path().join("outside.json");
    fs::write(&outside, r#"{"plugins":{}}"#).unwrap();
    let state_home = temp.path().join("state-home");
    let market = state_home.join(".tmp").join(MARKETPLACE_NAME);
    fs::create_dir_all(&market).unwrap();
    symlink(&outside, market.join("installed.json")).unwrap();
    assert!(plugin_market::installed_summary(&state_home).is_err());
    let config_home = temp.path().join("config-home");
    fs::create_dir_all(&config_home).unwrap();
    symlink(
        temp.path().join("missing-config.toml"),
        config_home.join("config.toml"),
    )
    .unwrap();
    assert!(plugin_market::installed_summary(&config_home).is_err());
    assert_eq!(fs::read_to_string(outside).unwrap(), r#"{"plugins":{}}"#);
    assert!(!temp.path().join("missing-config.toml").exists());
}

fn task(id: &str, updated_at_ms: u64) -> TaskRecord {
    TaskRecord {
        id: id.to_string(),
        agent_id: "codex".to_string(),
        prompt: "SECRET_PROMPT".to_string(),
        cwd: "/SECRET_WORKSPACE".to_string(),
        status: "running".to_string(),
        answer: "SECRET_ANSWER".to_string(),
        error: "SECRET_ERROR".to_string(),
        session_id: "SECRET_SESSION".to_string(),
        parent_task_id: Some("SECRET_PARENT".to_string()),
        created_at_ms: 1,
        updated_at_ms,
        cancel_requested: true,
        progress: "SECRET_PROGRESS".to_string(),
    }
}

fn write_collaboration(path: &Path, tasks: Vec<TaskRecord>) {
    let mut config = CollaborationConfig::default();
    config.enabled = true;
    config.agents[0].enabled = true;
    config.agents[0].executable = "/SECRET_EXECUTABLE".to_string();
    config.agents[0].model = "summary-model".to_string();
    config.agents[0].reuse_saved_auth = true;
    config.agents[0].max_concurrent_tasks = 3;
    let payload = json!({
        "config": config,
        "tasks": tasks,
        "queueOwners": {"queued": "SECRET_OWNER"}
    });
    let connection = Connection::open(path).unwrap();
    connection.execute_batch(
        "CREATE TABLE collaboration_state (id INTEGER PRIMARY KEY CHECK (id = 1), payload TEXT NOT NULL);"
    ).unwrap();
    connection
        .execute(
            "INSERT INTO collaboration_state (id, payload) VALUES (1, ?1)",
            params![payload.to_string()],
        )
        .unwrap();
}

fn assert_default_summary(summary: &Value) {
    assert_keys(summary, &["enabled", "agents", "tasks", "total"]);
    assert_eq!(summary["enabled"], false);
    assert_eq!(summary["tasks"], json!([]));
    assert_eq!(summary["total"], 0);
    let agents = summary["agents"].as_array().unwrap();
    assert_eq!(agents.len(), CollaborationConfig::default().agents.len());
    for agent in agents {
        assert_keys(
            agent,
            &["id", "name", "enabled", "model", "maxConcurrentTasks"],
        );
        assert_eq!(agent["enabled"], false);
        assert_eq!(agent["maxConcurrentTasks"], 1);
    }
}

#[test]
fn collaboration_summary_is_allowlisted_and_does_not_refresh_leases() {
    let temp = tempfile::tempdir().unwrap();
    // URI 特殊字符必须转义，不能被 SQLite 解释成查询参数或 fragment。
    let directory = temp.path().join(if cfg!(windows) {
        "state # 中文"
    } else {
        "state ?# 中文"
    });
    fs::create_dir(&directory).unwrap();
    let path = directory.join("collaboration.sqlite3");
    let mut queued = task("queued", 2);
    queued.status = "queued".to_string();
    write_collaboration(&path, vec![task("running", 1), queued]);
    let before = disk_snapshot(temp.path());
    let summary = Store::new(path).extension_summary(20).unwrap();
    assert_keys(&summary, &["enabled", "agents", "tasks", "total"]);
    assert_eq!(summary["enabled"], true);
    assert_eq!(summary["total"], 2);
    for agent in summary["agents"].as_array().unwrap() {
        assert_keys(
            agent,
            &["id", "name", "enabled", "model", "maxConcurrentTasks"],
        );
    }
    assert_eq!(
        summary["agents"][0],
        json!({
            "id": "codex", "name": "Codex", "enabled": true,
            "model": "summary-model", "maxConcurrentTasks": 3
        })
    );
    for task in summary["tasks"].as_array().unwrap() {
        assert_keys(
            task,
            &[
                "id",
                "agentId",
                "status",
                "createdAtMs",
                "updatedAtMs",
                "cancelRequested",
            ],
        );
    }
    assert_eq!(
        summary["tasks"],
        json!([
            {"id": "queued", "agentId": "codex", "status": "queued", "createdAtMs": 1, "updatedAtMs": 2, "cancelRequested": true},
            {"id": "running", "agentId": "codex", "status": "running", "createdAtMs": 1, "updatedAtMs": 1, "cancelRequested": true}
        ])
    );
    assert!(!summary.to_string().contains("SECRET_"));
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[test]
fn collaboration_summary_clamps_limit_and_sorts_by_update_then_id() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("collaboration.sqlite3");
    let mut tasks: Vec<_> = (0..205)
        .rev()
        .map(|index| task(&format!("task-{index:03}"), 100))
        .collect();
    tasks.push(task("newest", 101));
    tasks.push(task("oldest", 99));
    write_collaboration(&path, tasks);
    let store = Store::new(path);
    let before = disk_snapshot(temp.path());
    let minimum = store.extension_summary(0).unwrap();
    assert_eq!(minimum["total"], 207);
    assert_eq!(minimum["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(minimum["tasks"][0]["id"], "newest");
    let three = store.extension_summary(3).unwrap();
    assert_eq!(three["tasks"].as_array().unwrap().len(), 3);
    assert_eq!(three["tasks"][1]["id"], "task-000");
    assert_eq!(three["tasks"][2]["id"], "task-001");
    let maximum = store.extension_summary(usize::MAX).unwrap();
    assert_eq!(maximum["total"], 207);
    assert_eq!(maximum["tasks"].as_array().unwrap().len(), 200);
    assert_eq!(maximum["tasks"][199]["id"], "task-198");
    assert_eq!(maximum, store.extension_summary(200).unwrap());
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[test]
fn collaboration_summary_missing_database_creates_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let store = Store::new(temp.path().join("missing").join("collaboration.sqlite3"));
    assert_default_summary(&store.extension_summary(20).unwrap());
    assert!(disk_snapshot(temp.path()).is_empty());
    let store = Store::new(temp.path().join("collaboration.sqlite3"));
    assert_default_summary(&store.extension_summary(20).unwrap());
    assert!(disk_snapshot(temp.path()).is_empty());
}

#[test]
fn collaboration_summary_missing_schema_or_row_does_not_migrate() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("collaboration.sqlite3");
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch("CREATE TABLE unrelated (id INTEGER PRIMARY KEY);")
        .unwrap();
    drop(connection);
    let store = Store::new(path.clone());
    let before = disk_snapshot(temp.path());
    assert_default_summary(&store.extension_summary(20).unwrap());
    assert_eq!(disk_snapshot(temp.path()), before);
    let connection = Connection::open(&path).unwrap();
    connection
        .execute_batch(
            "CREATE TABLE collaboration_state (id INTEGER PRIMARY KEY, payload TEXT NOT NULL);",
        )
        .unwrap();
    drop(connection);
    let before = disk_snapshot(temp.path());
    assert_default_summary(&store.extension_summary(20).unwrap());
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[test]
fn collaboration_summary_wal_snapshot_does_not_create_sidecar_files() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("collaboration.sqlite3");
    write_collaboration(&path, vec![task("snapshot", 1)]);
    let connection = Connection::open(&path).unwrap();
    let mode: String = connection
        .query_row("PRAGMA journal_mode=WAL", [], |row| row.get(0))
        .unwrap();
    assert_eq!(mode, "wal");
    drop(connection);
    assert!(!temp.path().join("collaboration.sqlite3-wal").exists());
    assert!(!temp.path().join("collaboration.sqlite3-shm").exists());
    let before = disk_snapshot(temp.path());
    let summary = Store::new(path).extension_summary(20).unwrap();
    assert_eq!(summary["total"], 1);
    assert_eq!(summary["tasks"][0]["id"], "snapshot");
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[test]
fn collaboration_summary_corrupt_database_is_not_replaced() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("collaboration.sqlite3");
    fs::write(&path, b"not a database").unwrap();
    let before = disk_snapshot(temp.path());
    assert!(Store::new(path).extension_summary(20).is_err());
    assert_eq!(disk_snapshot(temp.path()), before);
}

#[cfg(unix)]
#[test]
fn collaboration_summary_rejects_existing_and_dangling_symlinks() {
    use std::os::unix::fs::symlink;

    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("actual.sqlite3");
    write_collaboration(&target, vec![]);
    let linked = temp.path().join("linked.sqlite3");
    symlink(&target, &linked).unwrap();
    let before = disk_snapshot(temp.path());
    assert!(
        Store::new(linked)
            .extension_summary(20)
            .unwrap_err()
            .to_string()
            .contains("符号链接")
    );
    let missing = temp.path().join("missing.sqlite3");
    let dangling = temp.path().join("dangling.sqlite3");
    symlink(&missing, &dangling).unwrap();
    assert!(
        Store::new(dangling)
            .extension_summary(20)
            .unwrap_err()
            .to_string()
            .contains("符号链接")
    );
    assert!(!missing.exists());
    assert_eq!(disk_snapshot(temp.path()), before);
}
