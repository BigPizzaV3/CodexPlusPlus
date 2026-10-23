param([string]$Locale = 'zh-Hans')
$ErrorActionPreference = 'Stop'
[Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
$entry = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_COPY)
$here = Split-Path $MyInvocation.MyCommand.Path -Parent
$hook = Join-Path $here 'hook.js'
$packaged = $env:CODEX_PLUS_CLAUDE_MSIX -eq '1'
$bashTimeout = if ($env:CODEX_PLUS_CLAUDE_BASH_TIMEOUT) { $env:CODEX_PLUS_CLAUDE_BASH_TIMEOUT | ConvertFrom-Json } else { $null }
$app = if ($packaged) { [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_SOURCE) } else { $entry }
if (-not (Test-Path -LiteralPath $app)) { throw "找不到 Claude：$app" }
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
$listener.Start()
$port = $listener.LocalEndpoint.Port
$listener.Stop()
$launchFile = Join-Path $here ('launch-' + [guid]::NewGuid().ToString('N') + '.json')
$environment = @{}
foreach ($name in @('BASH_DEFAULT_TIMEOUT_MS', 'BASH_MAX_TIMEOUT_MS')) {
  $value = [Environment]::GetEnvironmentVariable($name, 'Process')
  if (-not [string]::IsNullOrEmpty($value)) { $environment[$name] = $value }
}
if ($packaged) {
  $source = [IO.Path]::GetFullPath($env:CODEX_PLUS_CLAUDE_SOURCE)
  $package = @(Get-AppxPackage -Name Claude; Get-AppxPackage -Name AnthropicPBC.Claude) | Where-Object {
    $_.InstallLocation -and (Join-Path $_.InstallLocation 'app\Claude.exe') -eq $source
  } | Select-Object -First 1
  if (-not $package) { throw '找不到官方 Claude 包身份，请重新检测安装' }
  @{ app = $app; port = $port; environment = $environment } | ConvertTo-Json -Depth 4 | Set-Content -LiteralPath $launchFile -Encoding UTF8
  $wrapper = Join-Path $here 'start-in-package.ps1'
  # 隐藏的是脚本宿主，Claude 自身不设置隐藏窗口。
  $wrapperArgs = '-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File "' + $wrapper + '" -LaunchFile "' + $launchFile + '"'
  Invoke-CommandInDesktopPackage -PackageFamilyName $package.PackageFamilyName -AppId Claude -Command "$PSHOME\powershell.exe" -Args $wrapperArgs -PreventBreakaway
} else {
  $proc = Start-Process -FilePath $app -ArgumentList "--inspect=127.0.0.1:$port" -PassThru
}
$ws = $null
$cancel = $null
$created = $null
$nextId = 0
$verified = $false
$completed = $false
function Receive-CDP {
  $buffer = New-Object byte[] 65536
  $result = ''
  do {
    $part = $ws.ReceiveAsync([ArraySegment[byte]]::new($buffer), $cancel.Token).GetAwaiter().GetResult()
    if ($part.MessageType -eq [Net.WebSockets.WebSocketMessageType]::Close) { throw 'Claude inspector 连接已关闭' }
    $result += [Text.Encoding]::UTF8.GetString($buffer, 0, $part.Count)
  } while (-not $part.EndOfMessage)
  $response = $result | ConvertFrom-Json
  return $response
}
function Invoke-CDP([string]$expression, [string]$method = 'Runtime.evaluate', [hashtable]$parameters = @{}) {
  $script:nextId++
  $id = $script:nextId
  if ($method -eq 'Runtime.evaluate') { $parameters = @{ expression = $expression; returnByValue = $true; awaitPromise = $true } }
  $payload = @{ id = $id; method = $method; params = $parameters } | ConvertTo-Json -Compress
  $bytes = [Text.Encoding]::UTF8.GetBytes($payload)
  $ws.SendAsync([ArraySegment[byte]]::new($bytes), [Net.WebSockets.WebSocketMessageType]::Text, $true, $cancel.Token).GetAwaiter().GetResult() | Out-Null
  do {
    $response = Receive-CDP
  } while ($response.id -ne $id)
  if ($response.error) { throw "Claude 调试协议调用失败（$method）：$($response.error.message)" }
  if ($response.result.exceptionDetails) {
    $detail = [string]$response.result.exceptionDetails.exception.description
    $detail = ($detail -split '\r?\n')[0]
    if ($detail.Length -gt 240) { $detail = $detail.Substring(0, 240) }
    throw "Claude 主进程执行注入失败：$detail"
  }
  if ($method -eq 'Runtime.evaluate') { return $response.result.result.value }
  return $response.result
}
try {
  $deadline = (Get-Date).AddSeconds(45)
  $target = $null
  while ((Get-Date) -lt $deadline -and -not $target) {
    if (Test-Path -LiteralPath ($launchFile + '.error')) { throw (Get-Content -LiteralPath ($launchFile + '.error') -Raw -Encoding UTF8) }
    if ($packaged -and -not (Test-Path -LiteralPath ($launchFile + '.result'))) { Start-Sleep -Milliseconds 100; continue }
    if (-not $verified) {
      if ($packaged) {
        $launchResult = Get-Content -LiteralPath ($launchFile + '.result') -Raw -Encoding UTF8 | ConvertFrom-Json
        if ($launchResult.memoryFuseRestored -ne $true) { throw '启动调试开关未确认还原，未继续注入' }
        if ($launchResult.inspectorOwnedByChild -ne $true) { throw '未确认调试端口归属，未继续注入' }
      }
      $expectedPid = if ($packaged) { $launchResult.pid } else { $proc.Id }
      # Node 引导期尚未填充 process.pid / execPath，先校验本次新进程；连接失败也必须回收。
      $created = Get-Process -Id $expectedPid -ErrorAction Stop
      if ([IO.Path]::GetFullPath($created.Path) -ne $app) { throw '新进程路径与 Claude 不符，未继续注入' }
      $verified = $true
    }
    try { $target = Invoke-RestMethod "http://127.0.0.1:$port/json/list" -TimeoutSec 2 | Where-Object { $_.type -eq 'node' } | Select-Object -First 1 } catch {}
    if (-not $target) { Start-Sleep -Milliseconds 250 }
  }
  if (-not $target) { throw 'Claude inspector 未启动；未向其他进程注入' }
  $ws = [Net.WebSockets.ClientWebSocket]::new()
  $cancel = [Threading.CancellationTokenSource]::new(15000)
  $ws.ConnectAsync([Uri]$target.webSocketDebuggerUrl, $cancel.Token).GetAwaiter().GetResult() | Out-Null
  $options = @{
    app = $app; hook = $hook; locale = $Locale; packaged = $packaged
    overlay = $(if ($packaged) { Join-Path (Split-Path $entry -Parent) '.codex-plus-overlay' } else { $null })
    voiceInput = $env:CODEX_PLUS_CLAUDE_VOICE_INPUT -ne '0'
    localization = $env:CODEX_PLUS_CLAUDE_LOCALIZATION -ne '0'
    environment = $environment
    bashTimeout = $bashTimeout
    pid = $expectedPid
  } | ConvertTo-Json -Depth 4 -Compress
  $bootstrap = @"
(() => {
  const options = $options;
  if (!Number.isInteger(process.pid) || typeof process.resourcesPath !== 'string' || (options.packaged && process.windowsStore === undefined)) return false;
  const req = process.getBuiltinModule('module').createRequire(process.resourcesPath + '/app.asar/package.json');
  const fs = req('node:fs');
  if (process.pid !== options.pid || fs.realpathSync(process.execPath).toLowerCase() !== fs.realpathSync(options.app).toLowerCase()) throw Error('Unexpected Claude process');
  if (options.packaged && process.windowsStore !== true) throw Error('Missing package identity');
  globalThis.__claudeCnBashTimeout = options.bashTimeout;
  globalThis.__claudeCnBashTimeoutReady = req(req('node:path').join(req('node:path').dirname(options.hook), 'bash-timeout.js')).installTimeoutOverrides(options.bashTimeout);
  globalThis.__claudeCnBashTimeoutReady.catch(() => {});
  return true;
})()
"@
  $bootstrapped = $false
  while (-not $bootstrapped) {
    $bootstrapped = Invoke-CDP $bootstrap
    if (-not $bootstrapped) { Start-Sleep -Milliseconds 100 }
  }
  $eval = @"
(async () => {
  const options = $options;
  const req = typeof require === 'function' ? require : process.getBuiltinModule('module').createRequire(process.resourcesPath + '/app.asar/package.json');
  const fs = req('node:fs');
  if (fs.realpathSync(process.execPath).toLowerCase() !== fs.realpathSync(options.app).toLowerCase()) throw Error('Unexpected Claude executable');
  if (options.packaged && process.windowsStore !== true) throw Error('Missing package identity');
  for (const [key, value] of Object.entries(options.environment)) {
    if (process.env[key] !== value) throw Error('Timeout environment was not inherited');
  }
  const electron = req('electron');
  await electron.app.whenReady();
  globalThis.__claudeCnTargetLocale = options.locale;
  globalThis.__claudeCnVoiceInput = options.voiceInput;
  globalThis.__claudeCnLocalization = options.localization;
  globalThis.__claudeCnOverlayDir = options.overlay;
  globalThis.__claudeCnBashTimeout = options.bashTimeout;
  req(options.hook);
  if (globalThis.__claudeCnHookInstalled !== true) throw Error('Hook not installed');
  await Promise.race([
    globalThis.__claudeCnInjectionReady,
    new Promise((_, reject) => setTimeout(() => reject(Error('Main window injection did not complete')), 10000))
  ]);
  if (!electron.BrowserWindow.getAllWindows().some(w => !w.isDestroyed())) throw Error('No Claude main window; another instance may have intercepted startup');
  return { pid: process.pid, executable: process.execPath, windowsStore: process.windowsStore === true, userData: electron.app.getPath('userData'), hookInstalled: true, defaultTimeout: process.env.BASH_DEFAULT_TIMEOUT_MS || null, maxTimeout: process.env.BASH_MAX_TIMEOUT_MS || null, coworkBashTimeout: globalThis.__claudeCnBashTimeoutResult };
})()
"@
  $diagnostics = Invoke-CDP $eval
  $completed = $true
} finally {
  try {
    if ($verified -and -not $completed) {
      if (-not $created.HasExited) { $created.Kill() }
    } elseif ($verified) {
      try {
        $cancel.CancelAfter(3000)
        $null = Invoke-CDP "setTimeout(()=>(typeof require === 'function' ? require('node:inspector') : process.getBuiltinModule('inspector')).close(), 250); 'closing'"
        Start-Sleep -Milliseconds 600
        $client = [Net.Sockets.TcpClient]::new()
        try { $open = $client.ConnectAsync('127.0.0.1', $port).Wait(300) -and $client.Connected } catch { $open = $false } finally { $client.Dispose() }
        if ($open) { throw '调试入口仍然开放' }
      } catch {
        if (-not $created.HasExited) { $created.Kill() }
        throw '无法确认调试入口已关闭，已结束本次 Claude 启动，请重试'
      }
    }
  } finally {
    if ($ws) { $ws.Dispose() }
    if ($cancel) { $cancel.Dispose() }
    if ($created) { $created.Dispose() }
  }
}
$diagnostics | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $here 'launch-diagnostics.json') -Encoding UTF8
Write-Output 'Windows Claude 包身份与增强注入校验通过'
