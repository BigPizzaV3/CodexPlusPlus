//! Codex 里「微信连接」页的宿主路由（`weixin_connect_bridge`）：读写往返走管理器的同一套保存逻辑，
//! 页面拿不到登录凭据，也改不了白名单外的字段。

use codex_plus_core::settings::{BackendSettings, SettingsStore};
use codex_plus_manager_lib::weixin_connect_bridge::bridge_request;
use serde_json::json;

#[test]
fn weixin_bridge_round_trips_page_settings_without_exposing_the_token() {
    let temp = tempfile::tempdir().unwrap();
    unsafe {
        std::env::set_var("CODEX_PLUS_STATE_DIR", temp.path().join("state"));
        std::env::set_var("CODEX_HOME", temp.path().join("home"));
        std::env::set_var("CODEX_SQLITE_HOME", temp.path().join("sqlite"));
        std::env::set_var("APPDATA", temp.path().join("appdata"));
        std::env::set_var("XDG_CONFIG_HOME", temp.path().join("xdg"));
    }
    let store = SettingsStore::default();
    store
        .save(&BackendSettings {
            weixin_connect_token: "test-bot-token".into(),
            weixin_connect_account_id: "test-account".into(),
            weixin_connect_allow_from: "me@im.wechat".into(),
            weixin_connect_work_dir: temp.path().display().to_string(),
            ..BackendSettings::default()
        })
        .unwrap();

    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let state = bridge_request(json!({ "action": "state" })).await.unwrap();
        assert_eq!(state["status"], "ok");
        assert_eq!(state["settings"]["loggedIn"], true);
        assert_eq!(state["settings"]["weixinConnectAccountId"], "test-account");
        assert_eq!(state["connection"]["state"], "stopped");
        assert!(!state.to_string().contains("test-bot-token"), "页面拿不到登录凭据：{state}");

        let saved = bridge_request(json!({
            "action": "save",
            "settings": {
                "weixinConnectAllowFrom": "a@im.wechat",
                "weixinConnectSandbox": "workspace-write",
                "weixinConnectDefaultMode": "direct",
                "weixinConnectPushToWeixin": true,
                "weixinConnectModel": "gpt-5.5",
            },
        }))
        .await
        .unwrap();
        assert_eq!(saved["status"], "ok", "{saved}");
        assert_eq!(saved["settings"]["weixinConnectAllowFrom"], "a@im.wechat");
        assert_eq!(saved["settings"]["weixinConnectSandbox"], "workspace-write");

        // 落盘的是同一份设置：管理器读到的和页面看到的一致，凭据没被动过。
        let stored = store.load().unwrap();
        assert_eq!(stored.weixin_connect_allow_from, "a@im.wechat");
        assert_eq!(stored.weixin_connect_sandbox, "workspace-write");
        assert_eq!(stored.weixin_connect_default_mode, "direct");
        assert!(stored.weixin_connect_push_to_weixin);
        assert_eq!(stored.weixin_connect_model, "gpt-5.5");
        assert_eq!(stored.weixin_connect_token, "test-bot-token");

        let reread = bridge_request(json!({ "action": "state" })).await.unwrap();
        assert_eq!(reread["settings"], saved["settings"]);

        // 白名单外的字段、值域外的取值整体拒绝，不落盘。
        for patch in [
            json!({ "weixinConnectToken": "evil" }),
            json!({ "weixinConnectAllowFrom": "b@im.wechat", "weixinConnectSandbox": "root" }),
        ] {
            assert!(bridge_request(json!({ "action": "save", "settings": patch })).await.is_err());
        }
        let stored = store.load().unwrap();
        assert_eq!(stored.weixin_connect_allow_from, "a@im.wechat");
        assert_eq!(stored.weixin_connect_token, "test-bot-token");

        assert!(bridge_request(json!({ "action": "nope" })).await.is_err());
    });
}
