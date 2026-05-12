import json
import threading
import urllib.error
import urllib.request

from codex_session_delete.helper_server import HelperServer
from codex_session_delete.models import DeleteResult, DeleteStatus, ExportResult, ExportStatus, SessionRef


class FakeDeleteService:
    def __init__(self):
        self.deleted = []
        self.undone = []
        self.archived_title_queries = []

    def delete(self, session: SessionRef):
        self.deleted.append(session)
        return DeleteResult(DeleteStatus.LOCAL_DELETED, session.session_id, "Deleted locally", undo_token="u1")

    def undo(self, token: str):
        self.undone.append(token)
        return DeleteResult(DeleteStatus.UNDONE, "s1", "Restored", undo_token=token)

    def find_archived_thread_by_title(self, title: str):
        self.archived_title_queries.append(title)
        return SessionRef(session_id="archived-t1", title=title)

    def move_thread_workspace(self, session: SessionRef, target_cwd: str):
        return {"status": "moved", "session_id": session.session_id, "target_cwd": target_cwd}

    def thread_sort_key(self, session: SessionRef):
        return {"status": "ok", "session_id": session.session_id, "updated_at_ms": 123}

    def thread_sort_keys(self, sessions: list[SessionRef]):
        return {"status": "ok", "sort_keys": [{"session_id": session.session_id, "updated_at_ms": index + 1} for index, session in enumerate(sessions)]}


class FakeExportService:
    def __init__(self):
        self.exported = []

    def export(self, session: SessionRef):
        self.exported.append(session)
        return ExportResult(ExportStatus.EXPORTED, session.session_id, "Exported", filename="thread.md", markdown="# Thread\n")


def post_json(url, payload, headers=None):
    data = json.dumps(payload).encode("utf-8")
    request_headers = {"Content-Type": "application/json", **(headers or {})}
    request = urllib.request.Request(url, data=data, headers=request_headers, method="POST")
    with urllib.request.urlopen(request, timeout=3) as response:
        return json.loads(response.read().decode("utf-8"))


def test_helper_server_delete_and_undo():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service, allow_http_mutation=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        deleted = post_json(base + "/delete", {"session_id": "s1", "title": "First"})
        undone = post_json(base + "/undo", {"undo_token": "u1"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert deleted["status"] == "local_deleted"
    assert deleted["undo_token"] == "u1"
    assert undone["status"] == "undone"
    assert service.deleted[0].session_id == "s1"
    assert service.undone == ["u1"]


def test_helper_server_resolves_archived_thread_by_title():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service, allow_http_mutation=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        resolved = post_json(base + "/archived-thread", {"title": "Codex Thread"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert resolved == {"session_id": "archived-t1", "title": "Codex Thread"}
    assert service.archived_title_queries == ["Codex Thread"]


def test_helper_server_exports_markdown_when_authorized():
    delete_service = FakeDeleteService()
    export_service = FakeExportService()
    server = HelperServer("127.0.0.1", 0, delete_service, export_service, allow_http_mutation=True)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        exported = post_json(base + "/export-markdown", {"session_id": "s1", "title": "First"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert exported["status"] == "exported"
    assert exported["filename"] == "thread.md"
    assert export_service.exported[0].session_id == "s1"


def test_helper_server_rejects_http_mutation_by_default():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        try:
            post_json(base + "/delete", {"session_id": "s1", "title": "First"})
            assert False, "expected forbidden response"
        except urllib.error.HTTPError as exc:
            assert exc.code == 403
        try:
            post_json(base + "/export-markdown", {"session_id": "s1", "title": "First"})
            assert False, "expected forbidden response"
        except urllib.error.HTTPError as exc:
            assert exc.code == 403
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert service.deleted == []


def test_helper_server_accepts_http_mutation_token():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service, http_mutation_token="test-token")
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        try:
            post_json(base + "/delete", {"session_id": "s1", "title": "First"})
            assert False, "expected forbidden response"
        except urllib.error.HTTPError as exc:
            assert exc.code == 403
        deleted = post_json(base + "/delete", {"session_id": "s1", "title": "First"}, {"X-Codex-Session-Delete-Token": "test-token"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert deleted["status"] == "local_deleted"
    assert service.deleted[0].session_id == "s1"


def test_helper_server_moves_thread_workspace_without_http_mutation_token():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        moved = post_json(base + "/move-thread-workspace", {"session_id": "s1", "title": "First", "target_cwd": "/project/a"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert moved == {"status": "moved", "session_id": "s1", "target_cwd": "/project/a"}


def test_helper_server_returns_thread_sort_key():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        sort_key = post_json(base + "/thread-sort-key", {"session_id": "s1", "title": "First"})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert sort_key == {"status": "ok", "session_id": "s1", "updated_at_ms": 123}


def test_helper_server_returns_thread_sort_keys():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        base = f"http://127.0.0.1:{server.port}"
        sort_keys = post_json(base + "/thread-sort-keys", {"sessions": [{"session_id": "s1", "title": "First"}, {"session_id": "s2", "title": "Second"}]})
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert sort_keys == {"status": "ok", "sort_keys": [{"session_id": "s1", "updated_at_ms": 1}, {"session_id": "s2", "updated_at_ms": 2}]}


def test_helper_server_allows_private_network_preflight():
    service = FakeDeleteService()
    server = HelperServer("127.0.0.1", 0, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    try:
        request = urllib.request.Request(
            f"http://127.0.0.1:{server.port}/delete",
            method="OPTIONS",
            headers={
                "Origin": "file://",
                "Access-Control-Request-Method": "POST",
                "Access-Control-Request-Headers": "content-type",
                "Access-Control-Request-Private-Network": "true",
            },
        )
        with urllib.request.urlopen(request, timeout=3) as response:
            private_network = response.headers.get("Access-Control-Allow-Private-Network")
            allow_headers = response.headers.get("Access-Control-Allow-Headers")
    finally:
        server.shutdown()
        thread.join(timeout=3)

    assert private_network == "true"
    assert "X-Codex-Session-Delete-Token" in allow_headers
