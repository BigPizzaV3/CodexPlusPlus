from __future__ import annotations

import os
import socket
import subprocess
import sys
import time
import traceback
from datetime import datetime
from pathlib import Path


WATCHER_INTERVAL_SECONDS = 3.0
CDP_PROBE_TIMEOUT_SECONDS = 0.5
CDP_WAIT_TIMEOUT_SECONDS = 25.0
KILL_WAIT_TIMEOUT_SECONDS = 8.0
TAKEOVER_GRACE_SECONDS = 2.0
TAKEOVER_FAILURE_BACKOFF_SECONDS = 30.0
TAKEOVER_SUCCESS_COOLDOWN_SECONDS = 15.0
CODEX_PROCESS_NAMES = {"codex.exe"}


def data_root() -> Path:
    return Path.home() / ".codex-session-delete"


def watcher_log_path() -> Path:
    return data_root() / "watcher.log"


def watcher_disabled_flag() -> Path:
    return data_root() / "watcher.disabled"


def log(line: str) -> None:
    path = watcher_log_path()
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open("a", encoding="utf-8") as handle:
        handle.write(f"[{datetime.now().isoformat(timespec='seconds')}] {line}\n")


def cdp_listening(port: int) -> bool:
    try:
        with socket.create_connection(("127.0.0.1", port), timeout=CDP_PROBE_TIMEOUT_SECONDS):
            return True
    except OSError:
        return False


def _run_powershell(script: str, timeout: float = 8.0, env: dict[str, str] | None = None) -> str:
    try:
        result = subprocess.run(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script],
            capture_output=True,
            text=True,
            encoding="utf-8",
            errors="replace",
            timeout=timeout,
            env=env,
            creationflags=getattr(subprocess, "CREATE_NO_WINDOW", 0),
        )
        return result.stdout or ""
    except (OSError, subprocess.SubprocessError) as exc:
        log(f"powershell failed: {exc}")
        return ""


def find_codex_processes() -> list[int]:
    script = (
        "Get-CimInstance Win32_Process -Filter \"Name='Codex.exe' OR Name='codex.exe'\" "
        "| Select-Object ProcessId, ExecutablePath, CommandLine "
        "| ForEach-Object { \"$($_.ProcessId)`t$($_.ExecutablePath)`t$($_.CommandLine)\" }"
    )
    output = _run_powershell(script)
    pids: list[int] = []
    for line in output.splitlines():
        parts = line.split("\t", 2)
        if len(parts) < 2 or not parts[0].strip().isdigit():
            continue
        executable_path = parts[1].lower()
        if "\\windowsapps\\openai.codex_" not in executable_path:
            continue
        pids.append(int(parts[0]))
    return pids


def kill_processes(pids: list[int]) -> None:
    if not pids:
        return
    script = "; ".join(
        f"Stop-Process -Id {pid} -Force -ErrorAction SilentlyContinue" for pid in pids
    )
    _run_powershell(script, timeout=6.0)


def wait_until_no_codex(timeout: float = KILL_WAIT_TIMEOUT_SECONDS) -> bool:
    """Poll until no Codex process is left, or until timeout. Returns True if clean, False if still alive."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        remaining = find_codex_processes()
        if not remaining:
            return True
        # Be aggressive: re-issue kill for anything still alive.
        kill_processes(remaining)
        time.sleep(0.5)
    return not find_codex_processes()


def wait_for_cdp(port: int, timeout: float = CDP_WAIT_TIMEOUT_SECONDS) -> bool:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if cdp_listening(port):
            return True
        time.sleep(0.5)
    return False


def spawn_launcher(debug_port: int) -> subprocess.Popen | None:
    python = sys.executable
    pythonw = Path(python).with_name("pythonw.exe")
    exe = str(pythonw if pythonw.exists() else python)
    args = [exe, "-m", "codex_session_delete", "launch", "--debug-port", str(debug_port)]
    creationflags = 0
    if sys.platform == "win32":
        creationflags = (
            subprocess.CREATE_NEW_PROCESS_GROUP
            | getattr(subprocess, "DETACHED_PROCESS", 0x00000008)
            | getattr(subprocess, "CREATE_NO_WINDOW", 0)
        )
    try:
        return subprocess.Popen(
            args,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
            creationflags=creationflags,
        )
    except Exception as exc:
        log(f"failed to spawn launcher: {exc}")
        return None


def stop_launcher_processes() -> None:
    script = (
        "$self = [int]$env:CODEX_PLUS_PLUS_PID; "
        "$protect = New-Object System.Collections.Generic.HashSet[int]; "
        "$cur = $self; "
        "while ($cur -ne 0 -and $protect.Add($cur)) { "
        "$p = Get-CimInstance Win32_Process -Filter \"ProcessId=$cur\" -ErrorAction SilentlyContinue; "
        "if ($null -eq $p) { break }; $cur = [int]$p.ParentProcessId "
        "} "
        "Get-CimInstance Win32_Process -Filter \"Name='pythonw.exe' OR Name='python.exe'\" | "
        "Where-Object { -not $protect.Contains([int]$_.ProcessId) -and $_.CommandLine -match 'codex_session_delete\\s+launch' } | "
        "ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }"
    )
    env = {**os.environ, "CODEX_PLUS_PLUS_PID": str(os.getpid())}
    _run_powershell(script, timeout=6.0, env=env)


def takeover(debug_port: int) -> bool:
    """Perform one atomic takeover attempt: kill codex cleanly, spawn launcher, wait for CDP.

    Returns True on success (CDP up), False otherwise. On failure, caller should back off briefly.
    """
    if cdp_listening(debug_port):
        log("takeover: CDP became available before kill; skipping takeover")
        return True

    # Step 1: Kill existing launcher processes (stale / failed) so we start from a known state.
    stop_launcher_processes()

    # Step 2: Kill all Codex.exe and wait for them to disappear.
    pids = find_codex_processes()
    log(f"takeover: killing {len(pids)} codex pid(s): {pids}")
    kill_processes(pids)
    if not wait_until_no_codex():
        log("takeover: codex processes did not exit in time, aborting this attempt")
        return False

    # Step 3: Give AppX activation machinery a moment to reset the "app is running" state.
    time.sleep(1.5)

    # Step 4: Spawn a fresh launcher that will activate the packaged app with CDP args.
    proc = spawn_launcher(debug_port)
    if proc is None:
        return False

    # Step 5: Wait for CDP to come up. Launcher does injection in the background.
    if wait_for_cdp(debug_port):
        log(f"takeover: CDP is up on {debug_port} (launcher pid={proc.pid})")
        return True

    # Step 6: CDP did not come up. Clean up the launcher we spawned and any codex it started,
    # so the next pass can retry cleanly instead of staring at a broken window.
    log("takeover: CDP did not come up in time; cleaning up failed attempt")
    stop_launcher_processes()
    stragglers = find_codex_processes()
    if stragglers:
        kill_processes(stragglers)
        wait_until_no_codex(timeout=4.0)
    return False


def watch_loop(debug_port: int = 9229) -> int:
    if sys.platform != "win32":
        log("watcher only supported on Windows")
        return 1

    log(f"watcher started (interval={WATCHER_INTERVAL_SECONDS}s)")
    last_state = None
    backoff_until = 0.0
    cooldown_until = 0.0
    candidate_pids: tuple[int, ...] | None = None
    candidate_since = 0.0

    while True:
        try:
            if watcher_disabled_flag().exists():
                if last_state != "disabled":
                    log("disabled flag present; idling")
                last_state = "disabled"
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            if cdp_listening(debug_port):
                if last_state != "cdp_ok":
                    log("CDP is up")
                last_state = "cdp_ok"
                candidate_pids = None
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            codex_pids = find_codex_processes()
            if not codex_pids:
                if last_state != "idle":
                    log("no Codex running; idling")
                last_state = "idle"
                candidate_pids = None
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            now = time.time()
            if now < cooldown_until:
                if last_state != "cooldown":
                    log(f"in cooldown after takeover; {cooldown_until - now:.1f}s remaining")
                last_state = "cooldown"
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            if now < backoff_until:
                if last_state != "backoff":
                    log(f"in backoff after failed takeover; {backoff_until - now:.1f}s remaining")
                last_state = "backoff"
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            codex_key = tuple(sorted(codex_pids))
            if candidate_pids != codex_key:
                candidate_pids = codex_key
                candidate_since = now
                log(f"Codex running without CDP (pids={codex_pids}); waiting before takeover")
                last_state = "grace"
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            if now - candidate_since < TAKEOVER_GRACE_SECONDS:
                if last_state != "grace":
                    log(f"waiting for Codex CDP grace period (pids={codex_pids})")
                last_state = "grace"
                time.sleep(WATCHER_INTERVAL_SECONDS)
                continue

            if cdp_listening(debug_port):
                candidate_pids = None
                last_state = "cdp_ok"
                continue

            log(f"Codex running without CDP after grace period (pids={codex_pids}); attempting takeover")
            last_state = "takeover"
            success = takeover(debug_port)
            candidate_pids = None
            if success:
                cooldown_until = time.time() + TAKEOVER_SUCCESS_COOLDOWN_SECONDS
                last_state = "cdp_ok"
            else:
                backoff_until = time.time() + TAKEOVER_FAILURE_BACKOFF_SECONDS
                last_state = "failed"
        except Exception as exc:
            log("watch loop error: " + "".join(traceback.format_exception(type(exc), exc, exc.__traceback__)))

        time.sleep(WATCHER_INTERVAL_SECONDS)


def enable_watcher() -> None:
    flag = watcher_disabled_flag()
    if flag.exists():
        flag.unlink()


def disable_watcher() -> None:
    flag = watcher_disabled_flag()
    flag.parent.mkdir(parents=True, exist_ok=True)
    flag.touch()
