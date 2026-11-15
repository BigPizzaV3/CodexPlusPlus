#[tauri::command]
pub async fn skill_backup_usage() -> Result<codex_plus_core::skills::SkillBackupUsage, String> {
    tauri::async_runtime::spawn_blocking(|| {
        crate::commands::default_skills_manager().backup_usage()
            .map_err(|error| format!("统计 Skill 备份失败：{error:#}"))
    })
    .await
    .map_err(|error| format!("统计 Skill 备份失败：{error}"))?
}
