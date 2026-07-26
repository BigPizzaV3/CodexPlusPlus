#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionRef {
    pub session_id: String,
    pub title: String,
}

impl SessionRef {
    pub fn new(
        session_id: impl Into<String>,
        title: impl Into<String>,
    ) -> anyhow::Result<SessionRef> {
        let session_id = session_id.into();
        if session_id.is_empty() {
            anyhow::bail!("session_id cannot be empty");
        }

        Ok(SessionRef {
            session_id,
            title: title.into(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteStatus {
    ServerDeleted,
    LocalDeleted,
    Partial,
    Failed,
    Undone,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeleteResult {
    pub status: DeleteStatus,
    pub session_id: String,
    pub message: String,
    pub undo_token: Option<String>,
    pub backup_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Exported,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ExportResult {
    pub status: ExportStatus,
    pub session_id: String,
    pub message: String,
    pub filename: Option<String>,
    pub markdown: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageOutputStatus {
    Found,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageOutput {
    pub id: String,
    pub turn_id: Option<String>,
    pub assistant_text: Option<String>,
    pub data_url: String,
    pub output_format: String,
    pub revised_prompt: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ImageOutputResult {
    pub status: ImageOutputStatus,
    pub session_id: String,
    pub message: String,
    pub images: Vec<ImageOutput>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn session_ref_new_rejects_empty_session_id() {
        let err = SessionRef::new("", "Untitled").unwrap_err();

        assert!(err.to_string().contains("session_id"));
    }

    #[test]
    fn session_ref_new_preserves_fields() {
        let session = SessionRef::new("session-123", "My Session").unwrap();

        assert_eq!(
            session,
            SessionRef {
                session_id: "session-123".to_string(),
                title: "My Session".to_string(),
            }
        );
    }

    #[test]
    fn delete_status_uses_snake_case_json_values() {
        assert_eq!(
            serde_json::to_value(DeleteStatus::ServerDeleted).unwrap(),
            json!("server_deleted")
        );
        assert_eq!(
            serde_json::from_value::<DeleteStatus>(json!("server_deleted")).unwrap(),
            DeleteStatus::ServerDeleted
        );
    }

    #[test]
    fn export_status_uses_snake_case_json_values() {
        assert_eq!(
            serde_json::to_value(ExportStatus::Exported).unwrap(),
            json!("exported")
        );
        assert_eq!(
            serde_json::from_value::<ExportStatus>(json!("exported")).unwrap(),
            ExportStatus::Exported
        );
    }

    #[test]
    fn delete_result_json_shape_matches_rust_model() {
        let result = DeleteResult {
            status: DeleteStatus::Partial,
            session_id: "session-123".to_string(),
            message: "deleted locally only".to_string(),
            undo_token: Some("undo-123".to_string()),
            backup_path: None,
        };

        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(
            value,
            json!({
                "status": "partial",
                "session_id": "session-123",
                "message": "deleted locally only",
                "undo_token": "undo-123",
                "backup_path": null
            })
        );
        assert_eq!(
            serde_json::from_value::<DeleteResult>(value).unwrap(),
            result
        );
    }

    #[test]
    fn export_result_json_shape_matches_rust_model() {
        let result = ExportResult {
            status: ExportStatus::Exported,
            session_id: "session-123".to_string(),
            message: "exported markdown".to_string(),
            filename: Some("session-123.md".to_string()),
            markdown: Some("# Session\n\nBody".to_string()),
        };

        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(
            value,
            json!({
                "status": "exported",
                "session_id": "session-123",
                "message": "exported markdown",
                "filename": "session-123.md",
                "markdown": "# Session\n\nBody"
            })
        );
        assert_eq!(
            serde_json::from_value::<ExportResult>(value).unwrap(),
            result
        );
    }

    #[test]
    fn failed_export_result_accepts_null_filename_and_markdown() {
        let value = json!({
            "status": "failed",
            "session_id": "s1",
            "message": "err",
            "filename": null,
            "markdown": null
        });

        let result = serde_json::from_value::<ExportResult>(value.clone()).unwrap();

        assert_eq!(serde_json::to_value(result).unwrap(), value);
    }

    #[test]
    fn image_output_result_json_shape_matches_rust_model() {
        let result = ImageOutputResult {
            status: ImageOutputStatus::Found,
            session_id: "s1".to_string(),
            message: "found 1 image".to_string(),
            images: vec![ImageOutput {
                id: "img-1".to_string(),
                turn_id: Some("turn-1".to_string()),
                assistant_text: Some("done".to_string()),
                data_url: "data:image/png;base64,aGVsbG8=".to_string(),
                output_format: "png".to_string(),
                revised_prompt: Some("prompt".to_string()),
                timestamp: Some("2026-07-25T12:00:00Z".to_string()),
            }],
        };

        let value = serde_json::to_value(&result).unwrap();

        assert_eq!(
            value,
            json!({
                "status": "found",
                "session_id": "s1",
                "message": "found 1 image",
                "images": [{
                    "id": "img-1",
                    "turn_id": "turn-1",
                    "assistant_text": "done",
                    "data_url": "data:image/png;base64,aGVsbG8=",
                    "output_format": "png",
                    "revised_prompt": "prompt",
                    "timestamp": "2026-07-25T12:00:00Z"
                }]
            })
        );
        assert_eq!(
            serde_json::from_value::<ImageOutputResult>(value).unwrap(),
            result
        );
    }
}
