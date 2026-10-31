param([string]$Locale = 'zh-Hans')
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$app = $env:CODEX_PLUS_CLAUDE_COPY
$hook = Join-Path (Split-Path $MyInvocation.MyCommand.Path -Parent) 'hook.js'
if (-not (Test-Path $app)) { throw "找不到 Claude：$app" }
$port = 19000 + (Get-Random -Minimum 0 -Maximum 1000)
$proc = Start-Process -FilePath $app -ArgumentList "--inspect=127.0.0.1:$port" -PassThru
$deadline = (Get-Date).AddSeconds(45)
$target = $null
while ((Get-Date) -lt $deadline -and -not $target) {
  try { $target = Invoke-RestMethod "http://127.0.0.1:$port/json/list" -TimeoutSec 2 | Where-Object { $_.type -eq 'node' } | Select-Object -First 1 } catch {}
  # 进程已经退出就不用再等满 45 秒（#2818）。
  if (-not $target -and $proc.HasExited) { break }
  if (-not $target) { Start-Sleep -Milliseconds 500 }
}
if (-not $target) {
  $why = $(if ($proc.HasExited) { "Claude 在调试端口就绪前就退出了（退出码 $($proc.ExitCode)）" } else { '45 秒内没有等到 Claude 调试端口' })
  throw "Claude inspector 没有启动：$why（$app，端口 127.0.0.1:$port）。常见原因是启动的 Claude 关闭了 Electron 调试开关（EnableNodeCliInspectArguments fuse），或 Claude 已在后台运行把启动请求转交给了旧进程。请在任务管理器里结束所有 Claude 进程，再在 Codex++「Claude 增强」里点「制作增强版 / 更新副本」重新制作副本后启动；仍失败请在 issue 中附上 operation.log。"
}

$ws = [Net.WebSockets.ClientWebSocket]::new()
$ws.ConnectAsync([Uri]$target.webSocketDebuggerUrl, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
$nextId = 0
function Invoke-CDP([string]$expression) {
  $script:nextId++
  $id = $script:nextId
  $payload = @{ id = $id; method = 'Runtime.evaluate'; params = @{ expression = $expression; returnByValue = $true; includeCommandLineAPI = $true; awaitPromise = $true } } | ConvertTo-Json -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
  $ws.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, [Threading.CancellationToken]::None).GetAwaiter().GetResult()
  $buffer = New-Object byte[] 65536; $result = ''
  do { $part = $ws.ReceiveAsync([ArraySegment[byte]]::new($buffer), [Threading.CancellationToken]::None).GetAwaiter().GetResult(); $result += [Text.Encoding]::UTF8.GetString($buffer, 0, $part.Count) } while (-not $part.EndOfMessage)
  return ($result | ConvertFrom-Json)
}

$hookEscaped = $hook.Replace('\','\\').Replace("'", "\\'")
$localeEscaped = $Locale.Replace("'", "\\'")
$timeoutJson = $(if ($env:CODEX_PLUS_CLAUDE_BASH_TIMEOUT) { $env:CODEX_PLUS_CLAUDE_BASH_TIMEOUT } else { 'null' }) | ConvertTo-Json -Compress
$timeoutHelper = Join-Path (Split-Path $hook -Parent) 'bash-timeout.js'
$timeoutHelperJson = $timeoutHelper | ConvertTo-Json -Compress
$timeoutReportJson = @{ file = $env:CODEX_PLUS_CLAUDE_BASH_REPORT_FILE; launchId = $env:CODEX_PLUS_CLAUDE_BASH_LAUNCH_ID } | ConvertTo-Json -Compress
$bootstrap = @"
(() => {
  try {
    globalThis.__claudeCnBashTimeout = JSON.parse($timeoutJson);
    require($timeoutHelperJson).beginTimeoutOverrides(globalThis.__claudeCnBashTimeout, process.resourcesPath, $timeoutReportJson);
  } catch (error) {
    globalThis.__claudeCnBashTimeoutResult = { enabled: true, prepared: false, error: String(error.message || error) };
    globalThis.__claudeCnBashTimeoutReady = Promise.resolve(globalThis.__claudeCnBashTimeoutResult);
  }
  return true;
})()
"@
$null = Invoke-CDP $bootstrap
$eval = "globalThis.__claudeCnTargetLocale='$localeEscaped'; require('$hookEscaped'); globalThis.__claudeCnHookInstalled === true"
$response = Invoke-CDP $eval
if (-not $response.result.result.value) { throw 'Claude hook 注入失败' }
try {
  $timeoutResponse = Invoke-CDP 'globalThis.__claudeCnBashTimeoutReady'
  $timeoutResult = $timeoutResponse.result.result.value
  if ($timeoutResponse.result.exceptionDetails -or -not $timeoutResult) { throw '未收到 Cowork Bash 适配结果' }
  Write-Output ('CODEX_PLUS_CLAUDE_BASH_RESULT=' + ($timeoutResult | ConvertTo-Json -Compress))
  if ($timeoutResult.error) { Write-Warning ('Cowork Bash 时限增强未生效，其他增强继续启动：' + $timeoutResult.error) }
} catch {
  Write-Warning 'Cowork Bash 适配结果不可用，其他增强继续启动。'
}
try { Invoke-CDP "setImmediate(()=>require('node:inspector').close()); 'closing'" | Out-Null } catch {}
$ws.Dispose()
Write-Output 'Windows Claude 中文注入已完成'
