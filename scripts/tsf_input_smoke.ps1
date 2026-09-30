param([switch]$CompileOnly, [switch]$SkipBuild, [switch]$RegistrationReady, [switch]$FaultScenarios, [ValidateRange(0, 200)][int]$KeyDelayMs = 80, [ValidateSet('Wpf', 'WinForms')][string]$HostKind = 'Wpf')

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = 'x86_64-pc-windows-msvc'
$dll = Join-Path $repoRoot "target\$target\release\ime_windows_tsf.dll"
$resultPath = Join-Path $env:TEMP 'wufan-tsf-input-smoke.txt'
$tracePath = Join-Path $env:TEMP 'wufan-tsf-registration.trace'
$artifactPrefix = Join-Path $repoRoot "target/tsf-$HostKind-$KeyDelayMs"
if ($FaultScenarios) { $artifactPrefix += '-faults' }
$registered = $false
$brokerProcess = $null

Push-Location $repoRoot
try {
    if (-not $SkipBuild) {
        cargo build --locked -p ime-windows-tsf -p ime-broker --release --target $target
        if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }
    }

    $sourceDir = Join-Path $repoRoot 'tools/tsf-input-smoke'
    $source = [IO.File]::ReadAllText((Join-Path $sourceDir 'Scenarios.cs')) + [Environment]::NewLine +
        [IO.File]::ReadAllText((Join-Path $sourceDir "Host.$HostKind.cs"))
    $assemblies = if ($HostKind -eq 'Wpf') {
        @('PresentationFramework', 'PresentationCore', 'WindowsBase', 'System.Xaml')
    } else {
        @('System.Windows.Forms', 'System.Drawing')
    }
    Add-Type -TypeDefinition $source -Language CSharp -ReferencedAssemblies $assemblies
    if ($CompileOnly) {
        Write-Host "TSF $HostKind smoke host compiled successfully."
        return
    }

    $env:WUFAN_TSF_PROBE_TRACE = $tracePath
    Remove-Item -LiteralPath $tracePath -Force -ErrorAction SilentlyContinue
    if (-not $RegistrationReady) {
    $register = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" `
        -ArgumentList @('/s', "`"$dll`"") -WindowStyle Hidden -Wait -PassThru
    if ($register.ExitCode -ne 0) {
        if (Test-Path -LiteralPath $tracePath) {
            Write-Host 'Registration trace:'
            Get-Content -LiteralPath $tracePath
        }
        throw "DLL registration failed: $($register.ExitCode)"
    }
    $registered = $true
    }

    Remove-Item -LiteralPath "$artifactPrefix.result", "$artifactPrefix.trace" -Force -ErrorAction SilentlyContinue
    $brokerExe = Join-Path $repoRoot "target\$target\release\ime-broker.exe"
    if ($FaultScenarios) {
        $brokerExe = Join-Path $repoRoot "target\fault-acceptance\$target\release\ime-broker.exe"
        if (-not (Test-Path -LiteralPath $brokerExe)) { throw 'Build the separate fault-injection Broker with tsf_input_matrix.ps1 -FaultScenarios first.' }
        $faultDirectory = "$artifactPrefix-markers"
        [IO.Directory]::CreateDirectory($faultDirectory) | Out-Null
        $env:WUFAN_TSF_SMOKE_FAULT_DIR = $faultDirectory
        $env:WUFAN_TSF_SMOKE_FAULTS = '1'
    }
    $env:WUFAN_BROKER_EXE = $brokerExe
    $env:WUFAN_TSF_SMOKE_CHINESE = '1'
    $env:WUFAN_TSF_SMOKE_KEY_DELAY = [string]$KeyDelayMs
    $hostExe = (Get-Process -Id $PID).Path
    $brokerProcess = Start-Process -FilePath $brokerExe -ArgumentList @('--dev-client', "`"$hostExe`"") -WindowStyle Hidden `
        -RedirectStandardOutput (Join-Path $repoRoot 'target\tsf-broker.out') -RedirectStandardError (Join-Path $repoRoot 'target\tsf-broker.err') -PassThru
    Start-Sleep -Milliseconds 500
    if ($brokerProcess.HasExited) { throw "Broker startup failed: $(Get-Content (Join-Path $repoRoot 'target\tsf-broker.err') -Raw)" }
    $env:WUFAN_TSF_SMOKE_BROKER_PID = [string]$brokerProcess.Id
    $env:WUFAN_TSF_SMOKE_HOST_EXE = $hostExe

    $env:WUFAN_TSF_SMOKE_RESULT = $resultPath
    Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue

    $smokeExitCode = [WufanTsfInputSmoke]::Run()
    $actualText = if (Test-Path -LiteralPath $resultPath) {
        Get-Content -LiteralPath $resultPath -Raw -Encoding UTF8
    } else {
        '<no result returned>'
    }
    [IO.File]::WriteAllText((Join-Path $repoRoot 'target\tsf-desktop-smoke.result'), $actualText)
    [IO.File]::WriteAllText("$artifactPrefix.result", $actualText)
    if ($smokeExitCode -ne 0) {
        throw "TSF input smoke failed. TextBox contained: [$actualText]"
    }
    Write-Host "TSF input smoke passed: [$actualText]"
}
finally {
    if ($brokerProcess -and -not $brokerProcess.HasExited) { $brokerProcess.Kill() }
    if ($registered) {
        $unregister = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" `
            -ArgumentList @('/s', '/u', "`"$dll`"") -WindowStyle Hidden -Wait -PassThru
        if ($unregister.ExitCode -ne 0) {
            Write-Warning "DLL unregistration failed: $($unregister.ExitCode)"
        }
    }
    if (-not $CompileOnly -and (Test-Path -LiteralPath $tracePath)) {
        Copy-Item -LiteralPath $tracePath -Destination "$artifactPrefix.trace" -Force -ErrorAction Continue
    }
    Remove-Item Env:\WUFAN_TSF_SMOKE_RESULT -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_PROBE_TRACE -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_BROKER_EXE -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_CHINESE -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_KEY_DELAY -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_BROKER_PID -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_HOST_EXE -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_FAULT_DIR -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_SMOKE_FAULTS -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue
    Pop-Location
}
