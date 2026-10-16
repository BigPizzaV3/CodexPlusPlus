param([string]$Locale = 'zh-Hans')
$ErrorActionPreference = 'Stop'
$app = $env:CODEX_PLUS_CLAUDE_COPY
$hook = Join-Path (Split-Path $MyInvocation.MyCommand.Path -Parent) 'hook.js'
if (-not (Test-Path $app)) { throw "找不到 Claude：$app" }
$port = 19000 + (Get-Random -Minimum 0 -Maximum 1000)
$proc = Start-Process -FilePath $app -ArgumentList "--inspect=127.0.0.1:$port" -PassThru
$deadline = (Get-Date).AddSeconds(45)
$target = $null
while ((Get-Date) -lt $deadline -and -not $target) {
  try { $target = Invoke-RestMethod "http://127.0.0.1:$port/json/list" -TimeoutSec 2 | Where-Object { $_.type -eq 'node' } | Select-Object -First 1 } catch {}
  if (-not $target) { Start-Sleep -Milliseconds 500 }
}
if (-not $target) { throw 'Claude inspector 没有启动；此 Windows 版本可能禁用了 Electron 调试参数' }

$ws = [Net.WebSockets.ClientWebSocket]::new()
$ws.ConnectAsync([Uri]$target.webSocketDebuggerUrl, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
$nextId = 0
function Invoke-CDP([string]$expression) {
  $script:nextId++
  $id = $script:nextId
  $payload = @{ id = $id; method = 'Runtime.evaluate'; params = @{ expression = $expression; returnByValue = $true; includeCommandLineAPI = $true } } | ConvertTo-Json -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
  $ws.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
  $buffer = New-Object byte[] 65536; $result = ''
  do { $part = $ws.ReceiveAsync([ArraySegment[byte]]::new($buffer), [Threading.CancellationToken]::None).GetAwaiter().GetResult(); $result += [Text.Encoding]::UTF8.GetString($buffer, 0, $part.Count) } while (-not $part.EndOfMessage)
  return ($result | ConvertFrom-Json)
}

$hookEscaped = $hook.Replace('\','\\').Replace("'", "\\'")
$localeEscaped = $Locale.Replace("'", "\\'")
$eval = "globalThis.__claudeCnTargetLocale='$localeEscaped'; require('$hookEscaped'); globalThis.__claudeCnHookInstalled === true"
$response = Invoke-CDP $eval
if (-not $response.result.result.value) { throw 'Claude hook 注入失败' }
try { Invoke-CDP "setImmediate(()=>require('node:inspector').close()); 'closing'" | Out-Null } catch {}
$ws.Dispose()
Write-Output 'Windows Claude 中文注入已完成'
