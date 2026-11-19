//! Codex 里「微信连接」页能看到、能改的设置子集（core `connect::page`）：
//! 看不到登录凭据；只认白名单字段；值域不对整体拒绝、不做部分写入。

use codex_plus_core::connect::page::{apply_page_settings, page_settings};
use codex_plus_core::settings::BackendSettings;
use serde_json::json;

fn logged_in_settings() -> BackendSettings {
    BackendSettings {
        weixin_connect_token: "secret-bot-token".into(),
        weixin_connect_account_id: "bot@im.bot".into(),
        weixin_connect_allow_from: "me@im.wechat".into(),
        weixin_connect_work_dir: "/tmp/project".into(),
        ..BackendSettings::default()
    }
}

#[test]
fn page_settings_hide_the_token_and_report_login_state() {
    let settings = logged_in_settings();
    let view = page_settings(&settings);
    assert!(!view.to_string().contains("secret-bot-token"), "页面拿不到登录凭据：{view}");
    assert_eq!(view["loggedIn"], true);
    assert_eq!(view["weixinConnectAccountId"], "bot@im.bot");
    assert_eq!(view["weixinConnectAllowFrom"], "me@im.wechat");
    assert_eq!(view["weixinConnectSandbox"], settings.weixin_connect_sandbox.as_str());
    assert_eq!(view["weixinConnectDefaultMode"], settings.weixin_connect_default_mode.as_str());
    assert_eq!(page_settings(&BackendSettings::default())["loggedIn"], false);
}

#[test]
fn apply_page_settings_round_trips_whitelisted_fields() {
    let mut settings = logged_in_settings();
    apply_page_settings(
        &mut settings,
        &json!({
            "weixinConnectAllowFrom": " a@im.wechat,b@im.wechat ",
            "weixinConnectWorkDir": "/tmp/other",
            "weixinConnectModel": "gpt-5.5",
            "weixinConnectReasoningEffort": "high",
            "weixinConnectDefaultMode": "direct",
            "weixinConnectSandbox": "workspace-write",
            "weixinConnectPushToWeixin": true,
            "weixinConnectAllowOutboundAttachments": true,
        }),
    )
    .unwrap();
    let view = page_settings(&settings);
    assert_eq!(view["weixinConnectAllowFrom"], "a@im.wechat,b@im.wechat", "首尾空白去掉");
    assert_eq!(view["weixinConnectWorkDir"], "/tmp/other");
    assert_eq!(view["weixinConnectModel"], "gpt-5.5");
    assert_eq!(view["weixinConnectReasoningEffort"], "high");
    assert_eq!(view["weixinConnectDefaultMode"], "direct");
    assert_eq!(view["weixinConnectSandbox"], "workspace-write");
    assert_eq!(view["weixinConnectPushToWeixin"], true);
    assert_eq!(view["weixinConnectAllowOutboundAttachments"], true);
    assert_eq!(settings.weixin_connect_token, "secret-bot-token", "凭据不受影响");
}

#[test]
fn apply_page_settings_rejects_unknown_keys_and_bad_values_without_partial_writes() {
    let original = logged_in_settings();
    for patch in [
        json!({ "weixinConnectToken": "evil" }),
        json!({ "weixinConnectBaseUrl": "https://evil.example" }),
        json!({ "weixinConnectAllowFrom": "x@im.wechat", "weixinConnectAccountId": "evil" }),
        json!({ "weixinConnectAllowFrom": "x@im.wechat", "weixinConnectSandbox": "root" }),
        json!({ "weixinConnectDefaultMode": "auto" }),
        json!({ "weixinConnectPushToWeixin": "yes" }),
        json!({ "weixinConnectWorkDir": 42 }),
        json!(["weixinConnectAllowFrom"]),
    ] {
        let mut settings = original.clone();
        assert!(apply_page_settings(&mut settings, &patch).is_err(), "应拒绝 {patch}");
        assert_eq!(settings, original, "被拒的改动不能部分写入：{patch}");
    }
}
