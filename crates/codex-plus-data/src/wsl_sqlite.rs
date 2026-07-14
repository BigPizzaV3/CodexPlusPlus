use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};

#[cfg(windows)]
use std::io::Write;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
use std::process::{Command, Stdio};

const WSL_DISCOVERY_SCRIPT: &str = r#"
set -eu
if ! command -v sqlite3 >/dev/null 2>&1; then
    echo "WSL 中未找到 sqlite3" >&2
    exit 127
fi
scan_sqlite_dir() {
    [ -d "$1" ] || return 0
    find "$1" -maxdepth 1 -type f \( -name '*.db' -o -name '*.sqlite' -o -name '*.sqlite3' \) -print
}
if [ -n "${CODEX_SQLITE_HOME:-}" ]; then
    scan_sqlite_dir "$CODEX_SQLITE_HOME"
elif [ -n "${HOME:-}" ]; then
    scan_sqlite_dir "$HOME/.codex/sqlite"
    if [ -d "$HOME/.codex" ]; then
        find "$HOME/.codex" -maxdepth 1 -type f \( -name 'state_*.db' -o -name 'state_*.sqlite' -o -name 'state_*.sqlite3' \) -print
    fi
fi
"#;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WslSqliteUpdateCounts {
    pub(crate) provider_rows: usize,
    pub(crate) user_event_rows: usize,
    pub(crate) cwd_rows: usize,
}

impl WslSqliteUpdateCounts {
    pub(crate) fn total(&self) -> usize {
        self.provider_rows + self.user_event_rows + self.cwd_rows
    }

    fn add(&mut self, other: Self) {
        self.provider_rows += other.provider_rows;
        self.user_event_rows += other.user_event_rows;
        self.cwd_rows += other.cwd_rows;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct WslSqliteUpdateResult {
    pub(crate) counts: WslSqliteUpdateCounts,
    pub(crate) databases_updated: usize,
}

#[derive(Debug, Clone)]
struct WslSqliteDatabase {
    path: String,
    has_model_provider: bool,
    has_user_event: bool,
    has_cwd: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct WslSqliteBackup {
    pub(crate) source_path: String,
    backup_path: String,
    pub(crate) relative_path: PathBuf,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct WslSqliteContext {
    databases: Vec<WslSqliteDatabase>,
}

impl WslSqliteContext {
    pub(crate) fn discover_if_enabled(config_path: &Path) -> anyhow::Result<Option<Self>> {
        if !desktop_wsl_enabled(config_path) {
            return Ok(None);
        }

        #[cfg(windows)]
        {
            Self::discover().map(Some)
        }

        #[cfg(not(windows))]
        {
            Ok(None)
        }
    }

    #[cfg(windows)]
    fn discover() -> anyhow::Result<Self> {
        let output = run_wsl_command(&["--exec", "sh", "-lc", WSL_DISCOVERY_SCRIPT], None)
            .context("无法扫描 WSL 中的 Codex SQLite 索引")?;
        let mut seen = HashSet::new();
        let mut databases = Vec::new();
        for path in output
            .lines()
            .map(str::trim)
            .filter(|path| !path.is_empty())
        {
            if !seen.insert(path.to_string()) {
                continue;
            }
            let metadata = read_database_metadata(path)
                .with_context(|| format!("无法检查 WSL Codex 索引：{path}"))?;
            let Some((has_model_provider, has_user_event, has_cwd)) = metadata else {
                continue;
            };
            databases.push(WslSqliteDatabase {
                path: path.to_string(),
                has_model_provider,
                has_user_event,
                has_cwd,
            });
        }
        databases.sort_by(|left, right| left.path.cmp(&right.path));
        if databases.is_empty() {
            bail!("Codex Desktop 已启用 WSL，但没有发现包含 threads 表的 WSL SQLite 正本");
        }
        Ok(Self { databases })
    }

    pub(crate) fn provider_ids(&self) -> anyhow::Result<Vec<String>> {
        let mut ids = HashSet::new();
        for database in self
            .databases
            .iter()
            .filter(|database| database.has_model_provider)
        {
            let output = run_sqlite(
                &database.path,
                true,
                ".timeout 10000\n.bail on\nSELECT DISTINCT hex(CAST(model_provider AS BLOB)) FROM threads WHERE COALESCE(model_provider, '') <> '' ORDER BY 1;\n",
            )?;
            for line in output
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
            {
                let bytes = decode_hex(line).with_context(|| {
                    format!("WSL 索引返回了无效的 Provider ID：{}", database.path)
                })?;
                let id = String::from_utf8(bytes).with_context(|| {
                    format!("WSL 索引返回了非 UTF-8 Provider ID：{}", database.path)
                })?;
                if is_valid_provider_id(&id) {
                    ids.insert(id);
                }
            }
        }
        let mut ids = ids.into_iter().collect::<Vec<_>>();
        ids.sort();
        Ok(ids)
    }

    pub(crate) fn count_updates(
        &self,
        target_provider: &str,
        user_event_thread_ids: &HashSet<String>,
        cwd_by_thread_id: &HashMap<String, String>,
    ) -> anyhow::Result<WslSqliteUpdateCounts> {
        let mut total = WslSqliteUpdateCounts::default();
        for database in self
            .databases
            .iter()
            .filter(|database| database.has_model_provider)
        {
            let script = build_count_script(
                database,
                target_provider,
                user_event_thread_ids,
                cwd_by_thread_id,
            );
            let output = run_sqlite(&database.path, true, &script)
                .with_context(|| format!("无法读取 WSL Codex 索引：{}", database.path))?;
            total.add(
                parse_counts_with_integrity(&output)
                    .with_context(|| format!("无法解析 WSL Codex 索引统计：{}", database.path))?,
            );
        }
        Ok(total)
    }

    pub(crate) fn preflight_write(&self) -> anyhow::Result<()> {
        for database in self
            .databases
            .iter()
            .filter(|database| database.has_model_provider)
        {
            run_sqlite(
                &database.path,
                false,
                ".timeout 10000\n.bail on\nBEGIN IMMEDIATE;\nROLLBACK;\n",
            )
            .with_context(|| format!("WSL Codex 索引当前不可写：{}", database.path))?;
        }
        Ok(())
    }

    pub(crate) fn create_backups(&self, backup_dir: &Path) -> anyhow::Result<Vec<WslSqliteBackup>> {
        let databases = self
            .databases
            .iter()
            .filter(|database| database.has_model_provider)
            .collect::<Vec<_>>();
        if databases.is_empty() {
            return Ok(Vec::new());
        }

        let windows_dir = backup_dir.join("wsl-db");
        fs::create_dir_all(&windows_dir)?;
        let wsl_dir = windows_path_to_wsl(&windows_dir)?;
        let mut backups = Vec::new();
        for (index, database) in databases.into_iter().enumerate() {
            let base_name = database
                .path
                .rsplit('/')
                .next()
                .map(sanitize_backup_name)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| "state.sqlite".to_string());
            let file_name = format!("{index:03}-{base_name}");
            let relative_path = PathBuf::from("wsl-db").join(&file_name);
            let windows_path = backup_dir.join(&relative_path);
            let backup_path = format!("{}/{}", wsl_dir.trim_end_matches('/'), file_name);
            let script = format!(
                ".timeout 10000\n.bail on\n.backup {}\n",
                sqlite_dot_quote(&backup_path)
            );
            run_sqlite(&database.path, false, &script)
                .with_context(|| format!("无法备份 WSL Codex 索引：{}", database.path))?;
            if !windows_path.is_file() {
                bail!("WSL Codex 索引备份未生成：{}", windows_path.display());
            }
            let integrity = run_sqlite(
                &backup_path,
                true,
                ".timeout 10000\n.bail on\nPRAGMA quick_check;\n",
            )?;
            ensure_quick_check(&integrity)
                .with_context(|| format!("WSL Codex 索引备份校验失败：{backup_path}"))?;
            backups.push(WslSqliteBackup {
                source_path: database.path.clone(),
                backup_path,
                relative_path,
            });
        }
        Ok(backups)
    }

    pub(crate) fn apply_updates(
        &self,
        target_provider: &str,
        user_event_thread_ids: &HashSet<String>,
        cwd_by_thread_id: &HashMap<String, String>,
        backups: &[WslSqliteBackup],
    ) -> anyhow::Result<WslSqliteUpdateResult> {
        let mut result = WslSqliteUpdateResult::default();
        for database in self
            .databases
            .iter()
            .filter(|database| database.has_model_provider)
        {
            let update = (|| -> anyhow::Result<WslSqliteUpdateCounts> {
                let script = build_update_script(
                    database,
                    target_provider,
                    user_event_thread_ids,
                    cwd_by_thread_id,
                );
                let output = run_sqlite(&database.path, false, &script)
                    .with_context(|| format!("无法更新 WSL Codex 索引：{}", database.path))?;
                parse_counts_with_integrity(&output)
                    .with_context(|| format!("WSL Codex 索引更新结果无效：{}", database.path))
            })();
            let counts = match update {
                Ok(counts) => counts,
                Err(error) => {
                    let rollback = self.restore_backups(backups);
                    if let Err(rollback_error) = rollback {
                        bail!("{error}; WSL 索引回滚也失败：{rollback_error}");
                    }
                    return Err(error);
                }
            };
            if counts.total() > 0 {
                result.databases_updated += 1;
            }
            result.counts.add(counts);
        }
        Ok(result)
    }

    pub(crate) fn restore_backups(&self, backups: &[WslSqliteBackup]) -> anyhow::Result<()> {
        let mut errors = Vec::new();
        for backup in backups {
            let script = format!(
                ".timeout 10000\n.bail on\n.restore {}\nPRAGMA quick_check;\n",
                sqlite_dot_quote(&backup.backup_path)
            );
            match run_sqlite(&backup.source_path, false, &script)
                .and_then(|output| ensure_quick_check(&output))
            {
                Ok(()) => {}
                Err(error) => errors.push(format!("{}：{error}", backup.source_path)),
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            bail!(errors.join("；"))
        }
    }
}

fn desktop_wsl_enabled(config_path: &Path) -> bool {
    fs::read_to_string(config_path)
        .ok()
        .is_some_and(|text| desktop_wsl_enabled_in_text(&text))
}

fn desktop_wsl_enabled_in_text(text: &str) -> bool {
    let mut in_desktop = false;
    for raw_line in text.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_desktop = line.trim_start_matches('[').trim_end_matches(']').trim() == "desktop";
            continue;
        }
        if !in_desktop {
            continue;
        }
        let Some((raw_key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let key = raw_key.trim().trim_matches(['\'', '"']);
        if matches!(
            key,
            "runCodexInWindowsSubsystemForLinux" | "run_codex_in_windows_subsystem_for_linux"
        ) {
            return raw_value.trim().eq_ignore_ascii_case("true");
        }
    }
    false
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, ch) in line.char_indices() {
        if let Some(active_quote) = quote {
            if active_quote == '"' && escaped {
                escaped = false;
            } else if active_quote == '"' && ch == '\\' {
                escaped = true;
            } else if ch == active_quote {
                quote = None;
            }
        } else if matches!(ch, '\'' | '"') {
            quote = Some(ch);
        } else if ch == '#' {
            return &line[..index];
        }
    }
    line
}

fn read_database_metadata(path: &str) -> anyhow::Result<Option<(bool, bool, bool)>> {
    let output = run_sqlite(
        path,
        true,
        ".timeout 10000\n.bail on\nSELECT printf('%d|%d|%d|%d', EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'threads'), EXISTS(SELECT 1 FROM pragma_table_info('threads') WHERE name = 'model_provider'), EXISTS(SELECT 1 FROM pragma_table_info('threads') WHERE name = 'has_user_event'), EXISTS(SELECT 1 FROM pragma_table_info('threads') WHERE name = 'cwd'));\n",
    )?;
    parse_database_metadata(&output)
}

fn parse_database_metadata(output: &str) -> anyhow::Result<Option<(bool, bool, bool)>> {
    let line = output.lines().map(str::trim).find(|line| !line.is_empty());
    let Some(line) = line else {
        bail!("数据库元数据为空");
    };
    let values = line.split('|').collect::<Vec<_>>();
    if values.len() != 4 || values.iter().any(|value| !matches!(*value, "0" | "1")) {
        bail!("数据库元数据格式无效：{line}");
    }
    if values[0] == "0" {
        return Ok(None);
    }
    Ok(Some((values[1] == "1", values[2] == "1", values[3] == "1")))
}

fn build_count_script(
    database: &WslSqliteDatabase,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
) -> String {
    let mut script = String::from(".timeout 10000\n.bail on\n");
    script.push_str(&format!(
        "SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> {};\n",
        sqlite_text_expression(target_provider)
    ));
    script.push_str(&count_user_event_sql(
        database.has_user_event,
        user_event_thread_ids,
    ));
    script.push_str(&count_cwd_sql(database.has_cwd, cwd_by_thread_id));
    script.push_str("PRAGMA quick_check;\n");
    script
}

fn build_update_script(
    database: &WslSqliteDatabase,
    target_provider: &str,
    user_event_thread_ids: &HashSet<String>,
    cwd_by_thread_id: &HashMap<String, String>,
) -> String {
    let provider = sqlite_text_expression(target_provider);
    let mut script = String::from(".timeout 10000\n.bail on\nBEGIN IMMEDIATE;\n");
    script.push_str(&format!(
        "UPDATE threads SET model_provider = {provider} WHERE COALESCE(model_provider, '') <> {provider};\nSELECT changes();\n"
    ));
    script.push_str(&update_user_event_sql(
        database.has_user_event,
        user_event_thread_ids,
    ));
    script.push_str(&update_cwd_sql(database.has_cwd, cwd_by_thread_id));
    script.push_str("COMMIT;\nPRAGMA quick_check;\n");
    script
}

fn count_user_event_sql(has_column: bool, ids: &HashSet<String>) -> String {
    let values = sorted_id_values(ids);
    if !has_column || values.is_empty() {
        return "SELECT 0;\n".to_string();
    }
    format!(
        "WITH desired(id) AS (VALUES {})\nSELECT COUNT(*) FROM threads JOIN desired ON desired.id = threads.id WHERE COALESCE(threads.has_user_event, 0) <> 1;\n",
        values.join(", ")
    )
}

fn update_user_event_sql(has_column: bool, ids: &HashSet<String>) -> String {
    let values = sorted_id_values(ids);
    if !has_column || values.is_empty() {
        return "SELECT 0;\n".to_string();
    }
    format!(
        "WITH desired(id) AS (VALUES {})\nUPDATE threads SET has_user_event = 1 WHERE id IN (SELECT id FROM desired) AND COALESCE(has_user_event, 0) <> 1;\nSELECT changes();\n",
        values.join(", ")
    )
}

fn count_cwd_sql(has_column: bool, cwd_by_thread_id: &HashMap<String, String>) -> String {
    let values = sorted_cwd_values(cwd_by_thread_id);
    if !has_column || values.is_empty() {
        return "SELECT 0;\n".to_string();
    }
    format!(
        "WITH desired(id, cwd) AS (VALUES {})\nSELECT COUNT(*) FROM threads JOIN desired ON desired.id = threads.id WHERE COALESCE(threads.cwd, '') <> desired.cwd;\n",
        values.join(", ")
    )
}

fn update_cwd_sql(has_column: bool, cwd_by_thread_id: &HashMap<String, String>) -> String {
    let values = sorted_cwd_values(cwd_by_thread_id);
    if !has_column || values.is_empty() {
        return "SELECT 0;\n".to_string();
    }
    format!(
        "WITH desired(id, cwd) AS (VALUES {})\nUPDATE threads SET cwd = (SELECT desired.cwd FROM desired WHERE desired.id = threads.id) WHERE EXISTS (SELECT 1 FROM desired WHERE desired.id = threads.id AND COALESCE(threads.cwd, '') <> desired.cwd);\nSELECT changes();\n",
        values.join(", ")
    )
}

fn sorted_id_values(ids: &HashSet<String>) -> Vec<String> {
    let mut ids = ids.iter().collect::<Vec<_>>();
    ids.sort();
    ids.into_iter()
        .map(|id| format!("({})", sqlite_text_expression(id)))
        .collect()
}

fn sorted_cwd_values(cwd_by_thread_id: &HashMap<String, String>) -> Vec<String> {
    let mut items = cwd_by_thread_id.iter().collect::<Vec<_>>();
    items.sort_by(|left, right| left.0.cmp(right.0));
    items
        .into_iter()
        .map(|(id, cwd)| {
            format!(
                "({}, {})",
                sqlite_text_expression(id),
                sqlite_text_expression(cwd)
            )
        })
        .collect()
}

fn sqlite_text_expression(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len() * 2);
    for byte in value.as_bytes() {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    format!("CAST(X'{encoded}' AS TEXT)")
}

fn sqlite_dot_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

fn parse_counts_with_integrity(output: &str) -> anyhow::Result<WslSqliteUpdateCounts> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines.len() != 4 || lines[3] != "ok" {
        bail!("SQLite 返回结果或 quick_check 无效：{}", lines.join(" | "));
    }
    Ok(WslSqliteUpdateCounts {
        provider_rows: lines[0].parse()?,
        user_event_rows: lines[1].parse()?,
        cwd_rows: lines[2].parse()?,
    })
}

fn ensure_quick_check(output: &str) -> anyhow::Result<()> {
    let lines = output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    if lines == ["ok"] {
        Ok(())
    } else {
        bail!("quick_check 未通过：{}", lines.join(" | "))
    }
}

fn is_valid_provider_id(value: &str) -> bool {
    !value.trim().is_empty() && !value.chars().any(char::is_control)
}

fn decode_hex(value: &str) -> anyhow::Result<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        bail!("十六进制文本长度无效");
    }
    let mut decoded = Vec::with_capacity(value.len() / 2);
    let bytes = value.as_bytes();
    for pair in bytes.chunks_exact(2) {
        let high = decode_hex_digit(pair[0])?;
        let low = decode_hex_digit(pair[1])?;
        decoded.push((high << 4) | low);
    }
    Ok(decoded)
}

fn decode_hex_digit(value: u8) -> anyhow::Result<u8> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => bail!("无效的十六进制字符"),
    }
}

fn sanitize_backup_name(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn windows_path_to_wsl(path: &Path) -> anyhow::Result<String> {
    let raw = path.to_string_lossy();
    let output = run_wsl_command(&["--exec", "wslpath", "-a", "-u", &raw], None)
        .with_context(|| format!("无法把 Windows 备份路径映射到 WSL：{}", path.display()))?;
    let mapped = output.trim();
    if mapped.is_empty() {
        bail!("wslpath 返回了空路径：{}", path.display());
    }
    Ok(mapped.to_string())
}

fn run_sqlite(path: &str, readonly: bool, script: &str) -> anyhow::Result<String> {
    let mut args = vec!["--exec", "sqlite3", "-batch", "-noheader"];
    if readonly {
        args.push("-readonly");
    }
    args.push(path);
    run_wsl_command(&args, Some(script))
}

#[cfg(windows)]
fn run_wsl_command(args: &[&str], input: Option<&str>) -> anyhow::Result<String> {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut command = Command::new("wsl.exe");
    command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = command.spawn().context("无法启动 wsl.exe")?;
    if let Some(input) = input {
        let Some(mut stdin) = child.stdin.take() else {
            let _ = child.kill();
            let _ = child.wait();
            bail!("无法打开 WSL 命令的标准输入");
        };
        match stdin.write_all(input.as_bytes()) {
            Ok(()) => {}
            Err(error) => {
                drop(stdin);
                let _ = child.kill();
                let _ = child.wait();
                return Err(error).context("无法向 WSL 命令写入数据");
            }
        }
    }
    drop(child.stdin.take());
    let output = child.wait_with_output()?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        bail!(
            "wsl.exe 执行失败（{}）：{}",
            output
                .status
                .code()
                .map(|code| code.to_string())
                .unwrap_or_else(|| "terminated".to_string()),
            if stderr.is_empty() {
                "未返回错误详情"
            } else {
                &stderr
            }
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(not(windows))]
fn run_wsl_command(_args: &[&str], _input: Option<&str>) -> anyhow::Result<String> {
    bail!("WSL SQLite 同步仅在 Windows 上可用")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_wsl_switch_is_read_only_from_desktop_table() {
        assert!(desktop_wsl_enabled_in_text(
            r#"
model_provider = "custom"

[desktop]
runCodexInWindowsSubsystemForLinux = true # Desktop 使用 WSL
"#
        ));
        assert!(!desktop_wsl_enabled_in_text(
            r#"
runCodexInWindowsSubsystemForLinux = true
[desktop]
runCodexInWindowsSubsystemForLinux = false
"#
        ));
        assert!(desktop_wsl_enabled_in_text(
            r##"
[desktop]
run_codex_in_windows_subsystem_for_linux = true
label = "# 不是注释"
"##
        ));
    }

    #[test]
    fn sqlite_text_expression_encodes_quotes_unicode_and_control_bytes() {
        assert_eq!(
            sqlite_text_expression("a'清\n\0"),
            "CAST(X'6127E6B8850A00' AS TEXT)"
        );
    }

    #[test]
    fn generated_scripts_keep_values_out_of_sql_syntax() {
        let database = WslSqliteDatabase {
            path: "/tmp/state.sqlite".to_string(),
            has_model_provider: true,
            has_user_event: true,
            has_cwd: true,
        };
        let ids = HashSet::from(["thread'1".to_string()]);
        let cwd = HashMap::from([(
            "thread'1".to_string(),
            "C:/工作区'; DROP TABLE threads;".to_string(),
        )]);
        let script = build_update_script(&database, "future-provider", &ids, &cwd);

        assert!(script.contains("BEGIN IMMEDIATE;"));
        assert!(script.contains("COMMIT;\nPRAGMA quick_check;"));
        assert!(!script.contains("DROP TABLE"));
        assert!(!script.contains("thread'1"));
        assert_eq!(script.matches("SELECT changes();").count(), 3);
    }

    #[test]
    fn count_output_requires_three_counts_and_successful_integrity_check() {
        assert_eq!(
            parse_counts_with_integrity("4\n3\n2\nok\n").unwrap(),
            WslSqliteUpdateCounts {
                provider_rows: 4,
                user_event_rows: 3,
                cwd_rows: 2,
            }
        );
        assert!(parse_counts_with_integrity("4\n3\n2\ncorrupt\n").is_err());
        assert!(parse_counts_with_integrity("4\n3\nok\n").is_err());
    }

    #[test]
    fn database_metadata_requires_threads_and_tracks_optional_columns() {
        assert_eq!(
            parse_database_metadata("1|1|0|1\n").unwrap(),
            Some((true, false, true))
        );
        assert_eq!(parse_database_metadata("0|0|0|0\n").unwrap(), None);
        assert!(parse_database_metadata("1|yes|0|1\n").is_err());
    }

    #[test]
    fn provider_hex_and_backup_names_are_safe() {
        assert_eq!(
            decode_hex("746573745F70726F7669646572").unwrap(),
            b"test_provider"
        );
        assert!(decode_hex("ABC").is_err());
        assert_eq!(
            sanitize_backup_name("state 5/主.sqlite"),
            "state_5__.sqlite"
        );
        assert_eq!(
            sqlite_dot_quote("/mnt/c/A B/\"db\""),
            "\"/mnt/c/A B/\\\"db\\\"\""
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn non_windows_build_keeps_existing_provider_sync_behavior() {
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        fs::write(
            &config,
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = true\n",
        )
        .unwrap();
        assert!(
            WslSqliteContext::discover_if_enabled(&config)
                .unwrap()
                .is_none()
        );
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "需要由专项端到端验证准备隔离的 WSL SQLite 数据库"]
    fn isolated_wsl_sqlite_round_trip() {
        let expected_db = std::env::var("CODEX_PLUS_WSL_TEST_DB")
            .expect("CODEX_PLUS_WSL_TEST_DB 必须指向隔离测试数据库");
        let temp = tempfile::tempdir().unwrap();
        let config = temp.path().join("config.toml");
        fs::write(
            &config,
            "[desktop]\nrunCodexInWindowsSubsystemForLinux = true\n",
        )
        .unwrap();
        let context = WslSqliteContext::discover_if_enabled(&config)
            .unwrap()
            .unwrap();
        assert_eq!(context.databases.len(), 1);
        assert_eq!(context.databases[0].path, expected_db);

        let ids = HashSet::from(["thread-1".to_string()]);
        let cwd = HashMap::from([("thread-1".to_string(), "C:/workspace".to_string())]);
        let before = context
            .count_updates("future-provider", &ids, &cwd)
            .unwrap();
        assert_eq!(before.total(), 3);
        context.preflight_write().unwrap();
        let backups = context.create_backups(temp.path()).unwrap();
        let applied = context
            .apply_updates("future-provider", &ids, &cwd, &backups)
            .unwrap();
        assert_eq!(applied.counts.total(), 3);
        assert_eq!(applied.databases_updated, 1);
        assert_eq!(
            context
                .count_updates("future-provider", &ids, &cwd)
                .unwrap()
                .total(),
            0
        );
        context.restore_backups(&backups).unwrap();
        assert!(
            context
                .provider_ids()
                .unwrap()
                .contains(&"old-provider".to_string())
        );
    }
}
