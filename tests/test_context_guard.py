from __future__ import annotations

import json
import os
import time
from pathlib import Path

from codex_session_delete import context_guard


def write_rollout(root: Path, *, old: bool = False) -> Path:
    rollout_dir = root / "sessions" / "2026" / "05" / "13"
    rollout_dir.mkdir(parents=True, exist_ok=True)
    path = rollout_dir / "rollout-2026-05-13T00-00-00-019e1f97-6ea9-7181-ba90-59fb48309277.jsonl"
    records = [
        {
            "timestamp": "2026-05-13T00:00:00Z",
            "type": "session_meta",
            "payload": {
                "id": "019e1f97-6ea9-7181-ba90-59fb48309277",
                "cwd": "C:/tmp/demo",
            },
        },
        {"timestamp": "2026-05-13T00:01:00Z", "type": "turn_context", "payload": {"turn_id": "turn-1"}},
        {
            "timestamp": "2026-05-13T00:01:01Z",
            "type": "response_item",
            "payload": {
                "type": "message",
                "role": "user",
                "content": [{"type": "input_text", "text": "please continue the task"}],
            },
        },
        {
            "timestamp": "2026-05-13T00:02:00Z",
            "type": "event_msg",
            "payload": {
                "type": "token_count",
                "info": {
                    "model_context_window": 1000,
                    "last_token_usage": {"input_tokens": 900, "total_tokens": 900},
                    "total_token_usage": {"total_tokens": 1000},
                },
            },
        },
    ]
    path.write_text("".join(json.dumps(item) + "\n" for item in records), encoding="utf-8")
    if old:
        old_time = time.time() - 7200
        os.utime(path, (old_time, old_time))
    return path


def test_collect_handoff_report_reads_messages_and_token_pressure(tmp_path):
    path = write_rollout(tmp_path)
    report = context_guard.collect_handoff_report(path)

    assert report.stats.thread_id == "019e1f97-6ea9-7181-ba90-59fb48309277"
    assert report.stats.cwd == "C:/tmp/demo"
    assert report.messages[-1].text == "please continue the task"
    assert context_guard.token_pressure(report.last_token_count) is True


def test_write_handoff_contains_new_thread_instruction(tmp_path, monkeypatch):
    path = write_rollout(tmp_path)
    report = context_guard.collect_handoff_report(path)
    output = context_guard.write_handoff(report, tmp_path / "handoff.md")

    content = output.read_text(encoding="utf-8")
    assert "# Codex Thread Handoff" in content
    assert "Continue from this handoff" in content


def test_watch_once_writes_handoff_for_recent_full_context(tmp_path, monkeypatch):
    write_rollout(tmp_path)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))

    outputs = context_guard.watch_once(recent=5, max_age_seconds=1800, copy=False)

    assert len(outputs) == 1
    assert outputs[0].exists()


def test_watch_once_skips_old_rollout(tmp_path, monkeypatch):
    write_rollout(tmp_path, old=True)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))

    outputs = context_guard.watch_once(recent=5, max_age_seconds=1800, copy=False)

    assert outputs == []


def test_clipboard_prompt_mentions_handoff_file():
    path = Path("C:/tmp/handoff.md")
    text = context_guard.clipboard_prompt(path)

    assert "Handoff file" in text
    assert str(path) in text
