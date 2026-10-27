//! 「Agent 桌宠」页用的两个命令：列出内置图库、预览一张图片。
//! 预览与 Codex 里的桌宠共用同一个缓存目录，选过的图切到 Codex 时不必再下载。

use serde::Serialize;

use codex_plus_core::agent_pet::{self, AgentPetCatalogEntry};

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
        assert_eq!(value.as_array().unwrap().len(), 18);
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
    fn invalid_custom_url_fails_without_network() {
        let result = tauri::async_runtime::block_on(agent_pet_image(
            "custom".into(),
            "http://example.com/pet.png".into(),
        ));
        assert_eq!(result.status, "failed");
        assert!(result.message.unwrap().contains("https://"));
    }
}
