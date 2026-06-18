@echo off
chcp 65001 >nul
title Codex++ Update + Build

set REPO_DIR=C:\Users\fgy\Documents\codex-plus-plus
set MANAGER_DIR=%REPO_DIR%\apps\codex-plus-manager

echo ================================================
echo   Codex++ Auto Update & Build Script
echo   Repo: github.com/BigPizzaV3/CodexPlusPlus
echo   Manager: %MANAGER_DIR%
echo ================================================
echo.

:: ============================================================
:: Step 1: Check for updates (git pull)
:: ============================================================
echo [1/6] Checking GitHub updates ...
cd /d "%REPO_DIR%"

echo   Stashing local changes...
git stash push -m "auto-stash-before-update" 2>nul
if %errorlevel% equ 0 (
    echo   [OK] Local changes stashed
) else (
    echo   [--] No local changes to stash
)

echo   Pulling latest code...
git pull --rebase origin main 2>nul
if %errorlevel% equ 0 (
    echo   [OK] Up to date
) else (
    echo   [!!] Pull failed or already up to date
)

echo   Restoring local changes...
git stash pop 2>nul
if %errorlevel% equ 0 (
    echo   [OK] Local changes restored
) else (
    echo   [--] Nothing to restore (resolve conflicts manually if any)
)

:: ============================================================
:: Step 2: Ensure settings.rs has model_mappings fields
:: ============================================================
echo [2/6] Checking settings.rs for model mapping fields ...
set SETTINGS_FILE=%REPO_DIR%\crates\codex-plus-core\src\settings.rs

findstr "model_mappings_enabled" "%SETTINGS_FILE%" >nul 2>&1
if %errorlevel% neq 0 (
    echo   Adding model_mappings / model_mappings_enabled to settings.rs ...
    powershell -NoProfile -ExecutionPolicy Bypass -File "%MANAGER_DIR%\patch-settings.ps1" "%SETTINGS_FILE%"
    if %errorlevel% equ 0 (
        echo   [OK] settings.rs updated
    ) else (
        echo   [FAIL] settings.rs patch failed
    )
) else (
    echo   [OK] settings.rs already has model mapping fields
)

:: ============================================================
:: Step 3: Ensure protocol_proxy.rs has model rewriting logic
:: ============================================================
echo [3/6] Checking protocol_proxy.rs for model rewriting logic ...
set PROXY_FILE=%REPO_DIR%\crates\codex-plus-core\src\protocol_proxy.rs

findstr "Model name rewriting" "%PROXY_FILE%" >nul 2>&1
if %errorlevel% neq 0 (
    echo   Adding model name rewriting to protocol_proxy.rs ...
    powershell -NoProfile -ExecutionPolicy Bypass -File "%MANAGER_DIR%\patch-proxy.ps1" "%PROXY_FILE%"
    if %errorlevel% equ 0 (
        echo   [OK] protocol_proxy.rs updated
    ) else (
        echo   [FAIL] protocol_proxy.rs patch failed
    )
) else (
    echo   [OK] protocol_proxy.rs already has model rewriting logic
)

:: ============================================================
:: Step 4: Build frontend (Vite)
:: ============================================================
echo [4/6] Building Manager frontend ...
cd /d "%MANAGER_DIR%"
if not exist "node_modules" (
    echo   Installing npm dependencies...
    call npm install
)
call npx vite build
if %errorlevel% neq 0 (
    echo   [FAIL] Frontend build failed
    pause
    exit /b 1
)
echo   [OK] Frontend build complete

:: ============================================================
:: Step 5: Build Rust backend
:: ============================================================
echo [5/6] Compiling Rust backend ...
cd /d "%REPO_DIR%"

echo   Building codex-plus-core ...
cargo build --release -p codex-plus-core
if %errorlevel% neq 0 (
    echo   [FAIL] codex-plus-core build failed
    pause
    exit /b 1
)

echo   Building codex-plus-launcher ...
cargo build --release -p codex-plus-launcher
if %errorlevel% neq 0 (
    echo   [FAIL] codex-plus-launcher build failed
    pause
    exit /b 1
)

echo   Building codex-plus-manager ...
cargo build --release -p codex-plus-manager
if %errorlevel% neq 0 (
    echo   [FAIL] codex-plus-manager build failed
    pause
    exit /b 1
)

echo   [OK] Rust compilation complete

:: ============================================================
:: Step 6: Copy binaries to Manager dir
:: ============================================================
echo [6/6] Copying build artifacts to manager directory ...
copy /Y "%REPO_DIR%\target\release\codex-plus-plus.exe" "%MANAGER_DIR%\codex-plus-plus.exe"
copy /Y "%REPO_DIR%\target\release\codex-plus-plus-manager.exe" "%MANAGER_DIR%\codex-plus-plus-manager.exe"
echo   [OK] Copied to %MANAGER_DIR%

echo.
echo ================================================
echo   All done!
echo   Output:
echo     %MANAGER_DIR%\codex-plus-plus.exe
echo     %MANAGER_DIR%\codex-plus-plus-manager.exe
echo ================================================
echo.
echo   Press any key to exit...
pause >nul