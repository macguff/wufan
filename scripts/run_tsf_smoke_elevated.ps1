# Run this wrapper through Windows UAC; retain the result under the workspace.
param([switch]$RegisterOnly)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$report = Join-Path $taskRoot 'target\tsf-desktop-smoke.result'
if ($RegisterOnly) {
    $dll = Join-Path $taskRoot 'target\x86_64-pc-windows-msvc\release\ime_windows_tsf.dll'
    $ready = Join-Path $taskRoot 'target\tsf-registration.ready'
    $done = Join-Path $taskRoot 'target\tsf-registration.done'
    $env:WUFAN_TSF_PROBE_TRACE = Join-Path $taskRoot 'target\tsf-registration-admin.trace'
    $registered = $false
    try {
        $registration = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', "`"$dll`"") -WindowStyle Hidden -Wait -PassThru
        if ($registration.ExitCode -ne 0) { throw "Registration failed: $($registration.ExitCode)" }
        $registered = $true
        [IO.File]::WriteAllText($ready, 'READY')
        $deadline = [DateTime]::UtcNow.AddMinutes(3)
        while (-not (Test-Path -LiteralPath $done) -and [DateTime]::UtcNow -lt $deadline) { Start-Sleep -Milliseconds 200 }
    } catch { [IO.File]::WriteAllText((Join-Path $taskRoot 'target\tsf-registration-helper.result'), "FAIL: $_"); exit 1 }
    finally {
        if ($registered) {
            $cleanup = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', '/u', "`"$dll`"") -WindowStyle Hidden -Wait -PassThru
            [IO.File]::WriteAllText((Join-Path $taskRoot 'target\tsf-registration-helper.result'), "Cleanup exit code: $($cleanup.ExitCode)")
        }
    }
    exit 0
}
try {
    $identity = [Security.Principal.WindowsIdentity]::GetCurrent()
    $principal = [Security.Principal.WindowsPrincipal]::new($identity)
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'TSF machine profile registration requires Windows administrator elevation.' }
    & (Join-Path $PSScriptRoot 'tsf_input_smoke.ps1') -SkipBuild
    [IO.File]::WriteAllText($report, 'PASS: actual WPF input through TSF/Broker/librime and temporary registration cleanup.')
    exit 0
} catch {
    [IO.File]::WriteAllText($report, "FAIL: $($_.Exception.ToString())")
    exit 1
}
