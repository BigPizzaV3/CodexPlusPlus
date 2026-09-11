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
set "XUAN_BRIDGE_MANIFEST=%ROOT_DIR%\tools\xuan-bridge\Cargo.toml"
set "REMOTE_BRIDGE_MANIFEST=%ROOT_DIR%\apps\xuan-plus-remote\bridge\Cargo.toml"
set "XUAN_BRIDGE_BUILD=%ROOT_DIR%\tools\xuan-bridge\target\release\xuan-bridge.exe"
set "REMOTE_BRIDGE_BUILD=%ROOT_DIR%\apps\xuan-plus-remote\bridge\target\release\xuan-plus-remote-bridge.exe"
set "XUAN_BRIDGE_BIN=%BIN_DIR%\xuan-bridge.exe"
set "XUAN_REMOTE_BRIDGE_BIN=%BIN_DIR%\xuan-plus-remote-bridge.exe"
set "XUAN_MOBILE_BRIDGE_URL=http://127.0.0.1:17421"

echo [1/7] 检查导入环境...
call :require_command node.exe Node.js
if errorlevel 1 exit /b 1
call :require_command cargo.exe Rust
if errorlevel 1 exit /b 1
call :require_command codex.exe Codex CLI
if errorlevel 1 exit /b 1
call :require_command rg.exe ripgrep
if errorlevel 1 exit /b 1
if not exist "%XUAN_BRIDGE_MANIFEST%" (
  echo [错误] 未找到 xuan-bridge 清单文件。
  exit /b 1
)
if not exist "%REMOTE_BRIDGE_MANIFEST%" (
  echo [错误] 未找到手机连接 Bridge 清单文件。
  exit /b 1
)
if not exist "%ROOT_DIR%\.agents\plugins\marketplace.json" (
  echo [错误] 未找到 Xuan 插件市场清单。
  exit /b 1
)

if /i "%MODE%"=="check" (
  echo 环境检查通过，可以导入四项功能。
  exit /b 0
)

echo [2/7] 构建本地 Bridge...
cargo.exe build --release --locked --manifest-path "%XUAN_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [错误] xuan-bridge 构建失败。
  exit /b 1
)
cargo.exe build --release --locked --manifest-path "%REMOTE_BRIDGE_MANIFEST%"
if errorlevel 1 (
  echo [错误] 手机连接 Bridge 构建失败。
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
call :install_plugin xuan-workspace-search
if errorlevel 1 exit /b 1
call :install_plugin xuan-usage
if errorlevel 1 exit /b 1
call :install_plugin xuan-polish
if errorlevel 1 exit /b 1

echo [6/7] 安装手机连接 User Script...
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
echo   1. 工作区文件内容搜索：xuan-workspace-search 插件
echo   2. 用量查询：xuan-usage 插件
echo   3. 文本润色：xuan-polish 插件和 Composer User Script
echo   4. 手机连接：本地 Remote Bridge 和 User Script
echo.
echo 请重启 Codex++，使新安装的插件、环境变量和 User Script 生效。
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

:install_plugin
codex.exe plugin list | findstr.exe /i /r /c:"%~1@xuan-curated.*installed, enabled" >nul
if not errorlevel 1 (
  echo   [通过] %~1 已安装并启用。
  exit /b 0
)
codex.exe plugin add "%~1@xuan-curated"
if errorlevel 1 (
  echo [错误] 无法安装插件：%~1
  exit /b 1
)
echo   [通过] 已安装 %~1。
exit /b 0

:ensure_xuan_http
netstat.exe -ano | findstr.exe /r /c:":57324 .*LISTENING" >nul
if not errorlevel 1 (
  echo   [通过] xuan-bridge HTTP 服务已在 127.0.0.1:57324 监听。
  exit /b 0
)
powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:XUAN_BRIDGE_BIN -ArgumentList '--http','127.0.0.1:57324' -WindowStyle Hidden"
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
powershell.exe -NoProfile -NonInteractive -Command "Start-Process -FilePath $env:XUAN_REMOTE_BRIDGE_BIN -WindowStyle Hidden"
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
  timeout.exe /t 1 /nobreak >nul
)
exit /b 1
