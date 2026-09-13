$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$testRoot = Join-Path (Join-Path $root 'target') ('xuan-process-stop-test-' + [guid]::NewGuid().ToString('N'))
$children = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
$failed = $false

function Start-TestProcess {
    param([string]$File, [string[]]$Arguments, [string]$Bridge, [string]$WorkingDirectory, [string]$Remote)
    $start = [System.Diagnostics.ProcessStartInfo]::new($File)
    foreach ($argument in $Arguments) { $start.ArgumentList.Add($argument) }
    $start.WorkingDirectory = $WorkingDirectory
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['XUAN_BRIDGE_BIN'] = $Bridge
    $start.Environment['XUAN_HOME'] = Join-Path $testRoot 'home'
    $start.Environment['XUAN_UI_BRIDGE_DISABLE'] = '1'
    if ($Remote) { $start.Environment['XUAN_TEST_REMOTE_BIN'] = $Remote }
    $child = [System.Diagnostics.Process]::Start($start)
    $children.Add($child)
    return $child
}

function Get-TestBridge {
    param([System.Diagnostics.Process]$Parent, [ValidateSet('xuan-bridge.exe', 'xuan-plus-remote-bridge.exe')][string]$Name = 'xuan-bridge.exe')
    for ($attempt = 0; $attempt -lt 50; $attempt++) {
        if ($Parent.HasExited) { throw '测试用插件进程提前退出。' }
        $bridge = Get-CimInstance -ClassName Win32_Process -Filter "Name = '$Name' AND ParentProcessId = $($Parent.Id)"
        if ($bridge) {
            $child = [System.Diagnostics.Process]::GetProcessById($bridge.ProcessId)
            $children.Add($child)
            return $child
        }
        Start-Sleep -Milliseconds 100
    }
    throw '测试用插件未启动 Bridge。'
}

try {
    $batchBytes = [System.IO.File]::ReadAllBytes((Join-Path $root 'install-xuan-features.bat'))
    if ($batchBytes.Length -ge 3 -and $batchBytes[0] -eq 239 -and $batchBytes[1] -eq 187 -and $batchBytes[2] -eq 191) {
        throw '批处理不能包含 UTF-8 BOM。'
    }
    $batch = [System.Text.UTF8Encoding]::new($false, $true).GetString($batchBytes)
    if ($batch -match '(?<!\r)\n') { throw '批处理不能包含 LF-only 换行。' }
    $stopCall = $batch.IndexOf('pwsh.exe -NoLogo -NoProfile -NonInteractive -File "%PLUGIN_PROCESS_STOPPER%"')
    if ($stopCall -lt $batch.IndexOf('echo [5/7]') -or $stopCall -ge $batch.IndexOf('plugin add "%%P@xuan-curated"')) {
        throw '批处理必须在更新插件前调用进程清理。'
    }
    if ($batch.IndexOf('if /i "%MODE%"=="check"') -ge $stopCall -or
        $batch -notmatch '(?s)-File "%PLUGIN_PROCESS_STOPPER%"[^\r\n]+\r\nif errorlevel 1 \(\r\n[^\r\n]+\r\n  exit /b 1') {
        throw '检查模式必须跳过清理，且清理失败必须停止安装。'
    }
    [System.IO.Directory]::CreateDirectory($testRoot) | Out-Null
    $bin = Join-Path $testRoot 'runtime with spaces'
    $versioned = Join-Path (Join-Path $bin 'versions') ('a' * 64)
    $outside = $bin + '-other'
    $source = Join-Path $root 'tools\xuan-bridge\target\release\xuan-bridge.exe'
    foreach ($directory in @($bin, $versioned, $outside)) {
        [System.IO.Directory]::CreateDirectory($directory) | Out-Null
        Copy-Item -LiteralPath $source -Destination (Join-Path $directory 'xuan-bridge.exe')
    }
    $node = (Get-Command node.exe -ErrorAction Stop).Source
    $stopper = Join-Path $PSScriptRoot 'stop-xuan-plugin-processes.ps1'
    $pluginRoot = Join-Path $root 'plugins\xuan-workspace-search'
    $legacy = Start-TestProcess -File $node -Arguments @('./server.mjs') -Bridge (Join-Path $bin 'xuan-bridge.exe') -WorkingDirectory $pluginRoot
    $legacyBridge = Get-TestBridge -Parent $legacy
    $version = Start-TestProcess -File $node -Arguments @((Join-Path $pluginRoot 'server.mjs')) -Bridge (Join-Path $versioned 'xuan-bridge.exe') -WorkingDirectory $pluginRoot
    $versionBridge = Get-TestBridge -Parent $version
    $unrelated = Start-TestProcess -File $node -Arguments @('server.mjs') -Bridge (Join-Path $outside 'xuan-bridge.exe') -WorkingDirectory $pluginRoot
    $unrelatedBridge = Get-TestBridge -Parent $unrelated
    $hostBridge = Start-TestProcess -File (Join-Path $bin 'xuan-bridge.exe') -Bridge (Join-Path $bin 'xuan-bridge.exe') -WorkingDirectory $testRoot
    $otherEntry = 'const {spawn}=require("node:child_process");const child=spawn(process.env.XUAN_BRIDGE_BIN,[],{stdio:["pipe","ignore","ignore"],windowsHide:true});process.stdin.resume();process.stdin.on("end",()=>child.stdin.end());'
    $other = Start-TestProcess -File $node -Arguments @('-e', $otherEntry, 'server.mjs') -Bridge (Join-Path $bin 'xuan-bridge.exe') -WorkingDirectory $testRoot
    $otherBridge = Get-TestBridge -Parent $other

    # 使用 stdio Bridge 模拟手机子进程，只验证进程归属，不启动真实手机服务或监听端口。
    $remoteBinary = Join-Path $bin 'xuan-plus-remote-bridge.exe'
    Copy-Item -LiteralPath $source -Destination $remoteBinary
    $fixture = Join-Path (Join-Path $testRoot 'mobile plugin') 'server.mjs'
    [System.IO.Directory]::CreateDirectory((Split-Path -Parent $fixture)) | Out-Null
    $fixtureSource = @'
import { spawn } from "node:child_process";
const children = [process.env.XUAN_BRIDGE_BIN, process.env.XUAN_TEST_REMOTE_BIN].map(binary =>
  spawn(binary, [], { stdio: ["pipe", "ignore", "ignore"], windowsHide: true }));
process.stdin.resume();
process.stdin.on("end", () => children.forEach(child => child.stdin.end()));
'@
    [System.IO.File]::WriteAllText($fixture, $fixtureSource, [System.Text.UTF8Encoding]::new($false))
    $mobile = Start-TestProcess -File $node -Arguments @($fixture) -Bridge (Join-Path $bin 'xuan-bridge.exe') -WorkingDirectory $testRoot -Remote $remoteBinary
    $mobileBridge = Get-TestBridge -Parent $mobile
    $mobileRemote = Get-TestBridge -Parent $mobile -Name 'xuan-plus-remote-bridge.exe'
    $hostRemote = Start-TestProcess -File $remoteBinary -Bridge (Join-Path $bin 'xuan-bridge.exe') -WorkingDirectory $testRoot

    & pwsh.exe -NoLogo -NoProfile -NonInteractive -File $stopper -BinDirectory $bin -WhatIf
    if ($LASTEXITCODE -ne 0) { throw '插件进程清理预演失败。' }
    foreach ($child in $children) {
        if ($child.HasExited) { throw '预演不应结束任何进程。' }
    }
    & pwsh.exe -NoLogo -NoProfile -NonInteractive -File $stopper -BinDirectory $bin
    if ($LASTEXITCODE -ne 0) { throw '插件进程清理失败。' }
    foreach ($child in @($legacy, $legacyBridge, $version, $versionBridge, $mobile, $mobileBridge, $mobileRemote)) {
        if (-not $child.WaitForExit(5000)) { throw '目标插件进程未被清理。' }
    }
    $preserved = @($unrelated, $unrelatedBridge, $hostBridge, $other, $otherBridge, $hostRemote)
    foreach ($child in $preserved) {
        if ($child.HasExited) { throw '清理误杀了宿主 Bridge、其他入口或其他安装目录的进程。' }
    }
    & pwsh.exe -NoLogo -NoProfile -NonInteractive -File $stopper -BinDirectory $bin
    if ($LASTEXITCODE -ne 0) { throw '重复清理失败。' }
    foreach ($child in $preserved) {
        if ($child.HasExited) { throw '重复清理误杀了无关进程。' }
    }
    [pscustomobject]@{ batchContract = $true; whatIf = $true; legacy = $true; versioned = $true; mobileChild = $true; unrelatedPreserved = $true; hostPreserved = $true; otherEntryPreserved = $true; idempotent = $true } | ConvertTo-Json -Compress
} catch {
    $failed = $true
    Write-Error $_ -ErrorAction Continue
} finally {
    foreach ($child in $children) {
        try {
            if (-not $child.HasExited) {
                $child.Kill()
                $child.WaitForExit()
            }
        } finally {
            $child.Dispose()
        }
    }
    $expectedParent = [System.IO.Path]::GetFullPath((Join-Path $root 'target')) + [System.IO.Path]::DirectorySeparatorChar
    $resolved = [System.IO.Path]::GetFullPath($testRoot)
    if (-not $resolved.StartsWith($expectedParent, [StringComparison]::OrdinalIgnoreCase)) { throw '测试清理目标越界。' }
    if (Test-Path -LiteralPath $resolved) {
        $entries = Get-ChildItem -LiteralPath $resolved -Recurse -Force
        foreach ($entry in $entries | Where-Object { -not $_.PSIsContainer }) {
            Remove-Item -LiteralPath $entry.FullName -Force
        }
        foreach ($entry in $entries | Where-Object { $_.PSIsContainer } | Sort-Object { $_.FullName.Length } -Descending) {
            [System.IO.Directory]::Delete($entry.FullName, $false)
        }
        [System.IO.Directory]::Delete($resolved, $false)
    }
}
if ($failed) { exit 1 }
