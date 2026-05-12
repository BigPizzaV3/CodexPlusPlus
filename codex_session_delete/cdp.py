from __future__ import annotations

import json
import threading
import webbrowser
from dataclasses import dataclass
from pathlib import Path
from typing import Callable

import requests
import websocket


BridgeHandler = Callable[[str, dict[str, object]], dict[str, object]]
BRIDGE_BINDING_NAME = "codexSessionDeleteV2"


@dataclass(frozen=True)
class InjectionResult:
    websocket_url: str
    bridge_socket: websocket.WebSocket | None
    result: dict[str, object] | None


def list_targets(port: int) -> list[dict[str, object]]:
    session = requests.Session()
    session.trust_env = False
    response = session.get(f"http://127.0.0.1:{port}/json", timeout=3)
    response.raise_for_status()
    return response.json()


def pick_page_target(targets: list[dict[str, object]]) -> dict[str, object]:
    pages = [target for target in targets if target.get("type") == "page" and target.get("webSocketDebuggerUrl")]
    for target in pages:
        title = str(target.get("title", ""))
        url = str(target.get("url", ""))
        if "codex" in (title + " " + url).lower():
            return target
    if pages:
        return pages[0]
    raise RuntimeError("No injectable Codex page target found")


def evaluate_script(websocket_url: str, script: str) -> dict[str, object]:
    ws = websocket.create_connection(websocket_url, timeout=5)
    try:
        payload = {
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {"expression": script, "awaitPromise": False, "allowUnsafeEvalBlockedByCSP": True},
        }
        ws.send(json.dumps(payload))
        while True:
            message = json.loads(ws.recv())
            if message.get("id") == 1:
                if "error" in message:
                    raise RuntimeError(str(message["error"]))
                return message
    finally:
        ws.close()


def evaluate_user_scripts(websocket_url: str, script: str) -> dict[str, object] | None:
    if not script.strip():
        return None
    return evaluate_script(websocket_url, script)


def open_devtools(port: int) -> dict[str, object]:
    target = pick_page_target(list_targets(port))
    target_id = str(target.get("id", ""))
    if not target_id:
        return {"status": "failed", "message": "No DevTools target id"}
    webbrowser.open(f"http://127.0.0.1:{port}/devtools/inspector.html?ws=127.0.0.1:{port}/devtools/page/{target_id}")
    return {"status": "ok", "target_id": target_id}


def add_script_to_new_documents(websocket_url: str, script: str) -> dict[str, object]:
    ws = websocket.create_connection(websocket_url, timeout=5)
    try:
        return _add_script_to_new_documents_on_socket(ws, script, 1)
    finally:
        ws.close()


def _add_script_to_new_documents_on_socket(ws: websocket.WebSocket, script: str, message_id: int) -> dict[str, object]:
    payload = {
        "id": message_id,
        "method": "Page.addScriptToEvaluateOnNewDocument",
        "params": {"source": script},
    }
    ws.send(json.dumps(payload))
    return _wait_for_id(ws, message_id)


def build_bridge_script(binding_name: str) -> str:
    return f"""
(() => {{
  window.__codexSessionDeleteCallbacks = new Map();
  window.__codexSessionDeleteSeq = 0;
  window.__codexSessionDeleteResolve = (id, result) => {{
    const callback = window.__codexSessionDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    callback.resolve(result);
  }};
  window.__codexSessionDeleteReject = (id, message) => {{
    const callback = window.__codexSessionDeleteCallbacks.get(id);
    if (!callback) return;
    window.__codexSessionDeleteCallbacks.delete(id);
    callback.resolve({{ status: "failed", message }});
  }};
  window.__codexSessionDeleteBridge = (path, payload) => new Promise((resolve) => {{
    const id = String(++window.__codexSessionDeleteSeq);
    window.__codexSessionDeleteCallbacks.set(id, {{ resolve }});
    window.{binding_name}(JSON.stringify({{ id, path, payload }}));
  }});
}})();
"""


def install_bridge(websocket_url: str, binding_name: str, handler: BridgeHandler, new_document_scripts: list[str] | None = None) -> websocket.WebSocket:
    ws = websocket.create_connection(websocket_url, timeout=5)
    ws.send(json.dumps({"id": 1, "method": "Runtime.enable", "params": {}}))
    _wait_for_id(ws, 1)
    ws.send(json.dumps({"id": 2, "method": "Runtime.removeBinding", "params": {"name": binding_name}}))
    _wait_for_id(ws, 2)
    ws.send(json.dumps({"id": 3, "method": "Runtime.addBinding", "params": {"name": binding_name}}))
    _wait_for_id(ws, 3)
    bridge_script = build_bridge_script(binding_name)
    ws.send(json.dumps({"id": 4, "method": "Page.addScriptToEvaluateOnNewDocument", "params": {"source": bridge_script}}))
    _wait_for_id(ws, 4)
    ws.send(json.dumps({"id": 5, "method": "Runtime.evaluate", "params": {"expression": bridge_script, "awaitPromise": False, "allowUnsafeEvalBlockedByCSP": True}}))
    _wait_for_id(ws, 5)
    for script in new_document_scripts or []:
        _add_script_to_new_documents_on_socket(ws, script, _next_id())
    thread = threading.Thread(target=_bridge_loop, args=(ws, handler), daemon=True)
    thread.start()
    return ws


def inject_file(port: int, script_path: Path, helper_port: int, handler: BridgeHandler | None = None) -> InjectionResult:
    targets = list_targets(port)
    target = pick_page_target(targets)
    websocket_url = str(target["webSocketDebuggerUrl"])
    script = script_path.read_text(encoding="utf-8")
    prefix = f"window.__CODEX_SESSION_DELETE_HELPER__ = 'http://127.0.0.1:{helper_port}';\n"
    full_script = prefix + script
    bridge_socket = install_bridge(websocket_url, BRIDGE_BINDING_NAME, handler, [full_script]) if handler else None
    if not bridge_socket:
        add_script_to_new_documents(websocket_url, full_script)
    result = evaluate_script(websocket_url, full_script)
    return InjectionResult(websocket_url=websocket_url, bridge_socket=bridge_socket, result=result)


def _bridge_loop(ws: websocket.WebSocket, handler: BridgeHandler) -> None:
    while True:
        try:
            message = json.loads(ws.recv())
        except websocket.WebSocketTimeoutException:
            continue
        except Exception:
            return
        if message.get("method") != "Runtime.bindingCalled":
            continue
        params = message.get("params", {})
        try:
            payload = json.loads(str(params.get("payload", "{}")))
            request_id = str(payload["id"])
            result = handler(str(payload["path"]), dict(payload.get("payload", {})))
            _resolve_bridge(ws, request_id, result)
        except Exception as exc:
            request_id = str(locals().get("payload", {}).get("id", ""))
            if request_id:
                _reject_bridge(ws, request_id, str(exc))


def _resolve_bridge(ws: websocket.WebSocket, request_id: str, result: dict[str, object]) -> None:
    expression = f"window.__codexSessionDeleteResolve({json.dumps(request_id)}, {json.dumps(result)})"
    ws.send(json.dumps({"id": _next_id(), "method": "Runtime.evaluate", "params": {"expression": expression, "awaitPromise": False, "allowUnsafeEvalBlockedByCSP": True}}))


def _reject_bridge(ws: websocket.WebSocket, request_id: str, message: str) -> None:
    expression = f"window.__codexSessionDeleteReject({json.dumps(request_id)}, {json.dumps(message)})"
    ws.send(json.dumps({"id": _next_id(), "method": "Runtime.evaluate", "params": {"expression": expression, "awaitPromise": False, "allowUnsafeEvalBlockedByCSP": True}}))


def _wait_for_id(ws: websocket.WebSocket, message_id: int) -> dict[str, object]:
    while True:
        message = json.loads(ws.recv())
        if message.get("id") == message_id:
            if "error" in message:
                raise RuntimeError(str(message["error"]))
            return message


_id_lock = threading.Lock()
_id = 100


def _next_id() -> int:
    global _id
    with _id_lock:
        _id += 1
        return _id
