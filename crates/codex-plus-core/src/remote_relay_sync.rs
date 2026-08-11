use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::Context;
use base64::Engine;
use serde::Serialize;
use serde_json::Value;
use toml_edit::{DocumentMut, Item, Table};
use uuid::Uuid;

use crate::settings::{RelayMode, RelayProfile, RelayProtocol};

const MANAGED_ROOT_KEYS: &[&str] = &[
    "model_provider",
    "openai_base_url",
    "chatgpt_base_url",
    "base_url",
    "OPENAI_API_KEY",
    "model_catalog_json",
    "model_context_window",
    "model_auto_compact_token_limit",
    "codex_plus_chat_base_url",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
/// Metadata returned after a provider is synchronized to an SSH host.
pub struct RemoteRelaySyncResult {
    pub ssh_target: String,
    pub codex_home: String,
    pub backup_path: String,
    pub model_provider: String,
    pub app_server_restarted: bool,
}

/// Synchronizes one provider's routing and authentication without replacing
/// unrelated remote Codex configuration.
pub fn sync_relay_profile_to_ssh(
    ssh_target: &str,
    remote_codex_home: &str,
    profile: &RelayProfile,
) -> anyhow::Result<RemoteRelaySyncResult> {
    let ssh_target = validate_ssh_target(ssh_target)?;
    let remote_codex_home = validate_remote_codex_home(remote_codex_home)?;
    validate_remote_profile(profile)?;

    let existing_config = read_remote_config(&ssh_target, &remote_codex_home)?;
    let next_config = merge_remote_config(&existing_config, profile)?;
    let model_provider = root_string(&next_config, "model_provider").unwrap_or_default();
    let auth_contents = validated_auth_contents(profile)?;
    let token = Uuid::new_v4().simple().to_string();
    let remote_stage = format!("/tmp/codexpp-provider-{token}");
    let output = apply_remote_profile(
        &ssh_target,
        &remote_codex_home,
        &remote_stage,
        &next_config,
        &auth_contents,
    )?;
    let backup_path =
        marker_value(&output, "__CODEXPP_BACKUP__").context("远端切换完成但未返回备份路径")?;
    let app_server_restarted =
        marker_value(&output, "__CODEXPP_RESTARTED__").as_deref() == Some("1");

    Ok(RemoteRelaySyncResult {
        ssh_target,
        codex_home: if remote_codex_home.is_empty() {
            "~/.codex".to_string()
        } else {
            remote_codex_home
        },
        backup_path,
        model_provider,
        app_server_restarted,
    })
}

fn merge_remote_config(existing: &str, profile: &RelayProfile) -> anyhow::Result<String> {
    let mut remote = parse_doc(existing).context("远端 config.toml 解析失败")?;
    let previous_provider = active_provider(&remote);
    clear_managed_routing(&mut remote, previous_provider.as_deref());

    if profile.relay_mode == RelayMode::Official && !profile.official_mix_api_key {
        remote["model_provider"] = toml_edit::value("openai");
    } else {
        if profile.protocol != RelayProtocol::Responses {
            anyhow::bail!("SSH 远端同步目前只支持 Responses 协议供应商。");
        }
        let generated = crate::relay_config::complete_relay_profile_config(profile)?;
        let source = parse_doc(&generated).context("供应商 config.toml 解析失败")?;
        copy_root_keys(
            &source,
            &mut remote,
            &[
                "model",
                "model_provider",
                "openai_base_url",
                "base_url",
                "model_context_window",
                "model_auto_compact_token_limit",
            ],
        );
        copy_active_provider(&source, &mut remote)?;
    }

    remote.as_table_mut().remove("model_catalog_json");
    Ok(ensure_newline(remote.to_string()))
}

fn validate_remote_profile(profile: &RelayProfile) -> anyhow::Result<()> {
    if profile.relay_mode == RelayMode::Aggregate {
        anyhow::bail!("聚合供应商暂不支持 SSH 远端同步。");
    }
    if profile.relay_mode != RelayMode::Official && profile.protocol != RelayProtocol::Responses {
        anyhow::bail!("SSH 远端同步目前只支持 Responses 协议供应商。");
    }
    Ok(())
}

fn validated_auth_contents(profile: &RelayProfile) -> anyhow::Result<String> {
    let raw = profile.auth_contents.trim();
    if raw.is_empty() {
        anyhow::bail!("供应商未保存 auth.json，无法同步到远端。");
    }
    let auth: Value = serde_json::from_str(raw).context("供应商 auth.json JSON 解析失败")?;
    let object = auth
        .as_object()
        .context("供应商 auth.json 必须是 JSON 对象")?;
    if profile.relay_mode == RelayMode::Official {
        let is_chatgpt = object.get("auth_mode").and_then(Value::as_str) == Some("chatgpt")
            && object.get("tokens").and_then(Value::as_object).is_some();
        if !is_chatgpt {
            anyhow::bail!("官方登录供应商缺少 ChatGPT 登录凭据，已停止远端同步。");
        }
    } else if object
        .get("OPENAI_API_KEY")
        .and_then(Value::as_str)
        .is_none_or(|key| key.trim().is_empty())
    {
        anyhow::bail!("纯 API 供应商缺少 OPENAI_API_KEY，已停止远端同步。");
    }
    Ok(format!("{}\n", serde_json::to_string_pretty(&auth)?))
}

fn clear_managed_routing(doc: &mut DocumentMut, previous_provider: Option<&str>) {
    for key in MANAGED_ROOT_KEYS {
        doc.as_table_mut().remove(key);
    }
    for provider in previous_provider
        .into_iter()
        .chain(["custom", "CodexPlusPlus", "CodexPP"])
    {
        if provider != "openai" {
            remove_provider(doc, provider);
        }
    }
}

fn copy_root_keys(source: &DocumentMut, target: &mut DocumentMut, keys: &[&str]) {
    for key in keys {
        if let Some(value) = source.get(key) {
            target.as_table_mut().insert(key, value.clone());
        }
    }
}

fn copy_active_provider(source: &DocumentMut, target: &mut DocumentMut) -> anyhow::Result<()> {
    let provider_id = active_provider(source).context("供应商缺少 model_provider")?;
    let provider = source
        .get("model_providers")
        .and_then(Item::as_table)
        .and_then(|providers| providers.get(&provider_id))
        .cloned()
        .with_context(|| format!("供应商缺少 model_providers.{provider_id}"))?;
    if target
        .get("model_providers")
        .and_then(Item::as_table)
        .is_none()
    {
        target["model_providers"] = Item::Table(Table::new());
    }
    target["model_providers"]
        .as_table_mut()
        .context("远端 model_providers 不是 TOML table")?
        .insert(&provider_id, provider);
    Ok(())
}

fn remove_provider(doc: &mut DocumentMut, provider: &str) {
    let remove_parent = doc
        .get_mut("model_providers")
        .and_then(Item::as_table_mut)
        .map(|providers| {
            providers.remove(provider);
            providers.is_empty()
        })
        .unwrap_or(false);
    if remove_parent {
        doc.as_table_mut().remove("model_providers");
    }
}

fn active_provider(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(Item::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

fn root_string(contents: &str, key: &str) -> Option<String> {
    parse_doc(contents)
        .ok()?
        .get(key)
        .and_then(Item::as_str)
        .map(ToString::to_string)
}

fn parse_doc(contents: &str) -> anyhow::Result<DocumentMut> {
    if contents.trim().is_empty() {
        Ok(DocumentMut::new())
    } else {
        contents
            .trim_start_matches('\u{feff}')
            .parse::<DocumentMut>()
            .map_err(Into::into)
    }
}

fn ensure_newline(mut value: String) -> String {
    if !value.ends_with('\n') {
        value.push('\n');
    }
    value
}

fn validate_ssh_target(value: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        anyhow::bail!("请填写 SSH 主机别名。");
    }
    if value.starts_with('-')
        || value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || matches!(ch, '/' | '\\' | '?' | '#'))
    {
        anyhow::bail!("SSH 主机别名格式无效。");
    }
    Ok(value.to_string())
}

fn validate_remote_codex_home(value: &str) -> anyhow::Result<String> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() || value == "~/.codex" {
        return Ok(String::new());
    }
    if !value.starts_with('/') || value.chars().any(char::is_control) {
        anyhow::bail!("远端 CODEX_HOME 必须是绝对路径。");
    }
    Ok(value.trim_end_matches('/').to_string())
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn remote_home_assignment(remote_codex_home: &str) -> String {
    if remote_codex_home.is_empty() {
        "codex_home=\"$HOME/.codex\"".to_string()
    } else {
        format!("codex_home={}", shell_quote(remote_codex_home))
    }
}

fn ssh_command(target: &str) -> Command {
    let mut command = Command::new("ssh");
    command
        .arg("-o")
        .arg("BatchMode=yes")
        .arg("-o")
        .arg("ConnectTimeout=10")
        .arg("-o")
        .arg("ServerAliveInterval=10")
        .arg("-o")
        .arg("ServerAliveCountMax=3")
        .arg(target);
    command
}

fn read_remote_config(target: &str, remote_codex_home: &str) -> anyhow::Result<String> {
    let script = format!(
        "set -eu; {}; if [ -f \"$codex_home/config.toml\" ]; then cat \"$codex_home/config.toml\"; fi",
        remote_home_assignment(remote_codex_home)
    );
    let output = ssh_command(target)
        .arg(script)
        .output()
        .context("无法启动 ssh")?;
    command_stdout(output, "读取远端 config.toml")
}

fn apply_remote_profile(
    target: &str,
    remote_codex_home: &str,
    remote_stage: &str,
    config_contents: &str,
    auth_contents: &str,
) -> anyhow::Result<String> {
    let home_assignment = remote_home_assignment(remote_codex_home);
    let stage = shell_quote(remote_stage);
    let script = format!(
        r#"set -eu
umask 077
{home_assignment}
stage={stage}
mkdir "$stage"
cleanup_stage() {{
  rm -f "$stage/config.toml" "$stage/auth.json"
  rmdir "$stage" 2>/dev/null || true
}}
trap cleanup_stage EXIT HUP INT TERM
if ! command -v base64 >/dev/null 2>&1; then
  echo "远端缺少 base64 命令" >&2
  exit 1
fi
IFS= read -r config_payload
IFS= read -r auth_payload
decode_payload() {{
  payload=$1
  destination=$2
  if printf '%s' "$payload" | base64 --decode > "$destination" 2>/dev/null; then return 0; fi
  if printf '%s' "$payload" | base64 -d > "$destination" 2>/dev/null; then return 0; fi
  if printf '%s' "$payload" | base64 -D > "$destination" 2>/dev/null; then return 0; fi
  echo "远端无法解码供应商配置" >&2
  return 1
}}
decode_payload "$config_payload" "$stage/config.toml"
decode_payload "$auth_payload" "$stage/auth.json"
chmod 600 "$stage/config.toml" "$stage/auth.json"
mkdir -p "$codex_home/backups"
backup="$codex_home/backups/remote-provider-switch-$(date +%Y%m%d-%H%M%S)-$$"
mkdir "$backup"
had_config=0
had_auth=0
if [ -f "$codex_home/config.toml" ]; then cp -p "$codex_home/config.toml" "$backup/config.toml"; had_config=1; fi
if [ -f "$codex_home/auth.json" ]; then cp -p "$codex_home/auth.json" "$backup/auth.json"; had_auth=1; fi
restore_on_error() {{
  status=$?
  cleanup_stage
  if [ "$status" -ne 0 ]; then
    if [ "$had_config" -eq 1 ]; then cp -p "$backup/config.toml" "$codex_home/config.toml"; else rm -f "$codex_home/config.toml"; fi
    if [ "$had_auth" -eq 1 ]; then cp -p "$backup/auth.json" "$codex_home/auth.json"; else rm -f "$codex_home/auth.json"; fi
  fi
  exit "$status"
}}
trap restore_on_error EXIT HUP INT TERM
cp "$stage/config.toml" "$codex_home/.config.toml.codexpp-new"
cp "$stage/auth.json" "$codex_home/.auth.json.codexpp-new"
chmod 600 "$codex_home/.config.toml.codexpp-new" "$codex_home/.auth.json.codexpp-new"
mv "$codex_home/.config.toml.codexpp-new" "$codex_home/config.toml"
mv "$codex_home/.auth.json.codexpp-new" "$codex_home/auth.json"
restarted=0
pid_file="$codex_home/app-server-control/app-server.pid"
if [ -r "$pid_file" ]; then
  pid=$(cat "$pid_file" 2>/dev/null || true)
  case "$pid" in
    *[!0-9]*|'') ;;
    *)
      if kill -0 "$pid" 2>/dev/null; then
        descendant_pids=$(ps -eo pid=,ppid= | awk -v root="$pid" '
          {{ parent[$1] = $2 }}
          END {{
            for (candidate in parent) {{
              current = candidate
              depth = 0
              while ((current in parent) && parent[current] != root && parent[current] != current && depth < 1024) {{
                current = parent[current]
                depth++
              }}
              if ((current in parent) && parent[current] == root) print candidate
            }}
          }}
        ')
        managed_pids="$descendant_pids $pid"
        for managed_pid in $managed_pids; do
          kill "$managed_pid" 2>/dev/null || true
        done
        i=0
        while [ "$i" -lt 100 ]; do
          alive=0
          for managed_pid in $managed_pids; do
            if kill -0 "$managed_pid" 2>/dev/null; then alive=1; break; fi
          done
          if [ "$alive" -eq 0 ]; then break; fi
          sleep 0.1
          i=$((i + 1))
        done
        remaining=""
        for managed_pid in $managed_pids; do
          if kill -0 "$managed_pid" 2>/dev/null; then remaining="$remaining $managed_pid"; fi
        done
        if [ -n "$remaining" ]; then echo "旧 app-server 进程树未能正常退出:$remaining" >&2; exit 1; fi
        restarted=1
      fi
      ;;
  esac
fi
rm -f "$codex_home/app-server-control/app-server.pid" "$codex_home/app-server-control/app-server-control.sock"
printf '__CODEXPP_BACKUP__%s\n' "$backup"
printf '__CODEXPP_RESTARTED__%s\n' "$restarted"
trap - EXIT HUP INT TERM
cleanup_stage
"#
    );
    let mut child = ssh_command(target)
        .arg(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("无法启动 ssh")?;
    let config_payload = base64::engine::general_purpose::STANDARD.encode(config_contents);
    let auth_payload = base64::engine::general_purpose::STANDARD.encode(auth_contents);
    let write_result = (|| -> anyhow::Result<()> {
        let mut stdin = child.stdin.take().context("无法打开 ssh 标准输入")?;
        writeln!(stdin, "{config_payload}").context("无法传输远端 config.toml")?;
        writeln!(stdin, "{auth_payload}").context("无法传输远端 auth.json")?;
        Ok(())
    })();
    let output = child.wait_with_output().context("等待 ssh 结束失败")?;
    let stdout = command_stdout(output, "应用远端供应商配置")?;
    write_result?;
    Ok(stdout)
}

fn command_stdout(output: std::process::Output, action: &str) -> anyhow::Result<String> {
    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let detail = stderr.trim().chars().take(400).collect::<String>();
    if detail.is_empty() {
        anyhow::bail!("{action}失败，退出码 {:?}", output.status.code());
    }
    anyhow::bail!("{action}失败：{detail}");
}

fn marker_value(output: &str, marker: &str) -> Option<String> {
    output
        .lines()
        .find_map(|line| line.strip_prefix(marker))
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn profile(mode: RelayMode, config: &str) -> RelayProfile {
        RelayProfile {
            relay_mode: mode,
            protocol: RelayProtocol::Responses,
            config_contents: config.to_string(),
            auth_contents: if mode == RelayMode::Official {
                r#"{"auth_mode":"chatgpt","tokens":{}}"#.to_string()
            } else {
                r#"{"OPENAI_API_KEY":"secret"}"#.to_string()
            },
            ..RelayProfile::default()
        }
    }

    #[test]
    fn official_merge_preserves_remote_context_and_removes_relay() {
        let existing = r#"model = "gpt-test"
model_provider = "custom"
model_context_window = 999

[model_providers.custom]
base_url = "http://127.0.0.1:57325/v1"
wire_api = "responses"

[projects."/remote/work"]
trust_level = "trusted"
"#;
        let merged = merge_remote_config(existing, &profile(RelayMode::Official, "")).unwrap();
        assert!(merged.contains("model_provider = \"openai\""));
        assert!(merged.contains("model = \"gpt-test\""));
        assert!(merged.contains("[projects.\"/remote/work\"]"));
        assert!(!merged.contains("model_providers.custom"));
        assert!(!merged.contains("model_context_window"));
        assert!(!merged.contains("57325"));
    }

    #[test]
    fn api_merge_copies_only_routing_and_preserves_remote_context() {
        let existing = r#"model = "old"
model_provider = "openai"

[plugins.remote]
enabled = true
"#;
        let config = r#"model = "gpt-relay"
model_provider = "custom"

[model_providers.custom]
name = "custom"
wire_api = "responses"
base_url = "https://relay.example/v1"

[projects."/local/only"]
trust_level = "trusted"
"#;
        let merged = merge_remote_config(existing, &profile(RelayMode::PureApi, config)).unwrap();
        assert!(merged.contains("model = \"gpt-relay\""));
        assert!(merged.contains("model_provider = \"custom\""));
        assert!(merged.contains("https://relay.example/v1"));
        assert!(merged.contains("[plugins.remote]"));
        assert!(!merged.contains("/local/only"));
    }

    #[test]
    fn rejects_unsafe_remote_inputs() {
        assert!(validate_ssh_target("-oProxyCommand=bad").is_err());
        assert!(validate_ssh_target("host name").is_err());
        assert!(validate_remote_codex_home("relative/path").is_err());
        assert_eq!(validate_remote_codex_home("~/.codex/").unwrap(), "");
    }

    #[test]
    fn rejects_profiles_without_matching_authentication() {
        let mut official = profile(RelayMode::Official, "");
        official.auth_contents = r#"{"auth_mode":"chatgpt"}"#.to_string();
        assert!(validated_auth_contents(&official).is_err());

        let mut pure_api = profile(RelayMode::PureApi, "");
        pure_api.auth_contents = r#"{"OPENAI_API_KEY":""}"#.to_string();
        assert!(validated_auth_contents(&pure_api).is_err());
    }

    #[test]
    fn marker_parser_ignores_unrelated_output() {
        let output = "notice\n__CODEXPP_BACKUP__/tmp/backup\n";
        assert_eq!(
            marker_value(output, "__CODEXPP_BACKUP__").as_deref(),
            Some("/tmp/backup")
        );
    }
}
