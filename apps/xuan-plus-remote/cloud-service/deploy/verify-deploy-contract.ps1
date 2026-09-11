param(
  [string]$BashPath
)

$ErrorActionPreference = 'Stop'
$deployRoot = $PSScriptRoot

function Assert-Contains {
  param(
    [string]$Text,
    [string]$Expected,
    [string]$Message
  )
  if (-not $Text.Contains($Expected, [System.StringComparison]::Ordinal)) {
    throw $Message
  }
}

function Resolve-Bash {
  param([string]$ExplicitPath)
  if ($ExplicitPath) {
    return (Resolve-Path -LiteralPath $ExplicitPath).Path
  }
  $command = Get-Command bash -ErrorAction SilentlyContinue
  if ($null -ne $command) {
    return $command.Source
  }
  $gitCommand = Get-Command git -ErrorAction SilentlyContinue
  $gitCandidates = @()
  if ($null -ne $gitCommand) {
    $gitRoot = Split-Path (Split-Path $gitCommand.Source -Parent) -Parent
    $gitCandidates = @(
      (Join-Path $gitRoot 'bin\bash.exe'),
      (Join-Path $gitRoot 'usr\bin\bash.exe')
    )
  }
  $candidates = @(
    (Join-Path $env:ProgramFiles 'Git\bin\bash.exe'),
    (Join-Path ([Environment]::GetFolderPath('ProgramFilesX86')) 'Git\bin\bash.exe')
  ) + $gitCandidates
  foreach ($candidate in $candidates) {
    if ($candidate -and (Test-Path -LiteralPath $candidate -PathType Leaf)) {
      return (Resolve-Path -LiteralPath $candidate).Path
    }
  }
  return $null
}

$deployScript = Get-Content -LiteralPath (Join-Path $deployRoot 'workagents-remote-deploy') -Raw
$installScript = Get-Content -LiteralPath (Join-Path $deployRoot 'install-workagents-remote-deploy') -Raw
$pushScript = Get-Content -LiteralPath (Join-Path $deployRoot 'configure-workagents-remote-huawei-push') -Raw
$unit = Get-Content -LiteralPath (Join-Path $deployRoot 'workagents-remote-cloud.service') -Raw
$environmentExample = Get-Content -LiteralPath (Join-Path $deployRoot 'dev.env.example') -Raw

foreach ($entry in @(
  @("SERVICE_NAME='workagents-remote-cloud.service'", 'systemd 服务名发生变化'),
  @("SERVICE_BINARY='/opt/workagents-remote-dev/workagents-remote-cloud'", '现网二进制路径发生变化'),
  @("DATABASE_ROOT='/var/lib/workagents-remote-dev'", '现网数据库目录发生变化'),
  @("ENVIRONMENT_FILE='/etc/workagents-remote/dev.env'", '现网环境文件路径发生变化'),
  @("EXPECTED_SERVICE_VERSION='1.0.0'", '服务版本门禁缺失'),
  @("EXPECTED_CONTRACT_VERSION='2.0'", '协议版本门禁缺失'),
  @("EXPECTED_STORAGE_SCHEMA_VERSION='8'", '数据库结构版本门禁缺失'),
  @("EXPECTED_SOURCE_SCHEMA_VERSION='7'", '来源数据库结构版本门禁缺失'),
  @('--reset-database-from', '覆盖升级的显式数据库重建参数缺失'),
  @('--rollback-release', '同一发布脚本的人工回滚入口缺失'),
  @('backup_configuration "$BACKUP_DIR"', '发布前配置备份缺失'),
  @('move_database_files "$DATABASE_ROOT" "$BACKUP_DIR"', '发布前数据库备份缺失'),
  @('health_is_target_ready', '目标版本健康门禁缺失'),
  @('CANDIDATE_VERSION=', '候选二进制版本检查缺失'),
  @('apply_release_backup "$backup_dir" "$current_snapshot"', '人工回滚未使用统一恢复事务'),
  @('restore_rollback_attempt "$current_snapshot" "$failed_rollback"', '人工回滚失败时缺少反向恢复'),
  @('服务保持停止，请使用回滚暂存目录人工恢复', '人工回滚双重失败缺少明确的安全状态')
)) {
  Assert-Contains -Text $deployScript -Expected $entry[0] -Message $entry[1]
}

foreach ($forbidden in @(
  'xuan-plus-remote-cloud.service',
  '/opt/xuan-plus-remote',
  '/var/lib/xuan-plus-remote',
  'XUANPLUS_REMOTE_',
  'rm -rf',
  'rm -f -- "${SERVICE_BINARY}.new" "${ROOT_STAGE}/workagents-remote-cloud"'
)) {
  if ($deployScript.Contains($forbidden, [System.StringComparison]::Ordinal)) {
    throw "发布脚本包含禁止的平行部署或批量删除逻辑：$forbidden"
  }
}

Assert-Contains -Text $installScript -Expected "DEPLOY_COMMAND='/usr/local/sbin/workagents-remote-deploy'" -Message '原发布命令安装入口发生变化'
Assert-Contains -Text $unit -Expected 'EnvironmentFile=/etc/workagents-remote/dev.env' -Message 'systemd 环境文件入口发生变化'
Assert-Contains -Text $unit -Expected 'ExecStart=/opt/workagents-remote-dev/workagents-remote-cloud' -Message 'systemd 二进制入口发生变化'
Assert-Contains -Text $unit -Expected 'ReadWriteDirectories=/var/lib/workagents-remote-dev' -Message 'systemd 数据目录发生变化'

foreach ($name in @(
  'WORKAGENTS_REMOTE_ENVIRONMENT',
  'WORKAGENTS_REMOTE_LISTEN',
  'WORKAGENTS_REMOTE_DATABASE',
  'WORKAGENTS_REMOTE_PUSH_TOKEN_KEY',
  'WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL',
  'WORKAGENTS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64'
)) {
  Assert-Contains -Text $environmentExample -Expected "$name=" -Message "环境变量缺失：$name"
}
if ($environmentExample.Contains('XUANPLUS_REMOTE_', [System.StringComparison]::Ordinal)) {
  throw '环境示例不得引入平行变量体系'
}
Assert-Contains -Text $pushScript -Expected 'WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL' -Message 'Huawei Push 配置脚本未沿用现有环境变量'

$resolvedBash = Resolve-Bash -ExplicitPath $BashPath
$bashSyntax = 'not-run'
if ($resolvedBash) {
  foreach ($name in @(
    'workagents-remote-deploy',
    'install-workagents-remote-deploy',
    'configure-workagents-remote-huawei-push'
  )) {
    & $resolvedBash -n (Join-Path $deployRoot $name)
    if ($LASTEXITCODE -ne 0) {
      throw "Bash 语法检查失败：$name"
    }
  }
  $bashSyntax = 'passed'
}

[ordered]@{
  ok = $true
  service = 'workagents-remote-cloud.service'
  serviceVersion = '1.0.0'
  protocolVersion = '2.0'
  storageSchemaVersion = 8
  existingDeployEntry = '/usr/local/sbin/workagents-remote-deploy'
  databaseUpgrade = 'replace-after-backup'
  rollback = 'same-script-release-snapshot'
  bashSyntax = $bashSyntax
} | ConvertTo-Json -Compress
