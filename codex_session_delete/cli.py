from __future__ import annotations

import argparse
import os
import subprocess
import sys
import traceback
from pathlib import Path

from codex_session_delete.helper_server import HelperServer
from codex_session_delete.installers import InstallOptions, install_codex_plus_plus, uninstall_codex_plus_plus
from codex_session_delete.launcher import launch_and_inject, shutdown_helper
from codex_session_delete import updater
from codex_session_delete import watcher


def add_launch_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument("--app-dir", type=Path, default=None)
    parser.add_argument("--db", type=Path, default=Path.home() / ".codex" / "state_5.sqlite", help="SQLite database path for local deletion fallback")
    parser.add_argument("--backup-dir", type=Path, default=Path.home() / ".codex-session-delete" / "backups")
    parser.add_argument("--debug-port", type=int, default=9229)
    parser.add_argument("--helper-port", type=int, default=57321)


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description="Launch and install Codex++ for Codex App")
    subparsers = parser.add_subparsers(dest="command")

    launch_parser = subparsers.add_parser("launch", help="Launch Codex with Codex++ injection")
    add_launch_arguments(launch_parser)

    install_parser = subparsers.add_parser("install", help="Install the Codex++ launcher entry point")
    install_parser.add_argument("--install-root", type=Path, default=None)
    install_parser.add_argument("--launcher-command", default=None)

    setup_parser = subparsers.add_parser("setup", help="Install Codex++ with defaults")
    setup_parser.add_argument("--install-root", type=Path, default=None)

    uninstall_parser = subparsers.add_parser("uninstall", help="Remove the Codex++ launcher entry point")
    uninstall_parser.add_argument("--install-root", type=Path, default=None)
    uninstall_parser.add_argument("--remove-data", action="store_true")

    remove_parser = subparsers.add_parser("remove", help="Remove Codex++ with defaults")
    remove_parser.add_argument("--install-root", type=Path, default=None)
    remove_parser.add_argument("--remove-data", action="store_true")

    watch_parser = subparsers.add_parser("watch", help="Run the Codex watcher loop (auto-reinject when Codex is launched normally)")
    watch_parser.add_argument("--debug-port", type=int, default=9229)

    watch_install_parser = subparsers.add_parser("watch-install", help="Register the watcher to run at Windows logon")
    watch_install_parser.add_argument("--debug-port", type=int, default=9229)

    subparsers.add_parser("watch-remove", help="Unregister the watcher logon task")

    subparsers.add_parser("watch-enable", help="Re-enable the watcher loop after it was disabled")
    subparsers.add_parser("watch-disable", help="Disable the watcher loop without removing the logon task")

    subparsers.add_parser("check-update", help="Check GitHub Releases for a newer Codex++ version")
    subparsers.add_parser("update", help="Update Codex++ from the latest GitHub Release")

    add_launch_arguments(parser)
    return parser




def launch_log_path() -> Path:
    return Path.home() / ".codex-session-delete" / "launcher.log"


def log_launch_failure(exc: BaseException) -> None:
    path = launch_log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text("".join(traceback.format_exception(type(exc), exc, exc.__traceback__)), encoding="utf-8")


def wait_for_windows_process_id(process_id: int) -> None:
    if sys.platform != "win32":
        return
    import ctypes

    synchronize = 0x00100000
    infinite = 0xFFFFFFFF
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.OpenProcess.argtypes = [ctypes.c_ulong, ctypes.c_int, ctypes.c_ulong]
    kernel32.OpenProcess.restype = ctypes.c_void_p
    kernel32.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_ulong]
    kernel32.WaitForSingleObject.restype = ctypes.c_ulong
    kernel32.CloseHandle.argtypes = [ctypes.c_void_p]
    kernel32.CloseHandle.restype = ctypes.c_int

    handle = kernel32.OpenProcess(synchronize, False, process_id)
    if not handle:
        return
    try:
        kernel32.WaitForSingleObject(handle, infinite)
    finally:
        kernel32.CloseHandle(handle)


def wait_for_shutdown(server: HelperServer, codex_proc) -> None:
    try:
        if isinstance(codex_proc, int):
            wait_for_windows_process_id(codex_proc)
        elif codex_proc is None and sys.platform == "darwin":
            import time as _time
            while True:
                if not is_macos_codex_running():
                    break
                _time.sleep(2)
        elif codex_proc is not None:
            codex_proc.wait()
    except KeyboardInterrupt:
        pass
    finally:
        shutdown_helper(server)


def is_macos_codex_running() -> bool:
    result = subprocess.run(["ps", "-axo", "pid=,command="], capture_output=True, text=True, check=False)
    return any("/Codex.app/Contents/MacOS/Codex " in f"{line} " for line in result.stdout.splitlines())


def stop_existing_windows_launchers() -> None:
    if sys.platform != "win32":
        return
    # Protect our own process AND every ancestor up the chain. venv's python.exe
    # is a stub that re-spawns a second python.exe child (same CommandLine), and
    # shells/bash also carry launcher command lines in their ancestry. Killing
    # any of them (e.g. the stub parent) tears down stdio for the real worker
    # and aborts the launch before Codex is activated.
    script = (
        "$self = [int]$env:CODEX_PLUS_PLUS_PID; "
        "$protect = New-Object System.Collections.Generic.HashSet[int]; "
        "$cur = $self; "
        "while ($cur -ne 0 -and $protect.Add($cur)) { "
        "$p = Get-CimInstance Win32_Process -Filter \"ProcessId=$cur\" -ErrorAction SilentlyContinue; "
        "if ($null -eq $p) { break }; $cur = [int]$p.ParentProcessId "
        "} "
        "Get-CimInstance Win32_Process | "
        "Where-Object { -not $protect.Contains([int]$_.ProcessId) -and "
        "$_.CommandLine -match 'pythonw?(\\.exe)?\"?\\s+(-[A-Za-z]+\\s+)*-m\\s+codex_session_delete\\s+launch(\\s|$)' } | "
        "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"
    )
    env = {**os.environ, "CODEX_PLUS_PLUS_PID": str(os.getpid())}
    subprocess.run(["powershell", "-NoProfile", "-Command", script], check=False, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, env=env)


def run_launch(args: argparse.Namespace) -> int:
    stop_existing_windows_launchers()
    maybe_print_update_notice()
    try:
        server, codex_proc = launch_and_inject(args.app_dir, args.db, args.backup_dir, args.debug_port, args.helper_port)
    except Exception as exc:
        log_launch_failure(exc)
        raise
    print(f"Codex session delete helper running on http://127.0.0.1:{server.port}")
    print("Keep this terminal open while using the delete buttons. Press Ctrl+C to stop.")
    wait_for_shutdown(server, codex_proc)
    return 0


def print_release_notice(release: updater.Release) -> None:
    print(f"发现新版本 {release.version}: {release.url}")
    asset_name = getattr(release, "asset_name", None)
    if asset_name:
        print(f"更新包: {asset_name}")
    print("运行 `python -m codex_session_delete update` 可从 GitHub Release 更新。")


def maybe_print_update_notice() -> None:
    try:
        release = updater.check_for_update()
    except Exception:
        return
    if release is not None:
        print_release_notice(release)


def run_check_update() -> int:
    if updater.is_source_tree_mode():
        print("检测到当前正在从源码目录运行 Codex++。源码模式不检测 Release 版本；运行 `python -m codex_session_delete update` 可迁移到 Release 安装。")
        return 0
    release = updater.check_for_update()
    if release is None:
        print("当前已是最新版本。")
        return 0
    print_release_notice(release)
    return 0


def run_update() -> int:
    if updater.is_source_tree_mode():
        print("检测到当前正在从源码目录运行 Codex++，将迁移到 Release 安装。")
        release = updater.fetch_latest_release()
    else:
        release = updater.check_for_update()
        if release is None:
            print("当前已是最新版本。")
            return 0
    print_release_notice(release)
    updater.perform_update(release)
    print("更新完成。")
    return 0


WATCHER_RUN_NAME = "CodexPlusPlusWatcher"
WATCHER_RUN_KEY = "HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"
WATCHER_STARTUP_SHORTCUT_NAME = "CodexPlusPlusWatcher.lnk"


def _watcher_command(debug_port: int) -> tuple[str, str, str]:
    python = sys.executable
    pythonw = Path(python).with_name("pythonw.exe")
    exe = str(pythonw if pythonw.exists() else python)
    arguments = f"-m codex_session_delete watch --debug-port {debug_port}"
    full = f'"{exe}" {arguments}'
    return exe, arguments, full


def _ps_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def install_watcher_logon_task(debug_port: int) -> None:
    if sys.platform != "win32":
        raise RuntimeError("watch-install is only supported on Windows")
    exe, arguments, full_command = _watcher_command(debug_port)
    project_root = str(Path(__file__).resolve().parent.parent)
    script = f"""
$ErrorActionPreference = 'Stop'
$Exe = {_ps_quote(exe)}
$Args = {_ps_quote(arguments)}
$RunFullCommand = {_ps_quote(full_command)}
$ProjectRoot = {_ps_quote(project_root)}
$ShortcutName = {_ps_quote(WATCHER_STARTUP_SHORTCUT_NAME)}
# 1) HKCU Run value
New-Item -Path '{WATCHER_RUN_KEY}' -Force | Out-Null
Set-ItemProperty -Path '{WATCHER_RUN_KEY}' -Name '{WATCHER_RUN_NAME}' -Value $RunFullCommand
# 2) Startup folder .lnk (survives registry cleanups)
$Startup = [Environment]::GetFolderPath('Startup')
New-Item -ItemType Directory -Force -Path $Startup | Out-Null
$Shell = New-Object -ComObject WScript.Shell
$ShortcutPath = Join-Path $Startup $ShortcutName
$Shortcut = $Shell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath = $Exe
$Shortcut.Arguments = $Args
$Shortcut.WorkingDirectory = $ProjectRoot
$Shortcut.WindowStyle = 7
$Shortcut.Description = 'Codex++ watcher (auto-inject Codex on start)'
$Shortcut.Save()
# 3) Echo what was written for verification
$runValue = (Get-ItemProperty -Path '{WATCHER_RUN_KEY}' -Name '{WATCHER_RUN_NAME}').'{WATCHER_RUN_NAME}'
Write-Output ("watch-install: HKCU Run = " + $runValue)
Write-Output ("watch-install: Startup shortcut = " + $ShortcutPath)
""".strip()
    result = subprocess.run(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
        check=True,
        capture_output=True,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    if result.stdout:
        print(result.stdout.strip())
    # Start the watcher right now as well.
    subprocess.Popen(
        [exe, "-m", "codex_session_delete", "watch", "--debug-port", str(debug_port)],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        close_fds=True,
        creationflags=(
            subprocess.CREATE_NEW_PROCESS_GROUP
            | getattr(subprocess, "DETACHED_PROCESS", 0x00000008)
            | getattr(subprocess, "CREATE_NO_WINDOW", 0)
        ),
    )
    print("watch-install: watcher process spawned")


def uninstall_watcher_logon_task() -> None:
    if sys.platform != "win32":
        return
    script = f"""
Remove-ItemProperty -Path '{WATCHER_RUN_KEY}' -Name '{WATCHER_RUN_NAME}' -ErrorAction SilentlyContinue | Out-Null
$Startup = [Environment]::GetFolderPath('Startup')
$ShortcutPath = Join-Path $Startup {_ps_quote(WATCHER_STARTUP_SHORTCUT_NAME)}
if (Test-Path $ShortcutPath) {{ Remove-Item $ShortcutPath -Force -ErrorAction SilentlyContinue }}
Get-CimInstance Win32_Process -Filter "Name='pythonw.exe' OR Name='python.exe'" | Where-Object {{ $_.CommandLine -match 'codex_session_delete\\s+watch' }} | ForEach-Object {{ Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }}
""".strip()
    subprocess.run(
        ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script],
        check=False,
    )


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if args.command in {"install", "setup"}:
        install_codex_plus_plus(InstallOptions(install_root=args.install_root, launcher_command=getattr(args, "launcher_command", None)))
        return 0
    if args.command in {"uninstall", "remove"}:
        uninstall_codex_plus_plus(InstallOptions(install_root=args.install_root, remove_data=args.remove_data))
        uninstall_watcher_logon_task()
        return 0
    if args.command == "watch":
        return watcher.watch_loop(debug_port=args.debug_port)
    if args.command == "watch-install":
        install_watcher_logon_task(args.debug_port)
        return 0
    if args.command == "watch-remove":
        uninstall_watcher_logon_task()
        return 0
    if args.command == "watch-enable":
        watcher.enable_watcher()
        return 0
    if args.command == "watch-disable":
        watcher.disable_watcher()
        return 0
    if args.command == "check-update":
        return run_check_update()
    if args.command == "update":
        return run_update()
    return run_launch(args)


if __name__ == "__main__":
    raise SystemExit(main())
