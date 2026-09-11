param(
  [ValidateSet('Source', 'Cloud', 'Device', 'Hap')]
  [string]$Check = 'Source',
  [string]$NodePath = '',
  [string]$HdcPath = '',
  [switch]$Signed
)

$ErrorActionPreference = 'Stop'
try {
  if ($Check -eq 'Source') {
    $node = (Resolve-Path -LiteralPath $NodePath).Path
    foreach ($entry in @('verify-fork.mjs', 'environment/verify-shared-service-config.mjs',
      'protocol/verify-contract.mjs', 'protocol/verify-reference-harness.mjs')) {
      $arguments = @((Join-Path $PSScriptRoot $entry))
      & $node @arguments
      if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
    $parsePaths = @('app/build-dev.ps1', 'app/install-dev.ps1', 'app/sign-dev.ps1',
      'app/open-in-deveco.ps1', 'app/clear-signing-cache.ps1', 'verify.ps1')
    foreach ($entry in $parsePaths) {
      $tokens = $null
      $parseErrors = $null
      [void][Management.Automation.Language.Parser]::ParseFile(
        (Join-Path $PSScriptRoot $entry), [ref]$tokens, [ref]$parseErrors)
      if ($parseErrors.Count -gt 0) { throw "PowerShell 语法检查失败：$entry" }
    }
    Write-Output '{"sourceChecksPassed":true}'
  } elseif ($Check -eq 'Cloud') {
    $manifest = Join-Path $PSScriptRoot 'cloud-service/Cargo.toml'
    $arguments = @('test', '--manifest-path', $manifest, '--offline', '--locked')
    & cargo @arguments
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
  } elseif ($Check -eq 'Hap') {
    $hapName = if ($Signed) { 'entry-default-signed.hap' } else { 'entry-default-unsigned.hap' }
    $hapPath = Join-Path $PSScriptRoot "app/entry/build/default/outputs/default/$hapName"
    $archive = [IO.Compression.ZipFile]::OpenRead($hapPath)
    try {
      $entry = $archive.GetEntry('module.json')
      if ($null -eq $entry) { throw 'HAP 缺少内嵌模块配置。' }
      $reader = [IO.StreamReader]::new($entry.Open(), [Text.Encoding]::UTF8)
      try {
        $profile = $reader.ReadToEnd() | ConvertFrom-Json
      } finally {
        $reader.Dispose()
      }
      if ($profile.app.bundleName -ne 'com.dyys.workagents.remote.dev') {
        throw 'HAP 的应用标识与独立开发版不一致。'
      }
      [ordered]@{
        hapIdentityVerified = $true
        bundleName = $profile.app.bundleName
        signedArtifact = [bool]$Signed
        bytes = (Get-Item -LiteralPath $hapPath).Length
      } | ConvertTo-Json -Compress
    } finally {
      $archive.Dispose()
    }
  } else {
    $hdc = (Resolve-Path -LiteralPath $HdcPath).Path
    $arguments = @('list', 'targets')
    $result = @(& $hdc @arguments)
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    if (($result -join "`n") -match '(?i)error|failed') {
      throw '设备枚举失败。'
    }
    # 只汇报数量，不在诊断中输出设备序列号。
    $connected = @($result | Where-Object {
      -not [string]::IsNullOrWhiteSpace($_) -and $_.Trim() -ne '[Empty]'
    })
    [ordered]@{ connectedDevices = $connected.Count; deviceIdentifiersRedacted = $true } | ConvertTo-Json -Compress
  }
} catch {
  Write-Error $_
  exit 1
}
