from __future__ import annotations

import os
import plistlib
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


_MACOS_BUNDLE_IDENTIFIERS = {
    "com.openai.codex",
    "com.openai.chatgpt.codex",
    "com.openai.chatgpt",
}


def _macos_bundle_identifier(path: Path) -> str | None:
    plist_path = path / "Contents" / "Info.plist"
    try:
        with plist_path.open("rb") as handle:
            plist = plistlib.load(handle)
    except (OSError, plistlib.InvalidFileException):
        return None
    identifier = plist.get("CFBundleIdentifier")
    return identifier if isinstance(identifier, str) else None


def _is_macos_codex_app(path: Path) -> bool:
    if not path.is_dir() or path.suffix != ".app":
        return False
    identifier = _macos_bundle_identifier(path)
    if identifier in _MACOS_BUNDLE_IDENTIFIERS:
        return True
    name = path.stem.lower()
    return "codex" in name and "codex++" not in name


def _scan_macos_codex_apps(root: Path) -> list[Path]:
    try:
        return sorted(
            (path for path in root.iterdir() if _is_macos_codex_app(path)),
            key=lambda path: path.name.lower(),
        )
    except OSError:
        return []


def find_macos_codex_app(candidates: list[Path] | None = None) -> Path | None:
    search = candidates or [Path("/Applications"), Path.home() / "Applications"]
    for root in search:
        for path in _macos_app_candidates(root):
            if _is_macos_codex_app(path):
                return path
        for path in _scan_macos_codex_apps(root):
            return path
    return None


def resolve_codex_app_dir(app_dir: Path | None = None) -> Path | None:
    if app_dir is not None:
        return app_dir
    if sys.platform == "darwin":
        return find_macos_codex_app()
    return find_latest_codex_app_dir()
