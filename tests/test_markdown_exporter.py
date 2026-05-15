import sqlite3
import zipfile
from io import BytesIO
from datetime import datetime

from codex_session_delete.markdown_exporter import MarkdownExportService
from codex_session_delete.models import ExportStatus, SessionRef


def create_codex_thread_db(path, rollout_path, *, thread_id="t1", title="Codex Thread"):
    with sqlite3.connect(path) as db:
        db.execute("CREATE TABLE IF NOT EXISTS threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, archived INTEGER, archived_at INTEGER)")
        db.execute(
            "INSERT INTO threads (id, rollout_path, title, archived, archived_at) VALUES (?, ?, ?, 0, NULL)",
            (thread_id, str(rollout_path), title),
        )


def test_markdown_exporter_exports_user_and_assistant_messages_with_timestamps(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        "\n".join([
            '{"type":"session_meta","timestamp":"2026-05-10T13:00:00Z"}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Hello"}]}}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:09Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hi there"}]}}',
            "",
        ]),
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path)

    result = MarkdownExportService(db_path).export(SessionRef(session_id="t1", title="Ignored title"))
    expected_user_time = datetime.fromisoformat("2026-05-10T13:12:06+00:00").astimezone().strftime("%Y-%m-%d %H:%M:%S")
    expected_assistant_time = datetime.fromisoformat("2026-05-10T13:12:09+00:00").astimezone().strftime("%Y-%m-%d %H:%M:%S")

    assert result.status == ExportStatus.EXPORTED
    assert result.filename == "Codex Thread-t1.md"
    assert result.markdown == (
        "# Codex Thread\n\n"
        "### User\n"
        f"_{expected_user_time}_\n\n"
        "Hello\n\n"
        "### Assistant\n"
        f"_{expected_assistant_time}_\n\n"
        "Hi there\n"
    )


def test_markdown_exporter_ignores_non_message_and_non_user_assistant_items(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        "\n".join([
            '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"ignore"}]}}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:07Z","payload":{"type":"reasoning","summary":[]}}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:08Z","payload":{"type":"function_call","name":"tool"}}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:09Z","payload":{"type":"function_call_output","output":"ok"}}',
            '{"type":"response_item","timestamp":"2026-05-10T13:12:10Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Keep"}]}}',
            "",
        ]),
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path)

    result = MarkdownExportService(db_path).export(SessionRef(session_id="t1", title="Codex Thread"))

    assert result.status == ExportStatus.EXPORTED
    assert "Keep" in (result.markdown or "")
    assert "ignore" not in (result.markdown or "")
    assert "function_call" not in (result.markdown or "")


def test_markdown_exporter_serializes_images_without_inlining_data_urls(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        "\n".join([
            '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"user","content":[{"type":"input_image","image_url":"data:image/png;base64,AAAA"},{"type":"input_image","image_url":"https://example.com/image.png"}]}}',
            "",
        ]),
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path)

    result = MarkdownExportService(db_path).export(SessionRef(session_id="t1", title="Codex Thread"))

    assert result.status == ExportStatus.EXPORTED
    assert result.markdown is not None
    assert result.markdown.count("> Image attachment") == 2
    assert "[Image link](<https://example.com/image.png>)" in result.markdown
    assert "data:image/png;base64" not in result.markdown


def test_markdown_exporter_supports_local_prefixed_session_id(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path, thread_id="t1")

    result = MarkdownExportService(db_path).export(SessionRef(session_id="local:t1", title="Codex Thread"))

    assert result.status == ExportStatus.EXPORTED
    assert result.session_id == "t1"


def test_markdown_exporter_sanitizes_filename_and_appends_thread_id(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(
        db_path,
        rollout_path,
        thread_id="thread:1",
        title='  A  title  with  bad<>:"/\\\\|?*chars and a very long suffix that should still be trimmed safely for file names  ',
    )

    result = MarkdownExportService(db_path).export(SessionRef(session_id="thread:1", title="ignored"))

    assert result.status == ExportStatus.EXPORTED
    assert result.filename is not None
    assert result.filename.endswith("-thread-1.md")
    assert "<" not in result.filename
    assert len(result.filename.split("-thread-1.md")[0]) <= 80


def test_markdown_exporter_strips_trailing_space_or_dot_after_truncation(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path, thread_id="t1", title=("A" * 79) + ".")

    result = MarkdownExportService(db_path).export(SessionRef(session_id="t1", title="ignored"))

    assert result.status == ExportStatus.EXPORTED
    assert result.filename is not None
    assert not result.filename.startswith((" ", "."))
    assert not result.filename.split("-t1.md")[0].endswith((" ", "."))


def test_markdown_exporter_skips_invalid_timestamp_but_keeps_body(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        '{"type":"response_item","timestamp":"not-a-timestamp","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path)

    result = MarkdownExportService(db_path).export(SessionRef(session_id="t1", title="Codex Thread"))

    assert result.status == ExportStatus.EXPORTED
    assert result.markdown == "# Codex Thread\n\n### Assistant\n\nHello\n"


def test_markdown_exporter_fails_when_thread_missing_rollout_missing_or_no_messages(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    missing_rollout_db = tmp_path / "missing.sqlite"
    create_codex_thread_db(missing_rollout_db, tmp_path / "missing.jsonl")
    no_messages_db = tmp_path / "no_messages.sqlite"
    empty_rollout = tmp_path / "empty.jsonl"
    empty_rollout.write_text('{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"ignore"}]}}\n', encoding="utf-8")
    create_codex_thread_db(no_messages_db, empty_rollout)

    missing_db_service = MarkdownExportService(db_path)
    missing_rollout_service = MarkdownExportService(missing_rollout_db)
    no_messages_service = MarkdownExportService(no_messages_db)

    missing_thread = missing_db_service.export(SessionRef(session_id="t1", title="Codex Thread"))
    missing_rollout = missing_rollout_service.export(SessionRef(session_id="t1", title="Codex Thread"))
    no_messages = no_messages_service.export(SessionRef(session_id="t1", title="Codex Thread"))

    assert missing_thread.status == ExportStatus.FAILED
    assert missing_rollout.status == ExportStatus.FAILED
    assert no_messages.status == ExportStatus.FAILED


def test_markdown_exporter_exports_multiple_sessions_as_zip_with_failure_summary(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    first_rollout = tmp_path / "first.jsonl"
    second_rollout = tmp_path / "second.jsonl"
    first_rollout.write_text(
        '{"type":"response_item","timestamp":"2026-05-10T13:12:06Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"First body"}]}}\n',
        encoding="utf-8",
    )
    second_rollout.write_text(
        '{"type":"response_item","timestamp":"2026-05-10T13:12:07Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Second body"}]}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, first_rollout, thread_id="t1", title="First Thread")
    create_codex_thread_db(db_path, second_rollout, thread_id="t2", title="First Thread")

    result = MarkdownExportService(db_path).export_zip([
        SessionRef(session_id="t1", title="First"),
        SessionRef(session_id="missing", title="Missing Thread"),
        SessionRef(session_id="t2", title="Second"),
    ])

    assert result.status == ExportStatus.EXPORTED
    assert result.filename == "codex-sessions-2-of-3.zip"
    assert result.exported_count == 2
    assert result.failed_count == 1
    assert result.failures == [{"session_id": "missing", "title": "Missing Thread", "message": "未找到对应会话"}]
    assert result.zip_base64 is not None
    with zipfile.ZipFile(BytesIO(result.zip_bytes())) as archive:
        names = set(archive.namelist())
        assert "First Thread-t1.md" in names
        assert "First Thread-t2.md" in names
        assert "export-failures.md" in names
        assert "First body" in archive.read("First Thread-t1.md").decode("utf-8")
        assert "Second body" in archive.read("First Thread-t2.md").decode("utf-8")
        assert "Missing Thread" in archive.read("export-failures.md").decode("utf-8")
