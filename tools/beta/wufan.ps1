[CmdletBinding()]
param(
    [ValidateSet('Install','Start','Stop','Status','Configure','Rollback','Uninstall','Recover')]
    [string]$Action = 'Status',
    [string]$PackagePath = (Split-Path $PSScriptRoot -Parent),
    [string]$InstallRoot = (Join-Path $env:LOCALAPPDATA 'Wufan\beta'),
    [string[]]$ClientExe,
    [switch]$EnableAutostart,
    [switch]$DisableAutostart
)
. (Join-Path $PSScriptRoot 'common.ps1')
if (-not [Environment]::Is64BitProcess) { throw 'Use 64-bit Windows PowerShell.' }
if (Test-Administrator) { throw 'Run this controller without elevation. Only the registration helper requests UAC.' }
if ($EnableAutostart -and $DisableAutostart) { throw 'Choose one startup option.' }
$root = Get-SafePath $InstallRoot
if ($root -eq [IO.Path]::GetPathRoot($root).TrimEnd('\')) { throw 'A drive root cannot be an install directory.' }
$statePath = Get-ChildPath $root 'state.json'
$configPath = Get-ChildPath $root 'config.json'
$journalPath = Get-ChildPath $root 'transaction.json'
$processPath = Get-ChildPath $root 'broker-process.json'
$launchPath = Get-ChildPath $root 'broker-launch.json'
$sid = Get-UserSid

function Get-VersionRoot([string]$Version) {
    if ($Version -notmatch '^[0-9A-Za-z][0-9A-Za-z._-]{0,95}$') { throw 'Invalid installed version ID.' }
    Get-ChildPath $root "versions\$Version"
}
function Get-State {
    $value = Read-Json $statePath
    if ($value -and ($value.ownerSid -ne $sid -or $value.format -ne 1)) { throw 'Installation belongs to another user or uses an unsupported format.' }
    $value
}
function Get-Config {
    $value = Read-Json $configPath
    if ($null -eq $value) { $value = [pscustomobject]@{ format = 1; clients = @(); autostart = $false } }
    if ($value.format -ne 1 -or $value.autostart -isnot [bool]) { throw 'Unsupported startup configuration.' }
    $clients = @()
    foreach ($client in $value.clients) {
        $path = Get-SafePath ([string]$client)
        if ([IO.Path]::GetExtension($path) -ne '.exe') { throw "Invalid approved host path: $path" }
        $clients += $path
    }
    [pscustomobject]@{ format = 1; clients = @($clients | Select-Object -Unique); autostart = $value.autostart }
}
function Get-OwnedProcess {
    $record = Read-Json $processPath
    $process = if ($record) { Get-Process -Id $record.pid -ErrorAction SilentlyContinue } else { $null }
    if ($process -and $record.session -ne [Diagnostics.Process]::GetCurrentProcess().SessionId) {
        throw 'A recorded process is active in another Windows session. This beta supports one login session per user.'
    }
    if ($process -and ($process.Path -ne $record.exe -or
        $process.StartTime.ToUniversalTime().Ticks.ToString() -ne $record.startedUtcTicks)) {
        Write-Warning 'Ignoring a stale Broker PID record. The process now using that PID will not be controlled.'
        $process = $null
    }
    if (-not $process) {
        # Recover the narrow interruption window between CreateProcess and PID
        # persistence using a prewritten unique --ready-file launch token.
        $intent = Read-Json $launchPath
        if (-not $intent -or $intent.ownerSid -ne $sid) { return $null }
        $matching = @(Get-CimInstance Win32_Process -Filter "Name = 'ime-broker.exe'" | Where-Object {
            $_.ExecutablePath -eq $intent.exe -and $_.SessionId -eq [Diagnostics.Process]::GetCurrentProcess().SessionId -and
            $_.CommandLine -and $_.CommandLine.Contains((Quote-Argument $intent.ready))
        })
        if ($matching.Count -gt 1) { throw 'More than one process matches the launch token.' }
        if ($matching.Count -eq 0) { return $null }
        $process = Get-Process -Id $matching[0].ProcessId -ErrorAction Stop
        if ($process.StartTime.ToUniversalTime().Ticks -lt [long]$intent.startedUtcTicks) { throw 'Launch token process predates the intent.' }
        $record = [pscustomobject]@{ ownerSid = $sid; exe = $intent.exe; session = $intent.session;
            startedUtcTicks = $process.StartTime.ToUniversalTime().Ticks.ToString() }
    }
    if ($record.ownerSid -ne $sid -or $record.session -ne [Diagnostics.Process]::GetCurrentProcess().SessionId -or
        $process.Path -ne $record.exe -or $process.StartTime.ToUniversalTime().Ticks.ToString() -ne $record.startedUtcTicks) {
        throw 'Broker PID was reused or identity changed. Refusing to control that process.'
    }
    $native = Get-CimInstance Win32_Process -Filter "ProcessId = $($process.Id)"
    $owner = Invoke-CimMethod -InputObject $native -MethodName GetOwnerSid
    if ($owner.ReturnValue -ne 0 -or $owner.Sid -ne $sid) { throw 'Broker process owner differs from the installing user.' }
    $process
}
function Stop-Broker {
    $process = Get-OwnedProcess
    if ($process) {
        # Disconnecting the pipe invalidates outstanding requests; host-applied commits
        # are never replayed. No process-name-wide termination is permitted here.
        Stop-Process -Id $process.Id -ErrorAction Stop
        if (-not $process.WaitForExit(5000)) { throw 'Broker did not exit; installation remains recoverable.' }
    }
    if (Test-Path -LiteralPath $processPath) { Remove-Item -LiteralPath $processPath -Force }
    if (Test-Path -LiteralPath $launchPath) { Remove-Item -LiteralPath $launchPath -Force }
}
function Start-Broker {
    if (Get-OwnedProcess) { Write-Host 'Broker is already running.'; return }
    $state = Get-State
    if (-not $state -or -not $state.active) { throw 'No active installation. Run Install first.' }
    $versionRoot = Get-VersionRoot $state.active
    $null = Assert-Package $versionRoot
    if ((Get-ComDll) -ne (Join-Path $versionRoot 'ime_windows_tsf.dll')) { throw 'TSF registration differs from active state. Run Recover or reinstall.' }
    $config = Get-Config
    foreach ($client in $config.clients) {
        if (-not (Test-Path -LiteralPath $client -PathType Leaf)) { throw "Approved host is missing: $client. Use Configure to update the list." }
    }
    $exe = Join-Path $versionRoot 'ime-broker.exe'
    $dataRoot = Get-ChildPath $root "data\rime\$($state.active)"
    $runRoot = Get-ChildPath $root 'run'
    [IO.Directory]::CreateDirectory($runRoot) | Out-Null
    $ready = Get-ChildPath $runRoot ([Guid]::NewGuid().ToString('N') + '.ready')
    $arguments = @('--runtime', (Quote-Argument (Join-Path $versionRoot 'runtime')),
        '--user-data', (Quote-Argument $dataRoot), '--deploy', '--ready-file', (Quote-Argument $ready))
    foreach ($client in $config.clients) { $arguments += @('--dev-client', (Quote-Argument $client)) }
    # Remove acceptance overrides inherited from a development shell. No global environment is changed.
    $saved = @{}
    foreach ($item in Get-ChildItem Env: | Where-Object { $_.Name -like 'WUFAN_*' }) {
        $saved[$item.Name] = $item.Value
        [Environment]::SetEnvironmentVariable($item.Name, $null, 'Process')
    }
    $process = $null
    try {
        if (Test-Path -LiteralPath $processPath) { Remove-Item -LiteralPath $processPath -Force }
        Write-Json $launchPath @{ exe = $exe; ready = $ready; ownerSid = $sid;
            session = [Diagnostics.Process]::GetCurrentProcess().SessionId; startedUtcTicks = [DateTime]::UtcNow.Ticks.ToString() }
        $process = Start-Process -FilePath $exe -ArgumentList $arguments -WindowStyle Hidden -PassThru `
            -RedirectStandardOutput (Join-Path $runRoot 'broker.out.log') -RedirectStandardError (Join-Path $runRoot 'broker.err.log')
        Write-Json $processPath @{ pid = $process.Id; exe = $exe; ownerSid = $sid;
            session = $process.SessionId; startedUtcTicks = $process.StartTime.ToUniversalTime().Ticks.ToString() }
        $deadline = [DateTime]::UtcNow.AddSeconds(90)
        while (-not (Test-Path -LiteralPath $ready)) {
            $process.Refresh()
            if ($process.HasExited) { throw "Broker exited with $($process.ExitCode). See run\broker.err.log." }
            if ([DateTime]::UtcNow -ge $deadline) { throw 'Broker deployment/startup timed out (90 seconds).' }
            Start-Sleep -Milliseconds 100
        }
        if ((Get-Content -LiteralPath $ready -Raw) -ne [string]$process.Id) { throw 'Invalid Broker readiness signal.' }
        $process.Refresh()
        if ($process.HasExited) { throw 'Broker exited immediately after readiness.' }
        Write-Host "Broker ready, PID $($process.Id), approved hosts: $(@($config.clients).Count)."
    } catch {
        if ($process) {
            if (Test-Path -LiteralPath $processPath) { Stop-Broker }
            elseif (-not $process.HasExited) { $process.Kill(); $null = $process.WaitForExit(5000) }
        }
        throw
    } finally {
        foreach ($name in $saved.Keys) { [Environment]::SetEnvironmentVariable($name, $saved[$name], 'Process') }
        if (Test-Path -LiteralPath $ready) { Remove-Item -LiteralPath $ready -Force }
    }
}
function Invoke-Registration([string]$Mode, $NewDll, $OldDll) {
    $requestPath = Get-ChildPath $root ('run\registration-' + [Guid]::NewGuid().ToString('N') + '.json')
    Write-Json $requestPath @{ mode = $Mode; newDll = $NewDll; oldDll = $OldDll; ownerSid = $sid }
    # Invoke the packaged helper from a fully verified payload.
    $helperRoot = if ($NewDll) { Split-Path $NewDll -Parent } else { Split-Path $OldDll -Parent }
    $null = Assert-Package $helperRoot
    $helper = Join-Path $helperRoot 'tools\register.ps1'
    Write-Host 'Windows UAC will register the TIP. Allow elevation using this same Windows account.'
    $process = Start-Process -FilePath (Get-PowerShell) -Verb RunAs -WindowStyle Hidden -PassThru -Wait `
        -ArgumentList @('-NoProfile','-ExecutionPolicy','Bypass','-File', (Quote-Argument $helper), '-Request', (Quote-Argument $requestPath))
    $result = Read-Json ($requestPath + '.result')
    if (-not $result -or -not $result.ok -or $result.ownerSid -ne $sid -or $process.ExitCode -ne 0) {
        $message = if ($result) { $result.message } else { 'No helper result; UAC cancelled or helper interrupted.' }
        throw "Registration failed: $message. The transaction is retained for Recover."
    }
}
function Set-ConfiguredStartup($State, $Config) {
    if ($Config.autostart -and $State.active) {
        $launcher = Join-Path (Get-VersionRoot $State.active) 'tools\wufan.ps1'
        $command = (Quote-Argument (Get-PowerShell)) + ' -NoProfile -W Hidden -EP Bypass -File ' +
            (Quote-Argument $launcher) + ' -Action Start'
        if ($root -ne (Get-SafePath (Join-Path $env:LOCALAPPDATA 'Wufan\beta'))) {
            $command += ' -InstallRoot ' + (Quote-Argument $root)
        }
        if ($command.Length -gt 260) { throw 'Startup command exceeds the Windows Run limit. Use a shorter installation path or disable autostart.' }
        Set-StartupValue $command
    } else { Set-StartupValue $null }
}
function Restore-File([string]$Path, $Value) {
    if ($null -ne $Value) { Write-Json $Path $Value }
    elseif (Test-Path -LiteralPath $Path) { Remove-Item -LiteralPath $Path -Force }
}
function Restore-Transaction($Journal) {
    Stop-Broker
    $oldDll = if ($Journal.beforeState -and $Journal.beforeState.active) {
        Join-Path (Get-VersionRoot $Journal.beforeState.active) 'ime_windows_tsf.dll'
    } else { $null }
    $newDll = if ($Journal.targetVersion) { Join-Path (Get-VersionRoot $Journal.targetVersion) 'ime_windows_tsf.dll' } else { $oldDll }
    $owner = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($script:BetaOwnerKey)
    $owned = $null -ne $owner
    $ownedPath = $null
    $ownedSid = $null
    if ($owner) {
        try { $ownedPath = $owner.GetValue('DllPath'); $ownedSid = $owner.GetValue('OwnerSid') } finally { $owner.Dispose() }
    }
    $tip = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$script:BetaClsid")
    $tipExists = $null -ne $tip
    if ($tip) { $tip.Dispose() }
    if ($oldDll) {
        if ((Get-ComDll) -ne $oldDll -or $ownedPath -ne $oldDll -or $ownedSid -ne $sid -or -not $tipExists) {
            Invoke-Registration 'Register' $oldDll $newDll
        }
    }
    elseif ($owned) { Invoke-Registration 'Unregister' $newDll $oldDll }
    elseif (Get-ComDll) { throw 'Unexpected unmanaged COM registration during recovery.' }
    Restore-File $statePath $Journal.beforeState
    Restore-File $configPath $Journal.beforeConfig
    Set-StartupValue $Journal.beforeStartup
    $Journal.phase = 'Recovered'
    Write-Json $journalPath $Journal
    Write-Host 'Previous registration, configuration and startup state restored. User data and version files were retained.'
    if ($Journal.wasRunning -and $Journal.beforeState -and $Journal.beforeState.active) {
        try { Start-Broker }
        catch { Write-Warning "Previous version restored but Broker restart failed: $($_.Exception.Message). Use Configure or Start after resolving the startup cause." }
    }
}
function Begin-Transaction([string]$Kind, $TargetVersion) {
    $state = Get-State
    $startup = Get-StartupValue
    if (-not $state -and $startup) { throw 'An unmanaged Wufan startup value already exists.' }
    if ($state -and $startup -and -not ([string]$startup).Contains((Get-VersionRoot $state.active))) {
        throw 'Startup value changed outside this installation. Refusing to overwrite it.'
    }
    $expected = if ($state -and $state.active) { Join-Path (Get-VersionRoot $state.active) 'ime_windows_tsf.dll' } else { $null }
    $owner = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($script:BetaOwnerKey)
    try {
        if ($expected -and -not $owner) { throw 'Machine ownership record is missing. Refusing to change the existing registration.' }
        if ($owner -and ($owner.GetValue('OwnerSid') -ne $sid -or $owner.GetValue('DllPath') -ne $expected)) {
            throw 'Another installation owns the machine TIP.'
        }
        if ((Get-ComDll) -ne $expected) { throw 'COM path differs from this installation. Remove the development/other registration first.' }
        if (-not $expected) {
            $tip = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$script:BetaClsid")
            if ($tip) { $tip.Dispose(); throw 'An unmanaged machine TIP already exists.' }
        }
    } finally { if ($owner) { $owner.Dispose() } }
    $journal = [pscustomobject]@{ format = 1; ownerSid = $sid; kind = $Kind; phase = 'Pending';
        targetVersion = $TargetVersion; beforeState = $state; beforeConfig = Read-Json $configPath;
        beforeStartup = $startup; wasRunning = [bool](Get-OwnedProcess) }
    Write-Json $journalPath $journal
    $journal
}

[IO.Directory]::CreateDirectory($root) | Out-Null
$lockPath = Get-ChildPath $root 'controller.lock'
$lock = $null
try {
    try { $lock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'ReadWrite', 'None') }
    catch { throw 'Another Wufan controller is running. Close it before continuing.' }
    $pending = Read-Json $journalPath
    if ($pending -and ($pending.format -ne 1 -or $pending.ownerSid -ne $sid)) { throw 'Invalid transaction journal owner/format.' }
    if ($pending -and $pending.phase -eq 'Pending') {
        if ($Action -eq 'Recover') { Restore-Transaction $pending; return }
        if ($Action -notin @('Status','Stop')) { throw 'An interrupted transaction is pending. Run -Action Recover before making changes.' }
    }
    switch ($Action) {
        'Start' { Start-Broker }
        'Stop' { Stop-Broker; Write-Host 'Broker stopped. Installed TIP and user data retained.' }
        'Status' {
            $state = Get-State
            $process = Get-OwnedProcess
            [pscustomobject]@{ root = $root; active = if ($state) { $state.active } else { $null };
                previous = if ($state) { $state.previous } else { $null }; brokerPid = if ($process) { $process.Id } else { $null };
                registeredDll = Get-ComDll; autostart = [bool](Get-StartupValue);
                transaction = if ($pending) { $pending.phase } else { 'None' } } | Format-List
        }
        'Recover' { Write-Host 'No pending transaction.' }
        { $_ -in @('Install','Rollback','Uninstall','Configure') } {
            $state = Get-State
            $targetVersion = if ($state) { $state.active } else { $null }
            $config = Get-Config
            if ($PSBoundParameters.ContainsKey('ClientExe')) {
                $config.clients = @($ClientExe | ForEach-Object {
                    $path = Get-SafePath $_
                    if (-not (Test-Path -LiteralPath $path -PathType Leaf) -or [IO.Path]::GetExtension($path) -ne '.exe') { throw "Invalid host executable: $path" }
                    $path
                } | Select-Object -Unique)
            }
            if ($EnableAutostart) { $config.autostart = $true }
            if ($DisableAutostart) { $config.autostart = $false }
            if ($Action -eq 'Install') {
                $source = Get-SafePath $PackagePath
                $manifest = Assert-Package $source
                $digest = (Get-FileHash -LiteralPath (Join-Path $source 'manifest.json') -Algorithm SHA256).Hash.ToLowerInvariant()
                $targetVersion = "$($manifest.version)-$($digest.Substring(0,12))"
                $destination = Get-VersionRoot $targetVersion
                if (-not (Test-Path -LiteralPath $destination)) {
                    $staging = Get-ChildPath $root ('versions\.incoming-' + [Guid]::NewGuid().ToString('N'))
                    [IO.Directory]::CreateDirectory($staging) | Out-Null
                    foreach ($item in Get-ChildItem -LiteralPath $source -Force) {
                        Copy-Item -LiteralPath $item.FullName -Destination $staging -Recurse
                    }
                    $null = Assert-Package $staging
                    [IO.Directory]::Move($staging, $destination)
                }
                $null = Assert-Package $destination
            } elseif (-not $state -or -not $state.active) { throw 'No active installation.' }
            if ($Action -eq 'Rollback') {
                if (-not $state.previous) { throw 'No previous version available.' }
                $targetVersion = $state.previous
                $null = Assert-Package (Get-VersionRoot $targetVersion)
                $config = $state.previousConfig
            }
            $journal = Begin-Transaction $Action $targetVersion
            try {
                Stop-Broker
                $oldDll = if ($state -and $state.active) { Join-Path (Get-VersionRoot $state.active) 'ime_windows_tsf.dll' } else { $null }
                $newDll = Join-Path (Get-VersionRoot $targetVersion) 'ime_windows_tsf.dll'
                if ($Action -eq 'Uninstall') {
                    Invoke-Registration 'Unregister' $oldDll $null
                    Set-StartupValue $null
                    # Keep packages because a host may still have a pinned DLL mapped.
                    # A later reinstall can reuse them; there is no unsafe recursive deletion.
                    $newState = [pscustomobject]@{ format = 1; ownerSid = $sid; active = $null; previous = $state.active; previousConfig = $config }
                    Write-Json $statePath $newState
                } else {
                    if ($Action -ne 'Configure') { Invoke-Registration 'Register' $newDll $oldDll }
                    $previous = if ($state) { $state.previous } else { $null }
                    $previousConfig = if ($state) { $state.previousConfig } else { $null }
                    if ($state -and $state.active -and $state.active -ne $targetVersion) {
                        $previous = $state.active
                        $previousConfig = $journal.beforeConfig
                    }
                    $newState = [pscustomobject]@{ format = 1; ownerSid = $sid; active = $targetVersion;
                        previous = $previous; previousConfig = $previousConfig }
                    Write-Json $configPath $config
                    Write-Json $statePath $newState
                    Set-ConfiguredStartup $newState $config
                    if ($Action -ne 'Configure' -or $journal.wasRunning) { Start-Broker }
                }
                $journal.phase = 'Completed'
                Write-Json $journalPath $journal
                Write-Host "$Action completed. Restart applications that loaded the previous TSF DLL before trying Chinese input."
                if ($Action -eq 'Uninstall') { Write-Host 'TIP and startup entry removed. Version files, configuration, Rime data and mode memory retained.' }
            } catch {
                $failure = $_.Exception.Message
                Write-Warning "$Action failed: $failure. Restoring the recorded previous state."
                try { Restore-Transaction $journal }
                catch { Write-Warning "Recovery incomplete: $($_.Exception.Message). Run -Action Recover after resolving the cause." }
                throw $failure
            }
        }
    }
} finally { if ($lock) { $lock.Dispose() } }
