$ErrorActionPreference = 'Stop'
$systemTemporaryRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$temporaryRoot = Join-Path $systemTemporaryRoot ("workagents-remote-release-" + [Guid]::NewGuid().ToString('N'))

function Write-Utf8Sentinel {
  param(
    [string]$Path,
    [string]$Value
  )
  [IO.File]::WriteAllText($Path, $Value, [Text.UTF8Encoding]::new($false))
}

function Assert-FileContent {
  param(
    [string]$Path,
    [string]$Expected
  )
  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "缺少演练文件：$Path"
  }
  $actual = [IO.File]::ReadAllText($Path)
  if ($actual -ne $Expected) {
    throw "演练文件未恢复：$Path"
  }
}

try {
  $serviceRoot = Join-Path $temporaryRoot 'opt/workagents-remote-dev'
  $databaseRoot = Join-Path $temporaryRoot 'var/lib/workagents-remote-dev'
  $configRoot = Join-Path $temporaryRoot 'etc/workagents-remote'
  $unitRoot = Join-Path $temporaryRoot 'etc/systemd/system'
  $nginxRoot = Join-Path $temporaryRoot 'application/nginx/conf'
  $backupRoot = Join-Path $temporaryRoot 'var/lib/workagents-remote-deploy/backups/20260911000000'
  $failedRoot = Join-Path $temporaryRoot 'var/lib/workagents-remote-deploy/backups/20260911000000.rollback-attempt'

  foreach ($directory in @(
    $serviceRoot,
    $databaseRoot,
    $configRoot,
    $unitRoot,
    (Join-Path $nginxRoot 'vhosts'),
    $backupRoot,
    $failedRoot
  )) {
    [void](New-Item -ItemType Directory -Path $directory)
  }

  $serviceBinary = Join-Path $serviceRoot 'workagents-remote-cloud'
  $environmentFile = Join-Path $configRoot 'dev.env'
  $unitFile = Join-Path $unitRoot 'workagents-remote-cloud.service'
  $rateLimitFile = Join-Path $nginxRoot 'vhosts/00-workagents-remote-rate-limit.conf'
  $locationFile = Join-Path $nginxRoot 'workagents-remote-location.conf'
  $databaseFiles = @('remote.sqlite3', 'remote.sqlite3-wal', 'remote.sqlite3-shm')

  Write-Utf8Sentinel $serviceBinary 'old-binary'
  Write-Utf8Sentinel $environmentFile 'old-environment'
  Write-Utf8Sentinel $unitFile 'old-unit'
  Write-Utf8Sentinel $rateLimitFile 'old-rate-limit'
  Write-Utf8Sentinel $locationFile 'old-location'
  foreach ($name in $databaseFiles) {
    Write-Utf8Sentinel (Join-Path $databaseRoot $name) "old-$name"
  }

  Copy-Item -LiteralPath $serviceBinary -Destination (Join-Path $backupRoot 'workagents-remote-cloud')
  Copy-Item -LiteralPath $environmentFile -Destination (Join-Path $backupRoot 'dev.env')
  Copy-Item -LiteralPath $unitFile -Destination (Join-Path $backupRoot 'workagents-remote-cloud.service')
  Copy-Item -LiteralPath $rateLimitFile -Destination (Join-Path $backupRoot '00-workagents-remote-rate-limit.conf')
  Copy-Item -LiteralPath $locationFile -Destination (Join-Path $backupRoot 'workagents-remote-location.conf')
  foreach ($name in $databaseFiles) {
    Move-Item -LiteralPath (Join-Path $databaseRoot $name) -Destination (Join-Path $backupRoot $name)
  }

  Write-Utf8Sentinel $serviceBinary 'new-binary'
  Write-Utf8Sentinel $environmentFile 'new-environment'
  Write-Utf8Sentinel $unitFile 'new-unit'
  Write-Utf8Sentinel $rateLimitFile 'new-rate-limit'
  Write-Utf8Sentinel $locationFile 'new-location'
  foreach ($name in $databaseFiles) {
    Write-Utf8Sentinel (Join-Path $databaseRoot $name) "new-$name"
  }

  Copy-Item -LiteralPath $serviceBinary -Destination (Join-Path $failedRoot 'workagents-remote-cloud')
  foreach ($name in $databaseFiles) {
    Move-Item -LiteralPath (Join-Path $databaseRoot $name) -Destination (Join-Path $failedRoot $name)
  }
  Copy-Item -LiteralPath (Join-Path $backupRoot 'workagents-remote-cloud') -Destination $serviceBinary -Force
  Copy-Item -LiteralPath (Join-Path $backupRoot 'dev.env') -Destination $environmentFile -Force
  Copy-Item -LiteralPath (Join-Path $backupRoot 'workagents-remote-cloud.service') -Destination $unitFile -Force
  Copy-Item -LiteralPath (Join-Path $backupRoot '00-workagents-remote-rate-limit.conf') -Destination $rateLimitFile -Force
  Copy-Item -LiteralPath (Join-Path $backupRoot 'workagents-remote-location.conf') -Destination $locationFile -Force
  foreach ($name in $databaseFiles) {
    Copy-Item -LiteralPath (Join-Path $backupRoot $name) -Destination (Join-Path $databaseRoot $name)
  }

  Assert-FileContent $serviceBinary 'old-binary'
  Assert-FileContent $environmentFile 'old-environment'
  Assert-FileContent $unitFile 'old-unit'
  Assert-FileContent $rateLimitFile 'old-rate-limit'
  Assert-FileContent $locationFile 'old-location'
  foreach ($name in $databaseFiles) {
    Assert-FileContent (Join-Path $databaseRoot $name) "old-$name"
    Assert-FileContent (Join-Path $failedRoot $name) "new-$name"
  }

  [ordered]@{
    ok = $true
    serviceBinaryRestored = $true
    databaseFilesRestored = $databaseFiles.Count
    configurationFilesRestored = 4
    failedGenerationPreserved = $true
    parallelDeploymentCreated = $false
  } | ConvertTo-Json -Compress
} finally {
  if (Test-Path -LiteralPath $temporaryRoot) {
    $resolvedTemporaryRoot = (Resolve-Path -LiteralPath $temporaryRoot).Path
    if (-not $resolvedTemporaryRoot.StartsWith($systemTemporaryRoot, [StringComparison]::OrdinalIgnoreCase)) {
      throw "Refusing to remove unexpected rehearsal path: $resolvedTemporaryRoot"
    }
    Remove-Item -LiteralPath $resolvedTemporaryRoot -Recurse -Force
  }
}
