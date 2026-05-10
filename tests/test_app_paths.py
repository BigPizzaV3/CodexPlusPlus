from pathlib import Path

from codex_session_delete.app_paths import find_latest_codex_app_dir, find_macos_codex_app, resolve_codex_app_dir, user_data_candidates


def test_find_latest_codex_app_dir_uses_highest_version(tmp_path):
    older = tmp_path / "OpenAI.Codex_1.2.3.0_x64__abc" / "app"
    newer = tmp_path / "OpenAI.Codex_26.429.8261.0_x64__abc" / "app"
    older.mkdir(parents=True)
    newer.mkdir(parents=True)

    assert find_latest_codex_app_dir(tmp_path) == newer


def test_find_latest_codex_app_dir_falls_back_to_windowsapps(monkeypatch, tmp_path):
    windows_apps = tmp_path / "WindowsApps"
    app = windows_apps / "OpenAI.Codex_26.506.3741.0_x64__abc" / "app"
    app.mkdir(parents=True)
    monkeypatch.setattr("codex_session_delete.app_paths._find_codex_app_dir_with_appx", lambda: None)
    monkeypatch.setenv("ProgramFiles", str(tmp_path))

    assert find_latest_codex_app_dir() == app


def test_user_data_candidates_include_local_appdata(monkeypatch, tmp_path):
    monkeypatch.setenv("LOCALAPPDATA", str(tmp_path / "Local"))
    monkeypatch.setenv("APPDATA", str(tmp_path / "Roaming"))

    candidates = user_data_candidates()

    assert Path(tmp_path / "Local" / "OpenAI" / "Codex") in candidates


def test_find_macos_codex_app_prefers_applications(tmp_path):
    system_app = tmp_path / "Applications" / "Codex.app"
    user_app = tmp_path / "Users" / "me" / "Applications" / "Codex.app"
    system_app.mkdir(parents=True)
    user_app.mkdir(parents=True)

    result = find_macos_codex_app([system_app, user_app])

    assert result == system_app


def test_find_macos_codex_app_detects_openai_codex_bundle(tmp_path):
    openai_app = tmp_path / "Applications" / "OpenAI Codex.app"
    openai_app.mkdir(parents=True)

    result = find_macos_codex_app([tmp_path / "Applications"])

    assert result == openai_app


def test_find_macos_codex_app_returns_none_when_missing(tmp_path):
    result = find_macos_codex_app([tmp_path / "Applications" / "Codex.app"])

    assert result is None


def test_resolve_codex_app_dir_uses_macos_discovery_on_darwin(monkeypatch, tmp_path):
    mac_app = tmp_path / "Applications" / "OpenAI Codex.app"
    mac_app.mkdir(parents=True)
    monkeypatch.setattr("codex_session_delete.app_paths.sys.platform", "darwin")
    monkeypatch.setattr("codex_session_delete.app_paths.find_macos_codex_app", lambda: mac_app)
    monkeypatch.setattr("codex_session_delete.app_paths.find_latest_codex_app_dir", lambda: None)

    assert resolve_codex_app_dir() == mac_app
