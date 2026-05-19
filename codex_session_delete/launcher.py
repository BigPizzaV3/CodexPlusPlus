from __future__ import annotations

import ctypes
import json
import os
import socket
import subprocess
import sys
import threading
import time
import tomllib
import uuid
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any
from urllib.parse import urlsplit, urlunsplit

import requests

from codex_session_delete import cdp, zed_remote
from codex_session_delete.app_paths import resolve_codex_app_dir
from codex_session_delete.api_adapter import ApiAdapter, UnavailableApiAdapter
from codex_session_delete.backup_store import BackupStore
from codex_session_delete.cdp import evaluate_script, evaluate_user_scripts, inject_file_into_all_pages, open_devtools
from codex_session_delete.helper_server import HelperServer
from codex_session_delete.helper_server import fetch_ad_list
from codex_session_delete.markdown_exporter import MarkdownExportService
from codex_session_delete.models import DeleteResult, DeleteStatus, SessionRef
from codex_session_delete.provider_sync import ProviderSyncStatus, run_provider_sync
from codex_session_delete.settings_store import BackendSettings, SettingsStore
from codex_session_delete.storage_adapter import SQLiteStorageAdapter
from codex_session_delete.user_scripts import UserScriptManager


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

    def move_thread_workspace(self, session: SessionRef, target_cwd: str) -> dict[str, object]:
        if self.local_adapter is None:
            return {"status": DeleteStatus.FAILED.value, "session_id": session.session_id, "message": "No local database configured"}
        return self.local_adapter.move_codex_thread_workspace(session, target_cwd)

    def thread_sort_key(self, session: SessionRef) -> dict[str, object]:
        if self.local_adapter is None:
            return {"status": DeleteStatus.FAILED.value, "session_id": session.session_id, "message": "No local database configured"}
        return self.local_adapter.codex_thread_sort_key(session)

    def thread_sort_keys(self, sessions: list[SessionRef]) -> dict[str, object]:
        if self.local_adapter is None:
            return {"status": DeleteStatus.FAILED.value, "message": "No local database configured", "sort_keys": []}
        return self.local_adapter.codex_thread_sort_keys(sessions)


class InjectedHelperServer(HelperServer):
    bridge_socket: Any = None


@dataclass(frozen=True)
class OpenAICompatibleModelSource:
    source_id: str
    source_type: str
    name: str
    base_url: str
    api_key: str


@dataclass
class AttachedHelperServer:
    port: int
    bridge_socket: Any = None


@dataclass
class CodexPlusRuntime:
    websocket_url: str | None
    user_scripts: UserScriptManager
    debug_port: int | None = None
    websocket_urls: set[str] = field(default_factory=set)
    lock: threading.Lock = field(default_factory=threading.Lock)

    def add_websocket_url(self, websocket_url: str) -> None:
        with self.lock:
            self.websocket_url = websocket_url
            self.websocket_urls.add(websocket_url)

    def reload_user_scripts(self) -> dict[str, object]:
        script = self.user_scripts.build_enabled_bundle()
        with self.lock:
            websocket_urls = list(self.websocket_urls or ({self.websocket_url} if self.websocket_url else set()))
        failed_urls = []
        for websocket_url in websocket_urls:
            try:
                evaluate_user_scripts(websocket_url, script)
            except Exception:
                failed_urls.append(websocket_url)
        if failed_urls:
            with self.lock:
                self.websocket_urls.difference_update(failed_urls)
                if self.websocket_url in failed_urls:
                    self.websocket_url = next(iter(self.websocket_urls), None)
        inventory = self.user_scripts.inventory()
        inventory["target_count"] = len(websocket_urls) - len(failed_urls)
        return inventory

    def open_devtools(self) -> dict[str, object]:
        if self.debug_port is None:
            return {"status": "failed", "message": "No debug port configured"}
        return open_devtools(self.debug_port)

    def backend_status(self) -> dict[str, object]:
        return {"status": "ok", "message": "后端已连接"}

    def repair_backend(self) -> dict[str, object]:
        return self.backend_status()

    def ads(self) -> dict[str, object]:
        return fetch_ad_list()

    def codex_config_model(self) -> dict[str, object]:
        return read_codex_config_model()

    def codex_model_catalog(self) -> dict[str, object]:
        return read_codex_model_catalog()


def codex_home_path() -> Path:
    codex_home = os.environ.get("CODEX_HOME")
    return Path(codex_home) if codex_home else Path.home() / ".codex"


def codex_config_path() -> Path:
    return codex_home_path() / "config.toml"


def codex_auth_path() -> Path:
    return codex_home_path() / "auth.json"


def _string_config_value(value: object) -> str:
    return str(value).strip() if isinstance(value, str) else ""


def _load_codex_config(config_path: Path | None = None) -> tuple[Path, dict[str, Any], dict[str, Any], str]:
    path = config_path or codex_config_path()
    if not path.exists():
        return path, {}, {}, "missing"
    try:
        config = tomllib.loads(path.read_text(encoding="utf-8"))
    except (OSError, tomllib.TOMLDecodeError) as exc:
        return path, {}, {}, str(exc)

    effective = dict(config)
    profile_name = _string_config_value(config.get("profile"))
    profiles = config.get("profiles")
    if profile_name and isinstance(profiles, dict) and isinstance(profiles.get(profile_name), dict):
        effective.update(profiles[profile_name])
    return path, config, effective, ""


def read_codex_config_model(config_path: Path | None = None) -> dict[str, object]:
    path, config, effective, error = _load_codex_config(config_path)
    if error == "missing":
        return {"status": "missing", "path": str(path), "model": "", "model_provider": "", "provider_name": "", "models": []}
    if error:
        return {"status": "failed", "path": str(path), "message": error, "model": "", "model_provider": "", "provider_name": "", "models": []}

    model = _string_config_value(effective.get("model"))
    model_provider = _string_config_value(effective.get("model_provider"))
    providers = config.get("model_providers")
    provider_config = providers.get(model_provider) if isinstance(providers, dict) and model_provider else None
    provider_name = ""
    if isinstance(provider_config, dict):
        provider_name = str(provider_config.get("name") or model_provider).strip()

    models = [model] if model else []
    status = "ok" if model else "not_configured"
    return {
        "status": status,
        "path": str(path),
        "model": model,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "models": models,
    }


def read_codex_auth_api_key(auth_path: Path | None = None) -> str:
    path = auth_path or codex_auth_path()
    if not path.exists():
        return ""
    try:
        payload = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return ""
    if not isinstance(payload, dict):
        return ""
    for key in ("OPENAI_API_KEY", "api_key", "apikey", "access_token", "token"):
        value = _string_config_value(payload.get(key))
        if value:
            return value
    return ""


def _first_env_value(env: dict[str, str], names: tuple[str, ...]) -> str:
    for name in names:
        value = env.get(name)
        if value:
            return value.strip()
    return ""


def _safe_url_for_status(url: str) -> str:
    try:
        parts = urlsplit(url)
        hostname = parts.hostname or ""
        if parts.port is not None:
            hostname = f"{hostname}:{parts.port}"
        return urlunsplit((parts.scheme, hostname, parts.path, "", ""))
    except ValueError:
        return url.split("?", 1)[0].split("#", 1)[0]


def _provider_config_for_model_provider(config: dict[str, Any], model_provider: str) -> tuple[str, dict[str, Any] | None]:
    providers = config.get("model_providers")
    if not isinstance(providers, dict):
        return model_provider, None
    if model_provider and isinstance(providers.get(model_provider), dict):
        return model_provider, providers[model_provider]
    provider_items = [(name, provider) for name, provider in providers.items() if isinstance(name, str) and isinstance(provider, dict)]
    if not model_provider and len(provider_items) == 1:
        return provider_items[0]
    return model_provider, None


def _provider_api_key(provider_config: dict[str, Any], env: dict[str, str], auth_api_key: str) -> str:
    for key in ("experimental_bearer_token", "api_key", "apikey", "bearer_token", "token"):
        value = _string_config_value(provider_config.get(key))
        if value:
            return value
    for key in ("env_key", "api_key_env", "api_key_env_var", "key_env", "bearer_token_env"):
        env_name = _string_config_value(provider_config.get(key))
        if env_name and env.get(env_name):
            return env[env_name].strip()
    return _first_env_value(env, ("CODEX_PLUS_OPENAI_API_KEY", "CODEX_PLUS_API_KEY", "OPENAI_API_KEY")) or auth_api_key


def _models_endpoint(base_url: str) -> str:
    cleaned = _safe_url_for_status(base_url).rstrip("/")
    if not cleaned:
        return ""
    if cleaned.endswith("/models"):
        return cleaned
    if cleaned.endswith("/v1"):
        return f"{cleaned}/models"
    return f"{cleaned}/v1/models"


def _responses_endpoint(base_url: str) -> str:
    cleaned = _safe_url_for_status(base_url).rstrip("/")
    if not cleaned:
        return ""
    if cleaned.endswith("/responses"):
        return cleaned
    if cleaned.endswith("/models"):
        return f"{cleaned.removesuffix('/models')}/responses"
    if cleaned.endswith("/v1"):
        return f"{cleaned}/responses"
    return f"{cleaned}/v1/responses"


def _parse_model_payload(payload: object) -> list[str]:
    if isinstance(payload, list):
        names: list[str] = []
        for item in payload:
            if isinstance(item, str):
                names.append(item.strip())
            elif isinstance(item, dict):
                names.append(_string_config_value(item.get("id")) or _string_config_value(item.get("model")) or _string_config_value(item.get("name")))
        return [name for name in names if name]
    if not isinstance(payload, dict):
        return []
    for key in ("data", "models", "items"):
        value = payload.get(key)
        if isinstance(value, list):
            return _parse_model_payload(value)
        if isinstance(value, dict):
            nested = _parse_model_payload(value)
            if nested:
                return nested
    direct = _string_config_value(payload.get("id")) or _string_config_value(payload.get("model")) or _string_config_value(payload.get("name"))
    return [direct] if direct else []


def _unique_strings(values: list[str]) -> list[str]:
    seen: set[str] = set()
    result: list[str] = []
    for value in values:
        name = value.strip()
        if not name or name in seen:
            continue
        seen.add(name)
        result.append(name)
    return result


def _model_sources_from_environment(env: dict[str, str], auth_api_key: str) -> list[OpenAICompatibleModelSource]:
    base_url = _first_env_value(
        env,
        (
            "CODEX_PLUS_OPENAI_BASE_URL",
            "CODEX_PLUS_BASE_URL",
            "OPENAI_BASE_URL",
            "OPENAI_API_BASE_URL",
            "OPENAI_API_BASE",
            "OPENAI_API_URL",
        ),
    )
    if not base_url:
        return []
    api_key = _first_env_value(env, ("CODEX_PLUS_OPENAI_API_KEY", "CODEX_PLUS_API_KEY", "OPENAI_API_KEY")) or auth_api_key
    return [OpenAICompatibleModelSource("env:openai-compatible", "environment", "Environment", base_url, api_key)]


def _model_source_from_config(
    config: dict[str, Any],
    effective: dict[str, Any],
    env: dict[str, str],
    auth_api_key: str,
) -> OpenAICompatibleModelSource | None:
    model_provider, provider_config = _provider_config_for_model_provider(config, _string_config_value(effective.get("model_provider")))
    if not isinstance(provider_config, dict):
        return None
    base_url = _string_config_value(provider_config.get("base_url"))
    if not base_url:
        return None
    name = _string_config_value(provider_config.get("name")) or model_provider or "Codex config"
    api_key = _provider_api_key(provider_config, env, auth_api_key)
    return OpenAICompatibleModelSource(f"config:{model_provider or name}", "config", name, base_url, api_key)


def _response_error_message(response: Any) -> str:
    try:
        payload = response.json()
    except (AttributeError, ValueError):
        return str(getattr(response, "text", "") or "").strip()
    if isinstance(payload, dict):
        error = payload.get("error")
        if isinstance(error, dict):
            return _string_config_value(error.get("message")) or _string_config_value(error.get("code")) or json.dumps(error, ensure_ascii=False)
        if isinstance(error, str):
            return error.strip()
        for key in ("message", "detail", "error_description"):
            value = _string_config_value(payload.get(key))
            if value:
                return value
    return ""


def _looks_like_unsupported_responses_api(status_code: int, message: str) -> bool:
    if status_code in {404, 405, 410, 501}:
        return True
    lowered = message.lower()
    if status_code in {400, 422} and any(field in lowered for field in ("max_output_tokens", "input")):
        return any(phrase in lowered for phrase in ("unsupported", "unrecognized", "unknown", "not support", "invalid parameter"))
    if "responses" not in lowered:
        return False
    return any(
        phrase in lowered
        for phrase in (
            "not found",
            "no route",
            "unknown endpoint",
            "unsupported endpoint",
            "does not support",
            "not support",
            "unsupported api",
            "unsupported path",
        )
    )


def _probe_responses_api_support(source: OpenAICompatibleModelSource, model: str, requests_post: Any | None) -> dict[str, object]:
    endpoint = _responses_endpoint(source.base_url)
    safe_probe = {
        "status": "unknown",
        "endpoint": _safe_url_for_status(endpoint),
        "model": model,
    }
    if requests_post is None:
        return {**safe_probe, "message": "Responses API probe not run"}
    if not endpoint:
        return {**safe_probe, "message": "Missing base URL"}
    if not source.api_key:
        return {**safe_probe, "message": "Missing API key"}
    if not model:
        return {**safe_probe, "message": "No model available for Responses API probe"}

    headers = {"Accept": "application/json", "Content-Type": "application/json", "User-Agent": "CodexPlusPlus/1.0"}
    headers["Authorization"] = f"Bearer {source.api_key}"
    payload = {"model": model, "input": "ping", "max_output_tokens": 1}
    try:
        response = requests_post(endpoint, headers=headers, json=payload, timeout=8)
    except requests.RequestException as exc:
        return {**safe_probe, "status": "failed", "message": str(exc)}

    message = _response_error_message(response)
    status_code = int(getattr(response, "status_code", 0) or 0)
    if status_code < 400:
        return {**safe_probe, "status": "supported", "message": "Responses API probe succeeded"}
    if _looks_like_unsupported_responses_api(status_code, message):
        return {**safe_probe, "status": "unsupported", "message": message or f"HTTP {status_code}"}
    return {**safe_probe, "status": "failed", "message": message or f"HTTP {status_code}"}


def _preferred_responses_api_status(
    source_statuses: list[dict[str, object]],
    preferred_source_id: str,
) -> dict[str, object]:
    if not source_statuses:
        return {"status": "unknown", "message": "No OpenAI-compatible source configured"}
    preferred = next((source for source in source_statuses if source.get("id") == preferred_source_id), None) or source_statuses[0]
    responses_api = preferred.get("responses_api")
    if isinstance(responses_api, dict):
        return responses_api
    return {"status": "unknown", "message": "Responses API probe not run"}


def _fetch_models_from_source(source: OpenAICompatibleModelSource, requests_get: Any) -> tuple[list[str], dict[str, object]]:
    endpoint = _models_endpoint(source.base_url)
    safe_source = {
        "id": source.source_id,
        "type": source.source_type,
        "name": source.name,
        "base_url": _safe_url_for_status(source.base_url),
        "endpoint": _safe_url_for_status(endpoint),
        "auth": "present" if source.api_key else "missing",
    }
    if not endpoint:
        return [], {**safe_source, "status": "failed", "message": "Missing base URL", "models": 0}

    headers = {"Accept": "application/json", "User-Agent": "CodexPlusPlus/1.0"}
    if source.api_key:
        headers["Authorization"] = f"Bearer {source.api_key}"
    try:
        response = requests_get(endpoint, headers=headers, timeout=10)
        if response.status_code >= 400:
            return [], {**safe_source, "status": "failed", "message": f"HTTP {response.status_code}", "models": 0}
        models = _unique_strings(_parse_model_payload(response.json()))
        return models, {**safe_source, "status": "ok", "models": len(models)}
    except (requests.RequestException, ValueError) as exc:
        return [], {**safe_source, "status": "failed", "message": str(exc), "models": 0}


def read_codex_model_catalog(
    config_path: Path | None = None,
    auth_path: Path | None = None,
    env: dict[str, str] | None = None,
    requests_get: Any | None = None,
    requests_post: Any | None = None,
) -> dict[str, object]:
    source_env = env if env is not None else os.environ
    path, config, effective, error = _load_codex_config(config_path)
    auth_api_key = read_codex_auth_api_key(auth_path)
    model = _string_config_value(effective.get("model"))
    model_provider = _string_config_value(effective.get("model_provider"))
    provider_name = ""
    resolved_model_provider, provider_config = _provider_config_for_model_provider(config, model_provider)
    if resolved_model_provider and not model_provider:
        model_provider = resolved_model_provider
    if isinstance(provider_config, dict):
        provider_name = _string_config_value(provider_config.get("name")) or model_provider

    if error and error != "missing":
        return {
            "status": "failed",
            "path": str(path),
            "message": error,
            "model": model,
            "model_provider": model_provider,
            "provider_name": provider_name,
            "models": [],
            "sources": [],
            "responses_api": {"status": "unknown", "message": "Config could not be read"},
        }

    sources = _model_sources_from_environment(source_env, auth_api_key)
    config_source = _model_source_from_config(config, effective, source_env, auth_api_key) if not error else None
    preferred_source_id = config_source.source_id if config_source else ""
    if config_source and all(source.base_url.rstrip("/") != config_source.base_url.rstrip("/") for source in sources):
        sources.append(config_source)

    safe_sources: list[dict[str, object]] = []
    models: list[str] = []
    getter = requests_get or requests.get
    poster = requests_post if requests_post is not None else (requests.post if requests_get is None else None)
    for source in sources:
        source_models, source_status = _fetch_models_from_source(source, getter)
        probe_model = model if model in source_models else (source_models[0] if source_models else model)
        source_status["responses_api"] = _probe_responses_api_support(source, probe_model, poster)
        models.extend(source_models)
        safe_sources.append(source_status)

    models = _unique_strings(models)
    default_model = model if model in models else (models[0] if models else "")
    if models:
        status = "ok"
    elif sources and any(source.get("status") == "failed" for source in safe_sources):
        status = "failed"
    elif error == "missing":
        status = "missing"
    else:
        status = "not_configured"

    return {
        "status": status,
        "path": str(path),
        "model": model,
        "model_provider": model_provider,
        "provider_name": provider_name,
        "default_model": default_model,
        "models": models,
        "sources": safe_sources,
        "responses_api": _preferred_responses_api_status(safe_sources, preferred_source_id),
    }


def user_scripts_config_dir() -> Path:
    if sys.platform == "win32":
        base = os.environ.get("APPDATA")
        return Path(base) / "Codex++" if base else Path.home() / "AppData" / "Roaming" / "Codex++"
    return Path(os.environ.get("XDG_CONFIG_HOME", Path.home() / ".config")) / "Codex++"


def backend_settings() -> BackendSettings:
    return SettingsStore().load()


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


def has_proxy_environment(env: dict[str, str] | None = None) -> bool:
    source = env or os.environ
    return any(source.get(name) for name in ("HTTPS_PROXY", "HTTP_PROXY", "ALL_PROXY", "https_proxy", "http_proxy", "all_proxy"))


def local_proxy_url() -> str | None:
    for port in (7897, 7890, 10809, 10808, 1080):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                return f"http://127.0.0.1:{port}"
        except OSError:
            continue
    return None


def codex_process_environment() -> dict[str, str]:
    env = os.environ.copy()
    if has_proxy_environment(env):
        return env
    proxy = local_proxy_url()
    if proxy:
        env.setdefault("HTTP_PROXY", proxy)
        env.setdefault("HTTPS_PROXY", proxy)
        env.setdefault("ALL_PROXY", proxy)
    return env


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
    app_user_model_id = packaged_app_user_model_id(app_dir) if sys.platform == "win32" else None
    env = codex_process_environment()
    if app_user_model_id:
        proxy_keys = ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY")
        previous = {key: os.environ.get(key) for key in proxy_keys}
        os.environ.update({key: env[key] for key in proxy_keys if key in env})
        try:
            return activate_packaged_app(app_user_model_id, subprocess.list2cmdline(build_codex_arguments(debug_port)))
        finally:
            for key, value in previous.items():
                if value is None:
                    os.environ.pop(key, None)
                else:
                    os.environ[key] = value
    if app_dir.suffix == ".app":
        subprocess.run(["open", "-a", str(app_dir), "--args", *build_codex_arguments(debug_port)], check=True, env=env)
        return None
    return subprocess.Popen(build_codex_command(app_dir, debug_port), env=env)


def start_helper(service, export_service: MarkdownExportService | None = None, host: str = "127.0.0.1", port: int = 57321) -> HelperServer:
    server = InjectedHelperServer(host, port, service, export_service=export_service)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server


def helper_health_ok(port: int, host: str = "127.0.0.1") -> bool:
    try:
        session = requests.Session()
        session.trust_env = False
        response = session.get(f"http://{host}:{port}/health", timeout=1)
        response.raise_for_status()
        return bool(response.json().get("ok"))
    except Exception:
        return False


def start_or_attach_helper(
    service,
    export_service: MarkdownExportService | None = None,
    host: str = "127.0.0.1",
    port: int = 57321,
) -> HelperServer | AttachedHelperServer:
    if helper_health_ok(port, host):
        _log_runtime_event(f"attached existing helper host={host} port={port}")
        return AttachedHelperServer(port)
    return start_helper(service, export_service, host, port)


def shutdown_helper(server: HelperServer) -> None:
    if isinstance(server, AttachedHelperServer):
        return
    server.shutdown()
    server.server_close()


def inject_with_retry(
    debug_port: int,
    script_path: Path,
    helper_port: int,
    service: ApiFirstDeleteService,
    export_service: MarkdownExportService,
    runtime: CodexPlusRuntime,
    attempts: int = 20,
    delay: float = 0.5,
) -> Any:
    last_error: Exception | None = None
    for _ in range(attempts):
        try:
            def on_injection(injection):
                runtime.add_websocket_url(injection.websocket_url)
                evaluate_user_scripts(injection.websocket_url, runtime.user_scripts.build_enabled_bundle())

            injection = inject_file_into_all_pages(
                debug_port,
                script_path,
                helper_port,
                lambda path, payload: handle_bridge_request(service, export_service, path, payload, runtime),
                on_injection=on_injection,
            )
            _log_runtime_event(f"injected renderer bridge debug_port={debug_port} helper_port={helper_port}")
            if injection.websocket_url:
                runtime.add_websocket_url(injection.websocket_url)
            return injection
        except Exception as exc:
            last_error = exc
            _log_runtime_event(f"injection attempt failed debug_port={debug_port} helper_port={helper_port}: {exc}")
            time.sleep(delay)
    if last_error is not None:
        raise last_error
    raise RuntimeError("Codex injection failed")


def start_bridge_watchdog(
    debug_port: int,
    script_path: Path,
    helper_port: int,
    service: ApiFirstDeleteService,
    export_service: MarkdownExportService,
    runtime: CodexPlusRuntime,
    interval: float = 5.0,
) -> threading.Thread:
    def watch() -> None:
        while True:
            time.sleep(interval)
            check_and_reinject_bridge(debug_port, script_path, helper_port, service, export_service, runtime)

    thread = threading.Thread(target=watch, daemon=True)
    thread.start()
    return thread


def check_and_reinject_bridge(
    debug_port: int,
    script_path: Path,
    helper_port: int,
    service: ApiFirstDeleteService,
    export_service: MarkdownExportService,
    runtime: CodexPlusRuntime,
) -> bool:
    websocket_url = runtime.websocket_url
    if not websocket_url:
        return False
    try:
        result = evaluate_script(websocket_url, "typeof window.__codexSessionDeleteBridge === 'function'")
        if result.get("result", {}).get("result", {}).get("value"):
            return False
        _log_runtime_event(f"renderer bridge missing; reinjecting debug_port={debug_port} helper_port={helper_port}")
    except Exception as exc:
        _log_runtime_event(f"bridge health check failed; reinjecting debug_port={debug_port} helper_port={helper_port}: {exc}")
    try:
        inject_with_retry(debug_port, script_path, helper_port, service, export_service, runtime, attempts=3, delay=0.5)
        return True
    except Exception as exc:
        _log_runtime_event(f"bridge reinjection failed debug_port={debug_port} helper_port={helper_port}: {exc}")
        return False


def launch_and_inject(app_dir: Path | None, db_path: Path | None, backup_dir: Path, debug_port: int, helper_port: int) -> tuple[HelperServer, Any]:
    resolved_app_dir = resolve_codex_app_dir(app_dir)
    if resolved_app_dir is None:
        raise RuntimeError("Codex App directory not found")
    debug_port = select_windows_loopback_port(debug_port)
    helper_port = select_windows_loopback_port(helper_port)
    service = ApiFirstDeleteService(UnavailableApiAdapter(), db_path, backup_dir)
    export_service = MarkdownExportService(db_path)
    script_path = Path(__file__).parent / "inject" / "renderer-inject.js"
    builtin_user_scripts_dir = Path(__file__).parent / "user_scripts"
    user_config_dir = user_scripts_config_dir()
    user_script_manager = UserScriptManager(builtin_user_scripts_dir, user_config_dir / "user_scripts", user_config_dir / "user_scripts.json")
    runtime = CodexPlusRuntime(None, user_script_manager, debug_port)
    if backend_settings().provider_sync_enabled:
        sync_result = run_provider_sync()
        if sync_result.status == ProviderSyncStatus.SKIPPED:
            print(f"Provider sync skipped: {sync_result.message}")
    server = start_or_attach_helper(service, export_service, port=helper_port)
    codex_proc = None
    try:
        codex_proc = launch_codex_app(resolved_app_dir, debug_port)
        server.bridge_socket = inject_with_retry(debug_port, script_path, server.port, service, export_service, runtime)
        start_bridge_watchdog(debug_port, script_path, server.port, service, export_service, runtime)
        return server, codex_proc
    except Exception:
        shutdown_helper(server)
        # Kill any Codex process we just activated so the next attempt starts from a clean state
        # instead of staring at a half-rendered white window.
        if sys.platform == "win32":
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
            except (OSError, subprocess.SubprocessError):
                pass
        raise


def _log_runtime_event(message: str) -> None:
    try:
        from codex_session_delete.cli import log_runtime_event

        log_runtime_event(message)
    except Exception:
        pass


def handle_bridge_request(
    service: ApiFirstDeleteService,
    export_service: MarkdownExportService,
    path: str,
    payload: dict[str, object],
    runtime: CodexPlusRuntime | None = None,
) -> dict[str, object]:
    if path == "/settings/get" and runtime:
        return SettingsStore().load().to_dict()
    if path == "/settings/set" and runtime:
        return SettingsStore().update(payload).to_dict()
    if path == "/user-scripts/list" and runtime:
        return runtime.user_scripts.inventory()
    if path == "/user-scripts/set-enabled" and runtime:
        runtime.user_scripts.set_global_enabled(bool(payload.get("enabled", True)))
        return runtime.user_scripts.inventory()
    if path == "/user-scripts/set-script-enabled" and runtime:
        runtime.user_scripts.set_script_enabled(str(payload.get("key", "")), bool(payload.get("enabled", True)))
        return runtime.user_scripts.inventory()
    if path == "/user-scripts/reload" and runtime:
        return runtime.reload_user_scripts()
    if path == "/devtools/open" and runtime:
        return runtime.open_devtools()
    if path == "/backend/status" and runtime:
        return runtime.backend_status()
    if path == "/backend/repair" and runtime:
        return runtime.repair_backend()
    if path == "/zed-remote/status" and runtime:
        return zed_remote.zed_remote_status()
    if path == "/zed-remote/resolve-host" and runtime:
        return zed_remote.resolve_ssh_target_response(payload)
    if path == "/zed-remote/open" and runtime:
        return zed_remote.open_zed_remote(payload)
    if path == "/ads" and runtime:
        return runtime.ads()
    if path == "/codex-model-catalog" and runtime:
        return runtime.codex_model_catalog()
    if path == "/codex-config-model" and runtime:
        return runtime.codex_model_catalog()
    if path == "/delete":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return service.delete(session).to_dict()
    if path == "/undo":
        return service.undo(str(payload.get("undo_token", ""))).to_dict()
    if path == "/export-markdown":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return export_service.export(session).to_dict()
    if path == "/archived-thread":
        session = service.find_archived_thread_by_title(str(payload.get("title", "")))
        return {"session_id": session.session_id, "title": session.title} if session else {"session_id": "", "title": ""}
    if path == "/move-thread-workspace":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return service.move_thread_workspace(session, str(payload.get("target_cwd", "")))
    if path == "/thread-sort-key":
        session = SessionRef(session_id=str(payload.get("session_id", "")), title=str(payload.get("title", "")))
        return service.thread_sort_key(session)
    if path == "/thread-sort-keys":
        raw_sessions = payload.get("sessions", [])
        sessions = [
            SessionRef(session_id=str(item.get("session_id", "")), title=str(item.get("title", "")))
            for item in raw_sessions
            if isinstance(item, dict)
        ] if isinstance(raw_sessions, list) else []
        return service.thread_sort_keys(sessions)
    return {"status": DeleteStatus.FAILED.value, "session_id": str(payload.get("session_id", "")), "message": "Unknown bridge path"}
