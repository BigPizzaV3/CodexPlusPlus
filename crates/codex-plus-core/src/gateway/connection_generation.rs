use std::fs::OpenOptions;
use std::path::Path;

use anyhow::Context;
use fs2::FileExt;

const FILE_NAME: &str = "codex-plus-gateway-connection-generation";

pub fn read(home: &Path) -> anyhow::Result<String> {
    Ok(crate::gateway::restore::read_optional(&home.join(FILE_NAME))?.unwrap_or_default())
}

pub fn publish(home: &Path) -> anyhow::Result<String> {
    std::fs::create_dir_all(home)?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(home.join(format!("{FILE_NAME}.lock")))?;
    lock.lock_exclusive().context("获取网关连接代际写锁失败")?;
    let generation = uuid::Uuid::new_v4().to_string();
    crate::settings::atomic_write(&home.join(FILE_NAME), generation.as_bytes())?;
    FileExt::unlock(&lock)?;
    Ok(generation)
}
