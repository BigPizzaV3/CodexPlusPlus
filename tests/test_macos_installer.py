import os
import plistlib
import stat

from codex_session_delete.installers import InstallOptions
from codex_session_delete import __version__
from codex_session_delete import macos_installer
from codex_session_delete.macos_installer import install_macos_app, uninstall_macos_app


def test_install_macos_app_creates_app_bundle(tmp_path):
    options = InstallOptions(install_root=tmp_path, launcher_command="python -m codex_session_delete launch")

    install_macos_app(options)

    app = tmp_path / "Codex++.app"
    plist_path = app / "Contents" / "Info.plist"
    executable = app / "Contents" / "MacOS" / "CodexPlusPlus"
    assert plist_path.exists()
    assert executable.exists()
    if os.name == "posix":
        assert executable.stat().st_mode & stat.S_IXUSR

    plist = plistlib.loads(plist_path.read_bytes())
    assert plist["CFBundleName"] == "Codex++"
    assert plist["CFBundleExecutable"] == "CodexPlusPlus"
    assert plist["CFBundleIdentifier"] == "com.bigpizzav3.codexplusplus"
    assert plist["CFBundleIconFile"] == "codex-plus-plus.png"
    assert plist["CFBundleVersion"] == __version__
    assert plist["CFBundleShortVersionString"] == __version__
    assert (app / "Contents" / "Resources" / "codex-plus-plus.png").exists()

    script = executable.read_text(encoding="utf-8")
    assert "/opt/homebrew/bin" in script
    assert "/usr/local/bin" in script
    assert "python -m codex_session_delete launch" in script
    assert "exec" in script


def test_uninstall_macos_app_removes_app_bundle(tmp_path):
    options = InstallOptions(install_root=tmp_path, launcher_command="python -m codex_session_delete launch")
    install_macos_app(options)

    uninstall_macos_app(InstallOptions(install_root=tmp_path))

    assert not (tmp_path / "Codex++.app").exists()


def test_default_launcher_command_sets_pythonpath_in_source_tree(monkeypatch, tmp_path):
    package_dir = tmp_path / "codex_session_delete"
    package_dir.mkdir()
    (tmp_path / "pyproject.toml").write_text("[project]\nname='codex-session-delete'\n", encoding="utf-8")
    fake_file = package_dir / "macos_installer.py"
    fake_file.write_text("", encoding="utf-8")
    monkeypatch.setattr(macos_installer, "__file__", str(fake_file))

    command = macos_installer._launcher_command(InstallOptions())

    assert "PYTHONPATH=" in command
    assert str(tmp_path) in command
    assert "-m codex_session_delete launch" in command
