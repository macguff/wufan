# Compile both adapters, register once via UAC, then run ordinary-user hosts.
param(
    [switch]$SkipBuild,
    [switch]$FaultScenarios,
    [ValidateSet('Wpf', 'WinForms')][string[]]$HostKind = @('Wpf', 'WinForms'),
    [ValidateRange(0, 200)][int[]]$KeyDelayMs = @(80, 0)
)
$ErrorActionPreference = 'Stop'
$taskRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$smoke = Join-Path $PSScriptRoot 'tsf_input_smoke.ps1'
$wrapper = Join-Path $PSScriptRoot 'run_tsf_smoke_elevated.ps1'
$ready = Join-Path $taskRoot 'target/tsf-registration.ready'
$done = Join-Path $taskRoot 'target/tsf-registration.done'
$cleanupResult = Join-Path $taskRoot 'target/tsf-registration-helper.result'
if ($HostKind.Count -eq 0 -or $KeyDelayMs.Count -eq 0) { throw 'Select at least one host and input interval.' }
Push-Location $taskRoot
try {
    if (-not $SkipBuild) {
        cargo build --locked -p ime-windows-tsf -p ime-broker --release --target x86_64-pc-windows-msvc
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
    }
    if ($FaultScenarios) {
        cargo build --locked -p ime-broker --features fault-injection --release --target x86_64-pc-windows-msvc --target-dir target/fault-acceptance
        if ($LASTEXITCODE -ne 0) { throw "Fault acceptance Broker build failed: $LASTEXITCODE" }
    }
    foreach ($hostName in $HostKind) {
        & powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File $smoke -SkipBuild -CompileOnly -HostKind $hostName
        if ($LASTEXITCODE -ne 0) { throw "$hostName host compilation failed" }
    }
    Remove-Item -LiteralPath $ready, $done, $cleanupResult -Force -ErrorAction SilentlyContinue
    Write-Host 'Allow the Windows UAC registration helper; keep each test window in front during input.'
    $helper = Start-Process powershell.exe -Verb RunAs -WindowStyle Hidden -PassThru -ArgumentList @(
        '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', "`"$wrapper`"", '-RegisterOnly'
    )
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        while (-not (Test-Path -LiteralPath $ready)) {
            if ($helper.HasExited) { throw "Registration helper exited: $(Get-Content -LiteralPath $cleanupResult -ErrorAction SilentlyContinue)" }
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Registration helper did not become ready' }
            Start-Sleep -Milliseconds 200
        }
        foreach ($hostName in $HostKind) {
            foreach ($delay in $KeyDelayMs) {
                Write-Host "Running $hostName at $delay ms"
                $scenarioArguments = @('-NoProfile', '-STA', '-ExecutionPolicy', 'Bypass', '-File', $smoke, '-SkipBuild', '-RegistrationReady', '-HostKind', $hostName, '-KeyDelayMs', $delay)
                if ($FaultScenarios) { $scenarioArguments += '-FaultScenarios' }
                $artifactSuffix = if ($FaultScenarios) { '-faults' } else { '' }
                & powershell.exe @scenarioArguments | Tee-Object -FilePath (Join-Path $taskRoot "target/tsf-$hostName-$delay$artifactSuffix.console.log")
                if ($LASTEXITCODE -ne 0) { throw "$hostName acceptance failed at $delay ms; see target/tsf-$hostName-$delay$artifactSuffix.result and .trace" }
            }
        }
    } finally {
        [IO.File]::WriteAllText($done, 'DONE')
        if (-not $helper.WaitForExit(15000)) { Write-Warning 'Registration helper cleanup still pending; inspect target/tsf-registration-helper.result.' }
    }
    if (-not (Test-Path -LiteralPath $cleanupResult) -or
        [IO.File]::ReadAllText($cleanupResult).Trim() -ne 'Cleanup exit code: 0') {
        throw 'Temporary registration cleanup was not confirmed.'
    }
    Write-Host 'TSF host matrix passed; temporary registration cleanup succeeded.'
} finally { Pop-Location }
