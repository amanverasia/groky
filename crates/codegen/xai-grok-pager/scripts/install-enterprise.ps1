# Groky enterprise PowerShell installer compatibility entrypoint.
# Windows release assets are not available. Use a supported Linux release instead.
param(
    [Parameter(Position = 0)]
    [string]$Version
)

$ErrorActionPreference = 'Stop'
Write-Error 'Windows release assets are not available; Windows is not supported.'
exit 1
