param()
$ErrorActionPreference = 'Stop'

$sourceExe = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_SOURCE)
$copyExe = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_COPY)
$sourceRoot = Split-Path $sourceExe -Parent
$copyRoot = Split-Path $copyExe -Parent
$here = Split-Path $MyInvocation.MyCommand.Path -Parent

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

# Electron fuse（含 EnableNodeCliInspectArguments）由 Codex++ 在复制完成后改写并复核，
# 逐字节扫描上百 MB 的 exe 在 PowerShell 里太慢；官方安装目录始终只读不改。

function Copy-Catalogs([string]$ion) {
  foreach ($relative in @('zh-Hans.json', 'dynamic\zh-Hans.json', 'zh-Hans.overrides.json')) {
    $source = Join-Path $here (Join-Path 'catalogs' $relative)
    $target = Join-Path $ion (Join-Path 'i18n' $relative)
    New-Item -ItemType Directory -Force (Split-Path $target -Parent) | Out-Null
    Copy-Item -LiteralPath $source -Destination $target -Force
  }
  $old = '["en-US","de-DE","fr-FR","ko-KR","ja-JP","es-419","es-ES","it-IT","hi-IN","pt-BR","id-ID"]'
  $new = $old.Substring(0, $old.Length - 1) + ',"zh-Hans"]'
  $matches = 0
  Get-ChildItem -LiteralPath (Join-Path $ion 'assets') -Filter '*.js' -Recurse -File | ForEach-Object {
    $text = Get-Content -LiteralPath $_.FullName -Raw -Encoding UTF8
    if ($text.Contains($old)) { Set-Content -LiteralPath $_.FullName -Value $text.Replace($old, $new) -Encoding UTF8; $matches++ }
    elseif ($text.Contains($new)) { $matches++ }
  }
  if ($matches -eq 0) { throw '本地界面的语言协商格式已变化' }
  $index = Join-Path $ion 'index.html'
  $html = Get-Content -LiteralPath $index -Raw -Encoding UTF8
  $catalogs = (Get-ChildItem -LiteralPath (Join-Path $ion 'i18n') -Filter '*.json' -File | ForEach-Object { $_.BaseName } | Where-Object { $_ -notmatch '\.' } | Sort-Object | ForEach-Object { "${_}:1.1.1" }) -join ','
  $meta = "<meta name=`"i18n-catalogs`" data-i18n-catalogs=`"$catalogs`">"
  if ($html -match '<meta\b[^>]*name=["'']i18n-catalogs["''][^>]*>') { $html = [regex]::Replace($html, '<meta\b[^>]*name=["'']i18n-catalogs["''][^>]*>', [System.Text.RegularExpressions.MatchEvaluator]{ param($m) $meta }) }
  elseif ($html.Contains('</head>')) { $html = $html.Replace('</head>', "$meta</head>") }
  else { throw '本地界面的 HTML 格式已变化' }
  Set-Content -LiteralPath $index -Value $html -Encoding UTF8
}

if (-not (Test-Path $sourceExe)) { throw "找不到 Claude：$sourceExe" }
if ($args -contains '--check') {
  if (-not (Test-Path $copyExe)) { throw 'Windows Claude 副本不存在' }
  $sourceAsar = Join-Path $sourceRoot 'resources\app.asar'; $copyAsar = Join-Path $copyRoot 'resources\app.asar'
  if ((Get-Sha256Hex $sourceAsar) -ne (Get-Sha256Hex $copyAsar)) { throw 'app.asar 已被改动' }
  $ion = Find-Ion $copyRoot
  if (-not (Test-Path (Join-Path $ion 'i18n\zh-Hans.json'))) { throw '中文词表缺失' }
  Write-Output 'Windows Claude 副本、中文词表与 app.asar 一致性检查通过'; exit 0
}

if (Test-Path $copyRoot) { throw '构建目标必须是新的暂存目录' }
New-Item -ItemType Directory -Force (Split-Path $copyRoot -Parent) | Out-Null
$null = & robocopy.exe $sourceRoot $copyRoot /E /R:1 /W:1 /NFL /NDL /NJH /NJS /NP
if ($LASTEXITCODE -gt 7) {
  $hint = $(if ($sourceRoot -match '\\WindowsApps\\') { '；商店版（MSIX）Claude 位于受保护的 WindowsApps 目录，请用安装 Claude 的同一账户运行 Codex++，不要以管理员或其他用户身份运行' } else { '' })
  throw "复制 Windows Claude 副本失败（robocopy 退出码：$LASTEXITCODE），可能没有读取 $sourceRoot 的权限或磁盘空间不足$hint"
}
Copy-Catalogs (Find-Ion $copyRoot)
Write-Output 'Windows Claude 副本已创建，官方 app.asar 未修改'
