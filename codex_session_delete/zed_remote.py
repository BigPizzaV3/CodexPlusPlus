from __future__ import annotations

import ipaddress
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import quote


class ZedRemoteError(ValueError):
    """Raised when a Zed remote-open request cannot be built safely."""


@dataclass(frozen=True)
class SshTarget:
    user: str
    host: str
    port: int | None = None


def candidate_zed_app_paths() -> list[Path]:
    return [
        Path("/Applications/Zed.app"),
        Path("/Applications/Zed Preview.app"),
        Path("/Applications/Zed Nightly.app"),
        Path.home() / "Applications" / "Zed.app",
        Path.home() / "Applications" / "Zed Preview.app",
        Path.home() / "Applications" / "Zed Nightly.app",
    ]


def find_zed_app_path() -> Path | None:
    return next((path for path in candidate_zed_app_paths() if path.exists()), None)


def find_zed_cli_path() -> str:
    return shutil.which("zed") or ""


def zed_remote_status() -> dict[str, object]:
    app_path = find_zed_app_path()
    cli_path = find_zed_cli_path()
    platform_supported = sys.platform in {"darwin", "win32", "linux"}
    return {
        "status": "ok" if platform_supported else "failed",
        "platformSupported": platform_supported,
        "zedAppFound": app_path is not None,
        "zedCliFound": bool(cli_path),
        "zedAppPath": str(app_path) if app_path else "",
        "zedCliPath": cli_path,
    }


def string_value(value: object) -> str:
    return value.strip() if isinstance(value, str) else ""


def split_ssh_authority(value: str) -> tuple[str, str, int | None]:
    """Split a saved SSH authority into user, host, and optional port."""
    authority = value.strip()
    if not authority:
        return "", "", None
    user = ""
    if "@" in authority:
        user, authority = authority.rsplit("@", 1)
    port: int | None = None
    host = authority
    if authority.startswith("["):
        close_index = authority.find("]")
        if close_index >= 0:
            host = authority[: close_index + 1]
            suffix = authority[close_index + 1 :]
            if suffix.startswith(":"):
                port = parse_port(suffix[1:])
        return user.strip(), host.strip(), port
    if authority.count(":") == 1:
        candidate_host, candidate_port = authority.rsplit(":", 1)
        if candidate_port.isdigit():
            host = candidate_host
            port = parse_port(candidate_port)
    return user.strip(), host.strip(), port


def parse_port(value: object) -> int | None:
    if value in (None, ""):
        return None
    if isinstance(value, bool):
        raise ZedRemoteError("Invalid SSH port")
    if isinstance(value, int):
        port = value
    elif isinstance(value, str) and value.strip().isdigit():
        port = int(value.strip())
    else:
        raise ZedRemoteError("Invalid SSH port")
    if port < 1 or port > 65535:
        raise ZedRemoteError("Invalid SSH port")
    return port


def validate_ssh_host(host: str) -> str:
    host = host.strip()
    if not host:
        raise ZedRemoteError("Cannot determine remote SSH host for this file")
    if any(ord(char) < 32 or ord(char) == 127 or char.isspace() for char in host):
        raise ZedRemoteError("Invalid SSH host")
    if any(char in host for char in "/?#@"):
        raise ZedRemoteError("Invalid SSH host")
    if host.startswith("[") or host.endswith("]"):
        if not (host.startswith("[") and host.endswith("]")):
            raise ZedRemoteError("Invalid SSH host")
        inner = host[1:-1]
        try:
            ipaddress.IPv6Address(inner)
        except ValueError as exc:
            raise ZedRemoteError("Invalid SSH host") from exc
        return host
    if "[" in host or "]" in host:
        raise ZedRemoteError("Invalid SSH host")
    return host


def target_from_payload(payload: dict[str, object]) -> SshTarget:
    raw_ssh = payload.get("ssh")
    ssh = raw_ssh if isinstance(raw_ssh, dict) else {}
    raw_host = string_value(ssh.get("host") or ssh.get("hostname") or ssh.get("hostName"))
    authority_user, authority_host, authority_port = split_ssh_authority(raw_host)
    user = string_value(ssh.get("user") or ssh.get("username")) or authority_user
    host = validate_ssh_host(authority_host)
    port = parse_port(ssh.get("port")) if ssh.get("port") not in (None, "") else authority_port
    return SshTarget(user=user, host=host, port=port)


def encode_remote_path(path: str) -> str:
    if not path:
        raise ZedRemoteError("Remote path is required")
    if not path.startswith("/"):
        raise ZedRemoteError("Remote path must be absolute")
    return "/".join(quote(segment) for segment in path.split("/"))


def build_zed_remote_url(target: SshTarget, path: str) -> str:
    host = validate_ssh_host(target.host)
    port = parse_port(target.port)
    user_prefix = f"{quote(target.user.strip(), safe='')}@" if target.user.strip() else ""
    port_suffix = f":{port}" if port else ""
    encoded_path = encode_remote_path(path)
    return f"ssh://{user_prefix}{host}{port_suffix}{encoded_path}"


def launch_zed_url(url: str) -> None:
    app_path = find_zed_app_path()
    cli_path = find_zed_cli_path()
    if sys.platform == "darwin" and app_path is not None:
        subprocess.run(["open", "-a", str(app_path), url], check=True)
        return
    if cli_path:
        subprocess.run([cli_path, url], check=True)
        return
    raise ZedRemoteError("Zed is not installed or not available on PATH")


def codex_global_state_path() -> Path:
    """Return the Codex Desktop global-state file used for remote SSH metadata."""
    codex_home = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex")
    return codex_home / ".codex-global-state.json"


def target_from_managed_remote_connection(connection: dict[str, object]) -> SshTarget:
    """Build an SSH target from a Codex managed remote connection record."""
    ssh_host = string_value(connection.get("sshHost") or connection.get("hostname"))
    ssh_alias = string_value(connection.get("sshAlias") or connection.get("alias"))
    authority_user, authority_host, authority_port = split_ssh_authority(ssh_host)
    host = authority_host or ssh_alias
    user = string_value(connection.get("sshUser") or connection.get("user")) or authority_user
    port_value = connection.get("sshPort")
    port = parse_port(port_value) if port_value not in (None, "") else authority_port
    return SshTarget(user=user, host=validate_ssh_host(host), port=port)


def resolve_ssh_target_from_global_state(state: dict[str, object], host_id: str) -> SshTarget:
    """Resolve a Codex remote host id into a Zed-compatible SSH target."""
    if not host_id:
        raise ZedRemoteError("Remote host id is required")
    raw_connections = state.get("codex-managed-remote-connections")
    connections = raw_connections if isinstance(raw_connections, list) else []
    for connection in connections:
        if not isinstance(connection, dict):
            continue
        if string_value(connection.get("hostId")) != host_id:
            continue
        return target_from_managed_remote_connection(connection)
    raise ZedRemoteError("Cannot resolve remote SSH host for this file")


def resolve_ssh_target_for_host_id(host_id: str, state_path: Path | None = None) -> SshTarget:
    """Read Codex Desktop global state and resolve a remote host id."""
    path = state_path or codex_global_state_path()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except OSError as exc:
        raise ZedRemoteError("Cannot read Codex remote connection state") from exc
    except json.JSONDecodeError as exc:
        raise ZedRemoteError("Cannot parse Codex remote connection state") from exc
    if not isinstance(data, dict):
        raise ZedRemoteError("Cannot parse Codex remote connection state")
    return resolve_ssh_target_from_global_state(data, host_id)


def ordered_remote_projects_from_global_state(state: dict[str, object]) -> list[dict[str, object]]:
    """Return saved remote projects in Codex's most recent project order first."""
    raw_projects = state.get("remote-projects")
    projects = [project for project in raw_projects if isinstance(project, dict)] if isinstance(raw_projects, list) else []
    raw_order = state.get("project-order")
    project_order = [string_value(item) for item in raw_order] if isinstance(raw_order, list) else []
    by_id = {string_value(project.get("id")): project for project in projects if string_value(project.get("id"))}
    ordered = [by_id[project_id] for project_id in project_order if project_id in by_id]
    ordered_ids = {string_value(project.get("id")) for project in ordered}
    ordered.extend(project for project in projects if string_value(project.get("id")) not in ordered_ids)
    return ordered


def fallback_open_request_from_global_state(state: dict[str, object], host_id: str = "") -> dict[str, object]:
    """Build a Zed open request from Codex's selected remote workspace metadata."""
    selected_host_id = string_value(host_id) or string_value(state.get("selected-remote-host-id"))
    projects = ordered_remote_projects_from_global_state(state)
    selected_project = None
    for project in projects:
        project_host_id = string_value(project.get("hostId"))
        if selected_host_id and project_host_id != selected_host_id:
            continue
        remote_path = string_value(project.get("remotePath"))
        if not remote_path.startswith("/"):
            continue
        selected_project = project
        break
    if selected_project is None:
        raise ZedRemoteError("Cannot determine remote workspace or file for Zed")
    resolved_host_id = selected_host_id or string_value(selected_project.get("hostId"))
    if not resolved_host_id:
        raise ZedRemoteError("Remote host id is required")
    target = resolve_ssh_target_from_global_state(state, resolved_host_id)
    return {
        "hostId": resolved_host_id,
        "ssh": {"user": target.user, "host": target.host, "port": target.port},
        "path": string_value(selected_project.get("remotePath")),
    }


def fallback_open_request_response(payload: dict[str, object]) -> dict[str, object]:
    """Return a bridge response for opening the current selected remote workspace."""
    try:
        state = json.loads(codex_global_state_path().read_text(encoding="utf-8"))
    except OSError as exc:
        return {"status": "failed", "message": "Cannot read Codex remote connection state"}
    except json.JSONDecodeError as exc:
        return {"status": "failed", "message": "Cannot parse Codex remote connection state"}
    if not isinstance(state, dict):
        return {"status": "failed", "message": "Cannot parse Codex remote connection state"}
    try:
        request = fallback_open_request_from_global_state(state, string_value(payload.get("hostId")))
        return {"status": "ok", "request": request}
    except ZedRemoteError as exc:
        return {"status": "failed", "message": str(exc)}


def resolve_ssh_target_response(payload: dict[str, object]) -> dict[str, object]:
    """Return serialized SSH metadata for a remote host id bridge request."""
    try:
        target = resolve_ssh_target_for_host_id(string_value(payload.get("hostId")))
        return {"status": "ok", "ssh": {"user": target.user, "host": target.host, "port": target.port}}
    except ZedRemoteError as exc:
        return {"status": "failed", "message": str(exc)}


def open_zed_remote(payload: dict[str, object]) -> dict[str, object]:
    try:
        target = target_from_payload(payload)
        raw_path = payload.get("path")
        path = raw_path if isinstance(raw_path, str) else ""
        url = build_zed_remote_url(target, path)
        launch_zed_url(url)
        return {"status": "ok", "url": url}
    except ZedRemoteError as exc:
        return {"status": "failed", "message": str(exc)}
    except (OSError, subprocess.SubprocessError) as exc:
        return {"status": "failed", "message": f"Failed to launch Zed: {exc}"}
