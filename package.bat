@echo off
chcp 65001 >nul
setlocal EnableExtensions DisableDelayedExpansion

cd /d "%~dp0"
if errorlevel 1 (
  echo [错误] 无法进入项目根目录。
  exit /b 1
)

set "ROOT_DIR=%CD%"
set "MANAGER_DIR=%ROOT_DIR%\apps\codex-plus-manager"
set "WINDOWS_DIST=%ROOT_DIR%\dist\windows"
set "APP_DIST=%WINDOWS_DIST%\app"
set "PACKAGE_TARGET_DIR=%ROOT_DIR%\target\package"
set "NSIS_DIR=%ROOT_DIR%\scripts\installer\windows"
set "CHECK_ONLY=0"

if /i "%~1"=="check" set "CHECK_ONLY=1"
if not "%~1"=="" if /i not "%~1"=="check" (
  echo [错误] 不支持的参数：%~1
  echo 用法：package.bat [check]
  exit /b 1
)

echo [1/6] 检查打包环境...
where.exe node.exe >nul 2>&1
if errorlevel 1 (
  echo [错误] 未找到 Node.js，请先安装 Node.js 22 或更高版本。
  exit /b 1
)
where.exe npm.cmd >nul 2>&1
if errorlevel 1 (
  echo [错误] 未找到 npm，请检查 Node.js 安装。
  exit /b 1
)
where.exe cargo.exe >nul 2>&1
if errorlevel 1 (
  echo [错误] 未找到 Cargo，请先安装 Rust stable 工具链。
  exit /b 1
)
where.exe tar.exe >nul 2>&1
if errorlevel 1 (
  echo [错误] 未找到 tar.exe，无法生成便携版 ZIP。
  exit /b 1
)

set "MAKENSIS="
if defined LOCALAPPDATA if exist "%LOCALAPPDATA%\tauri\NSIS\makensis.exe" set "MAKENSIS=%LOCALAPPDATA%\tauri\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%ProgramFiles(x86)%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles(x86)%\NSIS\makensis.exe"
if not defined MAKENSIS if exist "%ProgramFiles%\NSIS\makensis.exe" set "MAKENSIS=%ProgramFiles%\NSIS\makensis.exe"
if not defined MAKENSIS for /f "delims=" %%I in ('where.exe makensis.exe 2^>nul') do if not defined MAKENSIS set "MAKENSIS=%%I"
if not defined MAKENSIS (
  echo [错误] 未找到 NSIS。已检查 Tauri 缓存、系统安装目录和 PATH。
  echo         请先通过 Tauri 下载 NSIS 工具，或单独安装 NSIS。
  exit /b 1
)
"%MAKENSIS%" /VERSION >nul 2>&1
if errorlevel 1 (
  echo [错误] 找到的 NSIS 工具无法正常运行。
  exit /b 1
)

set "VERSION="
for /f "delims=" %%V in ('node.exe -p "require('./apps/codex-plus-manager/package.json').version"') do set "VERSION=%%V"
if not defined VERSION (
  echo [错误] 无法从 package.json 读取版本号。
  exit /b 1
)
echo   [通过] 项目版本：%VERSION%
echo   [通过] NSIS 工具：%MAKENSIS%

if "%CHECK_ONLY%"=="1" (
  echo 环境检查通过，可以开始打包。
  exit /b 0
)

echo [2/6] 检查前端依赖...
pushd "%MANAGER_DIR%"
if errorlevel 1 (
  echo [错误] 无法进入前端目录。
  exit /b 1
)
if not exist "node_modules\." (
  popd
  echo [错误] 未找到前端依赖。请先在 apps\codex-plus-manager 中执行 npm ci。
  exit /b 1
)
echo   [通过] 使用现有前端依赖。

echo [3/6] 构建前端...
call npm.cmd run vite:build
if errorlevel 1 (
  popd
  echo [错误] 前端构建失败。
  exit /b 1
)
popd

echo [4/6] 构建 Release 程序...
echo   使用独立输出目录，避免运行中的程序锁定构建产物。
set "TAURI_PERMISSION_CACHE_CORRUPT="
if exist "%PACKAGE_TARGET_DIR%\release\build\." (
  for /r "%PACKAGE_TARGET_DIR%\release\build" %%F in (*.toml) do call :check_tauri_permission_cache "%%~fF"
)
if defined TAURI_PERMISSION_CACHE_CORRUPT (
  set "PACKAGE_TARGET_DIR=%ROOT_DIR%\target\package-%COMPUTERNAME%"
  echo   [修复] 检测到损坏的 Tauri 权限缓存，改用本机隔离目录重新构建。
)
set "CARGO_TARGET_DIR=%PACKAGE_TARGET_DIR%"
cargo.exe build --release
if errorlevel 1 (
  echo [错误] Rust Release 构建失败。
  exit /b 1
)

if not exist "%PACKAGE_TARGET_DIR%\release\codex-plus-plus.exe" (
  echo [错误] 未找到 codex-plus-plus.exe 构建产物。
  exit /b 1
)
if not exist "%PACKAGE_TARGET_DIR%\release\codex-plus-plus-manager.exe" (
  echo [错误] 未找到 codex-plus-plus-manager.exe 构建产物。
  exit /b 1
)

echo [5/6] 生成便携版 ZIP...
if not exist "%APP_DIST%" (
  mkdir "%APP_DIST%"
  if errorlevel 1 (
    echo [错误] 无法创建打包目录。
    exit /b 1
  )
)
copy /Y "%PACKAGE_TARGET_DIR%\release\codex-plus-plus.exe" "%APP_DIST%\" >nul
if errorlevel 1 (
  echo [错误] 复制 codex-plus-plus.exe 失败。
  exit /b 1
)
copy /Y "%PACKAGE_TARGET_DIR%\release\codex-plus-plus-manager.exe" "%APP_DIST%\" >nul
if errorlevel 1 (
  echo [错误] 复制 codex-plus-plus-manager.exe 失败。
  exit /b 1
)
set "ZIP_PATH=%WINDOWS_DIST%\CodexPlusPlus-%VERSION%-windows-x64.zip"
tar.exe -a -c -f "%ZIP_PATH%" -C "%APP_DIST%" codex-plus-plus.exe codex-plus-plus-manager.exe
if errorlevel 1 (
  echo [错误] 便携版 ZIP 生成失败。
  exit /b 1
)
if not exist "%ZIP_PATH%" (
  echo [错误] 未找到便携版 ZIP 产物。
  exit /b 1
)

echo [6/6] 生成 NSIS 安装包...
pushd "%NSIS_DIR%"
if errorlevel 1 (
  echo [错误] 无法进入 NSIS 脚本目录。
  exit /b 1
)
"%MAKENSIS%" /INPUTCHARSET UTF8 "/DVERSION=%VERSION%" CodexPlusPlus.nsi
if errorlevel 1 (
  popd
  echo [错误] NSIS 安装包生成失败。
  exit /b 1
)
popd
set "SETUP_PATH=%WINDOWS_DIST%\CodexPlusPlus-%VERSION%-windows-x64-setup.exe"
if not exist "%SETUP_PATH%" (
  echo [错误] 未找到 NSIS 安装包产物。
  exit /b 1
)

echo.
echo 打包完成：
echo   便携版：%ZIP_PATH%
echo   安装包：%SETUP_PATH%
exit /b 0

:check_tauri_permission_cache
set "TAURI_PERMISSION_FILE=%~1"
if /i "%TAURI_PERMISSION_FILE:\out\permissions\=%"=="%TAURI_PERMISSION_FILE%" exit /b 0
findstr.exe /L /C:"[" "%~1" >nul 2>&1
if errorlevel 1 set "TAURI_PERMISSION_CACHE_CORRUPT=1"
exit /b 0
