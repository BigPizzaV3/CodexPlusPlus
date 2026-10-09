//! 来自固定上游 fce34e3abd1fe8b616e287b9864305c7f0be81a0 的真实 UI 请求契约。
//! 校验读→修改→重新读取；只用 tempfile、测试媒体及合成用量，不连接供应商。

use base64::{Engine, engine::general_purpose::STANDARD};
use codex_plus_core::settings::BackendSettings;
use codex_plus_core::whale_full;
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    path: std::path::PathBuf,
    settings: BackendSettings,
    history: Value,
}
impl Fixture {
    fn new() -> Self {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("whale.sqlite3");
        let mut settings = BackendSettings::default();
        settings.codex_app_whale_widget_enabled = true;
        settings.codex_app_whale_balance_protocol = "off".into();
        let history = json!({"complete":true,"periods":{"today":{"usage":{"totalTokens":100}},"month":{"usage":{"totalTokens":400}},"all":{"usage":{"totalTokens":1000}}},
            "models":[{"model":"gpt-5-test","usage":{"totalTokens":700}},{"model":"other-test","usage":{"totalTokens":300}}],
            "days":[{"date":"2026-10-08","usage":{"totalTokens":100},"models":[{"model":"gpt-5-test","usage":{"totalTokens":70}},{"model":"other-test","usage":{"totalTokens":30}}]}],
            "machineSummary":{"ok":true,"todayTokens":100,"monthTokens":400,"totalTokens":1000,"byModel":{"gpt-5-test":{"tokens":700},"other-test":{"tokens":300}}},"codex":{"ok":true,"todayTokens":100,"monthTokens":400,"totalTokens":1000,"windows":{}}});
        Self {
            _temp: temp,
            path,
            settings,
            history,
        }
    }
    async fn request(&self, path: &str, method: &str, body: Value) -> Value {
        whale_full::handle(
            &self.settings,
            self.path.clone(),
            json!({"path":path,"method":method,"body":body,"query":{}}),
            self.history.clone(),
        )
        .await
    }
    async fn get(&self, path: &str) -> Value {
        self.request(path, "GET", Value::Null).await
    }
    async fn write(&self, path: &str, body: Value) -> Value {
        let response = self.request(path, "POST", body).await;
        assert_eq!(response["status"], 200, "{response}");
        assert_eq!(response["body"]["ok"], true, "{response}");
        response["body"].clone()
    }
}
fn png() -> String {
    "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+/f1sAAAAASUVORK5CYII=".into()
}
fn wav() -> (String, Vec<u8>) {
    let mut bytes = b"RIFF".to_vec();
    bytes.extend(38_u32.to_le_bytes());
    bytes.extend(b"WAVEfmt ");
    bytes.extend(16_u32.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(1_u16.to_le_bytes());
    bytes.extend(44100_u32.to_le_bytes());
    bytes.extend(88200_u32.to_le_bytes());
    bytes.extend(2_u16.to_le_bytes());
    bytes.extend(16_u16.to_le_bytes());
    bytes.extend(b"data");
    bytes.extend(2_u32.to_le_bytes());
    bytes.extend([0, 0]);
    (
        format!("data:audio/wav;base64,{}", STANDARD.encode(&bytes)),
        bytes,
    )
}
fn entry<'a>(rows: &'a Value, id: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == id)
        .unwrap()
}

#[tokio::test]
async fn additional_codex_monitor_receives_statistics_and_builtin_source_stays_codex() {
    let fixture = Fixture::new();
    fixture.write("/dsh-whale/api-models.json", json!({"action":"save","model":{"id":"extra-codex","provider":"codex","name":"Extra Codex","currency":"CNY"}})).await;
    let read = fixture.get("/dsh-whale/api-models.json").await;
    let monitor = entry(&read["body"]["models"], "extra-codex");
    assert_eq!(monitor["codex"]["todayTokens"], 100);
    assert_eq!(monitor["codex"]["totalTokens"], 1000);
    let rejected = fixture.request("/dsh-whale/api-models.json", "POST", json!({"action":"save","model":{"id":"codex","provider":"openai","name":"changed","currency":"USD"}})).await;
    assert_eq!(rejected["status"], 400);
    let read = fixture.get("/dsh-whale/api-models.json").await;
    assert_eq!(entry(&read["body"]["models"], "codex")["provider"], "codex");
}

#[tokio::test]
async fn uploaded_role_pin_unpin_and_delete_survive_fresh_reads() {
    let fixture = Fixture::new();
    let created = fixture
        .write(
            "/dsh-whale/roles.json",
            json!({"name":"测试角色","image":png(),"format":"apng"}),
        )
        .await;
    let role = created["roles"]
        .as_array()
        .unwrap()
        .iter()
        .find(|role| role["id"] != "default")
        .unwrap();
    let id = role["id"].as_str().unwrap();
    assert_eq!(role["format"], "apng");
    fixture
        .write("/dsh-whale/role-pin.json", json!({"id":id,"pinned":true}))
        .await;
    let pinned = fixture.get("/dsh-whale/roles.json").await;
    assert_eq!(pinned["body"]["roles"][0]["id"], id);
    assert_eq!(entry(&pinned["body"]["roles"], id)["pinned"], true);
    fixture
        .write("/dsh-whale/role-pin.json", json!({"id":id,"pinned":false}))
        .await;
    let unpinned = fixture.get("/dsh-whale/roles.json").await;
    assert_eq!(entry(&unpinned["body"]["roles"], id)["pinned"], false);
    fixture
        .write("/dsh-whale/role-delete.json", json!({"id":id}))
        .await;
    assert_eq!(
        fixture.get("/dsh-whale/roles.json").await["body"]["roles"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
async fn default_role_pin_can_be_removed_and_restored_like_original_ui() {
    let fixture = Fixture::new();
    fixture
        .write(
            "/dsh-whale/role-pin.json",
            json!({"id":"default","pinned":false}),
        )
        .await;
    let read = fixture.get("/dsh-whale/roles.json").await;
    assert_eq!(entry(&read["body"]["roles"], "default")["pinned"], false);
    fixture
        .write(
            "/dsh-whale/role-pin.json",
            json!({"id":"default","pinned":true}),
        )
        .await;
    let read = fixture.get("/dsh-whale/roles.json").await;
    assert_eq!(entry(&read["body"]["roles"], "default")["pinned"], true);
    assert!(
        entry(&read["body"]["roles"], "default")["pinnedAt"]
            .as_u64()
            .unwrap()
            > 1
    );
}

#[tokio::test]
async fn imported_audio_fragment_and_group_keep_empty_slot_silent() {
    let fixture = Fixture::new();
    let (data, bytes) = wav();
    let uploaded = fixture
        .write(
            "/dsh-whale/audio.json",
            json!({"action":"upload-fragment","name":"短声音","audio":data}),
        )
        .await;
    let fid = uploaded["id"].as_str().unwrap();
    let saved = fixture
        .write(
            "/dsh-whale/audio.json",
            json!({"action":"save-group","id":"","name":"点按静音组","press":fid,"release":""}),
        )
        .await;
    let group = saved["groups"]
        .as_array()
        .unwrap()
        .iter()
        .find(|g| g["preset"] == false)
        .unwrap();
    let gid = group["id"].as_str().unwrap();
    fixture
        .write(
            "/dsh-whale/audio.json",
            json!({"action":"pin-group","id":gid,"pinned":true}),
        )
        .await;
    let read = fixture.get("/dsh-whale/audio.json").await;
    assert_eq!(read["body"]["groups"][0]["id"], gid);
    assert_eq!(entry(&read["body"]["groups"], gid)["release"], "");
    let media = whale_full::handle(
        &fixture.settings,
        fixture.path.clone(),
        json!({"path":"/dsh-whale/audio-fragment.wav","query":{"id":fid}}),
        json!({}),
    )
    .await;
    assert_eq!(media["mimeType"], "audio/wav");
    assert_eq!(
        STANDARD.decode(media["data"].as_str().unwrap()).unwrap(),
        bytes
    );
    let release = whale_full::handle(
        &fixture.settings,
        fixture.path.clone(),
        json!({"path":"/dsh-whale/sound/release.mp3","query":{"set":gid}}),
        json!({}),
    )
    .await;
    assert_eq!(release["status"], 204);
    fixture
        .write(
            "/dsh-whale/audio.json",
            json!({"action":"delete-group","id":gid}),
        )
        .await;
    fixture
        .write(
            "/dsh-whale/audio.json",
            json!({"action":"delete-fragment","id":fid}),
        )
        .await;
    let read = fixture.get("/dsh-whale/audio.json").await;
    assert!(
        read["body"]["fragments"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["id"] != fid)
    );
    assert!(
        read["body"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["id"] != gid)
    );
}

#[tokio::test]
async fn bubble_sequences_weighted_parallel_modules_and_library_roundtrip() {
    let fixture = Fixture::new();
    let cfg = json!({"v":1,"tapAdvance":true,"items":[{"type":"parallel","id":"step-a","ttlSec":9,"children":[{"id":"weighted-a","w":3,"modules":[{"type":"text","text":"你好","rgb":"champagne"},{"type":"randimg","imgs":[{"imgId":"bimg_petpet","w":2},{"imgId":"bimg_money1","w":1}]}]},{"id":"weighted-b","w":1,"modules":[{"type":"link","text":"我的文档","url":"https://example.invalid/help"}]}]}],"lib":[{"id":"snippet-a","name":"我的模板","modules":[{"type":"rand","lines":[{"text":"甲","w":2},{"text":"乙","w":1}]}]}]});
    fixture.write("/dsh-whale/bubble.json", cfg.clone()).await;
    let read = fixture.get("/dsh-whale/bubble.json").await;
    assert_eq!(read["body"]["config"], cfg);
    let state=fixture.write("/dsh-whale/size.json",json!({"sound":true,"vol":0.36,"turnCostCloseMs":0,"soundSet":"fx1","bubbleOn":true,"scrollGapOn":true,"scrollGapPx":23,"menuBtnHide":true})).await;
    assert_eq!(state["turnCostCloseMs"], 0.0);
    fixture
        .write("/dsh-whale/size.json", json!({"scale":1.2}))
        .await;
    assert_eq!(
        fixture.get("/dsh-whale/size.json").await["body"]["vol"],
        0.36
    );
}

#[tokio::test]
async fn source_ui_can_save_monitor_without_optional_prices_then_set_usd_fx() {
    let fixture = Fixture::new();
    let blank = json!({"action":"save","model":{"id":"price_monitor","name":"我的本机模型","provider":"codex","currency":"USD","keyRef":"","baseUrl":"","matchIds":["gpt-5"],"price":{"hit":"","miss":"","out":"","cur":"CNY","rate":""}}});
    fixture
        .write("/dsh-whale/api-models.json", blank.clone())
        .await;
    let mut priced = blank;
    priced["model"]["price"] = json!({"hit":"0.1","miss":"2","out":"8","cur":"USD","rate":"7.2"});
    fixture.write("/dsh-whale/api-models.json", priced).await;
    let query = whale_full::history_query(&fixture.path);
    assert_eq!(query["prices"][0]["model"], "gpt-5");
    assert_eq!(query["prices"][0]["currency"], "CNY");
    assert!((query["prices"][0]["input"].as_f64().unwrap() - 14.4).abs() < 1e-9);
    assert!((query["prices"][0]["cachedInput"].as_f64().unwrap() - 0.72).abs() < 1e-9);
    let read = fixture.get("/dsh-whale/api-models.json").await;
    assert_eq!(
        entry(&read["body"]["models"], "price_monitor")["price"]["cur"],
        "USD"
    );
}

#[tokio::test]
async fn current_provider_alert_budget_patch_reaches_global_and_model_consumers() {
    let fixture = Fixture::new();
    fixture.write("/dsh-whale/api-models.json",json!({"action":"model-settings","id":"current-provider","alert":{"on":true,"below":3,"lines":[{"type":"text","text":"余额不足"}]},"budget":{"on":true,"amount":42}})).await;
    let settings = fixture.get("/dsh-whale/usage-settings.json").await;
    assert_eq!(settings["body"]["settings"]["alert"]["below"], 3);
    assert_eq!(settings["body"]["settings"]["budget"]["amount"], 42);
    let models = fixture.get("/dsh-whale/api-models.json").await;
    let current = entry(&models["body"]["models"], "current-provider");
    assert_eq!(current["settings"]["alert"]["below"], 3);
    assert_eq!(current["settings"]["budget"]["amount"], 42);
}

#[tokio::test]
async fn explicit_codex_quota_reset_uses_current_cumulative_and_consumes_reset_flag() {
    let mut fixture = Fixture::new();
    fixture.write("/dsh-whale/api-models.json",json!({"action":"model-settings","id":"codex","quota":{"on":true,"mode":"codex","unit":"tokens","total":10000,"used":25,"reset":"none","resetBase":true}})).await;
    let usage = fixture.get("/dsh-whale/usage-settings.json").await;
    assert_eq!(
        usage["body"]["settings"]["models"]["codex"]["quota"]["baseAt"],
        1000
    );
    assert!(
        usage["body"]["settings"]["models"]["codex"]["quota"]
            .get("resetBase")
            .is_none()
    );
    fixture.history["machineSummary"]["totalTokens"] = json!(1060);
    fixture.history["periods"]["all"]["usage"]["totalTokens"] = json!(1060);
    let models = fixture.get("/dsh-whale/api-models.json").await;
    assert_eq!(
        entry(&models["body"]["models"], "codex")["quota"]["autoUsed"],
        85
    );
}

#[tokio::test]
async fn resetting_sound_events_keeps_alert_budget_and_bubble_customization() {
    let fixture = Fixture::new();
    fixture.write("/dsh-whale/usage-settings.json",json!({"taskEnd":{"on":false,"sel":"frag:ya1"},"wait":{"charClose":true},"events":{"question":{"soundOn":true,"vol":0.22},"turnCost":{"autoClose":true,"ttlSec":9}},"budget":{"amount":39}})).await;
    fixture
        .write(
            "/dsh-whale/usage-settings.json",
            json!({"resetEvents":true}),
        )
        .await;
    let read = fixture.get("/dsh-whale/usage-settings.json").await;
    assert_eq!(
        read["body"]["settings"]["taskEnd"],
        json!({"on":true,"sel":"frag:end_a"})
    );
    assert_eq!(read["body"]["settings"]["wait"]["charClose"], false);
    assert_eq!(read["body"]["settings"]["budget"]["amount"], 39);
    assert!(
        read["body"]["settings"]["events"]["turnCost"]
            .get("ttlSec")
            .is_none()
    );
}

#[tokio::test]
async fn automatic_monitor_quota_uses_matching_model_and_codex_mode_uses_machine() {
    let mut fixture = Fixture::new();
    let profile_id = fixture.settings.active_relay_profile().id;
    fixture.write("/dsh-whale/api-models.json", json!({"action":"save","model":{
        "id":"matched_monitor","name":"特定模型额度","provider":"custom","currency":"USD","keyRef":profile_id,
        "matchIds":["gpt-5"],"balance":{"url":"","auth":"","json":{}}
    }})).await;
    fixture.write("/dsh-whale/api-models.json", json!({"action":"model-settings","id":"matched_monitor","quota":{
        "on":true,"mode":"auto","unit":"tokens","total":10000,"used":9,"reset":"none","resetBase":true
    }})).await;
    let settings = fixture.get("/dsh-whale/usage-settings.json").await;
    assert_eq!(
        settings["body"]["settings"]["models"]["matched_monitor"]["quota"]["baseAt"],
        700
    );
    fixture.history["models"][0]["usage"]["totalTokens"] = json!(750);
    fixture.history["machineSummary"]["totalTokens"] = json!(1060);
    let response = fixture.get("/dsh-whale/api-models.json").await;
    assert_eq!(
        entry(&response["body"]["models"], "matched_monitor")["quota"]["autoUsed"],
        59
    );
    fixture
        .write(
            "/dsh-whale/api-models.json",
            json!({"action":"model-settings","id":"matched_monitor","quota":{
                "mode":"codex","used":9,"resetBase":true
            }}),
        )
        .await;
    let settings = fixture.get("/dsh-whale/usage-settings.json").await;
    assert_eq!(
        settings["body"]["settings"]["models"]["matched_monitor"]["quota"]["baseAt"],
        1060
    );
}
