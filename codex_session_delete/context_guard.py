from __future__ import annotations

import json
import os
import re
import subprocess
import time
from dataclasses import dataclass, field
from datetime import datetime
from pathlib import Path
from typing import Any, Iterable


DEFAULT_MESSAGES = 14
DEFAULT_MESSAGE_CHARS = 1200
DEFAULT_TOKEN_RATIO = 0.72
DEFAULT_MAX_AGE_SECONDS = 30 * 60
DEFAULT_RECENT = 20
CONTEXT_ERROR_MARKERS = (
    "ran out of room in the model's context window",
    "start a new thread or clear earlier history",
)


@dataclass
class RolloutStats:
    path: Path
    size: int
    thread_id: str = ""
    cwd: str = ""
    lines: int = 0
    turns: int = 0
    large_lines: list[tuple[int, int, str]] = field(default_factory=list)


@dataclass
class HandoffMessage:
    line_no: int
    role: str
    text: str


@dataclass
class HandoffReport:
    stats: RolloutStats
    messages: list[HandoffMessage]
    last_token_count: dict[str, Any] | None
    context_error: bool


def codex_home() -> Path:
    return Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex").expanduser().resolve()


def iter_rollouts(root: Path | None = None) -> Iterable[Path]:
    sessions = (root or codex_home()) / "sessions"
    if sessions.exists():
        yield from sessions.rglob("*.jsonl")


def recent_rollouts(limit: int, root: Path | None = None) -> list[Path]:
    paths = list(iter_rollouts(root))
    paths.sort(key=lambda path: path.stat().st_mtime if path.exists() else 0, reverse=True)
    return paths[:limit]


def resolve_rollout(target: str | None, root: Path | None = None) -> Path:
    if not target:
        rollouts = recent_rollouts(1, root)
        if not rollouts:
            raise FileNotFoundError("No Codex rollout files found")
        return rollouts[0]

    candidate = Path(target).expanduser()
    if candidate.exists():
        return candidate.resolve()

    matches: list[Path] = []
    for path in iter_rollouts(root):
        if target.lower() in path.name.lower():
            matches.append(path)
            continue
        try:
            first = path.open("r", encoding="utf-8", errors="replace").readline()
        except OSError:
            continue
        if target in first:
            matches.append(path)

    if not matches:
        raise FileNotFoundError(f"No rollout matched target: {target}")
    if len(matches) > 1:
        raise ValueError("Multiple rollouts matched; use a longer thread id or file path")
    return matches[0].resolve()


def event_type(obj: Any) -> str:
    return str(obj.get("type", "")) if isinstance(obj, dict) else ""


def event_role(obj: Any) -> str:
    payload = obj.get("payload", {}) if isinstance(obj, dict) else {}
    return str(payload.get("role") or payload.get("type") or "") if isinstance(payload, dict) else ""


def normalize_text(text: str, limit: int) -> str:
    text = "\n".join(part.rstrip() for part in text.replace("\r\n", "\n").replace("\r", "\n").split("\n")).strip()
    if len(text) > limit:
        return text[:limit].rstrip() + "\n...[truncated]"
    return text


def extract_message_text(payload: dict[str, Any], max_chars: int) -> str:
    content = payload.get("content")
    parts: list[str] = []
    if isinstance(content, str):
        parts.append(content)
    elif isinstance(content, list):
        for item in content:
            if not isinstance(item, dict):
                continue
            if isinstance(item.get("text"), str):
                parts.append(item["text"])
            elif item.get("type") in {"input_image", "local_image"}:
                parts.append(f"[{item.get('type')} omitted]")
    return normalize_text("\n".join(parts), max_chars)


def analyze_rollout(path: Path, large_line_bytes: int = 64 * 1024) -> RolloutStats:
    stats = RolloutStats(path=path, size=path.stat().st_size)
    filename_ids = re.findall(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", path.name, re.IGNORECASE)
    if filename_ids:
        stats.thread_id = filename_ids[-1]
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        for line_no, line in enumerate(handle, start=1):
            stats.lines += 1
            raw_bytes = len(line.encode("utf-8", errors="replace"))
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                if raw_bytes >= large_line_bytes:
                    stats.large_lines.append((line_no, raw_bytes, "parse_error"))
                continue
            typ = event_type(obj)
            if typ == "session_meta":
                payload = obj.get("payload", {})
                if isinstance(payload, dict):
                    stats.thread_id = stats.thread_id or str(payload.get("id") or "")
                    stats.cwd = str(payload.get("cwd") or "")
            elif typ == "turn_context":
                stats.turns += 1
            if raw_bytes >= large_line_bytes:
                stats.large_lines.append((line_no, raw_bytes, f"{typ}/{event_role(obj)}".rstrip("/")))
    return stats


def collect_handoff_report(path: Path, messages: int = DEFAULT_MESSAGES, message_chars: int = DEFAULT_MESSAGE_CHARS) -> HandoffReport:
    stats = analyze_rollout(path)
    found_messages: list[HandoffMessage] = []
    last_token_count: dict[str, Any] | None = None
    context_error = False
    with path.open("r", encoding="utf-8", errors="replace") as handle:
        for line_no, line in enumerate(handle, start=1):
            lower = line.lower()
            if any(marker in lower for marker in CONTEXT_ERROR_MARKERS):
                context_error = True
            try:
                obj = json.loads(line)
            except json.JSONDecodeError:
                continue
            payload = obj.get("payload", {}) if isinstance(obj, dict) else {}
            if event_type(obj) == "response_item" and isinstance(payload, dict):
                if payload.get("type") == "message" and payload.get("role") in {"user", "assistant"}:
                    text = extract_message_text(payload, message_chars)
                    if text:
                        found_messages.append(HandoffMessage(line_no, str(payload.get("role")), text))
            if event_type(obj) == "event_msg" and isinstance(payload, dict) and payload.get("type") == "token_count":
                info = payload.get("info")
                if isinstance(info, dict):
                    last_token_count = info
    return HandoffReport(stats, found_messages[-messages:] if messages > 0 else found_messages, last_token_count, context_error)


def token_pressure(token_count: dict[str, Any] | None, ratio: float = DEFAULT_TOKEN_RATIO) -> bool:
    if not token_count:
        return False
    window = token_count.get("model_context_window")
    last = token_count.get("last_token_usage")
    total = token_count.get("total_token_usage")
    try:
        if window is not None and isinstance(last, dict) and int(last.get("input_tokens", 0)) >= int(window) * ratio:
            return True
        if window is not None and isinstance(total, dict) and int(total.get("total_tokens", 0)) >= int(window):
            return True
    except (TypeError, ValueError):
        return False
    return False


def handoff_output_path(report: HandoffReport, root: Path | None = None) -> Path:
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    thread = re.sub(r"[^A-Za-z0-9_.-]+", "-", report.stats.thread_id or "unknown-thread").strip("-")
    return (root or codex_home()) / "handoffs" / f"{stamp}-{thread}-handoff.md"


def render_handoff(report: HandoffReport) -> str:
    stats = report.stats
    lines = [
        "# Codex Thread Handoff",
        "",
        "Use this in a new Codex thread. Keep the old thread as reference only; do not paste full logs or base64 data.",
        "",
        "## Source",
        "",
        f"- thread_id: `{stats.thread_id or '(unknown)'}`",
        f"- rollout: `{stats.path}`",
        f"- cwd: `{stats.cwd or '(unknown)'}`",
        f"- size_bytes: `{stats.size}`",
        f"- lines: `{stats.lines}`",
        f"- turns: `{stats.turns}`",
        f"- large_lines: `{len(stats.large_lines)}`",
        f"- context_error_seen: `{report.context_error}`",
        f"- token_pressure_seen: `{token_pressure(report.last_token_count)}`",
        "",
        "## Recent Conversation",
        "",
    ]
    if report.messages:
        for message in report.messages:
            lines.extend([f"### {message.role} line {message.line_no}", "", "```text", message.text, "```", ""])
    else:
        lines.append("No recent user/assistant text messages were found.")
    lines.extend(
        [
            "## New Thread Instruction",
            "",
            "Continue from this handoff. Read only the files and commands needed for the next step. Do not load the old rollout or full logs unless explicitly requested.",
            "",
        ]
    )
    return "\n".join(lines)


def write_handoff(report: HandoffReport, output: Path | None = None) -> Path:
    output_path = output or handoff_output_path(report)
    output_path.parent.mkdir(parents=True, exist_ok=True)
    output_path.write_text(render_handoff(report), encoding="utf-8")
    return output_path


def clipboard_prompt(path: Path) -> str:
    return (
        "Please continue the task from this handoff. Read only what is needed for the next step, "
        "and do not load the old rollout or full logs unless I explicitly ask.\n\n"
        f"Handoff file:\n{path}\n"
    )


def copy_to_clipboard(text: str) -> bool:
    if sys_platform() != "win32":
        return False
    try:
        completed = subprocess.run(
            ["powershell.exe", "-NoProfile", "-Command", "Set-Clipboard -Value ([Console]::In.ReadToEnd())"],
            input=text,
            text=True,
            encoding="utf-8",
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )
    except OSError:
        return False
    return completed.returncode == 0


def sys_platform() -> str:
    import sys

    return sys.platform


def handoff(target: str | None, *, copy: bool = False, messages: int = DEFAULT_MESSAGES, message_chars: int = DEFAULT_MESSAGE_CHARS) -> Path:
    report = collect_handoff_report(resolve_rollout(target), messages=messages, message_chars=message_chars)
    output = write_handoff(report)
    if copy:
        copy_to_clipboard(clipboard_prompt(output))
    return output


def watch_once(*, recent: int = DEFAULT_RECENT, max_age_seconds: int = DEFAULT_MAX_AGE_SECONDS, token_ratio: float = DEFAULT_TOKEN_RATIO, copy: bool = False) -> list[Path]:
    now = time.time()
    outputs: list[Path] = []
    for path in recent_rollouts(recent):
        stat = path.stat()
        if max_age_seconds > 0 and now - stat.st_mtime > max_age_seconds:
            continue
        report = collect_handoff_report(path)
        if not report.context_error and not token_pressure(report.last_token_count, token_ratio):
            continue
        output = write_handoff(report)
        outputs.append(output)
        if copy:
            copy_to_clipboard(clipboard_prompt(output))
    return outputs
