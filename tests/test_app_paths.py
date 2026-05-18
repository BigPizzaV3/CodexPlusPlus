from pathlib import Path

from codex_session_delete.app_paths import find_latest_codex_app_dir, find_macos_codex_app, resolve_codex_app_dir, user_data_candidates


def test_find_latest_codex_app_dir_uses_highest_version(tmp_path):
    older = tmp_path / "OpenAI.Codex_1.2.3.0_x64__abc" / "app"
    newer = tmp_path / "OpenAI.Codex_26.429.8261.0_x64__abc" / "app"
    older.mkdir(parents=True)
    newer.mkdir(parents=True)

    assert find_latest_codex_app_dir(tmp_path) == newer


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


def test_find_macos_codex_app_uses_bundle_identifier_when_name_changes(tmp_path):
    renamed_app = tmp_path / "Applications" / "AI Workbench.app"
    contents = renamed_app / "Contents"
    contents.mkdir(parents=True)
    (contents / "Info.plist").write_text(
        """<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key>
  <string>com.openai.codex</string>
</dict>
</plist>
""",
        encoding="utf-8",
    )

    result = find_macos_codex_app([tmp_path / "Applications"])

    assert result == renamed_app


def test_find_macos_codex_app_ignores_codex_plus_plus_bundle(tmp_path):
    codex_plus = tmp_path / "Applications" / "Codex++.app"
    codex = tmp_path / "Applications" / "Codex.app"
    codex_plus.mkdir(parents=True)
    codex.mkdir(parents=True)

    result = find_macos_codex_app([tmp_path / "Applications"])

    assert result == codex


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
