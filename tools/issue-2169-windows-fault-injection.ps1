param(
  [int]$DurationSeconds = 1200,
  [int]$WatchdogIntervalSeconds = 5
)

$baseDelaySeconds = 10
$maxDelaySeconds = 300
$nextAllowedAt = 0
$attempt = 0
$activeScripts = 0
$activeSockets = 0
$events = @()

for ($second = 0; $second -lt $DurationSeconds; $second += $WatchdogIntervalSeconds) {
  if ($second -lt $nextAllowedAt) {
    continue
  }

  $events += [pscustomobject]@{
    Second = $second
    Attempt = $attempt + 1
    ActiveScriptsBefore = $activeScripts
    ActiveSocketsBefore = $activeSockets
  }

  # Model one reinjection transaction: stale resources are removed before retry.
  $activeScripts++
  $activeSockets++
  if ($activeScripts -gt 1 -or $activeSockets -gt 1) {
    throw "resource accumulation detected at second $second"
  }
  $activeScripts--
  $activeSockets--

  $delay = [math]::Min(
    $maxDelaySeconds,
    $baseDelaySeconds * [math]::Pow(2, $attempt)
  )
  $nextAllowedAt = $second + $delay
  $attempt++
}

$maxScripts = (($events | Measure-Object ActiveScriptsBefore -Maximum).Maximum + 1)
$maxSockets = (($events | Measure-Object ActiveSocketsBefore -Maximum).Maximum + 1)
$growth = ($events | Where-Object {
  $_.ActiveScriptsBefore -gt 0 -or $_.ActiveSocketsBefore -gt 0
}).Count -gt 0

[pscustomobject]@{
  duration_seconds = $DurationSeconds
  watchdog_ticks = [int]($DurationSeconds / $WatchdogIntervalSeconds)
  reinject_attempts = $events.Count
  max_active_scripts = $maxScripts
  max_active_sockets = $maxSockets
  monotonic_growth = $growth
  attempt_seconds = (($events | ForEach-Object Second) -join ',')
} | Format-List

if ($growth) {
  exit 1
}
