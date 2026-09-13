@echo off
chcp 65001 >nul
setlocal EnableExtensions DisableDelayedExpansion

cd /d "%~dp0"
if errorlevel 1 (
  echo [ERROR] Cannot enter project root.
  exit /b 1
)

set "ROOT_DIR=%CD%"
set "MODE=install"
if /i "%~1"=="check" (
  set "MODE=check"
)
if not "%~1"=="" if /i not "%~1"=="check" (
  echo [ERROR] Unsupported argument: %~1
  echo Usage: install-xuan-features.bat [check]
  exit /b 1
)

if not defined APPDATA (
  echo [ERROR] APPDATA is not set.
  exit /b 1
)
if not defined LOCALAPPDATA (
  echo [ERROR] LOCALAPPDATA is not set.
  exit /b 1
)

set "XUAN_HOME=%APPDATA%\XuanPlusPlus"
set "BIN_DIR=%LOCALAPPDATA%\XuanPlusPlus\bin"
set "CODEX_PLUS_USER_SCRIPT_DIR=%APPDATA%\Codex++\user_scripts"
set "XUAN_BRIDGE_MANIFEST=%ROOT_DIR%\tools\xuan-bridge\Cargo.toml"
set "XUAN_BRIDGE_BUILD=%ROOT_DIR%\tools\xuan-bridge\target\release\xuan-bridge.exe"
set "XUAN_BRIDGE_BIN=%BIN_DIR%\xuan-bridge.exe"
set "UI_BRIDGE_SOURCE=%ROOT_DIR%\tools\xuan-ui-bridge\xuan-ui-bridge.mjs"
set "RUNTIME_INSTALLER=%ROOT_DIR%\scripts\install-xuan-runtime.ps1"
set "REMOTE_BRIDGE_MANIFEST=%ROOT_DIR%\apps\xuan-plus-remote\bridge\Cargo.toml"
set "REMOTE_BRIDGE_BUILD=%ROOT_DIR%\apps\xuan-plus-remote\bridge\target\release\xuan-plus-remote-bridge.exe"
set "XUAN_REMOTE_BRIDGE_BIN=%BIN_DIR%\xuan-plus-remote-bridge.exe"
set "XUAN_MOBILE_BRIDGE_URL=http://127.0.0.1:17421"
set "XUAN_BRIDGE_ALLOWED_ORIGINS=app://-"
set "MARKETPLACE_PATH=%ROOT_DIR%\.agents\plugins\marketplace.json"
set "POLISH_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-polish\scripts\polish-composer.user.js"
set "POLISH_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-polish-composer.user.js"
set "USAGE_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-usage\scripts\usage-header.user.js"
set "USAGE_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-usage-header.user.js"
set "SEARCH_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-workspace-search\scripts\workspace-search.user.js"
set "SEARCH_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-workspace-search.user.js"
set "MOBILE_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-mobile\scripts\mobile-connect.user.js"
set "MOBILE_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-mobile-connect.user.js"
set "CODEX_CMD=%XUAN_CODEX_CMD%"
for /f "delims=" %%C in ('where.exe codex.exe 2^>nul') do if not defined CODEX_CMD set "CODEX_CMD=%%C"
if not defined CODEX_CMD for /f "delims=" %%C in ('where.exe codex.cmd 2^>nul') do if not defined CODEX_CMD set "CODEX_CMD=%%C"
if not defined CODEX_CMD if defined LOCALAPPDATA for /f "delims=" %%C in ('dir /b /s "%LOCALAPPDATA%\OpenAI\Codex\bin\codex.exe" 2^>nul') do if not defined CODEX_CMD set "CODEX_CMD=%%C"
if not defined CODEX_CMD if defined APPDATA if exist "%APPDATA%\npm\codex.cmd" set "CODEX_CMD=%APPDATA%\npm\codex.cmd"

echo [1/7] Checking plugin prerequisites...
call :require_command node.exe Node.js
if errorlevel 1 exit /b 1
node.exe -e "if(typeof WebSocket!=='function')process.exit(1)"
if errorlevel 1 (
  echo [错误] 独立插件界面需要 Node.js 22 或更高版本。
  exit /b 1
)
call :require_command cargo.exe Rust
if errorlevel 1 exit /b 1
call :require_command pwsh.exe PowerShell
if errorlevel 1 exit /b 1
if not defined CODEX_CMD (
  echo [ERROR] Codex CLI was not found.
  echo Install or enable the Codex CLI, then open a new terminal and retry.
  exit /b 1
)
echo   [OK] Codex CLI: %CODEX_CMD%
call :require_command rg.exe ripgrep
if errorlevel 1 exit /b 1
for %%F in (
  "%XUAN_BRIDGE_MANIFEST%"
  "%UI_BRIDGE_SOURCE%"
  "%RUNTIME_INSTALLER%"
  "%REMOTE_BRIDGE_MANIFEST%"
  "%MARKETPLACE_PATH%"
  "%POLISH_SCRIPT_SOURCE%"
  "%USAGE_SCRIPT_SOURCE%"
  "%SEARCH_SCRIPT_SOURCE%"
  "%MOBILE_SCRIPT_SOURCE%"
) do if not exist "%%~F" (
  echo [ERROR] Missing feature file: %%~F
  exit /b 1
)

if /i "%MODE%"=="check" (
  echo Environment check passed. Four independent plugins are ready to install.
  exit /b 0
)

echo [2/7] Building independent bridges...
cargo.exe build --release --locked --manifest-path "%XUAN_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [ERROR] xuan-bridge build failed.
  exit /b 1
)
cargo.exe build --release --locked --manifest-path "%REMOTE_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [ERROR] Mobile bridge build failed.
  exit /b 1
)
if not exist "%XUAN_BRIDGE_BUILD%" (
  echo [ERROR] xuan-bridge.exe was not produced.
  exit /b 1
)
if not exist "%REMOTE_BRIDGE_BUILD%" (
  echo [ERROR] xuan-plus-remote-bridge.exe was not produced.
  exit /b 1
)

echo [3/7] Installing independent bridges...
if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
if errorlevel 1 (
  echo [ERROR] Cannot create the local bridge directory.
  exit /b 1
)
pwsh.exe -NoLogo -NoProfile -NonInteractive -File "%RUNTIME_INSTALLER%" -BridgeSource "%XUAN_BRIDGE_BUILD%" -RemoteSource "%REMOTE_BRIDGE_BUILD%" -UiSource "%UI_BRIDGE_SOURCE%" -BinDirectory "%BIN_DIR%"
if errorlevel 1 (
  echo [错误] 独立插件运行文件安装失败。
  exit /b 1
)
echo   [通过] 独立插件运行文件已按版本安装，无需关闭或替换 Codex++。

echo [4/7] Initializing plugin configuration...
"%XUAN_BRIDGE_BUILD%" init "%XUAN_HOME%"
if errorlevel 1 (
  echo [ERROR] Xuan plugin configuration initialization failed.
  exit /b 1
)

echo [5/7] Registering and installing four Codex plugins...
call "%CODEX_CMD%" plugin marketplace list | findstr.exe /i /b /c:"xuan-curated" >nul
if errorlevel 1 (
  call "%CODEX_CMD%" plugin marketplace add "%ROOT_DIR%"
  if errorlevel 1 (
    echo [ERROR] Cannot register the local Xuan marketplace.
    exit /b 1
  )
) else (
  echo   [OK] Local Xuan marketplace is registered.
)
set "PLUGIN_LIST_FILE=%TEMP%\xuan-codex-plugin-list.txt"
for %%P in (xuan-workspace-search xuan-usage xuan-polish xuan-mobile) do (
    echo   更新独立插件 %%P...
    call "%CODEX_CMD%" plugin add "%%P@xuan-curated"
    if errorlevel 1 (
      echo [ERROR] Cannot install or enable plugin: %%P
      del /q "%PLUGIN_LIST_FILE%" >nul 2>&1
      exit /b 1
    )
    call "%CODEX_CMD%" plugin list > "%PLUGIN_LIST_FILE%"
    findstr.exe /i /r /c:"%%P@xuan-curated.*installed, enabled" "%PLUGIN_LIST_FILE%" >nul
    if errorlevel 1 (
      echo [ERROR] Cannot verify plugin: %%P
      del /q "%PLUGIN_LIST_FILE%" >nul 2>&1
      exit /b 1
    )
    echo   [OK] %%P installed and enabled.
)
del /q "%PLUGIN_LIST_FILE%" >nul 2>&1

echo [6/7] Installing four Codex++ User Scripts...
if not exist "%CODEX_PLUS_USER_SCRIPT_DIR%" mkdir "%CODEX_PLUS_USER_SCRIPT_DIR%"
if errorlevel 1 (
  echo [ERROR] Cannot create the Codex++ User Script directory.
  exit /b 1
)
copy /y "%POLISH_SCRIPT_SOURCE%" "%POLISH_SCRIPT_TARGET%" >nul
if errorlevel 1 (
  echo [ERROR] Cannot install polish User Script.
  exit /b 1
)
echo   [OK] polish User Script installed.
copy /y "%USAGE_SCRIPT_SOURCE%" "%USAGE_SCRIPT_TARGET%" >nul
if errorlevel 1 (
  echo [ERROR] Cannot install usage User Script.
  exit /b 1
)
echo   [OK] usage User Script installed.
copy /y "%SEARCH_SCRIPT_SOURCE%" "%SEARCH_SCRIPT_TARGET%" >nul
if errorlevel 1 (
  echo [ERROR] Cannot install search User Script.
  exit /b 1
)
echo   [OK] search User Script installed.
copy /y "%MOBILE_SCRIPT_SOURCE%" "%MOBILE_SCRIPT_TARGET%" >nul
if errorlevel 1 (
  echo [ERROR] Cannot install mobile User Script.
  exit /b 1
)
echo   [OK] mobile User Script installed.

echo [7/7] 独立插件运行环境已就绪。
echo   [通过] 插件自行管理通信和子进程，不修改 Codex++ 程序或更新逻辑。

echo.
echo Four features were installed as independent plugins:
echo   1. Polish: polish, cancel, restore, settings and Ctrl+Enter
echo   2. Usage: original panel with automatic OwlAI usage lookup
echo   3. Search: project search, preview, cancel and Ctrl+Shift+F
echo   4. Mobile: desktop entry, pairing QR, confirmation and task sync
echo.
echo 重新打开 Codex++ 并新建任务，以加载更新后的插件和用户脚本。
exit /b 0

:require_command
where.exe %~1 >nul 2>&1
if errorlevel 1 (
  echo [ERROR] %~2 was not found: %~1
  exit /b 1
)
echo   [OK] %~2
exit /b 0
