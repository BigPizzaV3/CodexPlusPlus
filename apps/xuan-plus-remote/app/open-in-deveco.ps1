param(
  [string]$DevEcoExecutable = '',

  [switch]$ResolveOnly
)

$ErrorActionPreference = 'Stop'

function Test-DevEcoExecutable {
  param([string]$Candidate)

  return -not [string]::IsNullOrWhiteSpace($Candidate) -and [IO.File]::Exists($Candidate)
}

$userProfileDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
$environmentDevEcoExecutable = if ([string]::IsNullOrWhiteSpace($env:DEVECO_ROOT)) {
  ''
} else {
  [IO.Path]::Combine($env:DEVECO_ROOT, 'bin\devecostudio64.exe')
}
$alternateDevEcoExecutable = 'E:\Program Files\Huawei\DevEco Studio\bin\devecostudio64.exe'
$perUserDevEcoExecutable = Join-Path $userProfileDirectory 'DevEco Studio\bin\devecostudio64.exe'
$programFilesDevEcoExecutable = 'C:\Program Files\Huawei\DevEco Studio\bin\devecostudio64.exe'

if (Test-DevEcoExecutable -Candidate $DevEcoExecutable) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $DevEcoExecutable).Path
} elseif (Test-DevEcoExecutable -Candidate $env:DEVECO_EXECUTABLE) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $env:DEVECO_EXECUTABLE).Path
} elseif (Test-DevEcoExecutable -Candidate $environmentDevEcoExecutable) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $environmentDevEcoExecutable).Path
} elseif (Test-DevEcoExecutable -Candidate $alternateDevEcoExecutable) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $alternateDevEcoExecutable).Path
} elseif (Test-DevEcoExecutable -Candidate $perUserDevEcoExecutable) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $perUserDevEcoExecutable).Path
} elseif (Test-DevEcoExecutable -Candidate $programFilesDevEcoExecutable) {
  $resolvedDevEcoExecutable = (Resolve-Path -LiteralPath $programFilesDevEcoExecutable).Path
} else {
  throw 'DevEco Studio 未找到。请传入 -DevEcoExecutable，或设置 DEVECO_EXECUTABLE / DEVECO_ROOT。'
}

if ($ResolveOnly) {
  Write-Output "resolvedDevEcoExecutable=$resolvedDevEcoExecutable"
  exit 0
}

$projectPath = (Resolve-Path -LiteralPath $PSScriptRoot).Path
Start-Process -FilePath $resolvedDevEcoExecutable -ArgumentList @("`"$projectPath`"") -WindowStyle Hidden
exit 0
