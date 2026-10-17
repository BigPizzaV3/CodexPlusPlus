param([string]$Locale = 'zh-Hans')
$ErrorActionPreference = 'Stop'
$app = $env:CODEX_PLUS_CLAUDE_COPY
$hook = Join-Path (Split-Path $MyInvocation.MyCommand.Path -Parent) 'hook.js'
if (-not (Test-Path $app)) { throw "找不到 Claude：$app" }
$port = 19000 + (Get-Random -Minimum 0 -Maximum 1000)
# 不带 -WindowStyle Hidden 时 Claude 会拿到一个可见控制台，用户关掉它等于直接结束
# Claude（CTRL_CLOSE_EVENT），所以必须隐藏。
# 这里不要加 -WindowStyle Hidden：实测（MSIX 版 Claude 2.37337.1.0）加上之后
# Claude 能起来，但 CDP 注入不会生效，脚本最后会抛 'Claude hook 注入失败'。
# 父进程已经由 run_script 用 CREATE_NO_WINDOW 隐藏，不需要再设子进程窗口样式。
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
# Inspector target 可能早于 Electron app ready 出现；此时 session.defaultSession 还不可用。
# 清掉失败模块的缓存并短暂重试，等主进程 ready 后再完成 hook 安装。
$eval = "globalThis.__claudeCnTargetLocale='$localeEscaped'; try { delete require.cache[require.resolve('$hookEscaped')] } catch {}; require('$hookEscaped'); globalThis.__claudeCnHookInstalled === true"
$hookDeadline = (Get-Date).AddSeconds(45)
$installed = $false
while ((Get-Date) -lt $hookDeadline) {
  $response = Invoke-CDP $eval
  if ($response.result.result.value) { $installed = $true; break }
  Start-Sleep -Milliseconds 250
}
if (-not $installed) { throw 'Claude hook 注入失败' }
try { Invoke-CDP "setImmediate(()=>require('node:inspector').close()); 'closing'" | Out-Null } catch {}
$ws.Dispose()
Write-Output 'Windows Claude 中文注入已完成'
