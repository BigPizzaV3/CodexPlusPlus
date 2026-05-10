from __future__ import annotations

import os


def _ensure_windows_system_env() -> None:
    if os.name != "nt":
        return
    system_root = os.environ.get("SystemRoot") or os.environ.get("windir") or r"C:\Windows"
    os.environ.setdefault("SystemRoot", system_root)
    os.environ.setdefault("windir", system_root)


_ensure_windows_system_env()

__version__ = "1.0.4"
