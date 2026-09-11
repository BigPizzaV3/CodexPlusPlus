param(
  [string]$DevEcoRoot = '',

  [string]$HarmonySdkRoot = '',

  [string]$SigningConfigSource = ''
)

$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$OutputEncoding = [Console]::OutputEncoding

function Test-DevEcoRoot {
  param([string]$Candidate)

  if ([string]::IsNullOrWhiteSpace($Candidate) -or -not [IO.Directory]::Exists($Candidate)) {
    return $false
  }
  return [IO.File]::Exists([IO.Path]::Combine($Candidate, 'tools\node\node.exe')) -and
    [IO.File]::Exists([IO.Path]::Combine($Candidate, 'tools\hvigor\bin\hvigorw.js')) -and
    [IO.Directory]::Exists([IO.Path]::Combine($Candidate, 'jbr\bin'))
}

function Test-HarmonySdkRoot {
  param([string]$Candidate)

  if ([string]::IsNullOrWhiteSpace($Candidate) -or -not [IO.Directory]::Exists($Candidate)) {
    return $false
  }
  return [IO.File]::Exists([IO.Path]::Combine($Candidate, 'default\sdk-pkg.json')) -and
    [IO.File]::Exists([IO.Path]::Combine($Candidate, 'default\hms\ets\kits\@kit.ScanKit.d.ts')) -and
    [IO.File]::Exists([IO.Path]::Combine($Candidate, 'default\hms\ets\kits\@kit.PushKit.d.ts'))
}

$userProfileDirectory = [Environment]::GetFolderPath([Environment+SpecialFolder]::UserProfile)
$alternateDevEcoRoot = 'E:\Program Files\Huawei\DevEco Studio'
$perUserDevEcoRoot = Join-Path $userProfileDirectory 'DevEco Studio'
$programFilesDevEcoRoot = 'C:\Program Files\Huawei\DevEco Studio'
if (Test-DevEcoRoot -Candidate $DevEcoRoot) {
  $resolvedDevEcoRoot = (Resolve-Path -LiteralPath $DevEcoRoot).Path
} elseif (Test-DevEcoRoot -Candidate $env:DEVECO_ROOT) {
  $resolvedDevEcoRoot = (Resolve-Path -LiteralPath $env:DEVECO_ROOT).Path
} elseif (Test-DevEcoRoot -Candidate $alternateDevEcoRoot) {
  $resolvedDevEcoRoot = (Resolve-Path -LiteralPath $alternateDevEcoRoot).Path
} elseif (Test-DevEcoRoot -Candidate $perUserDevEcoRoot) {
  $resolvedDevEcoRoot = (Resolve-Path -LiteralPath $perUserDevEcoRoot).Path
} elseif (Test-DevEcoRoot -Candidate $programFilesDevEcoRoot) {
  $resolvedDevEcoRoot = (Resolve-Path -LiteralPath $programFilesDevEcoRoot).Path
} else {
  throw 'DevEco Studio 未找到。请完成本机安装后传入 -DevEcoRoot，或设置 DEVECO_ROOT。'
}

$sdkInsideDevEcoRoot = Join-Path $resolvedDevEcoRoot 'sdk'
$perUserHarmonySdkRoot = Join-Path $userProfileDirectory 'AppData\Local\Huawei\Sdk'
if (Test-HarmonySdkRoot -Candidate $HarmonySdkRoot) {
  $resolvedHarmonySdkRoot = (Resolve-Path -LiteralPath $HarmonySdkRoot).Path
} elseif (Test-HarmonySdkRoot -Candidate $env:DEVECO_SDK_HOME) {
  $resolvedHarmonySdkRoot = (Resolve-Path -LiteralPath $env:DEVECO_SDK_HOME).Path
} elseif (Test-HarmonySdkRoot -Candidate $env:OHOS_BASE_SDK_HOME) {
  $resolvedHarmonySdkRoot = (Resolve-Path -LiteralPath $env:OHOS_BASE_SDK_HOME).Path
} elseif (Test-HarmonySdkRoot -Candidate $sdkInsideDevEcoRoot) {
  $resolvedHarmonySdkRoot = (Resolve-Path -LiteralPath $sdkInsideDevEcoRoot).Path
} elseif (Test-HarmonySdkRoot -Candidate $perUserHarmonySdkRoot) {
  $resolvedHarmonySdkRoot = (Resolve-Path -LiteralPath $perUserHarmonySdkRoot).Path
} else {
  throw 'HarmonyOS SDK 未找到。请完成本机 SDK 安装后传入 -HarmonySdkRoot，或设置 DEVECO_SDK_HOME。'
}

$nodePath = Join-Path $resolvedDevEcoRoot 'tools\node\node.exe'
$hvigorWrapperPath = Join-Path $resolvedDevEcoRoot 'tools\hvigor\bin\hvigorw.js'
$hvigorEngineRoot = Join-Path $resolvedDevEcoRoot 'tools\hvigor\hvigor'
$hvigorPluginRoot = Join-Path $resolvedDevEcoRoot 'tools\hvigor\hvigor-ohos-plugin'
$hvigorEnginePath = Join-Path $hvigorEngineRoot 'bin\hvigor.js'
$hvigorModuleAliasPath = Join-Path $PSScriptRoot 'hvigor\deveco-module-alias.cjs'
$devEcoJavaHome = Join-Path $resolvedDevEcoRoot 'jbr'
$devEcoJavaBin = Join-Path $devEcoJavaHome 'bin'
$harmonySdkPackage = Join-Path $resolvedHarmonySdkRoot 'default\sdk-pkg.json'
$scanKitDeclaration = Join-Path $resolvedHarmonySdkRoot 'default\hms\ets\kits\@kit.ScanKit.d.ts'
$pushKitDeclaration = Join-Path $resolvedHarmonySdkRoot 'default\hms\ets\kits\@kit.PushKit.d.ts'
$buildProfileTemplate = Join-Path $PSScriptRoot 'build-profile.example.json5'
$buildProfile = Join-Path $PSScriptRoot 'build-profile.json5'

foreach ($requiredPath in @($nodePath, $hvigorWrapperPath, $hvigorEnginePath, $hvigorPluginRoot, $hvigorModuleAliasPath,
  $resolvedHarmonySdkRoot, $devEcoJavaBin, $harmonySdkPackage, $scanKitDeclaration, $pushKitDeclaration,
  $buildProfileTemplate)) {
  if (-not (Test-Path -LiteralPath $requiredPath)) {
    throw "Required DevEco component not found: $requiredPath"
  }
}
if (-not (Test-Path -LiteralPath $buildProfile -PathType Leaf)) {
  Copy-Item -LiteralPath $buildProfileTemplate -Destination $buildProfile
}

$env:DEVECO_SDK_HOME = $resolvedHarmonySdkRoot
$env:OHOS_BASE_SDK_HOME = $resolvedHarmonySdkRoot
$env:JAVA_HOME = $devEcoJavaHome
$env:Path = $devEcoJavaBin + [System.IO.Path]::PathSeparator + $env:Path
$buildArgs = @(
  '--mode', 'module',
  '-p', 'product=default',
  '-p', 'buildMode=debug',
  '--no-stacktrace',
  '--no-daemon',
  'assembleHap'
)

$previousLocation = (Get-Location).Path
$originalBuildProfile = $null
$signedBuildProfile = $null
$signingRedactions = @()
$signingMutex = $null
$signingLockHeld = $false
try {
  if (-not [string]::IsNullOrWhiteSpace($SigningConfigSource)) {
    $lockNameBytes = [Text.Encoding]::UTF8.GetBytes([IO.Path]::GetFullPath($PSScriptRoot).ToLowerInvariant())
    $lockName = [Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($lockNameBytes))
    $signingMutex = [Threading.Mutex]::new($false, "Local\XuanPlusRemoteBuild-$lockName")
    $signingLockHeld = $signingMutex.WaitOne(0)
    if (-not $signingLockHeld) { throw '本目录已有签名构建正在运行。' }
    try {
      $sourceProfilePath = (Resolve-Path -LiteralPath $SigningConfigSource).Path
      if ([string]::Equals($sourceProfilePath, $buildProfile, [StringComparison]::OrdinalIgnoreCase)) {
        throw 'invalid-source'
      }
      $sourceDirectory = Split-Path -Parent $sourceProfilePath
      $sourceAppPath = Join-Path $sourceDirectory 'AppScope/app.json5'
      $sourceApp = [IO.File]::ReadAllText($sourceAppPath, [Text.Encoding]::UTF8) | ConvertFrom-Json
      $targetApp = [IO.File]::ReadAllText((Join-Path $PSScriptRoot 'AppScope/app.json5'), [Text.Encoding]::UTF8) | ConvertFrom-Json
      if ($sourceApp.app.bundleName -ne 'com.dyys.workagents.remote.dev' -or
        $targetApp.app.bundleName -ne $sourceApp.app.bundleName) {
        throw 'bundle-mismatch'
      }
      $sourceConfig = [IO.File]::ReadAllText($sourceProfilePath, [Text.Encoding]::UTF8) | ConvertFrom-Json
      $sourceProducts = @($sourceConfig.app.products | Where-Object { $_.name -eq 'default' })
      if ($sourceProducts.Count -ne 1 -or [string]::IsNullOrWhiteSpace($sourceProducts[0].signingConfig)) {
        throw 'missing-product-signing'
      }
      $signers = @($sourceConfig.app.signingConfigs | Where-Object { $_.name -eq $sourceProducts[0].signingConfig })
      if ($signers.Count -ne 1) { throw 'ambiguous-signing' }
      $signer = $signers[0]
      foreach ($field in @('certpath', 'profile', 'storeFile')) {
        $value = [string]$signer.material.$field
        if ([string]::IsNullOrWhiteSpace($value)) { throw 'missing-material' }
        $absolutePath = if ([IO.Path]::IsPathRooted($value)) { $value } else { Join-Path $sourceDirectory $value }
        if (-not (Test-Path -LiteralPath $absolutePath -PathType Leaf)) { throw 'missing-material' }
        $signer.material.$field = (Resolve-Path -LiteralPath $absolutePath).Path
      }
      foreach ($field in @('keyAlias', 'keyPassword', 'storePassword', 'signAlg')) {
        if ([string]::IsNullOrWhiteSpace([string]$signer.material.$field)) { throw 'missing-signing-field' }
      }
      $signingRedactions = @($signer.material.PSObject.Properties | ForEach-Object { [string]$_.Value })
      $originalBuildProfile = [IO.File]::ReadAllBytes($buildProfile)
      $targetConfig = [Text.Encoding]::UTF8.GetString($originalBuildProfile) | ConvertFrom-Json
      $targetProducts = @($targetConfig.app.products | Where-Object { $_.name -eq 'default' })
      if ($targetProducts.Count -ne 1) { throw 'ambiguous-target-product' }
      $targetConfig.app.signingConfigs = @($signer)
      $targetProducts[0] | Add-Member -NotePropertyName signingConfig -NotePropertyValue $signer.name -Force
      $signedBuildProfile = [Text.UTF8Encoding]::new($false).GetBytes(($targetConfig | ConvertTo-Json -Depth 30))
    } catch {
      # 配置解析异常可能包含原始 JSON，不能把底层异常或签名字段写进日志。
      throw '授权签名配置校验失败；请核对原项目默认签名、应用标识和本机签名文件。'
    }
    # 证书和密钥不复制，仅临时引用；退出时恢复本地配置的原始字节。
    [IO.File]::WriteAllBytes($buildProfile, $signedBuildProfile)
  }
  Set-Location -LiteralPath $PSScriptRoot
  $hvigorEntryPath = $hvigorWrapperPath
  if ($env:CODEX_CI -eq '1') {
    # Codex 的 Windows 沙箱禁止 Hvigor wrapper 创建目录联接，直接使用同一套内置引擎和插件。
    if ([string]::IsNullOrWhiteSpace($env:HVIGOR_USER_HOME)) {
      $env:HVIGOR_USER_HOME = Join-Path ([IO.Path]::GetTempPath()) 'xuan-plus-remote-hvigor-direct'
    }
    $env:XUANPLUS_HVIGOR_ENGINE = $hvigorEngineRoot
    $env:XUANPLUS_HVIGOR_PLUGIN = $hvigorPluginRoot
    $moduleAliasNodePath = $hvigorModuleAliasPath.Replace('\', '/')
    $moduleAliasOption = "--require=`"$moduleAliasNodePath`""
    $env:NODE_OPTIONS = if ([string]::IsNullOrWhiteSpace($env:NODE_OPTIONS)) {
      $moduleAliasOption
    } else {
      "$($env:NODE_OPTIONS) $moduleAliasOption"
    }
    $hvigorEntryPath = $hvigorEnginePath
  }
  & $nodePath $hvigorEntryPath @buildArgs 2>&1 | ForEach-Object {
    $line = [string]$_
    foreach ($value in $signingRedactions) {
      if ($value.Length -gt 0) { $line = $line.Replace($value, '[已脱敏]') }
    }
    Write-Output $line
  }
  $hvigorExitCode = $LASTEXITCODE
} finally {
  Set-Location -LiteralPath $previousLocation
  try {
    if ($null -ne $originalBuildProfile -and $null -ne $signedBuildProfile) {
      try {
        [IO.File]::WriteAllBytes($buildProfile, $originalBuildProfile)
      } finally {
        & (Join-Path $PSScriptRoot 'clear-signing-cache.ps1')
      }
    }
  } finally {
    if ($signingLockHeld) { $signingMutex.ReleaseMutex() }
    if ($null -ne $signingMutex) { $signingMutex.Dispose() }
  }
}
if ($hvigorExitCode -ne 0) {
  exit $hvigorExitCode
}
exit 0
