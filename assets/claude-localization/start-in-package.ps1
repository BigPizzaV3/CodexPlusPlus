param([Parameter(Mandatory = $true)][string]$LaunchFile)
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$launch = Get-Content -LiteralPath $LaunchFile -Raw -Encoding UTF8 | ConvertFrom-Json
foreach ($name in @('BASH_DEFAULT_TIMEOUT_MS', 'BASH_MAX_TIMEOUT_MS')) {
  $value = $launch.environment.$name
  [Environment]::SetEnvironmentVariable($name, $(if ([string]::IsNullOrEmpty($value)) { $null } else { [string]$value }), 'Process')
}
try {
  Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1') -ErrorAction Stop
  if ((Get-AuthenticodeSignature -LiteralPath $launch.app).Status -ne 'Valid') { throw '官方 Claude 签名无效，未启动增强' }
  Add-Type -Path (Join-Path $PSScriptRoot 'native-launch.cs')
  $childId = [ClaudeNativeLaunch]::Start([string]$launch.app, [int]$launch.port)
  @{ pid = $childId; memoryFuseRestored = $true; inspectorOwnedByChild = $true } | ConvertTo-Json | Set-Content -LiteralPath ($LaunchFile + '.result') -Encoding UTF8
} catch {
  $_.Exception.Message | Set-Content -LiteralPath ($LaunchFile + '.error') -Encoding UTF8
  throw
}
