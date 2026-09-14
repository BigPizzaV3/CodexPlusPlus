[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$BridgePath
)

$ErrorActionPreference = 'Stop'

$pluginMarkers = @(
    '--xuan-plugin=xuan-workspace-search',
    '--xuan-plugin=xuan-usage',
    '--xuan-plugin=xuan-polish',
    '--xuan-plugin=xuan-mobile'
)
$resolvedBridgePath = [System.IO.Path]::GetFullPath($BridgePath)
$processes = @(Get-CimInstance Win32_Process)
$processById = @{}
$hostProcessIds = [System.Collections.Generic.HashSet[int]]::new()

foreach ($process in $processes) {
    $processById[[int]$process.ProcessId] = $process
    if ($process.Name -ine 'node.exe') {
        continue
    }

    $commandLine = [string]$process.CommandLine
    foreach ($marker in $pluginMarkers) {
        if ($commandLine.IndexOf($marker, [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
            [void]$hostProcessIds.Add([int]$process.ProcessId)
            break
        }
    }
}

foreach ($process in $processes) {
    if ($process.Name -ine 'xuan-bridge.exe') {
        continue
    }

    $executablePath = [string]$process.ExecutablePath
    if ([string]::IsNullOrWhiteSpace($executablePath) -or
        -not [System.StringComparer]::OrdinalIgnoreCase.Equals($executablePath, $resolvedBridgePath)) {
        continue
    }

    $parentId = [int]$process.ParentProcessId
    if (-not $processById.ContainsKey($parentId)) {
        continue
    }

    $parent = $processById[$parentId]
    $parentCommandLine = [string]$parent.CommandLine
    if ($parent.Name -ieq 'node.exe' -and
        $parentCommandLine.IndexOf('server.mjs', [System.StringComparison]::OrdinalIgnoreCase) -ge 0) {
        [void]$hostProcessIds.Add($parentId)
    }
}

$stoppedCount = 0
foreach ($processId in @($hostProcessIds | Sort-Object)) {
    if ($null -eq (Get-Process -Id $processId -ErrorAction SilentlyContinue)) {
        continue
    }

    Stop-Process -Id $processId -Force
    $stoppedCount++
}

if ($stoppedCount -eq 0) {
    Write-Output '  [信息] 未发现需要停止的 Xuan 插件后台进程。'
} else {
    Write-Output "  [完成] 已停止 $stoppedCount 个 Xuan 插件后台进程。"
}
