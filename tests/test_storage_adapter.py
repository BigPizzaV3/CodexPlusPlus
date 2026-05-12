import sqlite3

from codex_session_delete.backup_store import BackupStore
from codex_session_delete.models import DeleteStatus, SessionRef
from codex_session_delete.storage_adapter import SQLiteStorageAdapter


def create_supported_db(path):
    with sqlite3.connect(path) as db:
        db.execute("CREATE TABLE sessions (id TEXT PRIMARY KEY, title TEXT NOT NULL)")
        db.execute("CREATE TABLE messages (id INTEGER PRIMARY KEY, session_id TEXT NOT NULL, body TEXT NOT NULL)")
        db.execute("INSERT INTO sessions (id, title) VALUES ('s1', 'First')")
        db.execute("INSERT INTO messages (session_id, body) VALUES ('s1', 'hello')")


def create_codex_thread_db(path, rollout_path):
    with sqlite3.connect(path) as db:
        db.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, rollout_path TEXT, title TEXT, cwd TEXT, archived INTEGER, archived_at INTEGER, updated_at INTEGER, updated_at_ms INTEGER)")
        db.execute("CREATE TABLE thread_dynamic_tools (thread_id TEXT NOT NULL, tool_name TEXT NOT NULL)")
        db.execute("CREATE TABLE thread_goals (thread_id TEXT NOT NULL, goal TEXT NOT NULL)")
        db.execute("CREATE TABLE thread_spawn_edges (parent_thread_id TEXT NOT NULL, child_thread_id TEXT NOT NULL, status TEXT NOT NULL)")
        db.execute("CREATE TABLE stage1_outputs (thread_id TEXT NOT NULL, output TEXT NOT NULL)")
        db.execute("CREATE TABLE agent_job_items (id TEXT PRIMARY KEY, assigned_thread_id TEXT)")
        db.execute("INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t1', ?, 'Codex Thread', '/old/project', 0, NULL, 100, 100000)", (str(rollout_path),))
        db.execute("INSERT INTO thread_dynamic_tools (thread_id, tool_name) VALUES ('t1', 'Read')")
        db.execute("INSERT INTO thread_goals (thread_id, goal) VALUES ('t1', 'delete me')")
        db.execute("INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status) VALUES ('t1', 'child', 'running')")
        db.execute("INSERT INTO thread_spawn_edges (parent_thread_id, child_thread_id, status) VALUES ('parent', 't1', 'done')")
        db.execute("INSERT INTO stage1_outputs (thread_id, output) VALUES ('t1', 'cached')")
        db.execute("INSERT INTO agent_job_items (id, assigned_thread_id) VALUES ('job1', 't1')")


def test_delete_local_session_creates_backup_and_removes_rows(tmp_path):
    db_path = tmp_path / "codex.sqlite"
    create_supported_db(db_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.delete_local(SessionRef(session_id="s1", title="First"))

    assert result.status == DeleteStatus.LOCAL_DELETED
    assert result.undo_token is not None
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT COUNT(*) FROM sessions").fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM messages").fetchone()[0] == 0


def test_undo_restores_deleted_rows(tmp_path):
    db_path = tmp_path / "codex.sqlite"
    create_supported_db(db_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))
    deleted = adapter.delete_local(SessionRef(session_id="s1", title="First"))

    restored = adapter.undo(deleted.undo_token or "")

    assert restored.status == DeleteStatus.UNDONE
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT title FROM sessions WHERE id = 's1'").fetchone()[0] == "First"
        assert db.execute("SELECT body FROM messages WHERE session_id = 's1'").fetchone()[0] == "hello"




def test_delete_codex_thread_schema_creates_backup_and_removes_thread_rows(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text('{"type":"message"}\n', encoding="utf-8")
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.delete_local(SessionRef(session_id="t1", title="Codex Thread"))

    assert result.status == DeleteStatus.LOCAL_DELETED
    assert result.undo_token is not None
    assert not rollout_path.exists()
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT COUNT(*) FROM threads WHERE id = 't1'").fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM thread_dynamic_tools WHERE thread_id = 't1'").fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM thread_goals WHERE thread_id = 't1'").fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = 't1' OR child_thread_id = 't1'").fetchone()[0] == 0
        assert db.execute("SELECT COUNT(*) FROM stage1_outputs WHERE thread_id = 't1'").fetchone()[0] == 0
        assert db.execute("SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'").fetchone()[0] is None


def test_undo_restores_deleted_codex_thread_schema_and_rollout_file(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text('{"type":"message"}\n', encoding="utf-8")
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))
    deleted = adapter.delete_local(SessionRef(session_id="t1", title="Codex Thread"))

    restored = adapter.undo(deleted.undo_token or "")

    assert restored.status == DeleteStatus.UNDONE
    assert rollout_path.read_text(encoding="utf-8") == '{"type":"message"}\n'
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT title FROM threads WHERE id = 't1'").fetchone()[0] == "Codex Thread"
        assert db.execute("SELECT tool_name FROM thread_dynamic_tools WHERE thread_id = 't1'").fetchone()[0] == "Read"
        assert db.execute("SELECT goal FROM thread_goals WHERE thread_id = 't1'").fetchone()[0] == "delete me"
        assert db.execute("SELECT COUNT(*) FROM thread_spawn_edges WHERE parent_thread_id = 't1' OR child_thread_id = 't1'").fetchone()[0] == 2
        assert db.execute("SELECT output FROM stage1_outputs WHERE thread_id = 't1'").fetchone()[0] == "cached"
        assert db.execute("SELECT assigned_thread_id FROM agent_job_items WHERE id = 'job1'").fetchone()[0] == "t1"


def test_delete_codex_thread_schema_accepts_local_prefixed_thread_id(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text('{"type":"message"}\n', encoding="utf-8")
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.delete_local(SessionRef(session_id="local:t1", title="Codex Thread"))

    assert result.status == DeleteStatus.LOCAL_DELETED
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT COUNT(*) FROM threads WHERE id = 't1'").fetchone()[0] == 0


def test_find_archived_codex_thread_by_title(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "archived.jsonl"
    create_codex_thread_db(db_path, rollout_path)
    with sqlite3.connect(db_path) as db:
        db.execute("UPDATE threads SET archived = 1, archived_at = 123 WHERE id = 't1'")
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    session = adapter.find_archived_thread_by_title("Codex Thread")

    assert session == SessionRef(session_id="t1", title="Codex Thread")


def test_find_archived_codex_thread_by_title_matches_rendered_archive_card_text(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "archived.jsonl"
    create_codex_thread_db(db_path, rollout_path)
    with sqlite3.connect(db_path) as db:
        db.execute("UPDATE threads SET archived = 1, archived_at = 123 WHERE id = 't1'")
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    session = adapter.find_archived_thread_by_title("Codex Thread 2026年5月9日，1:19 · RustGUI")

    assert session == SessionRef(session_id="t1", title="Codex Thread")


def test_find_archived_codex_thread_by_title_ignores_active_threads(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "active.jsonl"
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    session = adapter.find_archived_thread_by_title("Codex Thread")

    assert session is None


def test_delete_codex_thread_schema_reports_rollout_delete_failure(tmp_path, monkeypatch):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text('{"type":"message"}\n', encoding="utf-8")
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    def fail_unlink(self, missing_ok=False):
        if self == rollout_path:
            raise PermissionError("Access is denied")
        return original_unlink(self, missing_ok=missing_ok)

    original_unlink = type(rollout_path).unlink
    monkeypatch.setattr(type(rollout_path), "unlink", fail_unlink)

    result = adapter.delete_local(SessionRef(session_id="t1", title="Codex Thread"))

    assert result.status == DeleteStatus.FAILED
    assert "本地数据库已删除" in result.message
    assert "文件删除失败" in result.message
    assert "Access is denied" in result.message


def test_delete_local_session_uses_chinese_success_message(tmp_path):
    db_path = tmp_path / "codex.sqlite"
    create_supported_db(db_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.delete_local(SessionRef(session_id="s1", title="First"))

    assert result.message == "已从本地存储删除"


def test_delete_unsupported_schema_fails(tmp_path):
    db_path = tmp_path / "unknown.sqlite"
    with sqlite3.connect(db_path) as db:
        db.execute("CREATE TABLE unrelated (id TEXT PRIMARY KEY)")
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.delete_local(SessionRef(session_id="s1", title="First"))

    assert result.status == DeleteStatus.FAILED
    assert "Unsupported" in result.message


def test_move_codex_thread_workspace_updates_db_and_rollout_session_meta(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    rollout_path.write_text(
        '{"type":"session_meta","payload":{"id":"t1","cwd":"/old/project","title":"Codex Thread"}}\n'
        '{"type":"session_meta","payload":{"id":"other","cwd":"/old/project"}}\n',
        encoding="utf-8",
    )
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.move_codex_thread_workspace(SessionRef(session_id="local:t1", title="Codex Thread"), "/new/project")

    assert result["status"] == "moved"
    assert result["previous_cwd"] == "/old/project"
    assert result["target_cwd"] == "/new/project"
    assert result["rollout_updated"] is True
    assert result["rollout_error"] == ""
    assert result["updated_at"] == 100
    assert result["updated_at_ms"] == 100000
    with sqlite3.connect(db_path) as db:
        assert db.execute("SELECT cwd FROM threads WHERE id = 't1'").fetchone()[0] == "/new/project"
    text = rollout_path.read_text(encoding="utf-8")
    assert '"id":"t1","cwd":"/new/project"' in text
    assert '"id":"other","cwd":"/old/project"' in text


def test_codex_thread_sort_key_returns_thread_timestamps(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    create_codex_thread_db(db_path, rollout_path)
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.codex_thread_sort_key(SessionRef(session_id="local:t1", title="Codex Thread"))

    assert result["status"] == "ok"
    assert result["session_id"] == "t1"
    assert result["updated_at"] == 100
    assert result["updated_at_ms"] == 100000


def test_codex_thread_sort_keys_returns_batch_thread_timestamps(tmp_path):
    db_path = tmp_path / "state_5.sqlite"
    rollout_path = tmp_path / "rollout.jsonl"
    create_codex_thread_db(db_path, rollout_path)
    with sqlite3.connect(db_path) as db:
        db.execute("INSERT INTO threads (id, rollout_path, title, cwd, archived, archived_at, updated_at, updated_at_ms) VALUES ('t2', ?, 'Second', '/other/project', 0, NULL, 200, 200000)", (str(rollout_path),))
    adapter = SQLiteStorageAdapter(db_path, BackupStore(tmp_path / "backups"))

    result = adapter.codex_thread_sort_keys([
        SessionRef(session_id="local:t2", title="Second"),
        SessionRef(session_id="local:t1", title="Codex Thread"),
    ])

    assert result["status"] == "ok"
    assert result["sort_keys"] == [
        {"session_id": "t2", "updated_at": 200, "updated_at_ms": 200000, "created_at_ms": None},
        {"session_id": "t1", "updated_at": 100, "updated_at_ms": 100000, "created_at_ms": None},
    ]
