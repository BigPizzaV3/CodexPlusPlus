//! 「Agent 桌宠」页用的命令：图库与图片预览、音效目录与试听、导入自定义音效。
//! 预览与 Codex 里的桌宠共用同一个缓存目录，选过的图 / 音效切到 Codex 时不必再下载。

use base64::Engine;
use serde::Serialize;

use codex_plus_core::agent_pet::{self, AgentPetCatalogEntry, AgentPetSoundEntry};

/// 图片预览结果；失败时只带 `message`。
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetImageResult {
    /// `ok` 或 `failed`。
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 内置图库：`[{ id, name }]`。
#[tauri::command]
pub fn agent_pet_catalog() -> Vec<AgentPetCatalogEntry> {
    agent_pet::catalog()
}

/// 按压 / 松开音效，供页面试听：`{ duck: { press, release }, fx1: {…} }`，值为 data URI。
#[tauri::command]
pub fn agent_pet_sounds() -> serde_json::Value {
    codex_plus_core::assets::agent_pet_sounds()
}

/// 可下载音效组目录（含作者与授权）：`[{ id, name, author, license, sourceUrl }]`。
#[tauri::command]
pub fn agent_pet_sound_catalog() -> Vec<AgentPetSoundEntry> {
    agent_pet::sound_catalog()
}

/// 音效试听结果；失败时只带 `message`。
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetSoundResult {
    /// `ok` 或 `failed`。
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub press: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 取一组音效（下载组会联网并缓存）。前端 invoke 参数为 `{ soundSet, customSoundPress, customSoundRelease }`。
#[tauri::command]
pub async fn agent_pet_sound(
    sound_set: String,
    custom_sound_press: String,
    custom_sound_release: String,
) -> AgentPetSoundResult {
    let result = agent_pet::load_sound(
        &sound_set,
        &custom_sound_press,
        &custom_sound_release,
        &agent_pet::default_cache_dir(),
        &agent_pet::default_custom_dir(),
    )
    .await;
    match result {
        Ok(sound) => AgentPetSoundResult {
            status: "ok",
            key: Some(sound.key),
            press: sound.press,
            release: sound.release,
            message: None,
        },
        Err(error) => AgentPetSoundResult {
            status: "failed",
            key: None,
            press: None,
            release: None,
            message: Some(format!("{error:#}")),
        },
    }
}

/// 导入结果：成功时带保存后的文件名，前端写进 `customSoundPress` / `customSoundRelease`。
#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AgentPetImportResult {
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// 导入一段本地音频。前端 invoke 参数为 `{ slot, data }`，`data` 是文件内容的 base64。
#[tauri::command]
pub fn agent_pet_import_sound(slot: String, data: String) -> AgentPetImportResult {
    import_result(&slot, &data, &agent_pet::default_custom_dir())
}

fn import_result(slot: &str, data: &str, custom_dir: &std::path::Path) -> AgentPetImportResult {
    let result = base64::engine::general_purpose::STANDARD
        .decode(data.trim())
        .map_err(|_| anyhow::anyhow!("音频数据编码无效"))
        .and_then(|bytes| agent_pet::import_custom_sound(slot, &bytes, custom_dir));
    match result {
        Ok(file) => AgentPetImportResult {
            status: "ok",
            file: Some(file),
            message: None,
        },
        Err(error) => AgentPetImportResult {
            status: "failed",
            file: None,
            message: Some(format!("{error:#}")),
        },
    }
}

/// 取一张桌宠图片做预览。前端 invoke 参数为 `{ image, customImageUrl }`。
#[tauri::command]
pub async fn agent_pet_image(image: String, custom_image_url: String) -> AgentPetImageResult {
    image_result(
        agent_pet::load_image(&image, &custom_image_url, &agent_pet::default_cache_dir()).await,
    )
}

fn image_result(result: anyhow::Result<agent_pet::AgentPetImage>) -> AgentPetImageResult {
    match result {
        Ok(image) => AgentPetImageResult {
            status: "ok",
            image: Some(image.data_uri),
            message: None,
        },
        Err(error) => AgentPetImageResult {
            status: "failed",
            image: None,
            message: Some(format!("{error:#}")),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sounds_cover_both_sets_with_press_and_release() {
        let value = agent_pet_sounds();
        for set in ["duck", "fx1"] {
            for slot in ["press", "release"] {
                let uri = value[set][slot].as_str().unwrap();
                assert!(uri.starts_with("data:audio/mpeg;base64,"), "{set}.{slot}");
            }
        }
    }

    #[test]
    fn catalog_serializes_as_id_and_name() {
        let value = serde_json::to_value(agent_pet_catalog()).unwrap();
        assert_eq!(value[0], json!({"id": "whale", "name": "鲸鱼"}));
        assert_eq!(value.as_array().unwrap().len(), 22);
        let anime = value.as_array().unwrap().iter().find(|entry| entry["id"] == "chibi-bow").unwrap();
        assert_eq!(anime["credit"], "asrafil · CC0");
    }

    #[test]
    fn image_result_uses_status_image_message_fields() {
        let ok = image_result(Ok(agent_pet::AgentPetImage {
            data_uri: "data:image/png;base64,AA==".into(),
            source_url: "https://cdn.jsdelivr.net/x.png".into(),
            cached: true,
        }));
        assert_eq!(
            serde_json::to_value(ok).unwrap(),
            json!({"status": "ok", "image": "data:image/png;base64,AA=="})
        );
        let failed = image_result(Err(anyhow::anyhow!("自定义图片只支持 https:// 地址")));
        assert_eq!(
            serde_json::to_value(failed).unwrap(),
            json!({"status": "failed", "message": "自定义图片只支持 https:// 地址"})
        );
    }

    #[test]
    fn import_saves_audio_and_rejects_non_audio() {
        let dir = tempfile::tempdir().unwrap();
        let wav = base64::engine::general_purpose::STANDARD.encode(b"RIFF\x24\0\0\0WAVEfmt ");
        let ok = import_result("press", &wav, dir.path());
        assert_eq!(ok.status, "ok");
        let file = ok.file.unwrap();
        assert!(file.starts_with("press-") && file.ends_with(".wav"), "{file}");
        assert!(dir.path().join(&file).is_file());

        let text = base64::engine::general_purpose::STANDARD.encode(b"<html></html>");
        let failed = import_result("press", &text, dir.path());
        assert_eq!(failed.status, "failed");
        assert!(failed.message.unwrap().contains("音频"));
        assert_eq!(import_result("press", "!!", dir.path()).status, "failed");
        assert_eq!(import_result("other", &wav, dir.path()).status, "failed");
    }

    #[test]
    fn unknown_sound_set_fails_without_network() {
        let result = tauri::async_runtime::block_on(agent_pet_sound(
            "nope".into(),
            String::new(),
            String::new(),
        ));
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap().contains("未知的桌宠音效"));
        let embedded = tauri::async_runtime::block_on(agent_pet_sound(
            "duck".into(),
            String::new(),
            String::new(),
        ));
        assert_eq!(embedded.status, "ok");
        assert_eq!(embedded.key.as_deref(), Some("duck"));
    }

    #[test]
    fn invalid_custom_url_fails_without_network() {
        let result = tauri::async_runtime::block_on(agent_pet_image(
            "custom".into(),
            "http://example.com/pet.png".into(),
        ));
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap().contains("https://"));
    }
}
