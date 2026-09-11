param(
  [string]$HdcPath = '',

  [string]$HarmonySdkRoot = '',

  [switch]$ResolveOnly,

  [string]$HapPath = '',

  [string]$TargetId = ''
)

$ErrorActionPreference = 'Stop'

function Test-HdcPath {
  param([string]$Candidate)

  return -not [string]::IsNullOrWhiteSpace($Candidate) -and [IO.File]::Exists($Candidate)
}

function Test-HdcInSdkRoot {
  param([string]$SdkRoot)

  if ([string]::IsNullOrWhiteSpace($SdkRoot) -or -not [IO.Directory]::Exists($SdkRoot)) {
    return $false
  }
  return (Test-HdcPath -Candidate ([IO.Path]::Combine($SdkRoot, 'default\toolchains\hdc.exe'))) -or
    (Test-HdcPath -Candidate ([IO.Path]::Combine($SdkRoot, 'default\openharmony\toolchains\hdc.exe')))
}

function Resolve-HdcInSdkRoot {
  param([string]$SdkRoot)

  $defaultToolchainsHdc = [IO.Path]::Combine($SdkRoot, 'default\toolchains\hdc.exe')
  $openHarmonyToolchainsHdc = [IO.Path]::Combine($SdkRoot, 'default\openharmony\toolchains\hdc.exe')
  if (Test-HdcPath -Candidate $defaultToolchainsHdc) {
    return (Resolve-Path -LiteralPath $defaultToolchainsHdc).Path
  } elseif (Test-HdcPath -Candidate $openHarmonyToolchainsHdc) {
    return (Resolve-Path -LiteralPath $openHarmonyToolchainsHdc).Path
  }
  throw "HDC toolchain was not found under SDK root: $SdkRoot"
}

function Resolve-HdcPath {
  param(
    [string]$ExplicitHdcPath,
    [string]$ExplicitHarmonySdkRoot
  )

  $userProfileDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
  $alternateDevEcoSdkRoot = 'E:\Program Files\Huawei\DevEco Studio\sdk'
  $perUserDevEcoSdkRoot = Join-Path $userProfileDirectory 'DevEco Studio\sdk'
  $programFilesDevEcoSdkRoot = 'C:\Program Files\Huawei\DevEco Studio\sdk'
  $perUserHarmonySdkRoot = Join-Path $userProfileDirectory 'AppData\Local\Huawei\Sdk'
  $environmentDevEcoSdkRoot = if ([string]::IsNullOrWhiteSpace($env:DEVECO_ROOT)) {
    ''
  } else {
    [IO.Path]::Combine($env:DEVECO_ROOT, 'sdk')
  }

  if (Test-HdcPath -Candidate $ExplicitHdcPath) {
    return (Resolve-Path -LiteralPath $ExplicitHdcPath).Path
  } elseif (Test-HdcInSdkRoot -SdkRoot $ExplicitHarmonySdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $ExplicitHarmonySdkRoot
  } elseif (Test-HdcInSdkRoot -SdkRoot $env:DEVECO_SDK_HOME) {
    return Resolve-HdcInSdkRoot -SdkRoot $env:DEVECO_SDK_HOME
  } elseif (Test-HdcInSdkRoot -SdkRoot $env:OHOS_BASE_SDK_HOME) {
    return Resolve-HdcInSdkRoot -SdkRoot $env:OHOS_BASE_SDK_HOME
  } elseif (Test-HdcInSdkRoot -SdkRoot $environmentDevEcoSdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $environmentDevEcoSdkRoot
  } elseif (Test-HdcInSdkRoot -SdkRoot $alternateDevEcoSdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $alternateDevEcoSdkRoot
  } elseif (Test-HdcInSdkRoot -SdkRoot $perUserDevEcoSdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $perUserDevEcoSdkRoot
  } elseif (Test-HdcInSdkRoot -SdkRoot $programFilesDevEcoSdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $programFilesDevEcoSdkRoot
  } elseif (Test-HdcInSdkRoot -SdkRoot $perUserHarmonySdkRoot) {
    return Resolve-HdcInSdkRoot -SdkRoot $perUserHarmonySdkRoot
  } else {
    throw 'HDC 工具链未找到。请传入 -HdcPath，或传入 -HarmonySdkRoot / 设置 DEVECO_SDK_HOME 或 OHOS_BASE_SDK_HOME。'
  }
}

$resolvedHdcPath = Resolve-HdcPath -ExplicitHdcPath $HdcPath -ExplicitHarmonySdkRoot $HarmonySdkRoot
if ($ResolveOnly) {
  Write-Output "resolvedHdcPath=$resolvedHdcPath"
  exit 0
}
if ([string]::IsNullOrWhiteSpace($HapPath)) {
  throw 'HapPath is required unless -ResolveOnly is specified.'
}
$resolvedHapPath = (Resolve-Path -LiteralPath $HapPath).Path
$expectedHapPath = Join-Path $PSScriptRoot 'entry\build\default\outputs\default\entry-default-signed.hap'
$resolvedExpectedHapPath = (Resolve-Path -LiteralPath $expectedHapPath).Path
if (-not [string]::Equals($resolvedHapPath, $resolvedExpectedHapPath, [System.StringComparison]::OrdinalIgnoreCase)) {
  throw "Refusing to install an unregistered HAP: $resolvedHapPath"
}

$bundleName = 'com.dyys.workagents.remote.dev'
$hapArchive = [System.IO.Compression.ZipFile]::OpenRead($resolvedHapPath)
try {
  $moduleEntries = @($hapArchive.Entries | Where-Object { $_.FullName -eq 'module.json' })
  if ($moduleEntries.Count -ne 1) {
    throw "Expected one module.json in the development HAP, found $($moduleEntries.Count)."
  }
  $moduleReader = [System.IO.StreamReader]::new($moduleEntries[0].Open(), [System.Text.Encoding]::UTF8)
  try {
    $moduleProfile = $moduleReader.ReadToEnd() | ConvertFrom-Json
  } finally {
    $moduleReader.Dispose()
  }
  if (-not [string]::Equals($moduleProfile.app.bundleName, $bundleName, [System.StringComparison]::Ordinal)) {
    throw 'Refusing to install a HAP whose embedded bundle name is not the registered development bundle.'
  }
} finally {
  $hapArchive.Dispose()
}

$targets = @(& $resolvedHdcPath 'list' 'targets')
$listExitCode = $LASTEXITCODE
if ($listExitCode -ne 0) {
  exit $listExitCode
}
$connectedTargets = @($targets | Where-Object { -not [string]::IsNullOrWhiteSpace($_) -and $_.Trim() -ne '[Empty]' })
$requestedTargetId = $TargetId.Trim()
if ([string]::IsNullOrWhiteSpace($requestedTargetId)) {
  if ($connectedTargets.Count -ne 1) {
    throw "Expected exactly one connected target when TargetId is omitted, found $($connectedTargets.Count)."
  }
  $selectedTargetId = $connectedTargets[0].Trim()
} else {
  $matchingTargets = @($connectedTargets | Where-Object {
    [string]::Equals($_.Trim(), $requestedTargetId, [System.StringComparison]::Ordinal)
  })
  if ($matchingTargets.Count -ne 1) {
    throw '指定的设备不是唯一匹配的已连接目标。'
  }
  $selectedTargetId = $requestedTargetId
}

$installArgs = @('-t', $selectedTargetId, 'install', '-r', $resolvedHapPath)
$installOutput = (& $resolvedHdcPath @installArgs | Out-String).Trim()
$installExitCode = $LASTEXITCODE
Write-Output $installOutput
if ($installExitCode -ne 0) {
  exit $installExitCode
}
if ($installOutput -match '(?i)error:|failed to install|no signature' -or $installOutput -notmatch '(?i)success') {
  throw 'HDC did not report a successful HAP installation.'
}

$startArgs = @('-t', $selectedTargetId, 'shell', 'aa', 'start', '-a', 'EntryAbility', '-b', $bundleName)
$startOutput = (& $resolvedHdcPath @startArgs | Out-String).Trim()
$startExitCode = $LASTEXITCODE
Write-Output $startOutput
if ($startExitCode -ne 0) {
  exit $startExitCode
}
if ($startOutput -match '(?i)error|failed') {
  throw 'The installed development bundle did not start successfully.'
}

$processObserved = $false
$observationDeadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
do {
  $pidArgs = @('-t', $selectedTargetId, 'shell', 'pidof', $bundleName)
  $pidOutput = (& $resolvedHdcPath @pidArgs | Out-String).Trim()
  $pidExitCode = $LASTEXITCODE
  if ($pidExitCode -ne 0) {
    exit $pidExitCode
  }
  if ($pidOutput -match '^\d+(\s+\d+)*$') {
    $processObserved = $true
    break
  }

  $abilityDumpArgs = @('-t', $selectedTargetId, 'shell', 'aa', 'dump', '-a', $bundleName)
  $abilityDumpOutput = (& $resolvedHdcPath @abilityDumpArgs | Out-String).Trim()
  $abilityDumpExitCode = $LASTEXITCODE
  if ($abilityDumpExitCode -ne 0) {
    exit $abilityDumpExitCode
  }
  if (
    $abilityDumpOutput -match [regex]::Escape($bundleName) -and
    $abilityDumpOutput -match 'AppRunningRecord ID #[0-9]+' -and
    $abilityDumpOutput -match 'pid #[0-9]+'
  ) {
    $processObserved = $true
    break
  }
} while ([DateTimeOffset]::UtcNow -lt $observationDeadline)

if (-not $processObserved) {
  throw 'The development bundle started but no live process or AppRunningRecord was observed.'
}
Write-Output "installedBundle=$bundleName"
Write-Output 'installedTarget=redacted'
Write-Output "processObserved=true"
exit 0
