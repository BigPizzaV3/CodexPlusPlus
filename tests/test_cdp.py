import json
import webbrowser
from pathlib import Path

import codex_session_delete.cdp as cdp
import websocket

from codex_session_delete.cdp import BRIDGE_BINDING_NAME, _bridge_loop, add_script_to_new_documents, build_bridge_script, evaluate_user_scripts, install_bridge, list_targets, open_devtools, pick_page_target


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


def test_pick_page_target_prefers_codex_title():
    targets = [
        {"type": "background_page", "title": "bg", "webSocketDebuggerUrl": "ws://bg"},
        {"type": "page", "title": "Codex", "url": "app://codex", "webSocketDebuggerUrl": "ws://page"},
    ]

    assert pick_page_target(targets)["webSocketDebuggerUrl"] == "ws://page"


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
