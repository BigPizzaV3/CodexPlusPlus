$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$testRoot = Join-Path (Join-Path $root 'target') ('xuan-runtime-install-test-' + [guid]::NewGuid().ToString('N'))
$child = $null
$failed = $false
try {
    [System.IO.Directory]::CreateDirectory($testRoot) | Out-Null
    $bridge = Join-Path $root 'tools\xuan-bridge\target\release\xuan-bridge.exe'
    $remote = Join-Path $root 'apps\xuan-plus-remote\bridge\target\release\xuan-plus-remote-bridge.exe'
    $ui = Join-Path $testRoot 'xuan-ui-bridge.mjs'
    Copy-Item -LiteralPath (Join-Path $root 'tools\xuan-ui-bridge\xuan-ui-bridge.mjs') -Destination $ui
    $hiddenParent = [System.IO.Directory]::CreateDirectory((Join-Path $testRoot 'AppData'))
    $hiddenParent.Attributes = $hiddenParent.Attributes -bor [System.IO.FileAttributes]::Hidden
    $bin = Join-Path $hiddenParent.FullName 'XuanPlusPlus\bin'
    $installer = Join-Path $PSScriptRoot 'install-xuan-runtime.ps1'
    $parameters = @{
        BridgeSource = $bridge
        RemoteSource = $remote
        UiSource = $ui
        BinDirectory = $bin
    }
    & $installer @parameters -WhatIf
    if (Test-Path -LiteralPath $bin) { throw '预演不应创建安装目录。' }
    & $installer @parameters
    $first = Get-Content -LiteralPath (Join-Path $bin 'current.json') -Raw | ConvertFrom-Json
    $oldBinary = Join-Path (Join-Path (Join-Path $bin 'versions') $first.version) 'xuan-bridge.exe'
    $hash = (Get-FileHash -LiteralPath $oldBinary -Algorithm SHA256).Hash
    $start = [System.Diagnostics.ProcessStartInfo]::new($oldBinary)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardInput = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.Environment['XUAN_HOME'] = Join-Path $testRoot 'home'
    $child = [System.Diagnostics.Process]::Start($start)
    Start-Sleep -Milliseconds 200
    if ($child.HasExited) { throw '测试用旧版 Bridge 未能保持运行。' }
    & $installer @parameters
    $unchanged = Get-Content -LiteralPath (Join-Path $bin 'current.json') -Raw | ConvertFrom-Json
    if ($unchanged.version -ne $first.version) { throw '同一产物重复安装产生了额外版本。' }
    [System.IO.File]::AppendAllText($ui, "`n// 安装升级测试夹具。`n", [System.Text.UTF8Encoding]::new($false))
    & $installer @parameters
    $second = Get-Content -LiteralPath (Join-Path $bin 'current.json') -Raw | ConvertFrom-Json
    if ($second.version -eq $first.version) { throw '新版插件未切换运行文件索引。' }
    if ($child.HasExited) { throw '安装新版不应终止旧版 Bridge。' }
    if ((Get-FileHash -LiteralPath $oldBinary -Algorithm SHA256).Hash -ne $hash) { throw '安装新版修改了旧版可执行文件。' }
    [pscustomobject]@{ hiddenAncestor = $true; whatIf = $true; idempotent = $true; lockedUpgrade = $true; oldProcessPreserved = $true } | ConvertTo-Json -Compress
} catch {
    $failed = $true
    Write-Error $_ -ErrorAction Continue
} finally {
    if ($child) {
        $child.StandardInput.Close()
        if (-not $child.WaitForExit(5000)) {
            $child.Kill()
            $child.WaitForExit()
        }
        $child.Dispose()
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
