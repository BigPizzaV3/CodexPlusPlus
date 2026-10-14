param(
    [switch]$Full,
    [switch]$Build,
    [switch]$SkipSymlinkPrivilegeTest
)

$ErrorActionPreference = 'Stop'
$verificationRoot = Split-Path -Parent $PSScriptRoot
$verificationLogDir = Join-Path $verificationRoot 'target/429-validation'
New-Item -ItemType Directory -Force -Path $verificationLogDir | Out-Null
$verificationLog = Join-Path $verificationLogDir ('verification-' + (Get-Date -Format 'yyyyMMdd-HHmmss') + '.log')

function Invoke-VerificationCommand {
    param([string]$Command, [string[]]$Arguments)
    Write-Host ('> ' + $Command + ' ' + ($Arguments -join ' '))
    & $Command @Arguments
    if ($LASTEXITCODE -ne 0) {
        throw "$Command failed with exit code $LASTEXITCODE"
    }
}

Start-Transcript -Path $verificationLog | Out-Null
Push-Location $verificationRoot
try {
    Invoke-VerificationCommand -Command 'node' -Arguments @('--version')
    Invoke-VerificationCommand -Command 'node' -Arguments @('scripts/assemble-renderer-inject.mjs', '--check')
    Invoke-VerificationCommand -Command 'node' -Arguments @('scripts/assemble-native-browser-inspector.mjs', '--check')

    Push-Location (Join-Path $verificationRoot 'apps/codex-plus-manager')
    try {
        if ($Full) {
            Invoke-VerificationCommand -Command 'npm.cmd' -Arguments @('test')
            Invoke-VerificationCommand -Command 'npm.cmd' -Arguments @('run', 'check')
        } else {
            Invoke-VerificationCommand -Command 'node' -Arguments @('--test', 'src/rate-limit-auto-resume.test.ts', 'src/capacity-auto-retry.test.ts')
        }
        if ($Build) {
            Invoke-VerificationCommand -Command 'npm.cmd' -Arguments @('run', 'vite:build')
        }
    } finally {
        Pop-Location
    }

    if ($Full) {
        if ($SkipSymlinkPrivilegeTest) {
            Write-Host 'Explicit local exclusion: app_paths_resolves_portable_current_link_to_directory_version (requires Windows symlink privilege). This is not a full CI pass.'
            Invoke-VerificationCommand -Command 'cargo' -Arguments @('test', '--workspace', '--', '--skip', 'app_paths_resolves_portable_current_link_to_directory_version')
        } else {
            Invoke-VerificationCommand -Command 'cargo' -Arguments @('test', '--workspace')
        }
    } else {
        Invoke-VerificationCommand -Command 'cargo' -Arguments @('test', '-p', 'codex-plus-core', '--test', 'rate_limit_auto_resume_settings', '--test', 'capacity_auto_retry_settings')
    }
    if ($Build) {
        Invoke-VerificationCommand -Command 'cargo' -Arguments @('build', '--release')
    }
    Write-Host 'Requested local checks passed. macOS DMG packaging and real Codex 429 recovery are separate checks.'
    Write-Host "Log: $verificationLog"
} finally {
    Pop-Location
    Stop-Transcript | Out-Null
}
