use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::Context;

use crate::settings::BackendSettings;

pub const WRAPPER_EXE: &str = "codex-wrapper.exe";
pub const WRAPPER_SOURCE: &str = "codex-wrapper.cs";
const CLI_HOME_DIR: &str = ".codex-plus-plus-cli";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperInstall {
    pub wrapper_path: PathBuf,
    pub source_path: PathBuf,
    pub real_codex: PathBuf,
    pub codex_home: PathBuf,
}

pub fn ensure_cli_wrapper(settings: &BackendSettings) -> anyhow::Result<Option<WrapperInstall>> {
    let wrapper_dir = wrapper_dir();
    if !should_refresh_cli_wrapper(settings, &wrapper_dir) {
        return Ok(None);
    }
    let real_codex = resolve_real_codex().ok_or_else(|| {
        anyhow::anyhow!("未找到系统 Codex CLI，可先启动一次系统 Codex 或重新安装 Codex")
    })?;
    let codex_home = cli_home_dir();
    let wrapper_settings = wrapper_settings_for_refresh(settings, &wrapper_dir);
    install_cli_wrapper_to(&wrapper_dir, &real_codex, &codex_home, &wrapper_settings).map(Some)
}

pub fn should_refresh_cli_wrapper(settings: &BackendSettings, wrapper_dir: &Path) -> bool {
    settings.cli_wrapper_enabled || wrapper_dir.join(WRAPPER_EXE).is_file()
}

pub fn wrapper_settings_for_refresh(
    settings: &BackendSettings,
    wrapper_dir: &Path,
) -> BackendSettings {
    if settings.cli_wrapper_enabled {
        return settings.clone();
    }

    std::fs::read_to_string(wrapper_dir.join(WRAPPER_SOURCE))
        .ok()
        .and_then(|source| parse_wrapper_source_settings(&source))
        .unwrap_or_else(|| settings.clone())
}

pub fn parse_wrapper_source_settings(source: &str) -> Option<BackendSettings> {
    let mut settings = BackendSettings::default();
    settings.cli_wrapper_api_key_env =
        csharp_string_assignment(source, "apiKeyEnv").unwrap_or(settings.cli_wrapper_api_key_env);
    settings.cli_wrapper_base_url =
        csharp_environment_assignment(source, r#"["OPENAI_BASE_URL"]"#).unwrap_or_default();
    settings.cli_wrapper_api_key = csharp_environment_assignment(source, "[apiKeyEnv]")
        .or_else(|| csharp_string_assignment(source, "apiKey"))
        .unwrap_or_default();

    if settings.cli_wrapper_base_url.is_empty() && settings.cli_wrapper_api_key.is_empty() {
        None
    } else {
        Some(settings)
    }
}

pub fn install_cli_wrapper_to(
    wrapper_dir: &Path,
    real_codex: &Path,
    codex_home: &Path,
    settings: &BackendSettings,
) -> anyhow::Result<WrapperInstall> {
    std::fs::create_dir_all(wrapper_dir)
        .with_context(|| format!("failed to create wrapper dir {}", wrapper_dir.display()))?;
    std::fs::create_dir_all(codex_home)
        .with_context(|| format!("failed to create Codex++ CLI home {}", codex_home.display()))?;

    let source_path = wrapper_dir.join(WRAPPER_SOURCE);
    let wrapper_path = wrapper_dir.join(WRAPPER_EXE);
    let source = build_wrapper_source(real_codex, codex_home, settings);
    std::fs::write(&source_path, source)
        .with_context(|| format!("failed to write {}", source_path.display()))?;

    compile_wrapper(&source_path, &wrapper_path)?;
    Ok(WrapperInstall {
        wrapper_path,
        source_path,
        real_codex: real_codex.to_path_buf(),
        codex_home: codex_home.to_path_buf(),
    })
}

pub fn resolve_real_codex() -> Option<PathBuf> {
    let app_dir = crate::app_paths::resolve_codex_app_dir(None);
    resolve_real_codex_from_candidates(app_dir.as_deref(), &default_user_runtime_candidates())
}

pub fn resolve_real_codex_from_candidates(
    app_dir: Option<&Path>,
    user_runtime_candidates: &[PathBuf],
) -> Option<PathBuf> {
    user_runtime_candidates
        .iter()
        .chain(packaged_codex_candidates(app_dir).iter())
        .find(|path| path.is_file())
        .cloned()
}

pub fn default_user_runtime_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(windows) {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            candidates.push(
                local_app_data
                    .join("OpenAI")
                    .join("Codex")
                    .join("bin")
                    .join("codex.exe"),
            );
        }
    }
    candidates
}

pub fn packaged_codex_candidates(app_dir: Option<&Path>) -> Vec<PathBuf> {
    let Some(app_dir) = app_dir else {
        return Vec::new();
    };
    vec![
        app_dir.join("resources").join("codex.exe"),
        app_dir.join("resources").join("codex"),
    ]
}

pub fn wrapper_dir() -> PathBuf {
    if cfg!(windows) {
        if let Some(roaming) = std::env::var_os("APPDATA").map(PathBuf::from) {
            return wrapper_dir_from_roaming(&roaming);
        }
    }
    crate::paths::default_app_state_dir().join("cli-wrapper")
}

pub fn wrapper_dir_from_roaming(roaming: &Path) -> PathBuf {
    roaming.join("Codex++")
}

pub fn cli_home_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(CLI_HOME_DIR))
        .unwrap_or_else(|| PathBuf::from(CLI_HOME_DIR))
}

pub fn build_wrapper_source(
    real_codex: &Path,
    codex_home: &Path,
    settings: &BackendSettings,
) -> String {
    let base_url_line = if settings.cli_wrapper_base_url.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"        startInfo.EnvironmentVariables["OPENAI_BASE_URL"] = @{};"#,
            cs_string_literal(settings.cli_wrapper_base_url.trim())
        )
    };
    let api_key_line = if settings.cli_wrapper_api_key.trim().is_empty() {
        String::new()
    } else {
        format!(
            r#"        startInfo.EnvironmentVariables[apiKeyEnv] = @{};"#,
            cs_string_literal(settings.cli_wrapper_api_key.trim())
        )
    };
    let api_key_present = if settings.cli_wrapper_api_key.trim().is_empty() {
        "false"
    } else {
        "true"
    };

    format!(
        r#"using System;
using System.Diagnostics;
using System.IO;
using System.Text;

class CodexWrapper
{{
    static int Main(string[] args)
    {{
        string realCodex = @{real_codex};
        string codexHome = @{codex_home};
        string apiKeyEnv = @{api_key_env};
        Directory.CreateDirectory(codexHome);
        string logPath = Path.Combine(codexHome, "codex-wrapper.log");
        AppendLog(logPath, "codex-wrapper start args=" + string.Join(" ", args));
        AppendLog(logPath, "real_codex=" + realCodex);
        AppendLog(logPath, "CODEX_HOME=" + codexHome);
        AppendLog(logPath, "api_key_env=" + apiKeyEnv + " api_key_present={api_key_present}");
        var startInfo = new ProcessStartInfo(realCodex);
        startInfo.UseShellExecute = false;
        startInfo.RedirectStandardInput = false;
        startInfo.RedirectStandardOutput = false;
        startInfo.RedirectStandardError = false;
        startInfo.EnvironmentVariables["CODEX_HOME"] = codexHome;
{base_url_line}
{api_key_line}
        foreach (string arg in args) startInfo.Arguments += QuoteArgument(arg) + " ";
        using (var process = Process.Start(startInfo))
        {{
            process.WaitForExit();
            AppendLog(logPath, "exit_code=" + process.ExitCode);
            return process.ExitCode;
        }}
    }}

    static void AppendLog(string path, string message)
    {{
        File.AppendAllText(path, "[" + DateTime.Now.ToString("yyyy-MM-dd HH:mm:ss") + "] " + message + Environment.NewLine, Encoding.UTF8);
    }}

    static string QuoteArgument(string value)
    {{
        if (value.Length == 0) return "\"\"";
        if (value.IndexOfAny(new char[] {{ ' ', '\t', '\n', '\r', '\"' }}) < 0) return value;
        return "\"" + value.Replace("\\", "\\\\").Replace("\"", "\\\"") + "\"";
    }}
}}
"#,
        real_codex = cs_string_literal(&real_codex.to_string_lossy()),
        codex_home = cs_string_literal(&codex_home.to_string_lossy()),
        api_key_env = cs_string_literal(settings.cli_wrapper_api_key_env.trim()),
    )
}

fn compile_wrapper(source_path: &Path, wrapper_path: &Path) -> anyhow::Result<()> {
    let csc =
        find_csc().ok_or_else(|| anyhow::anyhow!("未找到 csc.exe，无法编译 Codex++ wrapper"))?;
    let output_arg = format!("/out:{}", wrapper_path.display());
    let mut command = Command::new(&csc);
    command
        .args(["/nologo", "/target:exe"])
        .arg(output_arg)
        .arg(source_path);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(crate::windows_integration::CREATE_NO_WINDOW);
    }
    let status = command
        .status()
        .with_context(|| format!("failed to run {}", csc.display()))?;
    if status.success() {
        Ok(())
    } else {
        anyhow::bail!("csc.exe exited with {status}")
    }
}

fn find_csc() -> Option<PathBuf> {
    if let Some(windir) = std::env::var_os("WINDIR").map(PathBuf::from) {
        for relative in [
            ["Microsoft.NET", "Framework64", "v4.0.30319", "csc.exe"],
            ["Microsoft.NET", "Framework", "v4.0.30319", "csc.exe"],
        ] {
            let path = relative
                .iter()
                .fold(windir.clone(), |path, segment| path.join(segment));
            if path.is_file() {
                return Some(path);
            }
        }
    }
    Some(PathBuf::from("csc.exe"))
}

fn cs_string_literal(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}

fn csharp_string_assignment(source: &str, variable: &str) -> Option<String> {
    let marker = format!("string {variable} = @\"");
    let rest = source.split_once(&marker)?.1;
    parse_csharp_verbatim_string(rest)
}

fn csharp_environment_assignment(source: &str, key: &str) -> Option<String> {
    let marker = format!("startInfo.EnvironmentVariables{key} = @\"");
    let rest = source.split_once(&marker)?.1;
    parse_csharp_verbatim_string(rest)
}

fn parse_csharp_verbatim_string(rest: &str) -> Option<String> {
    let mut value = String::new();
    let mut chars = rest.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.peek() == Some(&'"') {
                chars.next();
                value.push('"');
                continue;
            }
            return Some(value);
        }
        value.push(ch);
    }
    None
}
