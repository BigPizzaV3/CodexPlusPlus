param(
  [string]$DevEcoRoot = '',
  [string]$HarmonySdkRoot = '',
  [Parameter(Mandatory = $true)]
  [string]$SigningConfigSource
)

$ErrorActionPreference = 'Stop'
# 委托原装构建器读取用户授权的签名，不生成 SDK 公共密钥签名或读取设备 UDID。
$parameters = @{
  DevEcoRoot = $DevEcoRoot
  HarmonySdkRoot = $HarmonySdkRoot
  SigningConfigSource = $SigningConfigSource
}
& (Join-Path $PSScriptRoot 'build-dev.ps1') @parameters
exit $LASTEXITCODE
