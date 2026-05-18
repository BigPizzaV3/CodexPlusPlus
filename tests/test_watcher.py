from __future__ import annotations

import socket
import sys
import types
from pathlib import Path

import pytest

from codex_session_delete import watcher


def test_cdp_listening_returns_true_when_bound():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.bind(("127.0.0.1", 0))
        server.listen(1)
        port = server.getsockname()[1]
        assert watcher.cdp_listening(port) is True


def test_cdp_listening_returns_false_when_closed():
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as probe:
        probe.bind(("127.0.0.1", 0))
        port = probe.getsockname()[1]
    # After the probe socket closes, nothing should be listening on that port
    # (the port may get reused but the probe finishes with connection refused in normal conditions)
    assert watcher.cdp_listening(port) is False


def test_enable_watcher_removes_flag(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    flag = tmp_path / "watcher.disabled"
    flag.parent.mkdir(parents=True, exist_ok=True)
    flag.touch()
    assert flag.exists()
    watcher.enable_watcher()
    assert not flag.exists()


def test_disable_watcher_creates_flag(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    flag = tmp_path / "watcher.disabled"
    assert not flag.exists()
    watcher.disable_watcher()
    assert flag.exists()


def test_enable_watcher_is_idempotent(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    # Should not raise when flag does not exist
    watcher.enable_watcher()
    assert not (tmp_path / "watcher.disabled").exists()


def test_watch_loop_exits_on_non_windows(monkeypatch, tmp_path):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    monkeypatch.setattr(watcher.sys, "platform", "linux")
    assert watcher.watch_loop() == 1


def test_wait_until_no_codex_success(monkeypatch):
    calls = {"n": 0}

    def find() -> list[int]:
        calls["n"] += 1
        # First poll: one process, subsequent polls: empty
        return [1234] if calls["n"] == 1 else []

    monkeypatch.setattr(watcher, "find_codex_processes", find)
    killed: list[list[int]] = []
    monkeypatch.setattr(watcher, "kill_processes", lambda pids: killed.append(list(pids)))
    assert watcher.wait_until_no_codex(timeout=2.0) is True


def test_wait_until_no_codex_times_out(monkeypatch):
    monkeypatch.setattr(watcher, "find_codex_processes", lambda: [1])
    monkeypatch.setattr(watcher, "kill_processes", lambda pids: None)
    assert watcher.wait_until_no_codex(timeout=0.5) is False


def test_wait_for_cdp_returns_true_when_listening(monkeypatch):
    seq = iter([False, False, True])
    monkeypatch.setattr(watcher, "cdp_listening", lambda port: next(seq))
    assert watcher.wait_for_cdp(port=9229, timeout=2.0) is True


def test_wait_for_cdp_returns_false_on_timeout(monkeypatch):
    monkeypatch.setattr(watcher, "cdp_listening", lambda port: False)
    assert watcher.wait_for_cdp(port=9229, timeout=0.3) is False


def test_spawn_launcher_passes_debug_port(monkeypatch):
    calls = []

    class FakePopen:
        pass

    monkeypatch.setattr(watcher.subprocess, "Popen", lambda args, **kwargs: calls.append((args, kwargs)) or FakePopen())

    assert watcher.spawn_launcher(debug_port=9333) is not None

    args, _ = calls[0]
    assert "--debug-port" in args
    assert "9333" in args


def test_takeover_skips_kill_when_cdp_appears(monkeypatch):
    killed = []
    stopped = []
    monkeypatch.setattr(watcher, "cdp_listening", lambda port: True)
    monkeypatch.setattr(watcher, "stop_launcher_processes", lambda: stopped.append(True))
    monkeypatch.setattr(watcher, "kill_processes", lambda pids: killed.append(pids))

    assert watcher.takeover(debug_port=9229) is True
    assert stopped == []
    assert killed == []


def test_find_codex_processes_excludes_cli_commands(monkeypatch):
    output = "\n".join(
        [
            "101\tC:\\Program Files\\WindowsApps\\OpenAI.Codex_1.2.3\\Codex.exe\t\"C:\\Program Files\\WindowsApps\\OpenAI.Codex_1.2.3\\Codex.exe\"",
            "202\tC:\\Users\\User\\AppData\\Roaming\\npm\\codex.exe\t\"C:\\Users\\User\\AppData\\Roaming\\npm\\codex.exe\"",
            "303\tC:\\Users\\User\\AppData\\Local\\Programs\\codex\\codex.exe\t\"C:\\Users\\User\\AppData\\Local\\Programs\\codex\\codex.exe\" --model opus",
        ]
    )
    monkeypatch.setattr(watcher, "_run_powershell", lambda script: output)

    assert watcher.find_codex_processes() == [101]


def test_takeover_failure_backoff_is_not_too_short():
    assert watcher.TAKEOVER_FAILURE_BACKOFF_SECONDS >= 30.0


def test_stop_launcher_processes_protects_current_process_ancestry(monkeypatch):
    commands = []
    monkeypatch.setattr(watcher.os, "getpid", lambda: 2468)
    monkeypatch.setattr(watcher, "_run_powershell", lambda script, timeout=6.0, env=None: commands.append((script, timeout, env)) or "")

    watcher.stop_launcher_processes()

    assert len(commands) == 1
    script, timeout, env = commands[0]
    assert timeout == 6.0
    assert env["CODEX_PLUS_PLUS_PID"] == "2468"
    assert "CODEX_PLUS_PLUS_PID" in script
    assert "$protect.Contains([int]$_.ProcessId)" in script
    assert "ParentProcessId" in script
    assert "codex_session_delete\\s+launch" in script
