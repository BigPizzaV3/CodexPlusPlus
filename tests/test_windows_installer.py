from pathlib import Path
import sys

from codex_session_delete.installers import InstallOptions
from codex_session_delete import __version__
from codex_session_delete.windows_installer import build_install_shortcut_script, build_uninstall_shortcut_script


def test_build_install_shortcut_script_contains_codex_plus_shortcuts(tmp_path):
    options = InstallOptions(install_root=tmp_path, launcher_command="python -m codex_session_delete launch")

    script = build_install_shortcut_script(options)

    assert "Codex++.lnk" in script
    assert "codex-plus-plus.ico" in script
    assert "-m codex_session_delete launch" in script
    assert "CreateShortcut" in script
    assert "$Shortcut.TargetPath = $LauncherPython" in script
    assert "pythonw.exe" not in script
    assert "TargetPath = $Pythonw" not in script
    assert "TargetPath = $Python\n" not in script
    assert "IconLocation" in script
    assert "-EncodedCommand" not in script
    assert "powershell.exe" not in script
    assert "WorkingDirectory = $ProjectRoot" in script
    assert "codex-plus-plus.ico" in script
    assert "Codex.exe" not in script
    assert "IconLocation = $CodexPlusIcon" in script
    assert "$Python,0" not in script
    assert str(Path.cwd()) in script
    assert "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CodexPlusPlus" in script
    assert "DisplayName" in script
    assert "DisplayIcon" in script
    assert "UninstallString" in script
    assert "$UninstallCommand = 'cmd.exe /c cd /d \"' + $ProjectRoot + '\" && \"' + $LauncherPython + '\" -m codex_session_delete uninstall" in script
    assert "--install-root" in script
    assert "QuietUninstallString" in script
    assert f"DisplayVersion -Value '{__version__}'" in script


def test_default_windows_launcher_uses_current_python_executable(tmp_path):
    options = InstallOptions(install_root=tmp_path)

    script = build_install_shortcut_script(options)

    assert "$Python = " not in script
    assert "Get-Command python" not in script
    assert "-m codex_session_delete launch" in script
    assert "pythonw.exe" in script or "python.exe" in script or str(Path(sys.executable)) in script


def test_build_uninstall_shortcut_script_removes_codex_plus_shortcuts(tmp_path):
    options = InstallOptions(install_root=tmp_path)

    script = build_uninstall_shortcut_script(options)

    assert "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CodexPlusPlus" in script
    assert "Remove-Item" in script
    assert str(tmp_path) in script
