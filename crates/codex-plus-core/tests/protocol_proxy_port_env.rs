//! issue #2189：协议代理端口要能整体挪动（`CODEX_PLUS_PROTOCOL_PROXY_PORT`）。
//!
//! 该端口写死在 `config.toml` 的 `base_url` 里，少数 Windows 机器上 57321 恰好被
//! Hyper-V/WSL 划进动态端口排除区间，bind 永远报 os error 10013。这里验证
//! 环境变量覆盖后，写入侧（`relay_profile_base_url`）与检测侧
//! （`responses_proxy_configured_in_home`）读到的是同一个生效端口。

use std::sync::Mutex;

use codex_plus_core::protocol_proxy::{
    DEFAULT_PROTOCOL_PROXY_PORT, local_responses_proxy_base_url, protocol_proxy_port,
};
use codex_plus_core::relay_config::{relay_profile_base_url, responses_proxy_configured_in_home};
use codex_plus_core::settings::{RelayMode, RelayProfile};

/// 环境变量是进程全局的，同一二进制内的测试并发跑，必须串行化访问。
/// 中毒容忍：一个用例失败不应把后续用例的锁也一起带崩。
static ENV_LOCK: Mutex<()> = Mutex::new(());

fn lock_env() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|error| error.into_inner())
}

fn set_env_port(value: &str) {
    // SAFETY: 对环境变量的全部访问都在 ENV_LOCK 之内，本测试二进制无并发读取。
    unsafe { std::env::set_var("CODEX_PLUS_PROTOCOL_PROXY_PORT", value) };
}

fn clear_env_port() {
    // SAFETY: 同上。
    unsafe { std::env::remove_var("CODEX_PLUS_PROTOCOL_PROXY_PORT") };
}

#[test]
fn env_override_moves_the_whole_proxy_port_chain() {
    let _guard = lock_env();
    set_env_port("58077");

    assert_eq!(protocol_proxy_port(), 58077);
    // 聚合供应商的 base_url 直接走本地协议代理，是最能代表「生效端口」的出口。
    let aggregate_profile = RelayProfile {
        relay_mode: RelayMode::Aggregate,
        ..RelayProfile::default()
    };
    assert!(
        relay_profile_base_url(&aggregate_profile).contains("127.0.0.1:58077"),
        "aggregate profile base_url should follow the overridden port"
    );

    // 写入侧与检测侧使用同一端口：指向 58077 的 config.toml 应被判定为协议代理已激活。
    let home = tempfile::tempdir().unwrap();
    let config = format!(
        "model_provider = \"custom\"\n\n[model_providers.custom]\nname = \"custom\"\nwire_api = \"responses\"\nbase_url = \"{}\"\n",
        local_responses_proxy_base_url(58077)
    );
    std::fs::write(home.path().join("config.toml"), config).unwrap();
    assert!(responses_proxy_configured_in_home(home.path()));

    // 环境变量移除后生效端口回到默认：同一份配置不再被识别，
    // 这正是「挪端口必须连同 config.toml 一起重写」的一致性要求。
    clear_env_port();
    assert_eq!(protocol_proxy_port(), DEFAULT_PROTOCOL_PROXY_PORT);
    assert!(!responses_proxy_configured_in_home(home.path()));
}

#[test]
fn invalid_env_values_fall_back_to_the_default_port() {
    let _guard = lock_env();
    for value in ["", "  ", "abc", "0", "70000", "-1"] {
        set_env_port(value);
        assert_eq!(
            protocol_proxy_port(),
            DEFAULT_PROTOCOL_PROXY_PORT,
            "input: {value}"
        );
    }
    clear_env_port();
    assert_eq!(protocol_proxy_port(), DEFAULT_PROTOCOL_PROXY_PORT);
}
