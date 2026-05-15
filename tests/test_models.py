from codex_session_delete.models import BulkExportResult, DeleteResult, DeleteStatus, ExportResult, ExportStatus, SessionRef


def test_session_ref_requires_session_id():
    try:
        SessionRef(session_id="", title="Untitled")
    except ValueError as exc:
        assert "session_id" in str(exc)
    else:
        raise AssertionError("SessionRef accepted an empty session_id")


def test_delete_result_serializes_to_json_dict():
    result = DeleteResult(
        status=DeleteStatus.LOCAL_DELETED,
        session_id="abc123",
        message="Deleted locally",
        undo_token="undo-1",
        backup_path="C:/tmp/backup.json",
    )

    assert result.to_dict() == {
        "status": "local_deleted",
        "session_id": "abc123",
        "message": "Deleted locally",
        "undo_token": "undo-1",
        "backup_path": "C:/tmp/backup.json",
    }


def test_export_result_serializes_to_json_dict():
    result = ExportResult(
        status=ExportStatus.EXPORTED,
        session_id="abc123",
        message="Exported",
        filename="example.md",
        markdown="# Example\n",
    )

    assert result.to_dict() == {
        "status": "exported",
        "session_id": "abc123",
        "message": "Exported",
        "filename": "example.md",
        "markdown": "# Example\n",
    }


def test_bulk_export_result_serializes_to_json_dict():
    result = BulkExportResult(
        status=ExportStatus.EXPORTED,
        message="Exported 2 sessions",
        filename="codex-sessions.zip",
        zip_base64="UEsDBAo=",
        exported_count=2,
        failed_count=1,
        failures=[{"session_id": "missing", "title": "Missing", "message": "未找到对应会话"}],
    )

    assert result.to_dict() == {
        "status": "exported",
        "message": "Exported 2 sessions",
        "filename": "codex-sessions.zip",
        "zip_base64": "UEsDBAo=",
        "exported_count": 2,
        "failed_count": 1,
        "failures": [{"session_id": "missing", "title": "Missing", "message": "未找到对应会话"}],
    }
