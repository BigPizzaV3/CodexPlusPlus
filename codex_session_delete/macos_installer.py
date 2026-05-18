from __future__ import annotations

import plistlib
import shlex
import shutil
import stat
import sys
from pathlib import Path
from typing import TYPE_CHECKING

from codex_session_delete import __version__
from codex_session_delete.app_paths import find_macos_codex_app

ICON_ASSET = Path(__file__).resolve().parent / "assets" / "codex-plus-plus.png"
DEFAULT_GUI_PATH = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

if TYPE_CHECKING:
    from codex_session_delete.installers import InstallOptions


DEFAULT_INSTALL_ROOT = Path("/Applications")
APP_NAME = "Codex++.app"
EXECUTABLE_NAME = "CodexPlusPlus"


def _launcher_command(options: "InstallOptions") -> str:
    if options.launcher_command:
        return options.launcher_command
    project_root = Path(__file__).resolve().parent.parent
    if (project_root / "pyproject.toml").is_file():
        return f"env PYTHONPATH={shlex.quote(str(project_root))} {shlex.quote(sys.executable)} -m codex_session_delete launch"
    return f"{sys.executable} -m codex_session_delete launch"


def _app_root(options: "InstallOptions") -> Path:
    return (options.install_root or DEFAULT_INSTALL_ROOT) / APP_NAME


def install_macos_app(options: "InstallOptions") -> None:
    app = _app_root(options)
    contents = app / "Contents"
    macos = contents / "MacOS"
    resources = contents / "Resources"
    macos.mkdir(parents=True, exist_ok=True)
    resources.mkdir(parents=True, exist_ok=True)

    plist = {
        "CFBundleName": "Codex++",
        "CFBundleDisplayName": "Codex++",
        "CFBundleIdentifier": "com.bigpizzav3.codexplusplus",
        "CFBundleVersion": __version__,
        "CFBundleShortVersionString": __version__,
        "CFBundlePackageType": "APPL",
        "CFBundleExecutable": EXECUTABLE_NAME,
        "CFBundleIconFile": "codex-plus-plus.png",
        "LSUIElement": True,
        "LSMinimumSystemVersion": "12.0",
    }
    (contents / "Info.plist").write_bytes(plistlib.dumps(plist))

    executable = macos / EXECUTABLE_NAME
    executable.write_text(
        f"#!/bin/sh\nexport PATH=\"${{PATH:-{DEFAULT_GUI_PATH}}}:{DEFAULT_GUI_PATH}\"\nexec {_launcher_command(options)}\n",
        encoding="utf-8",
    )
    executable.chmod(executable.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    _copy_codex_icon(resources)


def uninstall_macos_app(options: "InstallOptions") -> None:
    app = _app_root(options)
    if app.exists():
        shutil.rmtree(app)


def _copy_codex_icon(resources: Path) -> None:
    if ICON_ASSET.is_file():
        shutil.copy2(ICON_ASSET, resources / "codex-plus-plus.png")
        return
    codex_app = find_macos_codex_app()
    if codex_app is None:
        return
    icon_src = codex_app / "Contents" / "Resources" / "electron.icns"
    if icon_src.is_file():
        shutil.copy2(icon_src, resources / "electron.icns")
