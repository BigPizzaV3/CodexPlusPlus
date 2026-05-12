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


def test_takeover_failure_backoff_is_not_too_short():
    assert watcher.TAKEOVER_FAILURE_BACKOFF_SECONDS >= 30.0


def test_log_does_not_raise_when_path_unwritable(tmp_path, monkeypatch):
    unwritable = tmp_path / "no-such-dir" / "nested"
    monkeypatch.setattr(watcher, "watcher_log_path", lambda: unwritable / "watcher.log")
    monkeypatch.setattr(watcher, "data_root", lambda: unwritable)
    # Make parent read-only on Windows by pointing to a file instead of dir
    (tmp_path / "no-such-dir").write_text("block", encoding="utf-8")
    # Should not raise
    watcher.log("test message")


def test_pid_file_written_and_singleton_acquired(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    assert watcher._acquire_singleton() is True
    pid_path = tmp_path / "watcher.pid"
    assert pid_path.exists()
    import os
    assert pid_path.read_text(encoding="utf-8").strip() == str(os.getpid())
    watcher._release_singleton()
    assert not pid_path.exists()


def test_singleton_rejects_duplicate_when_alive(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    import os
    real_pid = os.getpid()
    pid_path = tmp_path / "watcher.pid"
    pid_path.write_text(str(real_pid), encoding="utf-8")
    monkeypatch.setattr(watcher, "_is_process_alive", lambda pid: True)
    monkeypatch.setattr(watcher.os, "getpid", lambda: real_pid + 1)
    assert watcher._acquire_singleton() is False


def test_singleton_allows_stale_pid(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    pid_path = tmp_path / "watcher.pid"
    pid_path.write_text("99999", encoding="utf-8")
    monkeypatch.setattr(watcher, "_is_process_alive", lambda pid: False)
    assert watcher._acquire_singleton() is True


def test_heartbeat_file_updated(tmp_path, monkeypatch):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    hb = tmp_path / "watcher.heartbeat"
    assert not hb.exists()
    watcher._touch_heartbeat()
    assert hb.exists()
    content = hb.read_text(encoding="utf-8")
    assert float(content) > 0


def test_run_with_restart_retries_on_crash(monkeypatch, tmp_path):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    attempts = []

    def crashing_loop(debug_port=9229):
        attempts.append(1)
        if len(attempts) < 3:
            raise RuntimeError("simulated crash")
        return 0

    monkeypatch.setattr(watcher, "watch_loop", crashing_loop)
    monkeypatch.setattr(watcher.time, "sleep", lambda s: None)

    result = watcher.run_with_restart(debug_port=9229)

    assert result == 0
    assert len(attempts) == 3


def test_run_with_restart_gives_up_after_max_attempts(monkeypatch, tmp_path):
    monkeypatch.setattr(watcher, "data_root", lambda: tmp_path)
    attempts = []

    def always_crash(debug_port=9229):
        attempts.append(1)
        raise RuntimeError("always crash")

    monkeypatch.setattr(watcher, "watch_loop", always_crash)
    monkeypatch.setattr(watcher.time, "sleep", lambda s: None)

    result = watcher.run_with_restart(debug_port=9229)

    assert result == 1
    assert len(attempts) == watcher.RESTART_MAX_ATTEMPTS
