param(
  [Parameter(Mandatory = $true)]
  [string]$ExecutablePath
)

$ErrorActionPreference = 'Stop'
$resolvedExecutablePath = (Resolve-Path -LiteralPath $ExecutablePath).Path
$temporaryRoot = Join-Path ([System.IO.Path]::GetTempPath()) ("workagents-remote-cloud-" + [Guid]::NewGuid().ToString('N'))
$resolvedSystemTemporaryRoot = [System.IO.Path]::GetFullPath([System.IO.Path]::GetTempPath())
$serviceProcess = $null

try {
  [void](New-Item -ItemType Directory -Path $temporaryRoot)
  $databasePath = Join-Path $temporaryRoot 'remote.sqlite3'
  $stdoutPath = Join-Path $temporaryRoot 'stdout.log'
  $stderrPath = Join-Path $temporaryRoot 'stderr.log'

  $portProbe = [System.Net.Sockets.TcpListener]::new([System.Net.IPAddress]::Loopback, 0)
  $portProbe.Start()
  $listenPort = ([System.Net.IPEndPoint]$portProbe.LocalEndpoint).Port
  $portProbe.Stop()

  $tokenKeyBytes = [byte[]]::new(32)
  [System.Security.Cryptography.RandomNumberGenerator]::Fill($tokenKeyBytes)
  $tokenKey = [Convert]::ToBase64String($tokenKeyBytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
  $previousEnvironment = $env:WORKAGENTS_REMOTE_ENVIRONMENT
  $previousListen = $env:WORKAGENTS_REMOTE_LISTEN
  $previousDatabase = $env:WORKAGENTS_REMOTE_DATABASE
  $previousTokenKey = $env:WORKAGENTS_REMOTE_PUSH_TOKEN_KEY
  $previousHuaweiSendUrl = $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL
  $previousHuaweiServiceAccount = $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64
  try {
    $env:WORKAGENTS_REMOTE_ENVIRONMENT = 'dev'
    $env:WORKAGENTS_REMOTE_LISTEN = "127.0.0.1:$listenPort"
    $env:WORKAGENTS_REMOTE_DATABASE = $databasePath
    $env:WORKAGENTS_REMOTE_PUSH_TOKEN_KEY = $tokenKey
    $serviceAccountProjectId = '123456'
    $serviceAccountKey = [System.Security.Cryptography.RSA]::Create(2048)
    try {
      $serviceAccount = [ordered]@{
        project_id = $serviceAccountProjectId
        key_id = 'local-verification-key'
        private_key = $serviceAccountKey.ExportPkcs8PrivateKeyPem()
        sub_account = 'local-verification-service-account'
      } | ConvertTo-Json -Compress
      $serviceAccountBytes = [System.Text.Encoding]::UTF8.GetBytes($serviceAccount)
      $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64 = [Convert]::ToBase64String($serviceAccountBytes).TrimEnd('=').Replace('+', '-').Replace('/', '_')
    } finally {
      $serviceAccountKey.Dispose()
    }
    $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL = "https://push-api.cloud.huawei.com/v3/$serviceAccountProjectId/messages:send"
    $serviceProcess = Start-Process -FilePath $resolvedExecutablePath -PassThru -WindowStyle Hidden -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath
  } finally {
    $env:WORKAGENTS_REMOTE_ENVIRONMENT = $previousEnvironment
    $env:WORKAGENTS_REMOTE_LISTEN = $previousListen
    $env:WORKAGENTS_REMOTE_DATABASE = $previousDatabase
    $env:WORKAGENTS_REMOTE_PUSH_TOKEN_KEY = $previousTokenKey
    $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SEND_URL = $previousHuaweiSendUrl
    $env:WORKAGENTS_REMOTE_HUAWEI_PUSH_SERVICE_ACCOUNT_JSON_B64 = $previousHuaweiServiceAccount
  }

  $healthUri = "http://127.0.0.1:$listenPort/healthz"
  $deadline = [DateTimeOffset]::UtcNow.AddSeconds(10)
  $health = $null
  do {
    if ($serviceProcess.HasExited) {
      $stderr = if (Test-Path -LiteralPath $stderrPath) { Get-Content -LiteralPath $stderrPath -Raw } else { '' }
      throw "Cloud service exited before health verification: $stderr"
    }
    try {
      $health = Invoke-RestMethod -Method Get -Uri $healthUri -TimeoutSec 1
    } catch [System.Net.Http.HttpRequestException] {
      $health = $null
    } catch [System.Net.WebException] {
      $health = $null
    } catch [System.Threading.Tasks.TaskCanceledException] {
      $health = $null
    }
  } while ($null -eq $health -and [DateTimeOffset]::UtcNow -lt $deadline)

  if ($null -eq $health) {
    throw 'Cloud service health endpoint was not ready before the bounded deadline.'
  }
  if ($health.ok -ne $true -or $health.environment -ne 'dev' -or
    $health.service_version -ne '1.0.0' -or $health.contract_version -ne '2.0' -or
    $health.storage_schema_version -ne 8) {
    throw 'Cloud service health response did not match the registered development environment.'
  }

  $sentAt = [DateTimeOffset]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ssZ')
  $legacyRequest = [ordered]@{
    schemaVersion = '1.5'
    messageType = 'app/device-registration-challenge'
    messageId = 'legacy_message_0001'
    environment = 'dev'
    sentAt = $sentAt
    appDeviceId = 'legacy_device_0001'
    deviceKeyAlgorithm = 'ed25519'
    devicePublicKey = 'MCowBQYDK2VwAyEAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA'
  } | ConvertTo-Json -Compress
  $legacyResponse = Invoke-WebRequest -Method Post -Uri "http://127.0.0.1:$listenPort/v1/app-devices/challenges" `
    -ContentType 'application/json' -Body $legacyRequest -SkipHttpErrorCheck -TimeoutSec 3
  $legacyError = $legacyResponse.Content | ConvertFrom-Json
  if ($legacyResponse.StatusCode -ne 400 -or $legacyError.schemaVersion -ne '2.0' -or
    $legacyError.errorCode -ne 'invalid_request') {
    throw 'Protocol 1.5 request was not rejected by the 2.0 HTTP interface.'
  }

  $encodedSentAt = [Uri]::EscapeDataString($sentAt)
  $queryUri = "http://127.0.0.1:$listenPort/v1/pc-devices?schemaVersion=2.0&messageType=app%2Fpc-devices-query&messageId=auth_probe_000001&environment=dev&sentAt=$encodedSentAt&appDeviceId=auth_probe_device_01"
  $authResponse = Invoke-WebRequest -Method Get -Uri $queryUri -SkipHttpErrorCheck -TimeoutSec 3
  $authError = $authResponse.Content | ConvertFrom-Json
  if ($authResponse.StatusCode -ge 200 -and $authResponse.StatusCode -lt 300) {
    throw 'Unauthenticated app request unexpectedly succeeded.'
  }
  if ($authError.schemaVersion -ne '2.0' -or $authError.messageType -ne 'error') {
    throw 'Unauthenticated app request did not fail through the current protocol envelope.'
  }
  [ordered]@{
    ok = $true
    environment = $health.environment
    serviceVersion = $health.service_version
    contractVersion = $health.contract_version
    storageSchemaVersion = $health.storage_schema_version
    loopbackOnly = $true
    legacyProtocolRejected = $true
    unauthenticatedRequestRejected = $true
  } | ConvertTo-Json -Compress
} finally {
  if ($null -ne $serviceProcess -and -not $serviceProcess.HasExited) {
    Stop-Process -Id $serviceProcess.Id
    [void]$serviceProcess.WaitForExit(5000)
  }
  if (Test-Path -LiteralPath $temporaryRoot) {
    $resolvedTemporaryRoot = (Resolve-Path -LiteralPath $temporaryRoot).Path
    if (-not $resolvedTemporaryRoot.StartsWith($resolvedSystemTemporaryRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Refusing to remove an unexpected temporary path: $resolvedTemporaryRoot"
    }
    Remove-Item -LiteralPath $resolvedTemporaryRoot -Recurse -Force
  }
}
