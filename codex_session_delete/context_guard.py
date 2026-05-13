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
DEFAULT_STEWARD_INTERVAL_SECONDS = 15.0
DEFAULT_MAX_HANDOFFS = 2
CONTEXT_ERROR_MARKERS = (
    "ran out of room in the model's context window",
    "start a new thread or clear earlier history",
)
HIGH_RISK_MARKERS = (
    "git push",
    "gh pr create",
    "gh pr merge",
    "submit pr",
    "create pr",
    "publish",
    "release",
    "deploy",
    "delete file",
    "delete data",
    "remove file",
    "remove data",
    "rm -rf",
    "stop-process",
    "set-itemproperty",
    "scheduledtask",
    "提交pr",
    "创建pr",
    "合并pr",
    "发布",
    "部署",
    "删除",
    "卸载",
    "系统配置",
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


@dataclass
class StewardResult:
    status: str
    message: str
    handoff_path: str = ""
    prompt: str = ""
    copied: bool = False
    trigger: str = ""
    stop_reason: str = ""
    log_path: str = ""
    summary_path: str = ""
    thread_id: str = ""
    cwd: str = ""

    def to_dict(self) -> dict[str, Any]:
        return {
            "status": self.status,
            "message": self.message,
            "handoff_path": self.handoff_path,
            "path": self.handoff_path,
            "prompt": self.prompt,
            "copied": self.copied,
            "trigger": self.trigger,
            "stop_reason": self.stop_reason,
            "log_path": self.log_path,
            "summary_path": self.summary_path,
            "thread_id": self.thread_id,
            "cwd": self.cwd,
        }


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


def takeover_prompt(path: Path, report: HandoffReport) -> str:
    cwd = report.stats.cwd or "(unknown)"
    return (
        "Continue the same task from this handoff in the same working folder.\n\n"
        f"Handoff file:\n{path}\n"
        f"Working directory:\n{cwd}\n\n"
        "First read the handoff file and inspect only the files or commands needed for the next step. "
        "Confirm the current working directory and git status before changing files. Continue the original task only; "
        "do not expand scope. Stop and report if the next action would delete data, publish/deploy, submit or merge a PR, "
        "change system configuration, install a global service, or otherwise require user approval."
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


def handoff_result(target: str | None, *, copy: bool = False, messages: int = DEFAULT_MESSAGES, message_chars: int = DEFAULT_MESSAGE_CHARS) -> StewardResult:
    report = collect_handoff_report(resolve_rollout(target), messages=messages, message_chars=message_chars)
    output = write_handoff(report)
    prompt = takeover_prompt(output, report)
    copied = copy_to_clipboard(prompt) if copy else False
    return StewardResult(
        status="ok",
        message="handoff written",
        handoff_path=str(output),
        prompt=prompt,
        copied=copied,
        trigger="manual",
        thread_id=report.stats.thread_id,
        cwd=report.stats.cwd,
    )


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


def context_guard_root(root: Path | None = None) -> Path:
    return (root or codex_home()) / "context-guard"


def steward_state_path(root: Path | None = None) -> Path:
    return context_guard_root(root) / "steward-state.json"


def overnight_runs_dir(root: Path | None = None) -> Path:
    return context_guard_root(root) / "overnight-runs"


def now_iso() -> str:
    return datetime.now().isoformat(timespec="seconds")


def default_steward_state(cwd: str = "", max_handoffs: int = DEFAULT_MAX_HANDOFFS) -> dict[str, Any]:
    stamp = now_iso()
    return {
        "enabled": False,
        "cwd": cwd,
        "source_thread_id": "",
        "handled_thread_ids": [],
        "handoff_count": 0,
        "max_handoffs": max_handoffs,
        "last_handoff_path": "",
        "last_error": "",
        "stop_reason": "",
        "log_path": "",
        "summary_path": "",
        "started_at": stamp,
        "updated_at": stamp,
    }


def load_steward_state(root: Path | None = None) -> dict[str, Any]:
    path = steward_state_path(root)
    if not path.exists():
        return default_steward_state()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError):
        return default_steward_state()
    state = default_steward_state(str(data.get("cwd") or ""), int(data.get("max_handoffs") or DEFAULT_MAX_HANDOFFS))
    state.update(data)
    if not isinstance(state.get("handled_thread_ids"), list):
        state["handled_thread_ids"] = []
    return state


def save_steward_state(state: dict[str, Any], root: Path | None = None) -> Path:
    state["updated_at"] = now_iso()
    path = steward_state_path(root)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(state, ensure_ascii=False, indent=2), encoding="utf-8")
    return path


def start_steward(cwd: str, *, max_handoffs: int = DEFAULT_MAX_HANDOFFS, root: Path | None = None) -> dict[str, Any]:
    state = default_steward_state(cwd, max_handoffs)
    state["enabled"] = True
    save_steward_state(state, root)
    append_steward_log(state, "steward started", root)
    return state


def stop_steward(reason: str, *, root: Path | None = None) -> dict[str, Any]:
    state = load_steward_state(root)
    state["enabled"] = False
    state["stop_reason"] = reason
    append_steward_log(state, f"steward stopped: {reason}", root)
    summary = write_steward_summary(state, reason, root)
    state["summary_path"] = str(summary)
    save_steward_state(state, root)
    return state


def append_steward_log(state: dict[str, Any], line: str, root: Path | None = None) -> Path:
    raw_log_path = str(state.get("log_path") or "")
    if raw_log_path:
        log_path = Path(raw_log_path)
    else:
        stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
        log_path = overnight_runs_dir(root) / f"{stamp}-steward.log"
        state["log_path"] = str(log_path)
    log_path.parent.mkdir(parents=True, exist_ok=True)
    with log_path.open("a", encoding="utf-8") as handle:
        handle.write(f"[{now_iso()}] {line}\n")
    save_steward_state(state, root)
    return log_path


def write_steward_summary(state: dict[str, Any], reason: str, root: Path | None = None) -> Path:
    stamp = datetime.now().strftime("%Y%m%d-%H%M%S")
    summary_path = overnight_runs_dir(root) / f"{stamp}-steward-summary.md"
    summary_path.parent.mkdir(parents=True, exist_ok=True)
    lines = [
        "# Codex Context Guard Steward Summary",
        "",
        f"- status: `stopped`",
        f"- reason: `{reason}`",
        f"- cwd: `{state.get('cwd') or ''}`",
        f"- source_thread_id: `{state.get('source_thread_id') or ''}`",
        f"- handoff_count: `{state.get('handoff_count') or 0}`",
        f"- last_handoff_path: `{state.get('last_handoff_path') or ''}`",
        f"- last_error: `{state.get('last_error') or ''}`",
        f"- log_path: `{state.get('log_path') or ''}`",
        "",
        "Review the handoff and log before continuing unattended work.",
        "",
    ]
    summary_path.write_text("\n".join(lines), encoding="utf-8")
    return summary_path


def normalize_cwd(value: str | Path) -> str:
    return str(Path(value).expanduser().resolve())


def report_matches_cwd(report: HandoffReport, cwd: str) -> bool:
    if not cwd:
        return True
    if not report.stats.cwd:
        return False
    try:
        return Path(report.stats.cwd).expanduser().resolve() == Path(cwd).expanduser().resolve()
    except OSError:
        return report.stats.cwd == cwd


def find_recent_report_for_cwd(cwd: str, *, recent: int = DEFAULT_RECENT, root: Path | None = None) -> HandoffReport | None:
    for path in recent_rollouts(recent, root):
        report = collect_handoff_report(path)
        if report_matches_cwd(report, cwd):
            return report
    return None


def infer_recent_cwd(*, root: Path | None = None) -> str:
    for path in recent_rollouts(10, root):
        report = collect_handoff_report(path)
        if report.stats.cwd:
            return normalize_cwd(report.stats.cwd)
    return ""


def report_contains_high_risk_action(report: HandoffReport) -> bool:
    text = "\n".join(message.text for message in report.messages).lower()
    return any(marker in text for marker in HIGH_RISK_MARKERS)


def high_risk_marker(report: HandoffReport) -> str:
    user_messages = [message.text for message in report.messages if message.role == "user"]
    text = (user_messages[-1] if user_messages else "").lower()
    for marker in HIGH_RISK_MARKERS:
        if marker in text:
            return marker
    return ""


def steward_once(
    *,
    cwd: str,
    max_handoffs: int = DEFAULT_MAX_HANDOFFS,
    recent: int = DEFAULT_RECENT,
    copy: bool = True,
    root: Path | None = None,
) -> StewardResult:
    resolved_cwd = normalize_cwd(cwd) if cwd else infer_recent_cwd(root=root)
    if not resolved_cwd:
        return StewardResult(status="failed", message="cwd is required and no recent rollout cwd was found")
    state = load_steward_state(root)
    if not state.get("enabled"):
        state = start_steward(resolved_cwd, max_handoffs=max_handoffs, root=root)
    else:
        state["cwd"] = resolved_cwd
        state["max_handoffs"] = max_handoffs
        save_steward_state(state, root)

    report = find_recent_report_for_cwd(resolved_cwd, recent=recent, root=root)
    if report is None:
        append_steward_log(state, f"no rollout found for cwd: {resolved_cwd}", root)
        return StewardResult(status="idle", message="no rollout found for cwd", log_path=str(state.get("log_path") or ""), cwd=resolved_cwd)

    state["source_thread_id"] = state.get("source_thread_id") or report.stats.thread_id
    thread_id = report.stats.thread_id or str(report.stats.path)
    marker = high_risk_marker(report)
    if marker:
        state["source_thread_id"] = thread_id
        state["last_error"] = f"high-risk action detected: {marker}"
        save_steward_state(state, root)
        stopped = stop_steward("high-risk action detected", root=root)
        return StewardResult(
            status="stopped",
            message="high-risk action detected; steward stopped",
            stop_reason=str(stopped.get("stop_reason") or ""),
            log_path=str(stopped.get("log_path") or ""),
            summary_path=str(stopped.get("summary_path") or ""),
            thread_id=thread_id,
            cwd=resolved_cwd,
        )

    if thread_id in set(str(item) for item in state.get("handled_thread_ids", [])):
        append_steward_log(state, f"thread already handled: {thread_id}", root)
        return StewardResult(status="idle", message="thread already handled", log_path=str(state.get("log_path") or ""), thread_id=thread_id, cwd=resolved_cwd)

    if int(state.get("handoff_count") or 0) >= max_handoffs:
        stopped = stop_steward("max handoffs reached", root=root)
        return StewardResult(
            status="stopped",
            message="max handoffs reached; steward stopped",
            stop_reason=str(stopped.get("stop_reason") or ""),
            log_path=str(stopped.get("log_path") or ""),
            summary_path=str(stopped.get("summary_path") or ""),
            thread_id=thread_id,
            cwd=resolved_cwd,
        )

    if not report.context_error:
        if token_pressure(report.last_token_count):
            output = write_handoff(report)
            state["last_handoff_path"] = str(output)
            append_steward_log(state, f"token pressure observed; prewrote handoff without takeover: {output}", root)
            return StewardResult(
                status="prepared",
                message="token pressure observed; handoff prepared without takeover",
                handoff_path=str(output),
                trigger="token_pressure",
                log_path=str(state.get("log_path") or ""),
                thread_id=thread_id,
                cwd=resolved_cwd,
            )
        append_steward_log(state, f"no context error for thread: {thread_id}", root)
        return StewardResult(status="idle", message="no context error detected", log_path=str(state.get("log_path") or ""), thread_id=thread_id, cwd=resolved_cwd)

    output = write_handoff(report)
    prompt = takeover_prompt(output, report)
    copied = copy_to_clipboard(prompt) if copy else False
    handled = [str(item) for item in state.get("handled_thread_ids", [])]
    handled.append(thread_id)
    state["handled_thread_ids"] = handled
    state["handoff_count"] = int(state.get("handoff_count") or 0) + 1
    state["last_handoff_path"] = str(output)
    state["last_error"] = ""
    append_steward_log(state, f"context error handoff ready: thread={thread_id} path={output}", root)
    save_steward_state(state, root)
    return StewardResult(
        status="handoff_ready",
        message="context error detected; handoff ready for takeover",
        handoff_path=str(output),
        prompt=prompt,
        copied=copied,
        trigger="context_error",
        log_path=str(state.get("log_path") or ""),
        thread_id=thread_id,
        cwd=resolved_cwd,
    )


def steward_loop(
    *,
    cwd: str,
    max_handoffs: int = DEFAULT_MAX_HANDOFFS,
    interval_seconds: float = DEFAULT_STEWARD_INTERVAL_SECONDS,
    root: Path | None = None,
) -> int:
    resolved_cwd = normalize_cwd(cwd) if cwd else infer_recent_cwd(root=root)
    if not resolved_cwd:
        raise ValueError("cwd is required and no recent rollout cwd was found")
    start_steward(resolved_cwd, max_handoffs=max_handoffs, root=root)
    while load_steward_state(root).get("enabled"):
        result = steward_once(cwd=resolved_cwd, max_handoffs=max_handoffs, root=root)
        if result.status == "stopped":
            return 0
        time.sleep(interval_seconds)
    return 0
