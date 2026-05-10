from __future__ import annotations

import os
import re
import sys
import subprocess
from pathlib import Path


_VERSION_RE = re.compile(r"OpenAI\.Codex_([0-9.]+)_")


def _version_tuple(path: Path) -> tuple[int, ...]:
    match = _VERSION_RE.search(path.name)
    if not match:
        return ()
    return tuple(int(part) for part in match.group(1).split(".") if part.isdigit())


def find_latest_codex_app_dir(root: Path | None = None) -> Path | None:
    if root is not None:
        matches = [path for path in root.iterdir() if path.is_dir() and _version_tuple(path)]
        if not matches:
            return None
        latest = max(matches, key=_version_tuple)
        app = latest / "app"
        return app if app.is_dir() else latest

    discovered = _find_codex_app_dir_with_appx()
    if discovered is not None:
        return discovered

    program_files = os.environ.get("ProgramFiles", r"C:\Program Files")
    windows_apps = Path(program_files) / "WindowsApps"
    if windows_apps.is_dir():
        try:
            return find_latest_codex_app_dir(windows_apps)
        except OSError:
            return None
    return None


def _find_codex_app_dir_with_appx() -> Path | None:
    cmd = 'Get-AppxPackage -Name "OpenAI.Codex" | Select-Object -ExpandProperty InstallLocation'
    try:
        r = subprocess.run(
            ["powershell", "-NoProfile", "-Command", cmd],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=8,
            check=False,
        )
        if r.returncode != 0 or not (p := r.stdout.strip()):
            return None
        root = Path(p)
        app = root / "app"
        return app if app.is_dir() else root
    except (OSError, subprocess.SubprocessError):
        return None

def user_data_candidates() -> list[Path]:
    candidates: list[Path] = []
    local = os.environ.get("LOCALAPPDATA")
    roaming = os.environ.get("APPDATA")
    if local:
        candidates.extend([
            Path(local) / "OpenAI" / "Codex",
            Path(local) / "OpenAI.Codex",
            Path(local) / "Codex",
        ])
    if roaming:
        candidates.extend([
            Path(roaming) / "OpenAI" / "Codex",
            Path(roaming) / "OpenAI.Codex",
            Path(roaming) / "Codex",
        ])
    return candidates


def _macos_app_candidates(root: Path) -> list[Path]:
    if root.suffix == ".app":
        return [root]
    names = ["Codex.app", "OpenAI Codex.app", "OpenAI.Codex.app"]
    return [root / name for name in names]


def find_macos_codex_app(candidates: list[Path] | None = None) -> Path | None:
    search = candidates or [Path("/Applications"), Path.home() / "Applications"]
    for root in search:
        for path in _macos_app_candidates(root):
            if path.is_dir():
                return path
    return None


def resolve_codex_app_dir(app_dir: Path | None = None) -> Path | None:
    if app_dir is not None:
        return app_dir
    if sys.platform == "darwin":
        return find_macos_codex_app()
    return find_latest_codex_app_dir()
