param()
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)

$sourceExe = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_SOURCE)
$copyExe = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_COPY)
$sourceRoot = Split-Path $sourceExe -Parent
$copyRoot = Split-Path $copyExe -Parent
$here = Split-Path $MyInvocation.MyCommand.Path -Parent
$nativeLaunch = $env:CODEX_PLUS_CLAUDE_MSIX -eq '1'

function Get-Sha256Hex([string]$path) {
  $stream = $null
  $sha = $null
  try {
    $stream = [IO.File]::OpenRead($path)
    $sha = [Security.Cryptography.SHA256]::Create()
    return [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '')
  } catch {
    throw ('副本校验失败：' + $_.Exception.Message)
  } finally {
    if ($stream) { $stream.Dispose() }
    if ($sha) { $sha.Dispose() }
  }
}

function Find-Ion([string]$root) {
  foreach ($candidate in @((Join-Path $root 'resources\ion-dist'), (Join-Path $root 'ion-dist'))) {
    if (Test-Path (Join-Path $candidate 'index.html')) { return $candidate }
  }
  throw "找不到 Claude 本地界面 ion-dist"
}

function Find-FuseBinary([string]$root) {
  $wire = [Text.Encoding]::ASCII.GetBytes('dL7pKGdnNz796PbbjQWNKmHXBZaB9tsX')
  # 只检查自己的入口，不能递归进入映射到官方安装的资源目录。
  $files = @(Get-Item -LiteralPath (Join-Path $root 'Claude.exe'))
  foreach ($file in $files) {
    $bytes = [IO.File]::ReadAllBytes($file.FullName)
    # 由 .NET 搜索大型 Electron 二进制，避免逐字节 PowerShell 循环耗尽操作时限。
    $text = [Text.Encoding]::ASCII.GetString($bytes)
    $marker = [Text.Encoding]::ASCII.GetString($wire)
    $i = $text.IndexOf($marker, [StringComparison]::Ordinal)
    while ($i -ge 0 -and $i + $wire.Length + 2 -le $bytes.Length) {
      if ($bytes[$i + $wire.Length] -eq 1) {
        $count = $bytes[$i + $wire.Length + 1]
        if ($count -ge 4 -and $i + $wire.Length + 2 + $count -le $bytes.Length -and $bytes[$i + $wire.Length + 5] -in @(0x30, 0x31)) {
          return @{ Path = $file.FullName; Offset = $i + $wire.Length + 2; Count = $count }
        }
      }
      $i = $text.IndexOf($marker, $i + 1, [StringComparison]::Ordinal)
    }
  }
  throw '找不到 Electron fuse，无法安全启动注入'
}

function Patch-Fuse([string]$root) {
  $fuse = Find-FuseBinary $root
  $bytes = [IO.File]::ReadAllBytes($fuse.Path)
  if ($bytes[$fuse.Offset + 3] -ne 0x31) { $bytes[$fuse.Offset + 3] = 0x31; [IO.File]::WriteAllBytes($fuse.Path, $bytes) }
}

function Copy-Catalogs([string]$ion, [string]$sourceIon = '') {
  if (-not $sourceIon) { $sourceIon = $ion }
  foreach ($relative in @('zh-Hans.json', 'dynamic\zh-Hans.json', 'zh-Hans.overrides.json')) {
    $source = Join-Path $here (Join-Path 'catalogs' $relative)
    $target = Join-Path $ion (Join-Path 'i18n' $relative)
    New-Item -ItemType Directory -Force (Split-Path $target -Parent) | Out-Null
    Copy-Item -LiteralPath $source -Destination $target -Force
  }
  $old = '["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"]'
  $new = $old.Substring(0, $old.Length - 1) + ',"zh-Hans"]'
  $matches = 0
  Get-ChildItem -LiteralPath (Join-Path $sourceIon 'assets') -Filter '*.js' -Recurse -File | ForEach-Object {
    $text = Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8
    if ($text.Contains($old)) {
      $target = Join-Path $ion $_.FullName.Substring($sourceIon.Length + 1)
      New-Item -ItemType Directory -Force (Split-Path $target -Parent) | Out-Null
      Set-Content -LiteralPath $target -Value $text.Replace($old, $new) -Encoding UTF8
      $matches++
    }
    elseif ($text.Contains($new)) { $matches++ }
  }
  if ($matches -eq 0) { throw '本地界面的语言协商格式已变化' }
  $index = Join-Path $ion 'index.html'
  $html = Get-Content -LiteralPath (Join-Path $sourceIon 'index.html') -Raw -Encoding UTF8
  $names = @((Get-ChildItem -LiteralPath (Join-Path $sourceIon 'i18n') -Filter '*.json' -File | ForEach-Object { $_.BaseName }); 'zh-Hans')
  $catalogs = ($names | Where-Object { $_ -notmatch '\.' } | Sort-Object -Unique | ForEach-Object { "${_}:1.1.1" }) -join ','
  $meta = "<meta name=`"i18n-catalogs`" data-i18n-catalogs=`"$catalogs`">"
  if ($html -match '<meta\b[^>]*name=["'']i18n-catalogs["''][^>]*>') { $html = [regex]::Replace($html, '<meta\b[^>]*name=["'']i18n-catalogs["''][^>]*>', [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $meta }) }
  elseif ($html.Contains('</head>')) { $html = $html.Replace('</head>', "$meta</head>") }
  else { throw '本地界面的 HTML 格式已变化' }
  Set-Content -LiteralPath $index -Value $html -Encoding UTF8
}

if (-not (Test-Path $sourceExe)) { throw "找不到 Claude：$sourceExe" }
if ($sourceRoot.TrimEnd('\') -eq $copyRoot.TrimEnd('\')) { throw '增强版必须使用独立副本目录，不能修改官方安装目录' }
if ($nativeLaunch) {
  # 避免从 PowerShell 7 / Cargo 继承的 PSModulePath 指向不兼容的安全模块。
  Import-Module (Join-Path $PSHOME 'Modules\Microsoft.PowerShell.Security\Microsoft.PowerShell.Security.psd1') -ErrorAction Stop
  if ((Get-AuthenticodeSignature -LiteralPath $sourceExe).Status -ne 'Valid') { throw '官方 Claude 签名无效，未准备增强资源' }
}
if ($args -contains '--check') {
  if (-not (Test-Path $copyExe)) { throw 'Windows Claude 副本不存在' }
  if ($nativeLaunch) {
    $entry = Get-Content -LiteralPath $copyExe -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($entry.mode -ne 'native-runtime-v1' -or $entry.source -ne $sourceExe -or $entry.sha256 -ne (Get-Sha256Hex $sourceExe)) { throw '官方程序或增强入口已变化，请重新准备增强资源' }
  } else {
  $fuse = Find-FuseBinary $copyRoot
  $bytes = [IO.File]::ReadAllBytes($fuse.Path)
  if ($bytes[$fuse.Offset + 3] -ne 0x31) { throw '副本未启用启动注入，请重新制作增强版' }
  $expected = [IO.File]::ReadAllBytes($sourceExe)
  if ($expected.Length -ne $bytes.Length) { throw '官方程序已更新，请重新制作增强版' }
  $expected[$fuse.Offset + 3] = 0x31
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    if ([BitConverter]::ToString($sha.ComputeHash($bytes)) -ne [BitConverter]::ToString($sha.ComputeHash($expected))) { throw '增强入口与官方程序不一致，请重新制作增强版' }
  } finally { $sha.Dispose() }
  $sourceAsar = Join-Path $sourceRoot 'resources\app.asar'; $copyAsar = Join-Path $copyRoot 'resources\app.asar'
  if ((Get-Sha256Hex $sourceAsar) -ne (Get-Sha256Hex $copyAsar)) { throw 'app.asar 已被改动' }
  }
  if ($nativeLaunch) {
    $manifestPath = Join-Path $copyRoot '.codex-plus-overlay\manifest.json'
    $manifest = Get-Content -LiteralPath $manifestPath -Raw -Encoding UTF8 | ConvertFrom-Json
    if ($manifest.source -ne $sourceExe) { throw '官方安装位置已变化，请重新制作增强版' }
    foreach ($required in @('index.html', 'i18n/zh-Hans.json', 'i18n/dynamic/zh-Hans.json', 'i18n/zh-Hans.overrides.json')) {
      if ($required -notin $manifest.files.path) { throw '增强资源清单不完整，请重新制作增强版' }
    }
    if (-not @($manifest.files | Where-Object { $_.path -match '^assets/.+\.js$' }).Count) { throw '语言协商资源缺失，请重新制作增强版' }
    foreach ($file in $manifest.files) {
      $relative = [string]$file.path
      if ($relative -match '(^|[\\/])\.\.([\\/]|$)' -or [IO.Path]::IsPathRooted($relative)) { throw '增强资源路径无效' }
      $target = Join-Path (Split-Path $manifestPath -Parent) $relative
      if ((Get-Sha256Hex $target) -ne $file.sha256) { throw '增强资源校验失败' }
    }
    $ion = Split-Path $manifestPath -Parent
  } else { $ion = Find-Ion $copyRoot }
  if (-not (Test-Path (Join-Path $ion 'i18n\zh-Hans.json'))) { throw '中文词表缺失' }
  Write-Output 'Windows Claude 副本、中文词表与 app.asar 一致性检查通过'; exit 0
}

if (Test-Path $copyRoot) { throw '构建目标必须是新的暂存目录' }
New-Item -ItemType Directory -Force (Split-Path $copyRoot -Parent) | Out-Null
if ($nativeLaunch) {
  New-Item -ItemType Directory -Path $copyRoot | Out-Null
  # 原版 EXE 与所有运行库留在官方目录，仅保存增强资源和校验记录。
  $overlay = Join-Path $copyRoot '.codex-plus-overlay'
  Copy-Catalogs $overlay (Find-Ion $sourceRoot)
  $files = @(Get-ChildItem -LiteralPath $overlay -File -Recurse | ForEach-Object {
    @{ path = $_.FullName.Substring($overlay.Length + 1).Replace('\', '/'); sha256 = Get-Sha256Hex $_.FullName }
  })
  @{ source = $sourceExe; files = $files } | ConvertTo-Json -Depth 5 | Set-Content -LiteralPath (Join-Path $overlay 'manifest.json') -Encoding UTF8
  @{ mode = 'native-runtime-v1'; source = $sourceExe; sha256 = Get-Sha256Hex $sourceExe } | ConvertTo-Json | Set-Content -LiteralPath $copyExe -Encoding UTF8
} else {
  $null = & robocopy.exe $sourceRoot $copyRoot /E /R:1 /W:1 /NFL /NDL /NJH /NJS /NP
  if ($LASTEXITCODE -gt 7) { throw "复制 Windows Claude 副本失败，robocopy 退出码：$LASTEXITCODE" }
  Copy-Catalogs (Find-Ion $copyRoot)
}
if (-not $nativeLaunch) { Patch-Fuse $copyRoot }
Write-Output 'Windows Claude 增强入口已创建，官方应用与 app.asar 未修改'
