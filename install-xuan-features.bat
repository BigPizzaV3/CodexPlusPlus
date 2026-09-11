@echo off
chcp 65001 >nul
setlocal EnableExtensions DisableDelayedExpansion

cd /d "%~dp0"
if errorlevel 1 (
  echo [错误] 无法进入项目根目录。
  exit /b 1
)

set "ROOT_DIR=%CD%"
set "MODE=install"
set "START_SERVICES=1"

if /i "%~1"=="check" (
  set "MODE=check"
  set "START_SERVICES=0"
)
if /i "%~1"=="no-start" set "START_SERVICES=0"
if not "%~1"=="" if /i not "%~1"=="check" if /i not "%~1"=="no-start" (
  echo [错误] 不支持的参数：%~1
  echo 用法：install-xuan-features.bat [check^|no-start]
  exit /b 1
)

if not defined APPDATA (
  echo [错误] 未设置 APPDATA，无法确定用户配置目录。
  exit /b 1
)
if not defined LOCALAPPDATA (
  echo [错误] 未设置 LOCALAPPDATA，无法确定本地安装目录。
  exit /b 1
)

set "XUAN_HOME_DIR=%APPDATA%\XuanPlusPlus"
set "BIN_DIR=%LOCALAPPDATA%\XuanPlusPlus\bin"
set "CODEX_PLUS_USER_SCRIPT_DIR=%APPDATA%\Codex++\user_scripts"
set "XUAN_BRIDGE_MANIFEST=%ROOT_DIR%\tools\xuan-bridge\Cargo.toml"
set "REMOTE_BRIDGE_MANIFEST=%ROOT_DIR%\apps\xuan-plus-remote\bridge\Cargo.toml"
set "LAUNCHER_MANIFEST=%ROOT_DIR%\apps\codex-plus-launcher\Cargo.toml"
set "SEARCH_USER_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-workspace-search\scripts\workspace-search.user.js"
set "SEARCH_USER_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-workspace-search.user.js"
set "USAGE_USER_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-usage\scripts\usage-header.user.js"
set "USAGE_USER_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-usage-header.user.js"
set "POLISH_USER_SCRIPT_SOURCE=%ROOT_DIR%\plugins\xuan-polish\scripts\polish-composer.user.js"
set "POLISH_USER_SCRIPT_TARGET=%CODEX_PLUS_USER_SCRIPT_DIR%\xuan-polish-composer.user.js"
set "XUAN_BRIDGE_BIN=%BIN_DIR%\xuan-bridge.exe"
set "XUAN_REMOTE_BRIDGE_BIN=%BIN_DIR%\xuan-plus-remote-bridge.exe"
set "XUAN_MOBILE_BRIDGE_URL=http://127.0.0.1:17421"
set "XUAN_BRIDGE_ALLOWED_ORIGINS=app://-"
set "CODEX_PLUS_INSTALL_DIR="
set "CODEX_PLUS_LAUNCHER_TARGET="
set "LAUNCHER_UPDATE_PENDING=0"
set "XUAN_WORKSPACE_SEARCH_VERSION=0.1.2"
set "XUAN_USAGE_VERSION=0.1.2"
set "XUAN_POLISH_VERSION=0.1.2"

echo [1/7] 检查导入环境...
call :require_command node.exe Node.js
if errorlevel 1 exit /b 1
call :require_command cargo.exe Rust
if errorlevel 1 exit /b 1
call :require_command pwsh.exe PowerShell 7
if errorlevel 1 exit /b 1
call :require_command codex.exe Codex CLI
if errorlevel 1 exit /b 1
call :require_command rg.exe ripgrep
if errorlevel 1 exit /b 1
for /f "tokens=2" %%T in ('cargo.exe -vV ^| findstr.exe /b /c:"host:"') do set "RUST_HOST_TARGET=%%T"
if not defined RUST_HOST_TARGET (
  echo [错误] 无法确定 Rust 主机目标。
  exit /b 1
)
set "XUAN_BRIDGE_BUILD=%ROOT_DIR%\tools\xuan-bridge\target\%RUST_HOST_TARGET%\release\xuan-bridge.exe"
set "REMOTE_BRIDGE_BUILD=%ROOT_DIR%\apps\xuan-plus-remote\bridge\target\%RUST_HOST_TARGET%\release\xuan-plus-remote-bridge.exe"
set "LAUNCHER_BUILD=%ROOT_DIR%\target\%RUST_HOST_TARGET%\release\codex-plus-plus.exe"
for /f "tokens=2,*" %%A in ('reg.exe query "HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex++" /v InstallLocation 2^>nul ^| findstr.exe /i "InstallLocation"') do set "CODEX_PLUS_INSTALL_DIR=%%B"
if defined CODEX_PLUS_INSTALL_DIR set "CODEX_PLUS_LAUNCHER_TARGET=%CODEX_PLUS_INSTALL_DIR%\codex-plus-plus.exe"
echo   [通过] Rust 主机目标：%RUST_HOST_TARGET%
if not exist "%XUAN_BRIDGE_MANIFEST%" (
  echo [错误] 未找到 xuan-bridge 清单文件。
  exit /b 1
)
if not exist "%REMOTE_BRIDGE_MANIFEST%" (
  echo [错误] 未找到手机连接 Bridge 清单文件。
  exit /b 1
)
if not exist "%LAUNCHER_MANIFEST%" (
  echo [错误] 未找到 Codex++ Launcher 清单文件。
  exit /b 1
)
if not exist "%ROOT_DIR%\.agents\plugins\marketplace.json" (
  echo [错误] 未找到 Xuan 插件市场清单。
  exit /b 1
)
if not exist "%SEARCH_USER_SCRIPT_SOURCE%" (
  echo [错误] 未找到工作区搜索 User Script。
  exit /b 1
)
if not exist "%USAGE_USER_SCRIPT_SOURCE%" (
  echo [错误] 未找到用量展示 User Script。
  exit /b 1
)
if not exist "%POLISH_USER_SCRIPT_SOURCE%" (
  echo [错误] 未找到文本润色 Composer User Script。
  exit /b 1
)

if /i "%MODE%"=="check" (
  echo 环境检查通过，可以导入四项功能。
  exit /b 0
)

echo [2/7] 构建本地组件...
cargo.exe build --release --locked --target "%RUST_HOST_TARGET%" --manifest-path "%XUAN_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [错误] xuan-bridge 构建失败。
  exit /b 1
)
cargo.exe build --release --locked --target "%RUST_HOST_TARGET%" --manifest-path "%REMOTE_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [错误] 手机连接 Bridge 构建失败。
  exit /b 1
)
cargo.exe build --release --locked --target "%RUST_HOST_TARGET%" --manifest-path "%ROOT_DIR%\Cargo.toml" -p codex-plus-launcher
if errorlevel 1 (
  echo [错误] Codex++ Launcher 构建失败。
  exit /b 1
)
if not exist "%XUAN_BRIDGE_BUILD%" (
  echo [错误] 未找到 xuan-bridge.exe 构建产物。
  exit /b 1
)
if not exist "%REMOTE_BRIDGE_BUILD%" (
  echo [错误] 未找到 xuan-plus-remote-bridge.exe 构建产物。
  exit /b 1
)
if not exist "%LAUNCHER_BUILD%" (
  echo [错误] 未找到 codex-plus-plus.exe 构建产物。
  exit /b 1
)

echo [3/7] 安装本地 Bridge...
if not exist "%BIN_DIR%" mkdir "%BIN_DIR%"
if errorlevel 1 (
  echo [错误] 无法创建本地 Bridge 目录。
  exit /b 1
)
call :install_binary "%XUAN_BRIDGE_BUILD%" "%XUAN_BRIDGE_BIN%" xuan-bridge
if errorlevel 1 exit /b 1
call :install_binary "%REMOTE_BRIDGE_BUILD%" "%XUAN_REMOTE_BRIDGE_BIN%" xuan-plus-remote-bridge
if errorlevel 1 exit /b 1
if defined CODEX_PLUS_LAUNCHER_TARGET (
  call :install_launcher "%LAUNCHER_BUILD%" "%CODEX_PLUS_LAUNCHER_TARGET%" "%CODEX_PLUS_INSTALL_DIR%\codex-plus-plus.pending.exe"
  if errorlevel 1 exit /b 1
) else (
  echo   [提醒] 未找到 Codex++ 安装目录，Launcher 构建产物保留在：%LAUNCHER_BUILD%
)

echo [4/7] 初始化功能配置...
setx XUAN_HOME "%XUAN_HOME_DIR%" >nul
if errorlevel 1 (
  echo [错误] 无法写入 XUAN_HOME 用户环境变量。
  exit /b 1
)
setx XUAN_BRIDGE_BIN "%XUAN_BRIDGE_BIN%" >nul
if errorlevel 1 (
  echo [错误] 无法写入 XUAN_BRIDGE_BIN 用户环境变量。
  exit /b 1
)
setx XUAN_MOBILE_BRIDGE_URL "%XUAN_MOBILE_BRIDGE_URL%" >nul
if errorlevel 1 (
  echo [错误] 无法写入 XUAN_MOBILE_BRIDGE_URL 用户环境变量。
  exit /b 1
)
setx XUAN_BRIDGE_ALLOWED_ORIGINS "%XUAN_BRIDGE_ALLOWED_ORIGINS%" >nul
if errorlevel 1 (
  echo [错误] 无法写入 XUAN_BRIDGE_ALLOWED_ORIGINS 用户环境变量。
  exit /b 1
)
"%XUAN_BRIDGE_BIN%" init "%XUAN_HOME_DIR%"
if errorlevel 1 (
  echo [错误] Xuan 功能配置初始化失败。
  exit /b 1
)

echo [5/7] 注册并安装 Codex 插件...
codex.exe plugin marketplace list | findstr.exe /i /b /c:"xuan-curated" >nul
if errorlevel 1 (
  codex.exe plugin marketplace add "%ROOT_DIR%"
  if errorlevel 1 (
    echo [错误] 无法注册 Xuan 本地插件市场。
    exit /b 1
  )
) else (
  echo   [通过] Xuan 本地插件市场已注册。
)
call :install_plugin xuan-workspace-search %XUAN_WORKSPACE_SEARCH_VERSION%
if errorlevel 1 exit /b 1
call :install_plugin xuan-usage %XUAN_USAGE_VERSION%
if errorlevel 1 exit /b 1
call :install_plugin xuan-polish %XUAN_POLISH_VERSION%
if errorlevel 1 exit /b 1

echo [6/7] 安装 Codex++ User Scripts...
if not exist "%CODEX_PLUS_USER_SCRIPT_DIR%" mkdir "%CODEX_PLUS_USER_SCRIPT_DIR%"
if errorlevel 1 (
  echo [错误] 无法创建 Codex++ User Script 目录。
  exit /b 1
)
call :install_user_script "%POLISH_USER_SCRIPT_SOURCE%" "%POLISH_USER_SCRIPT_TARGET%" 文本润色
if errorlevel 1 exit /b 1
call :install_user_script "%SEARCH_USER_SCRIPT_SOURCE%" "%SEARCH_USER_SCRIPT_TARGET%" 工作区搜索
if errorlevel 1 exit /b 1
call :install_user_script "%USAGE_USER_SCRIPT_SOURCE%" "%USAGE_USER_SCRIPT_TARGET%" 用量展示
if errorlevel 1 exit /b 1
"%XUAN_REMOTE_BRIDGE_BIN%" install-user-script
if errorlevel 1 (
  echo [错误] 手机连接 User Script 安装失败。
  exit /b 1
)

echo [7/7] 启动本地 Bridge 服务...
if "%START_SERVICES%"=="1" (
  call :ensure_xuan_http
  if errorlevel 1 exit /b 1
  call :ensure_mobile_bridge
  if errorlevel 1 exit /b 1
) else (
  echo   [跳过] 已按 no-start 参数不启动后台服务。
)

echo.
echo 四项功能已导入：
echo   1. 工作区文件内容搜索：xuan-workspace-search 插件和顶部搜索入口
echo   2. 用量查询：xuan-usage 插件和会话标题栏用量入口
echo   3. 文本润色：xuan-polish 插件和 Composer User Script
echo   4. 手机连接：本地 Remote Bridge 和 User Script
echo.
echo 请重启 Codex++，使新安装的插件、环境变量和 User Script 生效。
if "%LAUNCHER_UPDATE_PENDING%"=="1" echo Launcher 正在被 Codex 占用；请完全退出 Codex++ 后再次执行本安装器，以应用原生桥接更新。
echo 用量查询和文本润色还需在 %XUAN_HOME_DIR%\xuan-plugins.json 配置供应商地址、模型和 API 密钥环境变量。
echo 手机连接还需要已签名的轩++远程 HAP 与已部署的兼容云端服务；本脚本不会处理签名、手机安装或云端部署。
exit /b 0

:require_command
where.exe %~1 >nul 2>&1
if errorlevel 1 (
  echo [错误] 未找到 %~2：%~1
  exit /b 1
)
echo   [通过] %~2
exit /b 0

:install_binary
if exist "%~2" (
  echo   [通过] %~3 已存在，保留现有文件：%~2
  exit /b 0
)
copy "%~1" "%~2" >nul
if errorlevel 1 (
  echo [错误] 无法安装 %~3。
  exit /b 1
)
echo   [通过] 已安装 %~3。
exit /b 0

:install_launcher
if not exist "%~2" (
  echo   [提醒] 未找到已安装的 Codex++ Launcher：%~2
  exit /b 0
)
fc.exe /b "%~1" "%~2" >nul
if not errorlevel 1 (
  echo   [通过] Codex++ Launcher 已是最新版本。
  exit /b 0
)
copy /y "%~1" "%~2" >nul
if not errorlevel 1 (
  if exist "%~3" del /q "%~3"
  echo   [通过] Codex++ Launcher 已更新。
  exit /b 0
)
copy /y "%~1" "%~3" >nul
if errorlevel 1 (
  echo [错误] 无法更新或暂存 Codex++ Launcher。
  exit /b 1
)
set "LAUNCHER_UPDATE_PENDING=1"
echo   [待应用] Codex++ Launcher 正在使用，更新已暂存：%~3
exit /b 0

:install_plugin
codex.exe plugin list | findstr.exe /i /r /c:"%~1@xuan-curated.*installed, enabled.*%~2" >nul
if not errorlevel 1 (
  echo   [通过] %~1 %~2 已安装并启用。
  exit /b 0
)
codex.exe plugin add "%~1@xuan-curated"
if errorlevel 1 (
  codex.exe plugin list | findstr.exe /i /r /c:"%~1@xuan-curated.*installed, enabled.*%~2" >nul
  if errorlevel 1 (
    echo [错误] 无法安装或更新插件：%~1
    exit /b 1
  )
  echo   [通过] %~1 已安装并启用。
  exit /b 0
)
echo   [通过] 已安装并刷新 %~1。
exit /b 0

:install_user_script
if exist "%~2" (
  fc.exe /b "%~1" "%~2" >nul
  if not errorlevel 1 (
    echo   [通过] %~3 User Script 已安装。
    exit /b 0
  )
  copy /y "%~1" "%~2" >nul
  if errorlevel 1 (
    echo [错误] 无法更新 %~3 User Script。
    exit /b 1
  )
  echo   [通过] 已更新 %~3 User Script。
  exit /b 0
)
copy "%~1" "%~2" >nul
if errorlevel 1 (
  echo [错误] 无法安装 %~3 User Script。
  exit /b 1
)
echo   [通过] 已安装 %~3 User Script。
exit /b 0

:ensure_xuan_http
netstat.exe -ano | findstr.exe /r /c:":57324 .*LISTENING" >nul
if not errorlevel 1 (
  echo   [通过] xuan-bridge HTTP 服务已在 127.0.0.1:57324 监听。
  exit /b 0
)
pwsh.exe -NoLogo -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:XUAN_BRIDGE_BIN -ArgumentList '--http','127.0.0.1:57324' -WindowStyle Hidden"
if errorlevel 1 (
  echo [错误] 无法启动 xuan-bridge HTTP 服务。
  exit /b 1
)
call :wait_for_port 57324
if errorlevel 1 (
  echo [错误] xuan-bridge HTTP 服务未能监听 127.0.0.1:57324。
  exit /b 1
)
echo   [通过] xuan-bridge HTTP 服务已启动。
exit /b 0

:ensure_mobile_bridge
netstat.exe -ano | findstr.exe /r /c:":17421 .*LISTENING" >nul
if not errorlevel 1 (
  echo   [通过] 手机连接 Bridge 已在 127.0.0.1:17421 监听。
  exit /b 0
)
pwsh.exe -NoLogo -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:XUAN_REMOTE_BRIDGE_BIN -WindowStyle Hidden"
if errorlevel 1 (
  echo [错误] 无法启动手机连接 Bridge。
  exit /b 1
)
call :wait_for_port 17421
if errorlevel 1 (
  echo [错误] 手机连接 Bridge 未能监听 127.0.0.1:17421。
  exit /b 1
)
echo   [通过] 手机连接 Bridge 已启动。
exit /b 0

:wait_for_port
for /l %%I in (1,1,5) do (
  netstat.exe -ano | findstr.exe /r /c:":%~1 .*LISTENING" >nul
  if not errorlevel 1 exit /b 0
  pwsh.exe -NoLogo -NoProfile -NonInteractive -Command "Start-Sleep -Seconds 1"
)
exit /b 1
