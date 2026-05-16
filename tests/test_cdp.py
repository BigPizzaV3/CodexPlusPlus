import json
import webbrowser
from pathlib import Path

import codex_session_delete.cdp as cdp
import websocket

from codex_session_delete.cdp import BRIDGE_BINDING_NAME, _bridge_loop, add_script_to_new_documents, build_bridge_script, codex_page_targets, evaluate_user_scripts, install_bridge, list_targets, open_devtools, pick_page_target


class TimeoutThenMessageSocket:
    def __init__(self):
        self.recv_count = 0
        self.sent = []

    def recv(self):
        self.recv_count += 1
        if self.recv_count == 1:
            raise websocket.WebSocketTimeoutException("idle")
        if self.recv_count == 2:
            return json.dumps({
                "method": "Runtime.bindingCalled",
                "params": {"payload": json.dumps({"id": "1", "path": "/diagnostic", "payload": {"session_id": "s1"}})},
            })
        raise RuntimeError("stop after response")

    def send(self, payload):
        self.sent.append(payload)


class SingleResponseSocket:
    def __init__(self):
        self.sent = []
        self.closed = False

    def send(self, payload):
        self.sent.append(json.loads(payload))

    def recv(self):
        return json.dumps({"id": 1, "result": {"identifier": "script-1"}})

    def close(self):
        self.closed = True


class BridgeInstallSocket:
    def __init__(self):
        self.sent = []
        self.next_response_id = 1

    def send(self, payload):
        self.sent.append(json.loads(payload))

    def recv(self):
        response = {"id": self.next_response_id, "result": {}}
        self.next_response_id += 1
        return json.dumps(response)


class FakeThread:
    def __init__(self, **kwargs):
        self.kwargs = kwargs
        self.started = False

    def start(self):
        self.started = True


def test_pick_page_target_prefers_codex_title():
    targets = [
        {"type": "background_page", "title": "bg", "webSocketDebuggerUrl": "ws://bg"},
        {"type": "page", "title": "Codex", "url": "app://codex", "webSocketDebuggerUrl": "ws://page"},
    ]

    assert pick_page_target(targets)["webSocketDebuggerUrl"] == "ws://page"


def test_codex_page_targets_keeps_all_codex_windows():
    targets = [
        {"id": "devtools", "type": "page", "title": "DevTools", "url": "devtools://devtools", "webSocketDebuggerUrl": "ws://devtools"},
        {"id": "main", "type": "page", "title": "Codex", "url": "app://-/index.html", "webSocketDebuggerUrl": "ws://main"},
        {"id": "child", "type": "page", "title": "Codex", "url": "app://-/index.html?initialRoute=%2Flocal%2F1", "webSocketDebuggerUrl": "ws://child"},
        {"id": "worker", "type": "worker", "title": "", "url": "", "webSocketDebuggerUrl": "ws://worker"},
    ]

    assert [target["id"] for target in codex_page_targets(targets)] == ["main", "child"]


def test_list_targets_bypasses_proxy_environment(monkeypatch):
    seen = {}

    class FakeResponse:
        def raise_for_status(self):
            pass

        def json(self):
            return [{"type": "page"}]

    class FakeSession:
        def __init__(self):
            self.trust_env = True

        def get(self, url, timeout):
            seen["trust_env"] = self.trust_env
            seen["url"] = url
            seen["timeout"] = timeout
            return FakeResponse()

    monkeypatch.setattr("codex_session_delete.cdp.requests.Session", FakeSession)

    assert list_targets(9229) == [{"type": "page"}]
    assert seen == {
        "trust_env": False,
        "url": "http://127.0.0.1:9229/json",
        "timeout": 3,
    }


def test_pick_page_target_rejects_missing_websocket():
    try:
        pick_page_target([{"type": "page", "title": "Codex"}])
    except RuntimeError as exc:
        assert "No injectable" in str(exc)
    else:
        raise AssertionError("target without websocket was accepted")


def test_build_bridge_script_installs_binding_callbacks():
    script = build_bridge_script("codexSessionDelete")

    assert "window.codexSessionDelete" in script
    assert "window.__codexSessionDeleteResolve" in script
    assert "window.__codexSessionDeleteReject" in script
    assert "window.__codexSessionDeleteCallbacks instanceof Map" in script
    assert "Number.isFinite(window.__codexSessionDeleteSeq)" in script


def test_bridge_binding_name_is_versioned_for_reinjection():
    assert BRIDGE_BINDING_NAME == "codexSessionDeleteV2"


def test_add_script_to_new_documents_registers_reload_injection(monkeypatch):
    ws = SingleResponseSocket()
    monkeypatch.setattr(websocket, "create_connection", lambda *args, **kwargs: ws)

    result = add_script_to_new_documents("ws://page", "window.__codexPlusTest = true;")

    assert result["result"]["identifier"] == "script-1"
    assert ws.sent[0]["method"] == "Page.addScriptToEvaluateOnNewDocument"
    assert ws.sent[0]["params"]["source"] == "window.__codexPlusTest = true;"
    assert ws.closed is True


def test_install_bridge_enables_runtime_before_adding_binding(monkeypatch):
    ws = BridgeInstallSocket()
    monkeypatch.setattr(websocket, "create_connection", lambda url, timeout: ws)
    monkeypatch.setattr(cdp.threading, "Thread", lambda **kwargs: type("FakeThread", (), {"start": lambda self: None})())

    install_bridge("ws://page", BRIDGE_BINDING_NAME, lambda path, payload: {}, ["window.__codexPlusTest = true;"])

    assert ws.sent[0] == {"id": 1, "method": "Runtime.enable", "params": {}}
    assert ws.sent[1]["method"] == "Runtime.removeBinding"
    assert ws.sent[2]["method"] == "Runtime.addBinding"
    assert ws.sent[3]["method"] == "Page.addScriptToEvaluateOnNewDocument"
    assert ws.sent[5]["method"] == "Page.addScriptToEvaluateOnNewDocument"
    assert ws.sent[5]["params"]["source"] == "window.__codexPlusTest = true;"


def test_inject_file_exposes_packaged_sponsor_images_as_data_uris(monkeypatch, tmp_path):
    script_path = tmp_path / "renderer.js"
    script_path.write_text("window.__rendererLoaded = true;", encoding="utf-8")
    captured = {}

    monkeypatch.setattr(cdp, "list_targets", lambda port: [{"type": "page", "title": "Codex", "url": "app://codex", "webSocketDebuggerUrl": "ws://page"}])
    monkeypatch.setattr(cdp, "install_bridge", lambda websocket_url, binding_name, handler, scripts: captured.setdefault("new_document", scripts[0]) or object())
    monkeypatch.setattr(cdp, "evaluate_script", lambda websocket_url, script: captured.setdefault("evaluated", script) or {"result": {}})

    cdp.inject_file(9229, script_path, 57321, lambda path, payload: {})

    assert "window.__CODEX_PLUS_SPONSOR_IMAGES__" in captured["evaluated"]
    assert '"alipay": "data:image/jpeg;base64,' in captured["evaluated"]
    assert '"wechat": "data:image/jpeg;base64,' in captured["evaluated"]
    assert captured["new_document"] == captured["evaluated"]


def test_inject_file_into_all_pages_injects_existing_codex_windows(monkeypatch, tmp_path):
    script_path = tmp_path / "renderer.js"
    script_path.write_text("window.__rendererLoaded = true;", encoding="utf-8")
    thread = FakeThread()
    targets = [
        {"id": "main", "type": "page", "title": "Codex", "url": "app://-/index.html", "webSocketDebuggerUrl": "ws://main"},
        {"id": "child", "type": "page", "title": "Codex", "url": "app://-/index.html?initialRoute=%2Flocal%2F1", "webSocketDebuggerUrl": "ws://child"},
        {"id": "worker", "type": "worker", "title": "", "url": "", "webSocketDebuggerUrl": "ws://worker"},
    ]
    injected = []
    callbacks = []

    def fake_inject_target(target, full_script, handler):
        injected.append((target["id"], full_script))
        return cdp.InjectionResult(websocket_url=target["webSocketDebuggerUrl"], bridge_socket=None, result={"status": target["id"]})

    monkeypatch.setattr(cdp, "list_targets", lambda port: targets)
    monkeypatch.setattr(cdp, "inject_target", fake_inject_target)
    monkeypatch.setattr(cdp.threading, "Thread", lambda **kwargs: thread)

    manager = cdp.inject_file_into_all_pages(9229, script_path, 57321, lambda path, payload: {}, on_injection=callbacks.append)

    assert [item[0] for item in injected] == ["main", "child"]
    assert "window.__CODEX_SESSION_DELETE_HELPER__ = 'http://127.0.0.1:57321';" in injected[0][1]
    assert sorted(manager.injections) == ["child", "main"]
    assert [callback.websocket_url for callback in callbacks] == ["ws://main", "ws://child"]
    assert manager.websocket_url == "ws://main"
    assert thread.started is True


def test_inject_file_into_all_pages_keeps_successful_targets_when_one_fails(monkeypatch, tmp_path):
    script_path = tmp_path / "renderer.js"
    script_path.write_text("window.__rendererLoaded = true;", encoding="utf-8")
    targets = [
        {"id": "main", "type": "page", "title": "Codex", "url": "app://-/index.html", "webSocketDebuggerUrl": "ws://main"},
        {"id": "child", "type": "page", "title": "Codex", "url": "app://-/index.html?initialRoute=%2Flocal%2F1", "webSocketDebuggerUrl": "ws://child"},
    ]

    def fake_inject_target(target, full_script, handler):
        if target["id"] == "child":
            raise RuntimeError("page still loading")
        return cdp.InjectionResult(websocket_url=target["webSocketDebuggerUrl"], bridge_socket=None, result={"status": target["id"]})

    monkeypatch.setattr(cdp, "list_targets", lambda port: targets)
    monkeypatch.setattr(cdp, "inject_target", fake_inject_target)
    monkeypatch.setattr(cdp.threading, "Thread", lambda **kwargs: FakeThread())

    manager = cdp.inject_file_into_all_pages(9229, script_path, 57321)

    assert sorted(manager.injections) == ["main"]


def test_open_devtools_opens_chrome_devtools_frontend(monkeypatch):
    targets = [{"type": "page", "title": "Codex", "url": "app://codex", "id": "page-1", "webSocketDebuggerUrl": "ws://page"}]
    opened = []
    monkeypatch.setattr(cdp, "list_targets", lambda port: targets)
    monkeypatch.setattr(webbrowser, "open", lambda url: opened.append(url) or True)

    result = open_devtools(9229)

    assert result == {"status": "ok", "target_id": "page-1"}
    assert opened == ["http://127.0.0.1:9229/devtools/inspector.html?ws=127.0.0.1:9229/devtools/page/page-1"]


def test_bridge_loop_continues_after_idle_timeout():
    ws = TimeoutThenMessageSocket()

    _bridge_loop(ws, lambda path, payload: {"status": "ok", "path": path})

    assert ws.recv_count == 3
    assert "__codexSessionDeleteResolve" in ws.sent[0]
