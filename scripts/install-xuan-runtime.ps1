[CmdletBinding(SupportsShouldProcess)]
param(
    [Parameter(Mandatory = $true)][string]$BridgeSource,
    [Parameter(Mandatory = $true)][string]$RemoteSource,
    [Parameter(Mandatory = $true)][string]$UiSource,
    [Parameter(Mandatory = $true)][string]$BinDirectory
)

$ErrorActionPreference = 'Stop'
$lock = $null
$temporaryPointer = $null
try {
    $root = [System.IO.Path]::GetFullPath($BinDirectory).TrimEnd('\', '/')
    if ($root -eq [System.IO.Path]::GetPathRoot($root).TrimEnd('\', '/')) {
        throw '插件安装目录不能是磁盘根目录。'
    }
    $sources = @(
        @{ Source = $BridgeSource; Name = 'xuan-bridge.exe' },
        @{ Source = $RemoteSource; Name = 'xuan-plus-remote-bridge.exe' },
        @{ Source = $UiSource; Name = 'xuan-ui-bridge.mjs' }
    )
    foreach ($item in $sources) {
        $item.Source = (Get-Item -LiteralPath $item.Source -ErrorAction Stop).FullName
        $item.Hash = (Get-FileHash -LiteralPath $item.Source -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    $fingerprint = [System.Text.Encoding]::UTF8.GetBytes(($sources.Hash -join ':'))
    $version = [System.Convert]::ToHexString([System.Security.Cryptography.SHA256]::HashData($fingerprint)).ToLowerInvariant()
    $destination = Join-Path (Join-Path $root 'versions') $version
    if (-not $destination.StartsWith($root + [System.IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
        throw '插件安装目标超出指定目录。'
    }
    # 禁止通过目录链接把更新写入宿主目录或其他位置。
    $ancestor = $destination
    while ($ancestor) {
        if (Test-Path -LiteralPath $ancestor) {
            $entry = Get-Item -LiteralPath $ancestor
            if ($entry.Attributes -band [System.IO.FileAttributes]::ReparsePoint) {
                throw '插件安装路径不能包含目录链接。'
            }
        }
        $ancestor = [System.IO.Path]::GetDirectoryName($ancestor)
    }
    if (-not $PSCmdlet.ShouldProcess($root, '安装独立插件运行文件并切换版本索引')) { return }
    [System.IO.Directory]::CreateDirectory($root) | Out-Null
    $lock = [System.IO.File]::Open((Join-Path $root 'install.lock'), 'OpenOrCreate', 'ReadWrite', 'None')
    [System.IO.Directory]::CreateDirectory($destination) | Out-Null
    foreach ($item in $sources) {
        $target = Join-Path $destination $item.Name
        if ((Test-Path -LiteralPath $target) -and ((Get-Item -LiteralPath $target).Attributes -band [System.IO.FileAttributes]::ReparsePoint)) {
            throw '插件运行文件不能是链接。'
        }
        if (-not (Test-Path -LiteralPath $target)) {
            Copy-Item -LiteralPath $item.Source -Destination $target -ErrorAction Stop
        }
        if ((Get-FileHash -LiteralPath $target -Algorithm SHA256).Hash.ToLowerInvariant() -ne $item.Hash) {
            throw "插件运行文件校验失败：$($item.Name)"
        }
    }
    # 运行中的旧版本保持不动，新插件进程从原子更新后的索引启动。
    $temporaryPointer = Join-Path $root ("current." + [guid]::NewGuid().ToString('N') + '.json')
    $content = @{ version = $version } | ConvertTo-Json -Compress
    [System.IO.File]::WriteAllText($temporaryPointer, $content, [System.Text.UTF8Encoding]::new($false))
    [System.IO.File]::Move($temporaryPointer, (Join-Path $root 'current.json'), $true)
    $temporaryPointer = $null
    Write-Output '独立插件运行文件安装完成；运行中的旧版本将在退出后释放。'
} catch {
    Write-Error $_
    exit 1
} finally {
    if ($temporaryPointer -and (Test-Path -LiteralPath $temporaryPointer)) {
        Remove-Item -LiteralPath $temporaryPointer -Force
    }
    if ($lock) { $lock.Dispose() }
}
