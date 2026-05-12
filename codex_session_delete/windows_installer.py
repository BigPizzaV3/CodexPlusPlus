from __future__ import annotations

import subprocess
import sys
from pathlib import Path
from typing import TYPE_CHECKING

from codex_session_delete import __version__

if TYPE_CHECKING:
    from codex_session_delete.installers import InstallOptions


def _ps_quote(value: str) -> str:
    return "'" + value.replace("'", "''") + "'"


def _default_python_executable() -> str:
    executable = Path(sys.executable)
    pythonw = executable.with_name("pythonw.exe")
    return str(pythonw if pythonw.exists() else executable)


def _launcher_command(options: "InstallOptions") -> str:
    return options.launcher_command or f"{_default_python_executable()} -m codex_session_delete launch"


def _install_root_expr(options: "InstallOptions") -> str:
    if options.install_root is not None:
        return _ps_quote(str(options.install_root))
    return "$([Environment]::GetFolderPath('Desktop'))"


def _project_root_expr() -> str:
    return _ps_quote(str(Path(__file__).resolve().parent.parent))


def _icon_path_expr() -> str:
    return _ps_quote(str(Path(__file__).resolve().parent / "assets" / "codex-plus-plus.ico"))


def _split_launcher_command(command: str) -> tuple[str, str]:
    python_module = " -m codex_session_delete launch"
    if command.endswith(python_module):
        return command[: -len(python_module)], python_module.strip()
    prefix = "python "
    if command.startswith(prefix):
        return "python", command[len(prefix):]
    return command, ""


def build_install_shortcut_script(options: "InstallOptions") -> str:
    root = _install_root_expr(options)
    project_root = _project_root_expr()
    icon_path = _icon_path_expr()
    target, arguments = _split_launcher_command(_launcher_command(options))
    target_expr = _ps_quote(target)
    arguments_expr = _ps_quote(arguments)
    return f"""
$InstallRoot = {root}
$ProjectRoot = {project_root}
$CodexPlusIcon = {icon_path}
New-Item -ItemType Directory -Force -Path $InstallRoot | Out-Null
$ShortcutPath = Join-Path $InstallRoot 'Codex++.lnk'
$LauncherPython = {target_expr}
$Shell = New-Object -ComObject WScript.Shell
$Shortcut = $Shell.CreateShortcut($ShortcutPath)
$Shortcut.TargetPath = $LauncherPython
$Shortcut.Arguments = {arguments_expr}
$Shortcut.WorkingDirectory = $ProjectRoot
$Shortcut.Description = 'Launch Codex with Codex++ injection'
$Shortcut.IconLocation = $CodexPlusIcon
$Shortcut.Save()
$LegacyUninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Codex++'
if (Test-Path $LegacyUninstallKey) {{ Remove-Item $LegacyUninstallKey -Force }}
$UninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CodexPlusPlus'
$UninstallCommand = 'cmd.exe /c cd /d "' + $ProjectRoot + '" && "' + $LauncherPython + '" -m codex_session_delete uninstall --install-root "' + $InstallRoot + '"'
New-Item -Path $UninstallKey -Force | Out-Null
Set-ItemProperty -Path $UninstallKey -Name DisplayName -Value 'Codex++'
Set-ItemProperty -Path $UninstallKey -Name DisplayVersion -Value '{__version__}'
Set-ItemProperty -Path $UninstallKey -Name Publisher -Value 'BigPizzaV3'
Set-ItemProperty -Path $UninstallKey -Name DisplayIcon -Value $CodexPlusIcon
Set-ItemProperty -Path $UninstallKey -Name InstallLocation -Value $ProjectRoot
Set-ItemProperty -Path $UninstallKey -Name UninstallString -Value $UninstallCommand
Set-ItemProperty -Path $UninstallKey -Name QuietUninstallString -Value $UninstallCommand
""".strip()


def build_uninstall_shortcut_script(options: "InstallOptions") -> str:
    root = _install_root_expr(options)
    return f"""
$InstallRoot = {root}
$ShortcutPath = Join-Path $InstallRoot 'Codex++.lnk'
if (Test-Path $ShortcutPath) {{ Remove-Item $ShortcutPath -Force }}
$LegacyUninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Codex++'
if (Test-Path $LegacyUninstallKey) {{ Remove-Item $LegacyUninstallKey -Force }}
$UninstallKey = 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\CodexPlusPlus'
if (Test-Path $UninstallKey) {{ Remove-Item $UninstallKey -Force }}
""".strip()


def _run_powershell(script: str) -> None:
    subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script], check=True)


def install_windows_shortcuts(options: "InstallOptions") -> None:
    _run_powershell(build_install_shortcut_script(options))


def uninstall_windows_shortcuts(options: "InstallOptions") -> None:
    _run_powershell(build_uninstall_shortcut_script(options))
