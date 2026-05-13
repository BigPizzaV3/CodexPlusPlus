from __future__ import annotations

import json
import os
import time
from pathlib import Path

from codex_session_delete import context_guard


def write_rollout(root: Path, *, old: bool = False, context_error: bool = False, user_text: str = "please continue the task") -> Path:
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
                "content": [{"type": "input_text", "text": user_text}],
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
    if context_error:
        records.append(
            {
                "timestamp": "2026-05-13T00:03:00Z",
                "type": "response_item",
                "payload": {
                    "type": "message",
                    "role": "assistant",
                    "content": "Codex ran out of room in the model's context window. Start a new thread or clear earlier history before retrying.",
                },
            }
        )
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


def test_steward_once_context_error_writes_handoff_and_log(tmp_path, monkeypatch):
    write_rollout(tmp_path, context_error=True)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))
    monkeypatch.setattr(context_guard, "copy_to_clipboard", lambda text: True)

    result = context_guard.steward_once(cwd="C:/tmp/demo", root=tmp_path)

    assert result.status == "handoff_ready"
    assert result.trigger == "context_error"
    assert result.copied is True
    assert Path(result.handoff_path).exists()
    assert Path(result.log_path).exists()
    assert "Working directory" in result.prompt


def test_steward_once_token_pressure_prepares_without_takeover(tmp_path, monkeypatch):
    write_rollout(tmp_path, context_error=False)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))

    result = context_guard.steward_once(cwd="C:/tmp/demo", root=tmp_path)

    assert result.status == "prepared"
    assert result.trigger == "token_pressure"
    assert result.prompt == ""
    assert Path(result.handoff_path).exists()


def test_steward_once_does_not_repeat_same_thread(tmp_path, monkeypatch):
    write_rollout(tmp_path, context_error=True)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))
    monkeypatch.setattr(context_guard, "copy_to_clipboard", lambda text: True)

    first = context_guard.steward_once(cwd="C:/tmp/demo", root=tmp_path)
    second = context_guard.steward_once(cwd="C:/tmp/demo", root=tmp_path)

    assert first.status == "handoff_ready"
    assert second.status == "idle"
    assert second.message == "thread already handled"


def test_steward_once_stops_at_max_handoffs(tmp_path, monkeypatch):
    write_rollout(tmp_path, context_error=True)
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))
    state = context_guard.start_steward("C:/tmp/demo", max_handoffs=1, root=tmp_path)
    state["handoff_count"] = 1
    context_guard.save_steward_state(state, tmp_path)

    result = context_guard.steward_once(cwd="C:/tmp/demo", max_handoffs=1, root=tmp_path)

    assert result.status == "stopped"
    assert result.stop_reason == "max handoffs reached"
    assert Path(result.summary_path).exists()


def test_steward_once_stops_on_high_risk_action(tmp_path, monkeypatch):
    write_rollout(tmp_path, context_error=True, user_text="please git push and create PR tonight")
    monkeypatch.setenv("CODEX_HOME", str(tmp_path))

    result = context_guard.steward_once(cwd="C:/tmp/demo", root=tmp_path)

    assert result.status == "stopped"
    assert result.stop_reason == "high-risk action detected"
    assert Path(result.summary_path).exists()
