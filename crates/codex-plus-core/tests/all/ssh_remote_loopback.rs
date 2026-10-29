//! issue #1195：本机回环中转地址 + Codex 登记了 SSH 远端时，给出 ssh -R 转发提示。

use codex_plus_core::ssh_remote_loopback::{
    inspect_ssh_remote_loopback_for_home, is_loopback_base_url, ssh_remote_loopback_notice,
};
use serde_json::json;

const LOOPBACK_CONFIG: &str = r#"model_provider = "custom"
openai_base_url = "http://127.0.0.1:57321/v1"

[model_providers.custom]
name = "custom"
base_url = "http://127.0.0.1:57322/v1"
"#;

fn state_with_connection() -> serde_json::Value {
    json!({
        "codex-managed-remote-connections": [
            {
                "hostId": "host-1",
                "sshAlias": "devbox",
                "sshHost": "dev.example.test",
                "sshUser": "alice",
                "sshPort": 2222
            }
        ]
    })
}

#[test]
fn loopback_hosts_are_detected() {
    for url in [
        "http://127.0.0.1:57321/v1",
        "http://127.8.9.10:1/v1",
        "http://localhost:8080",
        "http://LOCALHOST:8080",
        "http://gw.localhost:8080/v1",
        "http://[::1]:57321/v1",
    ] {
        assert!(is_loopback_base_url(url), "{url}");
    }
    for url in [
        "https://relay.example.test/v1",
        "http://192.168.1.2:8080/v1",
        "http://10.0.0.1/v1",
        "not a url",
        "",
    ] {
        assert!(!is_loopback_base_url(url), "{url}");
    }
}

#[test]
fn notice_lists_loopback_endpoints_and_forward_commands_for_each_remote() {
    let notice = ssh_remote_loopback_notice(LOOPBACK_CONFIG, &state_with_connection()).unwrap();

    let keys = notice
        .loopback_endpoints
        .iter()
        .map(|endpoint| endpoint.key.as_str())
        .collect::<Vec<_>>();
    assert_eq!(keys, ["openai_base_url", "model_providers.custom.base_url"]);
    assert_eq!(notice.ports, [57321, 57322]);
    assert_eq!(notice.remotes.len(), 1);
    let remote = &notice.remotes[0];
    assert_eq!(remote.label, "devbox");
    assert_eq!(
        remote.forward_command,
        "ssh -R 57321:127.0.0.1:57321 -R 57322:127.0.0.1:57322 -p 2222 devbox"
    );
    assert_eq!(
        remote.ssh_config_lines,
        [
            "RemoteForward 57321 127.0.0.1:57321",
            "RemoteForward 57322 127.0.0.1:57322"
        ]
    );
}

#[test]
fn forward_command_uses_user_and_host_without_alias_and_rejects_unsafe_values() {
    let state = json!({
        "codex-managed-remote-connections": [
            { "hostId": "a", "sshHost": "dev.example.test", "sshUser": "alice" },
            { "hostId": "b", "sshHost": "evil.test; rm -x", "sshUser": "bob" }
        ]
    });
    let notice = ssh_remote_loopback_notice(LOOPBACK_CONFIG, &state).unwrap();
    assert_eq!(notice.remotes.len(), 2);
    assert!(
        notice.remotes[0]
            .forward_command
            .ends_with(" alice@dev.example.test")
    );
    assert!(notice.remotes[1].forward_command.ends_with(" user@server"));
}

#[test]
fn notice_keeps_localhost_and_ipv6_forward_targets() {
    let config = r#"model_provider = "relay"

[model_providers.relay]
base_url = "http://[::1]:9000/v1"
"#;
    let notice = ssh_remote_loopback_notice(config, &state_with_connection()).unwrap();
    assert_eq!(notice.ports, [9000]);
    assert!(
        notice.remotes[0]
            .forward_command
            .contains("-R 9000:[::1]:9000")
    );

    let config = "model_provider = \"relay\"\n\n[model_providers.relay]\nbase_url = \"http://localhost:9001/v1\"\n";
    let notice = ssh_remote_loopback_notice(config, &state_with_connection()).unwrap();
    assert!(
        notice.remotes[0]
            .forward_command
            .contains("-R 9001:localhost:9001")
    );
}

#[test]
fn openai_session_identity_checks_custom_transport() {
    let config = r#"model_provider = "openai"

[model_providers.custom]
base_url = "http://127.0.0.1:57321/v1"
"#;
    let notice = ssh_remote_loopback_notice(config, &state_with_connection()).unwrap();
    assert_eq!(
        notice.loopback_endpoints[0].key,
        "model_providers.custom.base_url"
    );
}

#[test]
fn no_notice_without_remote_connections_or_loopback_urls() {
    // 没有 SSH 远端：本机回环地址是正常用法，不提示。
    assert!(ssh_remote_loopback_notice(LOOPBACK_CONFIG, &json!({})).is_none());
    assert!(
        ssh_remote_loopback_notice(
            LOOPBACK_CONFIG,
            &json!({ "codex-managed-remote-connections": [] })
        )
        .is_none()
    );
    // 中转地址不是回环：远端也能直接访问，不提示。
    let remote_config = r#"model_provider = "custom"

[model_providers.custom]
base_url = "https://relay.example.test/v1"
"#;
    assert!(ssh_remote_loopback_notice(remote_config, &state_with_connection()).is_none());
    // 非活动 provider 的回环地址不算。
    let inactive = r#"model_provider = "custom"

[model_providers.custom]
base_url = "https://relay.example.test/v1"

[model_providers.other]
base_url = "http://127.0.0.1:1/v1"
"#;
    assert!(ssh_remote_loopback_notice(inactive, &state_with_connection()).is_none());
    // 坏掉的 TOML 不报错，只是不提示。
    assert!(ssh_remote_loopback_notice("[[[", &state_with_connection()).is_none());
}

#[test]
fn unresolvable_remote_uses_placeholder_host() {
    let state =
        json!({ "remote-projects": [{ "id": "p1", "hostId": "h1", "remotePath": "/srv" }] });
    let notice = ssh_remote_loopback_notice(LOOPBACK_CONFIG, &state).unwrap();
    assert_eq!(notice.remotes.len(), 1);
    assert!(notice.remotes[0].forward_command.ends_with(" user@server"));
}

#[test]
fn inspect_reads_codex_home_files() {
    let temp = tempfile::tempdir().unwrap();
    assert!(inspect_ssh_remote_loopback_for_home(temp.path()).is_none());
    std::fs::write(temp.path().join("config.toml"), LOOPBACK_CONFIG).unwrap();
    assert!(inspect_ssh_remote_loopback_for_home(temp.path()).is_none());
    std::fs::write(
        temp.path().join(".codex-global-state.json"),
        state_with_connection().to_string(),
    )
    .unwrap();
    let notice = inspect_ssh_remote_loopback_for_home(temp.path()).unwrap();
    assert_eq!(notice.ports, [57321, 57322]);
}
