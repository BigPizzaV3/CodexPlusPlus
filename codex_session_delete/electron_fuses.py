"""Utilities for reading and patching Electron fuses in macOS app bundles.

Electron embeds fuse configuration as a byte sequence in the framework binary.
Format after sentinel: [version: 1B] [count: 1B] [fuse_0 ... fuse_N: each 0x30=off or 0x31=on]
"""
from __future__ import annotations

import subprocess
import sys
from pathlib import Path

_SENTINEL = b"dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX"
_DISABLED = 0x30
_ENABLED = 0x31

# Fuse index within the fuse byte array
_FUSE_CLI_INSPECT_ARGS = 3  # EnableNodeCliInspectArguments


def _electron_framework_binary(app_path: Path) -> Path | None:
    fw = (
        app_path
        / "Contents"
        / "Frameworks"
        / "Electron Framework.framework"
        / "Versions"
        / "A"
        / "Electron Framework"
    )
    return fw if fw.is_file() else None


def _fuse_offset(data: bytes, fuse_index: int) -> int | None:
    pos = data.find(_SENTINEL)
    if pos == -1:
        return None
    # sentinel + version byte (1) + count byte (1) + fuse_index
    offset = pos + len(_SENTINEL) + 2 + fuse_index
    if offset >= len(data):
        return None
    return offset


def is_cli_inspect_enabled(app_path: Path) -> bool:
    """Return True if EnableNodeCliInspectArguments is already on (or binary not found)."""
    binary = _electron_framework_binary(app_path)
    if binary is None:
        return True
    data = binary.read_bytes()
    offset = _fuse_offset(data, _FUSE_CLI_INSPECT_ARGS)
    if offset is None:
        return True
    return data[offset] == _ENABLED


def ensure_cli_inspect_enabled(app_path: Path) -> None:
    """Enable EnableNodeCliInspectArguments and re-sign the app if needed.

    Raises RuntimeError if the fuse cannot be patched.
    """
    if is_cli_inspect_enabled(app_path):
        return

    binary = _electron_framework_binary(app_path)
    if binary is None:
        return

    data = bytearray(binary.read_bytes())
    offset = _fuse_offset(bytes(data), _FUSE_CLI_INSPECT_ARGS)
    if offset is None:
        raise RuntimeError("Could not locate Electron fuse table in Codex framework binary.")

    data[offset] = _ENABLED
    try:
        binary.write_bytes(bytes(data))
    except PermissionError as exc:
        raise RuntimeError(
            f"Cannot write to {binary} — try running Codex++ with elevated permissions."
        ) from exc

    try:
        subprocess.run(
            ["codesign", "--force", "--deep", "--sign", "-", str(app_path)],
            check=True,
            capture_output=True,
        )
    except subprocess.CalledProcessError as exc:
        raise RuntimeError(
            f"Ad-hoc re-signing of {app_path} failed: {exc.stderr.decode(errors='replace')}"
        ) from exc
