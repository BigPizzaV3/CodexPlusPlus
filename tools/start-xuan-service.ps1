[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [string]$ExecutablePath,

    [string]$HttpAddress = ''
)

$ErrorActionPreference = 'Stop'
$startParameters = @{
    FilePath = $ExecutablePath
    WindowStyle = 'Hidden'
}

if (-not [string]::IsNullOrWhiteSpace($HttpAddress)) {
    $startParameters.ArgumentList = @('--http', $HttpAddress)
}

Start-Process @startParameters | Out-Null
