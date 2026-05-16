from __future__ import annotations

import base64
import json
import threading
import webbrowser
from dataclasses import dataclass, field
from importlib import resources
from pathlib import Path
from typing import Callable

import requests
import websocket


BridgeHandler = Callable[[str, dict[str, object]], dict[str, object]]
InjectionCallback = Callable[["InjectionResult"], None]
BRIDGE_BINDING_NAME = "codexSessionDeleteV2"


@dataclass(frozen=True)
class InjectionResult:
    websocket_url: str
    bridge_socket: websocket.WebSocket | None
    result: dict[str, object] | None


@dataclass
class MultiPageInjection:
    injections: dict[str, InjectionResult] = field(default_factory=dict)
    stop_event: threading.Event = field(default_factory=threading.Event)
    watcher_thread: threading.Thread | None = None
    lock: threading.Lock = field(default_factory=threading.Lock)

    @property
    def websocket_url(self) -> str | None:
        with self.lock:
            first = next(iter(self.injections.values()), None)
        return first.websocket_url if first else None

    @property
    def bridge_socket(self) -> websocket.WebSocket | None:
        with self.lock:
            first = next((injection.bridge_socket for injection in self.injections.values() if injection.bridge_socket), None)
        return first

    @property
    def result(self) -> dict[str, object] | None:
        with self.lock:
            first = next(iter(self.injections.values()), None)
        return first.result if first else None


def list_targets(port: int) -> list[dict[str, object]]:
    session = requests.Session()
    session.trust_env = False
    response = session.get(f"http://127.0.0.1:{port}/json", timeout=3)
    response.raise_for_status()
    return response.json()


def codex_page_targets(targets: list[dict[str, object]]) -> list[dict[str, object]]:
    pages = [target for target in targets if target.get("type") == "page" and target.get("webSocketDebuggerUrl")]
    codex_pages = []
    for target in pages:
        title = str(target.get("title", ""))
        url = str(target.get("url", ""))
        haystack = (title + " " + url).lower()
        if "codex" in haystack or url.startswith("app://"):
            codex_pages.append(target)
    return codex_pages or pages


def pick_page_target(targets: list[dict[str, object]]) -> dict[str, object]:
    pages = codex_page_targets(targets)
    if pages:
        return pages[0]
    raise RuntimeError("No injectable Codex page target found")


def evaluate_script(websocket_url: str, script: str, *, await_promise: bool = False, timeout: float = 5) -> dict[str, object]:
    ws = websocket.create_connection(websocket_url, timeout=timeout)
    try:
        payload = {
            "id": 1,
            "method": "Runtime.evaluate",
            "params": {"expression": script, "awaitPromise": await_promise, "allowUnsafeEvalBlockedByCSP": True},
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
  window.__codexSessionDeleteCallbacks = window.__codexSessionDeleteCallbacks instanceof Map ? window.__codexSessionDeleteCallbacks : new Map();
  window.__codexSessionDeleteSeq = Number.isFinite(window.__codexSessionDeleteSeq) ? window.__codexSessionDeleteSeq : 0;
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


def sponsor_image_data_uris() -> dict[str, str]:
    assets = resources.files("codex_session_delete").joinpath("assets")
    return {
        "alipay": "data:image/jpeg;base64," + base64.b64encode(assets.joinpath("sponsor-alipay.jpg").read_bytes()).decode("ascii"),
        "wechat": "data:image/jpeg;base64," + base64.b64encode(assets.joinpath("sponsor-wechat.jpg").read_bytes()).decode("ascii"),
    }


def target_key(target: dict[str, object]) -> str:
    return str(target.get("id") or target.get("webSocketDebuggerUrl") or "")


def build_full_injection_script(script_path: Path, helper_port: int) -> str:
    script = script_path.read_text(encoding="utf-8")
    prefix = (
        f"window.__CODEX_SESSION_DELETE_HELPER__ = 'http://127.0.0.1:{helper_port}';\n"
        f"window.__CODEX_PLUS_SPONSOR_IMAGES__ = {json.dumps(sponsor_image_data_uris())};\n"
    )
    return prefix + script


def inject_target(target: dict[str, object], full_script: str, handler: BridgeHandler | None = None) -> InjectionResult:
    websocket_url = str(target["webSocketDebuggerUrl"])
    bridge_socket = install_bridge(websocket_url, BRIDGE_BINDING_NAME, handler, [full_script]) if handler else None
    if not bridge_socket:
        add_script_to_new_documents(websocket_url, full_script)
    result = evaluate_script(websocket_url, full_script)
    return InjectionResult(websocket_url=websocket_url, bridge_socket=bridge_socket, result=result)


def inject_file(port: int, script_path: Path, helper_port: int, handler: BridgeHandler | None = None) -> InjectionResult:
    target = pick_page_target(list_targets(port))
    return inject_target(target, build_full_injection_script(script_path, helper_port), handler)


def inject_file_into_all_pages(
    port: int,
    script_path: Path,
    helper_port: int,
    handler: BridgeHandler | None = None,
    on_injection: InjectionCallback | None = None,
    poll_interval: float = 0.75,
) -> MultiPageInjection:
    manager = MultiPageInjection()
    full_script = build_full_injection_script(script_path, helper_port)

    def inject_available_pages() -> int:
        injected = 0
        last_error: Exception | None = None
        for target in codex_page_targets(list_targets(port)):
            key = target_key(target)
            with manager.lock:
                already_injected = key in manager.injections
            if not key or already_injected:
                continue
            try:
                injection = inject_target(target, full_script, handler)
                with manager.lock:
                    manager.injections[key] = injection
                if on_injection:
                    on_injection(injection)
                injected += 1
            except Exception as exc:
                last_error = exc
        with manager.lock:
            has_injections = bool(manager.injections)
        if not has_injections and last_error is not None:
            raise last_error
        return injected

    inject_available_pages()
    with manager.lock:
        has_injections = bool(manager.injections)
    if not has_injections:
        raise RuntimeError("No injectable Codex page target found")

    def watch_pages() -> None:
        while not manager.stop_event.wait(poll_interval):
            try:
                inject_available_pages()
            except Exception:
                continue

    manager.watcher_thread = threading.Thread(target=watch_pages, daemon=True)
    manager.watcher_thread.start()
    return manager


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
