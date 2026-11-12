use std::net::TcpListener;
use std::time::Duration;

use codex_plus_core::settings::{BackendSettings, SettingsStore};
use codex_plus_manager_lib::commands::{
    weixin_connect_start, weixin_connect_status, weixin_connect_stop,
};

#[test]
fn stops_connection_when_settings_cannot_be_saved() {
    let temp = tempfile::tempdir().unwrap();
    let state = temp.path().join("state");
    unsafe {
        std::env::set_var("CODEX_PLUS_STATE_DIR", &state);
        std::env::set_var("CODEX_HOME", temp.path().join("home"));
        std::env::set_var("CODEX_SQLITE_HOME", temp.path().join("sqlite"));
        std::env::set_var("APPDATA", temp.path().join("appdata"));
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join("xdg"));
        std::env::set_var("NO_PROXY", "127.0.0.1,localhost,::1");
    }

    let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
    listener.set_nonblocking(true).unwrap();
    let store = SettingsStore::default();
    store
        .save(&BackendSettings {
            weixin_connect_base_url: format!("http://{}", listener.local_addr().unwrap()),
            weixin_connect_token: "test-token".into(),
            weixin_connect_account_id: "test-account".into(),
            weixin_connect_work_dir: temp.path().display().to_string(),
            ..BackendSettings::default()
        })
        .unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let started = weixin_connect_start().await;
        assert_eq!(started.status, "ok", "{}", started.message);
        let _connection = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                match listener.accept() {
                    Ok((connection, _)) => break connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        tokio::time::sleep(Duration::from_millis(20)).await;
                    }
                    Err(error) => panic!("无法接收模拟网关连接：{error}"),
                }
            }
        })
        .await
        .expect("微信长轮询没有连接到模拟网关");

        std::fs::create_dir(state.join("settings.json.tmp")).unwrap();
        let stopped = weixin_connect_stop().await;
        assert_eq!(stopped.status, "failed");
        tokio::time::timeout(Duration::from_secs(5), async {
            while weixin_connect_status().payload.state != "stopped" {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("保存失败后，微信连接仍未停止");
        assert!(stopped.message.contains("保存"));
        assert!(stopped.message.contains("下次启动"));
        let serialized = serde_json::to_value(&stopped).unwrap();
        assert_eq!(serialized["message"], stopped.message);
        assert!(store.load().unwrap().weixin_connect_enabled);
    });
}
