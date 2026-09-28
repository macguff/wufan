param(
    [switch] $Unregister
)

# One-command TSF probe registration. Self-elevates via UAC.
$ErrorActionPreference = 'Stop'
$dll = Join-Path $PSScriptRoot 'target\x86_64-pc-windows-msvc\release\ime_windows_tsf.dll'

if (-not (Test-Path $dll)) {
    Write-Host "DLL not found, building release..." -ForegroundColor Cyan
    cargo build --locked -p ime-windows-tsf --release --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw "build failed" }
}

$action = if ($Unregister) { '/u' } else { '' }
$verb   = if ($Unregister) { 'Unregistering' } else { 'Registering' }

# Re-invoke this script elevated if we are not admin.
$isAdmin = ([Security.Principal.WindowsPrincipal] `
    [Security.Principal.WindowsIdentity]::GetCurrent()
).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)

if (-not $isAdmin) {
    Write-Host "$verb (elevating via UAC)..." -ForegroundColor Cyan
    $argList = @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$PSCommandPath`"")
    if ($Unregister) { $argList += '-Unregister' }
    $p = Start-Process powershell.exe -ArgumentList $argList -Verb RunAs -Wait -PassThru
    exit $p.ExitCode
}

Write-Host "$verb $dll" -ForegroundColor Cyan
$args = @('/s')
if ($Unregister) { $args += '/u' }
$args += "`"$dll`""
$p = Start-Process regsvr32.exe -ArgumentList $args -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "regsvr32 failed with exit code $($p.ExitCode)" }

if ($Unregister) {
    Write-Host "Unregistered." -ForegroundColor Green
} else {
    Write-Host "Registered. Sign out/in, then pick 'Wufan TSF Probe' under Chinese (Simplified)." -ForegroundColor Green
}
