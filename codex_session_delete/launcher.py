from __future__ import annotations

import ctypes
import os
import socket
import subprocess
import sys
import threading
import time
import uuid
from pathlib import Path
from typing import Any

from codex_session_delete.app_paths import resolve_codex_app_dir
from codex_session_delete.api_adapter import ApiAdapter, UnavailableApiAdapter
from codex_session_delete.backup_store import BackupStore
from codex_session_delete.cdp import inject_file
from codex_session_delete.helper_server import HelperServer
from codex_session_delete.models import DeleteResult, DeleteStatus, SessionRef
from codex_session_delete.storage_adapter import SQLiteStorageAdapter


class ApiFirstDeleteService:
    def __init__(self, api_adapter: ApiAdapter, db_path: Path | None, backup_dir: Path):
        self.api_adapter = api_adapter
        self.local_adapter = SQLiteStorageAdapter(db_path, BackupStore(backup_dir)) if db_path else None

    def delete(self, session: SessionRef) -> DeleteResult:
        api_result = self.api_adapter.delete(session)
        if api_result is not None:
            return api_result
        if self.local_adapter is None:
            return DeleteResult(DeleteStatus.FAILED, session.session_id, "No confirmed server API or local database configured")
        return self.local_adapter.delete_local(session)

    def undo(self, token: str) -> DeleteResult:
        if self.local_adapter is None:
            return DeleteResult(DeleteStatus.FAILED, "", "No local backup adapter configured", undo_token=token)
        return self.local_adapter.undo(token)

    def find_archived_thread_by_title(self, title: str) -> SessionRef | None:
        if self.local_adapter is None:
            return None
        return self.local_adapter.find_archived_thread_by_title(title)


class InjectedHelperServer(HelperServer):
    bridge_socket: Any = None


def _can_bind_loopback_port(port: int) -> bool:
    if port == 0:
        return True
    try:
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
            if sys.platform == "win32" and hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
                probe.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
            probe.bind(("127.0.0.1", port))
            return True
    except OSError:
        return False


def _find_available_loopback_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        if sys.platform == "win32" and hasattr(socket, "SO_EXCLUSIVEADDRUSE"):
            probe.setsockopt(socket.SOL_SOCKET, socket.SO_EXCLUSIVEADDRUSE, 1)
        probe.bind(("127.0.0.1", 0))
        return int(probe.getsockname()[1])


def select_windows_loopback_port(requested_port: int) -> int:
    if sys.platform != "win32" or _can_bind_loopback_port(requested_port):
        return requested_port
    return _find_available_loopback_port()


def build_codex_arguments(debug_port: int) -> list[str]:
    return [
        f"--remote-debugging-port={debug_port}",
        f"--remote-allow-origins=http://127.0.0.1:{debug_port}",
    ]


def build_codex_executable(app_dir: Path) -> Path:
    if app_dir.suffix == ".app":
        return app_dir / "Contents" / "MacOS" / "Codex"
    candidates = [app_dir / "Codex.exe", app_dir / "codex.exe"]
    return next((path for path in candidates if path.exists()), candidates[-1])


def build_codex_command(app_dir: Path, debug_port: int) -> list[str]:
    return [str(build_codex_executable(app_dir)), *build_codex_arguments(debug_port)]


def packaged_app_user_model_id(app_dir: Path) -> str | None:
    package_dir = app_dir.parent if app_dir.name.lower() == "app" else app_dir
    if not package_dir.name.startswith("OpenAI.Codex_") or "__" not in package_dir.name:
        return None
    identity_name = package_dir.name.split("_", 1)[0]
    publisher_id = package_dir.name.rsplit("__", 1)[1]
    if not publisher_id:
        return None
    return f"{identity_name}_{publisher_id}!App"


def packaged_app_local_executable(app_dir: Path) -> Path | None:
    package_dir = app_dir.parent if app_dir.name.lower() == "app" else app_dir
    if not package_dir.name.startswith("OpenAI.Codex_") or "__" not in package_dir.name:
        return None
    publisher_id = package_dir.name.rsplit("__", 1)[1]
    if not publisher_id:
        return None
    local_app_data = os.environ.get("LOCALAPPDATA")
    if not local_app_data:
        return None
    executable = (
        Path(local_app_data)
        / "Packages"
        / f"OpenAI.Codex_{publisher_id}"
        / "LocalCache"
        / "Local"
        / "OpenAI"
        / "Codex"
        / "bin"
        / "codex.exe"
    )
    return executable if executable.exists() else None


class _GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", ctypes.c_uint32),
        ("Data2", ctypes.c_uint16),
        ("Data3", ctypes.c_uint16),
        ("Data4", ctypes.c_ubyte * 8),
    ]

    def __init__(self, value: str):
        parsed = uuid.UUID(value)
        data4 = bytes([parsed.clock_seq_hi_variant, parsed.clock_seq_low]) + parsed.node.to_bytes(6, "big")
        super().__init__(parsed.time_low, parsed.time_mid, parsed.time_hi_version, (ctypes.c_ubyte * 8)(*data4))


def _raise_for_hresult(hr: int, operation: str) -> None:
    if hr < 0:
        raise OSError(f"{operation} failed with HRESULT 0x{hr & 0xFFFFFFFF:08X}")


def activate_packaged_app(app_user_model_id: str, arguments: str) -> int:
    if sys.platform != "win32":
        raise RuntimeError("Packaged app activation is only supported on Windows")

    ole32 = ctypes.OleDLL("ole32")
    ole32.CoInitializeEx.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    ole32.CoInitializeEx.restype = ctypes.c_long
    ole32.CoUninitialize.argtypes = []
    ole32.CoUninitialize.restype = None
    ole32.CoCreateInstance.argtypes = [
        ctypes.POINTER(_GUID),
        ctypes.c_void_p,
        ctypes.c_ulong,
        ctypes.POINTER(_GUID),
        ctypes.POINTER(ctypes.c_void_p),
    ]
    ole32.CoCreateInstance.restype = ctypes.c_long

    coinit_hr = ole32.CoInitializeEx(None, 2)
    should_uninitialize = coinit_hr >= 0
    if coinit_hr < 0 and coinit_hr != -2147417850:  # RPC_E_CHANGED_MODE
        _raise_for_hresult(coinit_hr, "CoInitializeEx")

    activation_manager = ctypes.c_void_p()
    try:
        clsid = _GUID("45BA127D-10A8-46EA-8AB7-56EA9078943C")
        iid = _GUID("2e941141-7f97-4756-ba1d-9decde894a3d")
        _raise_for_hresult(
            ole32.CoCreateInstance(ctypes.byref(clsid), None, 1, ctypes.byref(iid), ctypes.byref(activation_manager)),
            "CoCreateInstance(ApplicationActivationManager)",
        )

        activate_application_type = ctypes.WINFUNCTYPE(
            ctypes.c_long,
            ctypes.c_void_p,
            ctypes.c_wchar_p,
            ctypes.c_wchar_p,
            ctypes.c_ulong,
            ctypes.POINTER(ctypes.c_ulong),
        )

        vtable = ctypes.cast(activation_manager, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents
        activate_application = activate_application_type(vtable[3])

        process_id = ctypes.c_ulong()
        _raise_for_hresult(
            activate_application(activation_manager, app_user_model_id, arguments, 0, ctypes.byref(process_id)),
            "ActivateApplication",
        )
        return int(process_id.value)
    finally:
        if activation_manager.value:
            release = ctypes.WINFUNCTYPE(ctypes.c_ulong, ctypes.c_void_p)(
                ctypes.cast(activation_manager, ctypes.POINTER(ctypes.POINTER(ctypes.c_void_p))).contents[2]
            )
            release(activation_manager)
        if should_uninitialize:
            ole32.CoUninitialize()


def launch_codex_app(app_dir: Path, debug_port: int) -> Any:
    if sys.platform == "win32":
        executable = build_codex_executable(app_dir)
        if executable.exists():
            return subprocess.Popen([str(executable), *build_codex_arguments(debug_port)])
    app_user_model_id = packaged_app_user_model_id(app_dir) if sys.platform == "win32" else None
    if app_user_model_id:
        return activate_packaged_app(app_user_model_id, subprocess.list2cmdline(build_codex_arguments(debug_port)))
    if app_dir.suffix == ".app":
        subprocess.run(["open", "-a", str(app_dir), "--args", *build_codex_arguments(debug_port)], check=True)
        return None
    return subprocess.Popen(build_codex_command(app_dir, debug_port))


def stop_windows_codex_processes() -> None:
    if sys.platform != "win32":
        return
    try:
        subprocess.run(
            [
                "powershell.exe",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Get-CimInstance Win32_Process -Filter \"Name='Codex.exe' OR Name='codex.exe'\" | "
                "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }",
            ],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            timeout=6,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        time.sleep(1.0)
    except (OSError, subprocess.SubprocessError):
        pass


def start_helper(service, host: str = "127.0.0.1", port: int = 57321) -> HelperServer:
    server = InjectedHelperServer(host, port, service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def shutdown_helper(server: HelperServer) -> None:
    server.shutdown()
    server.server_close()


def inject_with_retry(debug_port: int, script_path: Path, helper_port: int, service: ApiFirstDeleteService, attempts: int = 90, delay: float = 1.0) -> Any:
    last_error: Exception | None = None
    for _ in range(attempts):
        try:
            return inject_file(debug_port, script_path, helper_port, lambda path, payload: handle_bridge_request(service, path, payload))
        except Exception as exc:
            last_error = exc
            time.sleep(delay)
    if last_error is not None:
        raise last_error
    raise RuntimeError("Codex injection failed")


def launch_and_inject(app_dir: Path | None, db_path: Path | None, backup_dir: Path, debug_port: int, helper_port: int) -> tuple[HelperServer, Any]:
    resolved_app_dir = resolve_codex_app_dir(app_dir)
    if resolved_app_dir is None:
        raise RuntimeError("Codex App directory not found")
    debug_port = _find_available_loopback_port() if sys.platform == "win32" else select_windows_loopback_port(debug_port)
    helper_port = select_windows_loopback_port(helper_port)
    stop_windows_codex_processes()
    service = ApiFirstDeleteService(UnavailableApiAdapter(), db_path, backup_dir)
    server = start_helper(service, port=helper_port)
    codex_proc = None
    try:
        codex_proc = launch_codex_app(resolved_app_dir, debug_port)
        script_path = Path(__file__).parent / "inject" / "renderer-inject.js"
        server.bridge_socket = inject_with_retry(debug_port, script_path, server.port, service)
        return server, codex_proc
    except Exception:
        shutdown_helper(server)
        # Kill any Codex process we just activated so the next attempt starts from a clean state
        # instead of staring at a half-rendered white window.
        stop_windows_codex_processes()
        raise


def handle_bridge_request(service: ApiFirstDeleteService, path: str, payload: dict[str, object]) -> dict[str, object]:
    if path == "/delete":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return service.delete(session).to_dict()
    if path == "/undo":
        return service.undo(str(payload.get("undo_token", ""))).to_dict()
    if path == "/archived-thread":
        session = service.find_archived_thread_by_title(str(payload.get("title", "")))
        return {"session_id": session.session_id, "title": session.title} if session else {"session_id": "", "title": ""}
    return {"status": DeleteStatus.FAILED.value, "session_id": str(payload.get("session_id", "")), "message": "Unknown bridge path"}
