# Provider Sync Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add an optional default-off Codex++ provider sync that runs before Codex launches and keeps historical conversations visible after `model_provider` changes.

**Architecture:** Add backend settings in `~/.codex-session-delete/settings.json`, expose those settings through the existing CDP bridge/menu, and implement provider sync as a focused Python module called by `launcher.launch_and_inject()` before `launch_codex_app()`. The sync module reads `~/.codex/config.toml`, backs up writable state, locks with `~/.codex/tmp/provider-sync.lock`, updates rollout first-line metadata and SQLite provider metadata, logs skippable lock/busy conditions, and only blocks launch on unsafe write/restore failures.

**Tech Stack:** Python 3.11 stdlib (`json`, `sqlite3`, `shutil`, `pathlib`, `dataclasses`, `time`), existing renderer injection JavaScript, pytest.

---

## File Structure

- Create `codex_session_delete/settings_store.py`: backend-readable Codex++ settings store with defaults and atomic writes.
- Create `codex_session_delete/provider_sync.py`: provider sync implementation and result model.
- Modify `codex_session_delete/launcher.py`: load settings, expose bridge endpoints, call provider sync before launch, and log skipped/fatal sync results.
- Modify `codex_session_delete/inject/renderer-inject.js`: add backend-backed Provider sync toggle to Codex++ menu.
- Modify `tests/test_launcher_user_scripts.py`: bridge settings endpoint tests.
- Modify `tests/test_launcher_cli.py`: launch pre-sync tests.
- Create `tests/test_settings_store.py`: backend settings tests.
- Create `tests/test_provider_sync.py`: provider sync unit tests.
- Modify `tests/test_renderer_script.py`: renderer contract for Provider sync menu UI and backend endpoints.

---

### Task 1: Backend Settings Store

**Files:**
- Create: `codex_session_delete/settings_store.py`
- Test: `tests/test_settings_store.py`

- [ ] **Step 1: Write the failing settings tests**

Create `tests/test_settings_store.py` with:

```python
import json

from codex_session_delete.settings_store import BackendSettings, SettingsStore


def test_settings_store_defaults_provider_sync_disabled(tmp_path):
    store = SettingsStore(tmp_path / "settings.json")

    settings = store.load()

    assert settings == BackendSettings(provider_sync_enabled=False)
    assert settings.to_dict() == {"providerSyncEnabled": False}


def test_settings_store_saves_and_reloads_provider_sync(tmp_path):
    store = SettingsStore(tmp_path / "settings.json")

    saved = store.save(BackendSettings(provider_sync_enabled=True))

    assert saved == BackendSettings(provider_sync_enabled=True)
    assert json.loads((tmp_path / "settings.json").read_text(encoding="utf-8")) == {"providerSyncEnabled": True}
    assert store.load() == BackendSettings(provider_sync_enabled=True)


def test_settings_store_ignores_malformed_json(tmp_path):
    path = tmp_path / "settings.json"
    path.write_text("not json", encoding="utf-8")
    store = SettingsStore(path)

    assert store.load() == BackendSettings(provider_sync_enabled=False)
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
python -m pytest -q tests/test_settings_store.py
```

Expected: FAIL with `ModuleNotFoundError: No module named 'codex_session_delete.settings_store'`.

- [ ] **Step 3: Implement settings store**

Create `codex_session_delete/settings_store.py` with:

```python
from __future__ import annotations

import json
import os
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class BackendSettings:
    provider_sync_enabled: bool = False

    @classmethod
    def from_dict(cls, data: dict[str, object]) -> "BackendSettings":
        return cls(provider_sync_enabled=bool(data.get("providerSyncEnabled", False)))

    def to_dict(self) -> dict[str, object]:
        return {"providerSyncEnabled": self.provider_sync_enabled}


class SettingsStore:
    def __init__(self, path: Path | None = None):
        self.path = path or default_settings_path()

    def load(self) -> BackendSettings:
        try:
            data = json.loads(self.path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            return BackendSettings()
        return BackendSettings.from_dict(data if isinstance(data, dict) else {})

    def save(self, settings: BackendSettings) -> BackendSettings:
        self.path.parent.mkdir(parents=True, exist_ok=True)
        temp_path = self.path.with_name(f"{self.path.name}.tmp")
        temp_path.write_text(json.dumps(settings.to_dict(), ensure_ascii=False, indent=2), encoding="utf-8")
        os.replace(temp_path, self.path)
        return settings

    def update(self, values: dict[str, object]) -> BackendSettings:
        current = self.load().to_dict()
        if "providerSyncEnabled" in values:
            current["providerSyncEnabled"] = bool(values["providerSyncEnabled"])
        return self.save(BackendSettings.from_dict(current))


def default_settings_path() -> Path:
    return Path.home() / ".codex-session-delete" / "settings.json"
```

- [ ] **Step 4: Run tests to verify they pass**

Run:

```bash
python -m pytest -q tests/test_settings_store.py
```

Expected: `3 passed`.

- [ ] **Step 5: Commit settings store**

Run:

```bash
git add codex_session_delete/settings_store.py tests/test_settings_store.py
git commit -m "Add backend settings store"
```

---

### Task 2: Provider Sync Core

**Files:**
- Create: `codex_session_delete/provider_sync.py`
- Test: `tests/test_provider_sync.py`

- [ ] **Step 1: Write the failing provider sync tests**

Create `tests/test_provider_sync.py` with:

```python
import json
import sqlite3

from codex_session_delete.provider_sync import ProviderSyncStatus, run_provider_sync


def write_rollout(path, provider="openai", thread_id="thread-1", cwd="C:/old"):
    path.parent.mkdir(parents=True, exist_ok=True)
    first = {
        "type": "session_meta",
        "payload": {
            "id": thread_id,
            "model_provider": provider,
            "cwd": cwd,
        },
    }
    path.write_text(json.dumps(first) + "\n" + json.dumps({"type": "event_msg", "payload": {"type": "user_message"}}) + "\n", encoding="utf-8")


def create_state_db(path):
    con = sqlite3.connect(path)
    con.execute("CREATE TABLE threads (id TEXT PRIMARY KEY, model_provider TEXT, archived INTEGER, has_user_event INTEGER, cwd TEXT)")
    con.execute("INSERT INTO threads VALUES ('thread-1', 'old-provider', 0, 0, 'C:/old')")
    con.commit()
    con.close()


def test_provider_sync_updates_rollout_and_sqlite_to_current_provider(tmp_path):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    (codex_home / "config.toml").write_text('model_provider = "apigather"\n', encoding="utf-8")
    rollout = codex_home / "sessions" / "2026" / "rollout-abc.jsonl"
    write_rollout(rollout, provider="openai", thread_id="thread-1", cwd="C:/workspace")
    create_state_db(codex_home / "state_5.sqlite")

    result = run_provider_sync(codex_home)

    assert result.status == ProviderSyncStatus.SYNCED
    first = json.loads(rollout.read_text(encoding="utf-8").splitlines()[0])
    assert first["payload"]["model_provider"] == "apigather"
    con = sqlite3.connect(codex_home / "state_5.sqlite")
    row = con.execute("SELECT model_provider, has_user_event, cwd FROM threads WHERE id = 'thread-1'").fetchone()
    con.close()
    assert row == ("apigather", 1, "C:/workspace")
    assert result.changed_session_files == 1
    assert result.sqlite_rows_updated == 1
    assert result.backup_dir is not None
    assert (result.backup_dir / "session-meta-backup.json").exists()


def test_provider_sync_skips_when_lock_exists(tmp_path):
    codex_home = tmp_path / ".codex"
    (codex_home / "tmp" / "provider-sync.lock").mkdir(parents=True)
    (codex_home / "config.toml").write_text('model_provider = "apigather"\n', encoding="utf-8")

    result = run_provider_sync(codex_home)

    assert result.status == ProviderSyncStatus.SKIPPED
    assert "lock" in result.message.lower()


def test_provider_sync_prunes_backups_to_five(tmp_path):
    codex_home = tmp_path / ".codex"
    codex_home.mkdir()
    (codex_home / "config.toml").write_text('model_provider = "apigather"\n', encoding="utf-8")
    backup_root = codex_home / "backups_state" / "provider-sync"
    for index in range(6):
        backup = backup_root / f"2000010100000{index}"
        backup.mkdir(parents=True)
        (backup / "metadata.json").write_text(json.dumps({"managedBy": "Codex++ provider sync"}), encoding="utf-8")
    write_rollout(codex_home / "sessions" / "rollout-new.jsonl", provider="openai")

    result = run_provider_sync(codex_home)

    assert result.status == ProviderSyncStatus.SYNCED
    backups = [path for path in backup_root.iterdir() if path.is_dir()]
    assert len(backups) == 5
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
python -m pytest -q tests/test_provider_sync.py
```

Expected: FAIL with `ModuleNotFoundError: No module named 'codex_session_delete.provider_sync'`.

- [ ] **Step 3: Implement provider sync module**

Create `codex_session_delete/provider_sync.py` with:

```python
from __future__ import annotations

import json
import os
import shutil
import sqlite3
import time
from dataclasses import dataclass
from enum import Enum
from pathlib import Path

DEFAULT_PROVIDER = "openai"
BACKUP_KEEP_COUNT = 5
SESSION_DIRS = ("sessions", "archived_sessions")


class ProviderSyncStatus(str, Enum):
    DISABLED = "disabled"
    SKIPPED = "skipped"
    SYNCED = "synced"


@dataclass(frozen=True)
class ProviderSyncResult:
    status: ProviderSyncStatus
    message: str
    target_provider: str = DEFAULT_PROVIDER
    backup_dir: Path | None = None
    changed_session_files: int = 0
    sqlite_rows_updated: int = 0


@dataclass(frozen=True)
class SessionChange:
    path: Path
    original_first_line: str
    next_first_line: str
    separator: str
    thread_id: str | None
    cwd: str | None
    has_user_event: bool


def default_codex_home() -> Path:
    return Path.home() / ".codex"


def run_provider_sync(codex_home: Path | None = None) -> ProviderSyncResult:
    home = codex_home or default_codex_home()
    if not home.exists():
        return ProviderSyncResult(ProviderSyncStatus.SKIPPED, f"Codex home not found: {home}")
    target_provider = read_current_provider(home / "config.toml")
    lock_dir = home / "tmp" / "provider-sync.lock"
    try:
        acquire_lock(lock_dir)
    except FileExistsError:
        return ProviderSyncResult(ProviderSyncStatus.SKIPPED, f"Provider sync lock exists: {lock_dir}", target_provider)
    try:
        changes = collect_session_changes(home, target_provider)
        thread_ids_with_user_events = {change.thread_id for change in changes if change.thread_id and change.has_user_event}
        cwd_by_thread_id = {change.thread_id: change.cwd for change in changes if change.thread_id and change.cwd}
        sqlite_update_count = count_sqlite_updates(home / "state_5.sqlite", target_provider, thread_ids_with_user_events, cwd_by_thread_id)
        if not changes and sqlite_update_count == 0:
            return ProviderSyncResult(ProviderSyncStatus.SYNCED, "Provider sync already up to date", target_provider)
        backup_dir = create_backup(home, target_provider, changes)
        try:
            apply_session_changes(changes)
            sqlite_rows_updated = apply_sqlite_update(home / "state_5.sqlite", target_provider, thread_ids_with_user_events, cwd_by_thread_id)
            prune_backups(home)
        except Exception:
            restore_session_changes(changes)
            raise
        return ProviderSyncResult(
            ProviderSyncStatus.SYNCED,
            "Provider sync complete",
            target_provider,
            backup_dir,
            len(changes),
            sqlite_rows_updated,
        )
    except (sqlite3.OperationalError, OSError) as exc:
        return ProviderSyncResult(ProviderSyncStatus.SKIPPED, f"Provider sync skipped: {exc}", target_provider)
    finally:
        release_lock(lock_dir)


def read_current_provider(config_path: Path) -> str:
    try:
        lines = config_path.read_text(encoding="utf-8").splitlines()
    except OSError:
        return DEFAULT_PROVIDER
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("model_provider") and "=" in stripped:
            raw = stripped.split("=", 1)[1].strip()
            if raw.startswith('"') and raw.endswith('"') and len(raw) >= 2:
                return raw[1:-1] or DEFAULT_PROVIDER
    return DEFAULT_PROVIDER


def acquire_lock(lock_dir: Path) -> None:
    lock_dir.parent.mkdir(parents=True, exist_ok=True)
    lock_dir.mkdir()
    (lock_dir / "owner.json").write_text(json.dumps({"pid": os.getpid(), "startedAt": time.time()}), encoding="utf-8")


def release_lock(lock_dir: Path) -> None:
    shutil.rmtree(lock_dir, ignore_errors=True)


def rollout_files(home: Path) -> list[Path]:
    files: list[Path] = []
    for dirname in SESSION_DIRS:
        root = home / dirname
        if root.exists():
            files.extend(sorted(path for path in root.rglob("rollout-*.jsonl") if path.is_file()))
    return files


def split_first_line(text: str) -> tuple[str, str]:
    if "\n" not in text:
        return text, ""
    first, rest = text.split("\n", 1)
    return first, "\n" + rest


def collect_session_changes(home: Path, target_provider: str) -> list[SessionChange]:
    changes: list[SessionChange] = []
    for path in rollout_files(home):
        text = path.read_text(encoding="utf-8")
        first_line, separator = split_first_line(text)
        if not first_line.strip():
            continue
        try:
            record = json.loads(first_line)
        except json.JSONDecodeError:
            continue
        payload = record.get("payload") if isinstance(record, dict) else None
        if not isinstance(payload, dict):
            continue
        thread_id = payload.get("id") if isinstance(payload.get("id"), str) else None
        cwd = payload.get("cwd") if isinstance(payload.get("cwd"), str) else None
        has_user_event = '"user_message"' in separator or '"user_input"' in separator
        if payload.get("model_provider") == target_provider:
            continue
        payload["model_provider"] = target_provider
        next_first_line = json.dumps(record, ensure_ascii=False, separators=(",", ":"))
        changes.append(SessionChange(path, first_line, next_first_line, separator, thread_id, cwd, has_user_event))
    return changes


def create_backup(home: Path, target_provider: str, changes: list[SessionChange]) -> Path:
    backup_root = home / "backups_state" / "provider-sync"
    backup_dir = backup_root / time.strftime("%Y%m%d%H%M%S")
    suffix = 0
    while backup_dir.exists():
        suffix += 1
        backup_dir = backup_root / f"{time.strftime('%Y%m%d%H%M%S')}-{suffix}"
    backup_dir.mkdir(parents=True)
    for name in ("config.toml", ".codex-global-state.json", ".codex-global-state.json.bak"):
        source = home / name
        if source.exists():
            shutil.copy2(source, backup_dir / name)
    db_dir = backup_dir / "db"
    for name in ("state_5.sqlite", "state_5.sqlite-wal", "state_5.sqlite-shm"):
        source = home / name
        if source.exists():
            db_dir.mkdir(exist_ok=True)
            shutil.copy2(source, db_dir / name)
    manifest = [
        {"path": str(change.path), "originalFirstLine": change.original_first_line, "separator": change.separator}
        for change in changes
    ]
    (backup_dir / "session-meta-backup.json").write_text(json.dumps(manifest, ensure_ascii=False, indent=2), encoding="utf-8")
    (backup_dir / "metadata.json").write_text(
        json.dumps({"managedBy": "Codex++ provider sync", "targetProvider": target_provider}, ensure_ascii=False, indent=2),
        encoding="utf-8",
    )
    return backup_dir


def apply_session_changes(changes: list[SessionChange]) -> None:
    for change in changes:
        change.path.write_text(change.next_first_line + change.separator, encoding="utf-8")


def restore_session_changes(changes: list[SessionChange]) -> None:
    for change in changes:
        change.path.write_text(change.original_first_line + change.separator, encoding="utf-8")


def table_columns(con: sqlite3.Connection, table: str) -> set[str]:
    return {row[1] for row in con.execute(f'PRAGMA table_info("{table}")')}


def count_sqlite_updates(db_path: Path, target_provider: str, user_event_thread_ids: set[str | None], cwd_by_thread_id: dict[str | None, str]) -> int:
    if not db_path.exists():
        return 0
    con = sqlite3.connect(db_path)
    try:
        columns = table_columns(con, "threads")
        if "model_provider" not in columns:
            return 0
        total = con.execute("SELECT COUNT(*) FROM threads WHERE COALESCE(model_provider, '') <> ?", (target_provider,)).fetchone()[0]
        if "has_user_event" in columns:
            for thread_id in user_event_thread_ids:
                if thread_id:
                    total += con.execute("SELECT COUNT(*) FROM threads WHERE id = ? AND COALESCE(has_user_event, 0) <> 1", (thread_id,)).fetchone()[0]
        if "cwd" in columns:
            for thread_id, cwd in cwd_by_thread_id.items():
                if thread_id and cwd:
                    total += con.execute("SELECT COUNT(*) FROM threads WHERE id = ? AND COALESCE(cwd, '') <> ?", (thread_id, cwd)).fetchone()[0]
        return int(total)
    finally:
        con.close()


def apply_sqlite_update(db_path: Path, target_provider: str, user_event_thread_ids: set[str | None], cwd_by_thread_id: dict[str | None, str]) -> int:
    if not db_path.exists():
        return 0
    con = sqlite3.connect(db_path)
    try:
        columns = table_columns(con, "threads")
        if "model_provider" not in columns:
            return 0
        total = con.execute("UPDATE threads SET model_provider = ? WHERE COALESCE(model_provider, '') <> ?", (target_provider, target_provider)).rowcount
        if "has_user_event" in columns:
            for thread_id in user_event_thread_ids:
                if thread_id:
                    total += con.execute("UPDATE threads SET has_user_event = 1 WHERE id = ? AND COALESCE(has_user_event, 0) <> 1", (thread_id,)).rowcount
        if "cwd" in columns:
            for thread_id, cwd in cwd_by_thread_id.items():
                if thread_id and cwd:
                    total += con.execute("UPDATE threads SET cwd = ? WHERE id = ? AND COALESCE(cwd, '') <> ?", (cwd, thread_id, cwd)).rowcount
        con.commit()
        return total
    except Exception:
        con.rollback()
        raise
    finally:
        con.close()


def prune_backups(home: Path, keep_count: int = BACKUP_KEEP_COUNT) -> None:
    backup_root = home / "backups_state" / "provider-sync"
    if not backup_root.exists():
        return
    managed = []
    for path in backup_root.iterdir():
        if not path.is_dir():
            continue
        try:
            metadata = json.loads((path / "metadata.json").read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if metadata.get("managedBy") == "Codex++ provider sync":
            managed.append(path)
    managed.sort(key=lambda path: path.name, reverse=True)
    for path in managed[keep_count:]:
        shutil.rmtree(path, ignore_errors=True)
```

- [ ] **Step 4: Run provider sync tests**

Run:

```bash
python -m pytest -q tests/test_provider_sync.py
```

Expected: `3 passed`.

- [ ] **Step 5: Commit provider sync core**

Run:

```bash
git add codex_session_delete/provider_sync.py tests/test_provider_sync.py
git commit -m "Add provider sync core"
```

---

### Task 3: Launcher Startup Integration

**Files:**
- Modify: `codex_session_delete/launcher.py`
- Test: `tests/test_launcher_cli.py`

- [ ] **Step 1: Write failing launcher tests**

Add these tests after `test_launch_and_inject_returns_windows_packaged_process_id` in `tests/test_launcher_cli.py`:

```python
def test_launch_and_inject_runs_provider_sync_before_launch_when_enabled(monkeypatch, tmp_path):
    events = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})
    monkeypatch.setattr(launcher, "backend_settings", lambda: type("Settings", (), {"provider_sync_enabled": True})())
    monkeypatch.setattr(launcher, "run_provider_sync", lambda: events.append("sync") or type("Result", (), {"status": "synced", "message": "ok"})())
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: events.append("launch") or 1234)

    launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert events == ["sync", "launch"]


def test_launch_and_inject_skips_provider_sync_when_disabled(monkeypatch, tmp_path):
    events = []
    monkeypatch.setattr(launcher, "resolve_codex_app_dir", lambda app_dir=None: tmp_path)
    monkeypatch.setattr(launcher, "start_helper", lambda *args, **kwargs: FakeServer())
    monkeypatch.setattr(launcher, "inject_with_retry", lambda *args, **kwargs: {"result": {}})
    monkeypatch.setattr(launcher, "backend_settings", lambda: type("Settings", (), {"provider_sync_enabled": False})())
    monkeypatch.setattr(launcher, "run_provider_sync", lambda: (_ for _ in ()).throw(AssertionError("sync should not run")))
    monkeypatch.setattr(launcher, "launch_codex_app", lambda *args: events.append("launch") or 1234)

    launcher.launch_and_inject(None, None, tmp_path / "backups", 9229, 57321)

    assert events == ["launch"]
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
python -m pytest -q tests/test_launcher_cli.py::test_launch_and_inject_runs_provider_sync_before_launch_when_enabled tests/test_launcher_cli.py::test_launch_and_inject_skips_provider_sync_when_disabled
```

Expected: FAIL with `AttributeError` for `backend_settings` or `run_provider_sync`.

- [ ] **Step 3: Modify launcher imports and helper**

In `codex_session_delete/launcher.py`, add imports near existing imports:

```python
from codex_session_delete.provider_sync import ProviderSyncStatus, run_provider_sync
from codex_session_delete.settings_store import BackendSettings, SettingsStore
```

Add this function after `user_scripts_config_dir()`:

```python
def backend_settings() -> BackendSettings:
    return SettingsStore().load()
```

- [ ] **Step 4: Call provider sync before launch**

In `launch_and_inject()`, before `server = start_helper(...)`, add:

```python
    if backend_settings().provider_sync_enabled:
        sync_result = run_provider_sync()
        if sync_result.status == ProviderSyncStatus.SKIPPED:
            print(f"Provider sync skipped: {sync_result.message}")
```

- [ ] **Step 5: Run launcher tests**

Run:

```bash
python -m pytest -q tests/test_launcher_cli.py::test_launch_and_inject_runs_provider_sync_before_launch_when_enabled tests/test_launcher_cli.py::test_launch_and_inject_skips_provider_sync_when_disabled
```

Expected: `2 passed`.

- [ ] **Step 6: Commit launcher integration**

Run:

```bash
git add codex_session_delete/launcher.py tests/test_launcher_cli.py
git commit -m "Run provider sync before Codex launch"
```

---

### Task 4: Bridge Settings Endpoints

**Files:**
- Modify: `codex_session_delete/launcher.py`
- Test: `tests/test_launcher_user_scripts.py`

- [ ] **Step 1: Write failing bridge tests**

Add this import to `tests/test_launcher_user_scripts.py`:

```python
from codex_session_delete.settings_store import SettingsStore
```

Add these tests after `test_handle_bridge_request_reports_and_repairs_backend_status`:

```python
def test_handle_bridge_request_gets_backend_settings(monkeypatch, tmp_path):
    store = SettingsStore(tmp_path / "settings.json")
    store.update({"providerSyncEnabled": True})
    monkeypatch.setattr("codex_session_delete.launcher.SettingsStore", lambda: store)
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/settings/get", {}, runtime)

    assert result == {"providerSyncEnabled": True}


def test_handle_bridge_request_sets_backend_settings(monkeypatch, tmp_path):
    store = SettingsStore(tmp_path / "settings.json")
    monkeypatch.setattr("codex_session_delete.launcher.SettingsStore", lambda: store)
    manager = UserScriptManager(tmp_path / "builtin", tmp_path / "user", tmp_path / "config.json")
    runtime = FakeRuntime(manager)

    result = handle_bridge_request(FakeDeleteService(), FakeExportService(), "/settings/set", {"providerSyncEnabled": True}, runtime)

    assert result == {"providerSyncEnabled": True}
    assert store.load().provider_sync_enabled is True
```

- [ ] **Step 2: Run tests to verify they fail**

Run:

```bash
python -m pytest -q tests/test_launcher_user_scripts.py::test_handle_bridge_request_gets_backend_settings tests/test_launcher_user_scripts.py::test_handle_bridge_request_sets_backend_settings
```

Expected: FAIL because `/settings/get` and `/settings/set` are not handled.

- [ ] **Step 3: Add bridge endpoint handling**

In `codex_session_delete/launcher.py`, add these branches in `handle_bridge_request()` before `/user-scripts/list`:

```python
    if path == "/settings/get" and runtime:
        return SettingsStore().load().to_dict()
    if path == "/settings/set" and runtime:
        return SettingsStore().update(payload).to_dict()
```

- [ ] **Step 4: Run bridge tests**

Run:

```bash
python -m pytest -q tests/test_launcher_user_scripts.py::test_handle_bridge_request_gets_backend_settings tests/test_launcher_user_scripts.py::test_handle_bridge_request_sets_backend_settings
```

Expected: `2 passed`.

- [ ] **Step 5: Commit bridge endpoints**

Run:

```bash
git add codex_session_delete/launcher.py tests/test_launcher_user_scripts.py
git commit -m "Expose backend settings bridge"
```

---

### Task 5: Provider Sync Menu Toggle

**Files:**
- Modify: `codex_session_delete/inject/renderer-inject.js`
- Modify: `tests/test_renderer_script.py`

- [ ] **Step 1: Write failing renderer contract test**

Add this test to `tests/test_renderer_script.py` near the Codex++ menu tests:

```python
def test_renderer_script_has_backend_provider_sync_toggle():
    text = Path("codex_session_delete/inject/renderer-inject.js").read_text(encoding="utf-8")

    assert "Provider 同步" in text
    assert "data-codex-backend-setting=\"providerSyncEnabled\"" in text
    assert "/settings/get" in text
    assert "/settings/set" in text
    assert "loadBackendSettings" in text
    assert "setBackendSetting" in text
```

- [ ] **Step 2: Run test to verify it fails**

Run:

```bash
python -m pytest -q tests/test_renderer_script.py::test_renderer_script_has_backend_provider_sync_toggle
```

Expected: FAIL with missing `Provider 同步`.

- [ ] **Step 3: Add renderer backend settings functions**

In `codex_session_delete/inject/renderer-inject.js`, after `function setCodexPlusSetting(...)`, add:

```javascript
  let codexPlusBackendSettings = { providerSyncEnabled: false };

  async function loadBackendSettings() {
    try {
      const settings = await bridgeCall("/settings/get", {});
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
      refreshCodexPlusBackendToggles();
    } catch (_) {
      refreshCodexPlusBackendToggles();
    }
  }

  async function setBackendSetting(key, value) {
    codexPlusBackendSettings = { ...codexPlusBackendSettings, [key]: value };
    refreshCodexPlusBackendToggles();
    try {
      const settings = await bridgeCall("/settings/set", { [key]: value });
      codexPlusBackendSettings = { ...codexPlusBackendSettings, ...settings };
    } finally {
      refreshCodexPlusBackendToggles();
    }
  }

  function refreshCodexPlusBackendToggles() {
    document.querySelectorAll(".codex-plus-toggle[data-codex-backend-setting]").forEach((button) => {
      const key = button.getAttribute("data-codex-backend-setting");
      button.dataset.enabled = String(!!codexPlusBackendSettings[key]);
    });
  }
```

- [ ] **Step 4: Add Provider sync row to settings menu**

In the existing settings rows near `conversationTimeline`, add:

```html
            <div class="codex-plus-row">
              <div><div class="codex-plus-row-title">Provider 同步</div><div class="codex-plus-row-description">启动 Codex 前同步历史会话到当前 model_provider。</div></div>
              <button type="button" class="codex-plus-toggle" data-codex-backend-setting="providerSyncEnabled"><span></span></button>
            </div>
```

- [ ] **Step 5: Wire click handler and initial load**

In the Codex++ menu click handler, after existing `[data-codex-plus-setting]` handling, add:

```javascript
      const backendToggle = target?.closest("[data-codex-backend-setting]");
      if (backendToggle) {
        const key = backendToggle.getAttribute("data-codex-backend-setting");
        setBackendSetting(key, !codexPlusBackendSettings[key]);
        return;
      }
```

In `installCodexPlusMenu()`, after the menu is installed and normal toggles refresh, call:

```javascript
    loadBackendSettings();
```

- [ ] **Step 6: Run renderer contract test**

Run:

```bash
python -m pytest -q tests/test_renderer_script.py::test_renderer_script_has_backend_provider_sync_toggle
```

Expected: `1 passed`.

- [ ] **Step 7: Commit renderer toggle**

Run:

```bash
git add codex_session_delete/inject/renderer-inject.js tests/test_renderer_script.py
git commit -m "Add provider sync settings toggle"
```

---

### Task 6: Final Verification

**Files:**
- Verify all changed files.

- [ ] **Step 1: Run focused tests**

Run:

```bash
python -m pytest -q tests/test_settings_store.py tests/test_provider_sync.py tests/test_launcher_user_scripts.py tests/test_launcher_cli.py tests/test_renderer_script.py
```

Expected: all focused tests pass. If `tests/test_renderer_script.py` fails on the pre-existing `right: 140px` assertion, update that assertion to accept `right: var(--codex-plus-menu-right, 140px)` because the current source uses the CSS variable from commit `9ab834f`.

- [ ] **Step 2: Run full test suite**

Run:

```bash
python -m pytest -q
```

Expected: all tests pass.

- [ ] **Step 3: Inspect final diff**

Run:

```bash
git diff --stat HEAD~5..HEAD
```

Expected: only provider sync/settings/menu/test files are changed.

- [ ] **Step 4: Report completion status**

Report exact commands run and pass/fail counts. Do not claim completion unless the full test suite passes.
