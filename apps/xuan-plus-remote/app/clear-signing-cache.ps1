[CmdletBinding(SupportsShouldProcess)]
param()

$ErrorActionPreference = 'Stop'
$appRoot = (Resolve-Path -LiteralPath $PSScriptRoot).Path
$cachePath = [IO.Path]::GetFullPath((Join-Path $appRoot '.hvigor/cache/task-cache.json'))
if (-not $cachePath.StartsWith($appRoot + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
  throw '签名缓存路径超出本应用目录。'
}
if (-not (Test-Path -LiteralPath $cachePath -PathType Leaf)) { return }
$cursor = Get-Item -LiteralPath $cachePath -Force
while ($null -ne $cursor) {
  if (($cursor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) {
    throw '签名缓存清理不允许经过符号链接或目录联接。'
  }
  $cursor = if ($cursor -is [IO.FileInfo]) { $cursor.Directory } else { $cursor.Parent }
}
# Hvigor 将签名配置序列化进这个生成文件；只移除此文件，不递归清理缓存目录。
if ($PSCmdlet.ShouldProcess($cachePath, '清理本应用签名构建的任务缓存')) {
  Remove-Item -LiteralPath $cachePath
}
