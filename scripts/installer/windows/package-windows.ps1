param(
  [string]$Version = "",
  [switch]$SkipInstall,
  [switch]$SkipBuild,
  [switch]$SkipInstaller
)

$ErrorActionPreference = "Stop"

function Run-Step {
  param(
    [string]$Label,
    [scriptblock]$Command
  )
  Write-Host "==> $Label"
  & $Command
  if ($LASTEXITCODE -ne 0) {
    throw "$Label failed with exit code $LASTEXITCODE"
  }
}

$Root = (Resolve-Path (Join-Path $PSScriptRoot "..\..\..")).Path
$AppOut = Join-Path $Root "dist\windows\app"
$TaskboardOut = Join-Path $AppOut "codex-taskboard"

Push-Location $Root
try {
  if (-not $Version) {
    $Version = (Get-Content "apps\codex-plus-manager\package.json" -Raw | ConvertFrom-Json).version
  }

  if (-not $SkipInstall) {
    Run-Step "Install manager dependencies" {
      npm --prefix apps/codex-plus-manager install --package-lock=false
    }
    Run-Step "Install Taskboard dependencies" {
      npm --prefix apps/codex-taskboard install --package-lock=false
    }
  }

  if (-not $SkipBuild) {
    Run-Step "Build manager frontend" {
      npm --prefix apps/codex-plus-manager run vite:build
    }
    Run-Step "Build Taskboard frontend" {
      npm --prefix apps/codex-taskboard run build:web
    }
    Run-Step "Build Windows binaries" {
      cargo build --release -p codex-plus-launcher -p codex-plus-manager
    }
  }

  New-Item -ItemType Directory -Force $AppOut | Out-Null
  Copy-Item "target\release\codex-plus-plus.exe" $AppOut -Force
  Copy-Item "target\release\codex-plus-plus-manager.exe" $AppOut -Force

  if (Test-Path $TaskboardOut) {
    $ResolvedTaskboardOut = (Resolve-Path $TaskboardOut).Path
    $ResolvedAppOut = (Resolve-Path $AppOut).Path
    if (-not $ResolvedTaskboardOut.StartsWith($ResolvedAppOut, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Refusing to clean unexpected Taskboard staging path: $ResolvedTaskboardOut"
    }
    Remove-Item -LiteralPath $TaskboardOut -Recurse -Force
  }
  New-Item -ItemType Directory -Force $TaskboardOut | Out-Null
  foreach ($Entry in @("dist", "inject", "scripts", "server", "shared", "skills")) {
    Copy-Item "apps\codex-taskboard\$Entry" (Join-Path $TaskboardOut $Entry) -Recurse -Force
  }

  foreach ($Required in @(
    "codex-taskboard\scripts\codex-injector.mjs",
    "codex-taskboard\dist\web\index.html",
    "codex-taskboard\server\index.mjs"
  )) {
    $Path = Join-Path $AppOut $Required
    if (-not (Test-Path $Path)) {
      throw "Missing staged file: $Path"
    }
  }

  if (-not $SkipInstaller) {
    $Makensis = "${env:ProgramFiles(x86)}\NSIS\makensis.exe"
    if (-not (Test-Path $Makensis)) {
      $MakensisCommand = Get-Command makensis -ErrorAction SilentlyContinue
      if (-not $MakensisCommand) {
        throw "NSIS makensis was not found. Install NSIS first, for example: winget install NSIS.NSIS"
      }
      $Makensis = $MakensisCommand.Source
    }

    Push-Location "scripts\installer\windows"
    try {
      Run-Step "Build Windows installer" {
        & $Makensis "/INPUTCHARSET" "UTF8" "/DVERSION=$Version" CodexPlusPlus.nsi
      }
    } finally {
      Pop-Location
    }
  }

  Write-Host "Staged: $AppOut"
  if (-not $SkipInstaller) {
    Write-Host "Installer: $(Join-Path $Root "dist\windows\CodexPlusPlus-$Version-windows-x64-setup.exe")"
  }
} finally {
  Pop-Location
}
