[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory = $true)][string]$BinDirectory
)

$ErrorActionPreference = 'Stop'
try {
    if (-not $IsWindows) { throw '插件进程清理仅支持 Windows。' }
    $root = [System.IO.Path]::GetFullPath($BinDirectory).TrimEnd('\', '/')
    if ($root -eq [System.IO.Path]::GetPathRoot($root).TrimEnd('\', '/')) {
        throw '插件运行目录不能是磁盘根目录。'
    }
    $binaryPattern = '^' + [regex]::Escape($root) + '\\(?:versions\\[a-f0-9]{64}\\)?xuan-(?:bridge|plus-remote-bridge)\.exe$'
    $bridges = @(Get-CimInstance -ClassName Win32_Process `
        -Filter "Name = 'xuan-bridge.exe' OR Name = 'xuan-plus-remote-bridge.exe'" `
        -Property ProcessId, ParentProcessId, Name, ExecutablePath, CreationDate |
        Where-Object { $_.ExecutablePath -and $_.ExecutablePath -match $binaryPattern })
    $parents = @{}
    foreach ($bridge in $bridges | Where-Object { $_.Name -eq 'xuan-bridge.exe' }) {
        $parent = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $($bridge.ParentProcessId)"
        if (-not $parent -or $parent.Name -ne 'node.exe' -or $parent.CreationDate -gt $bridge.CreationDate) {
            continue
        }
        # 同时核对入口和已安装的 Bridge，不能按 node.exe 名称批量结束进程。
        if ($parent.CommandLine -match '^(?:"[^"]+"|\S+)\s+(?:"(?:[^"]*[/\\])?server\.mjs"|(?:[^\s"]*[/\\])?server\.mjs)(?:\s|$)') {
            $parents[$parent.ProcessId] = $parent
        }
    }
    $targets = @($parents.Values) + @($bridges | Where-Object {
        $parent = $parents[$_.ParentProcessId]
        $parent -and $_.CreationDate -ge $parent.CreationDate
    })
    $stopped = 0
    # 先关闭插件入口，再处理未随入口退出的 Bridge；宿主直接管理的 Bridge 不在目标中。
    foreach ($target in $targets) {
        $process = $null
        try {
            try {
                $process = [System.Diagnostics.Process]::GetProcessById($target.ProcessId)
            } catch [System.ArgumentException] {
                continue
            }
            if ($process.HasExited) { continue }
            $null = $process.Handle
            $current = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $($target.ProcessId)"
            if (-not $current) { continue }
            if ($current.CreationDate -ne $target.CreationDate -or $current.ExecutablePath -ne $target.ExecutablePath) {
                throw '插件进程身份发生变化，已停止清理以避免误杀。'
            }
            if ($PSCmdlet.ShouldProcess($target.Name, '强制终止已核实的 Xuan 插件进程')) {
                Stop-Process -InputObject $process -Force -ErrorAction Stop
                if (-not $process.WaitForExit(5000)) { throw '插件进程未及时退出，已停止安装。' }
                $stopped++
            }
        } catch [System.InvalidOperationException] {
            if (-not $process -or -not $process.HasExited) { throw }
        } finally {
            if ($process) { $process.Dispose() }
        }
    }
    if (-not $WhatIfPreference) {
        foreach ($target in $targets) {
            $current = Get-CimInstance -ClassName Win32_Process -Filter "ProcessId = $($target.ProcessId)"
            if ($current -and $current.CreationDate -eq $target.CreationDate) {
                throw '仍有 Xuan 插件进程未退出，已停止安装。'
            }
        }
        Write-Output "Xuan 插件进程清理完成，强制结束 $stopped 个进程；未结束 Codex++ 主程序。"
    }
} catch {
    Write-Error $_ -ErrorAction Continue
    exit 1
}
