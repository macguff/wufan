# This helper never starts a Broker. UAC must use the same Windows account.
[CmdletBinding()]
param([Parameter(Mandatory)][string]$Request)
. (Join-Path $PSScriptRoot 'common.ps1')
$requestPath = Get-SafePath $Request
$job = Read-Json $requestPath
$resultPath = $requestPath + '.result'
$registrationLock = $null
$lockHeld = $false
try {
    if (-not [Environment]::Is64BitProcess -or -not (Test-Administrator)) { throw '64-bit elevated PowerShell is required.' }
    if ((Get-UserSid) -ne $job.ownerSid) { throw 'UAC used another account; cancel and use the installing account. Nothing was registered.' }
    $registrationLock = [Threading.Mutex]::new($false, 'Global\Wufan.TechnicalBeta.Registration')
    try { $lockHeld = $registrationLock.WaitOne(0) }
    catch [Threading.AbandonedMutexException] { $lockHeld = $true }
    if (-not $lockHeld) { throw 'Another registration helper is still running. Wait before recovering.' }
    $newDll = $null
    $oldDll = $null
    foreach ($name in @('newDll','oldDll')) {
        if ($job.$name) {
            $path = Get-SafePath $job.$name
            if ([IO.Path]::GetFileName($path) -ne 'ime_windows_tsf.dll') { throw 'Unexpected DLL name' }
            $null = Assert-Package ([IO.Path]::GetDirectoryName($path))
            Set-Variable -Name $name -Value $path
        }
    }
    $owner = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey($script:BetaOwnerKey)
    $ownerSid = $null
    $ownedDll = $null
    if ($null -ne $owner) {
        try { $ownerSid = $owner.GetValue('OwnerSid'); $ownedDll = $owner.GetValue('DllPath') } finally { $owner.Dispose() }
    }
    $currentDll = Get-ComDll
    $tip = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$script:BetaClsid")
    $tipExists = $null -ne $tip
    if ($tip) { $tip.Dispose() }
    if ($ownerSid -and $ownerSid -ne $job.ownerSid) { throw 'Another Windows account owns the machine TIP.' }
    if (-not $ownerSid -and ($tipExists -or $currentDll)) { throw 'Existing unmanaged TSF registration; unregister the development/other installation first.' }
    if ($ownerSid -and $ownedDll -and $ownedDll -ne $newDll -and $ownedDll -ne $oldDll) { throw 'Machine ownership changed; refusing to overwrite it.' }
    if ($currentDll -and $currentDll -ne $newDll -and $currentDll -ne $oldDll) { throw 'COM registration changed; refusing to overwrite it.' }
    # Run the export inside this helper: terminating the helper also terminates the
    # registration call. A detached regsvr32 child could outlive the transaction lock.
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class WufanBetaRegistrar {
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr LoadLibraryEx(string path, IntPtr file, uint flags);
    [DllImport("kernel32.dll", CharSet=CharSet.Ansi, ExactSpelling=true, SetLastError=true)]
    static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll")] static extern bool FreeLibrary(IntPtr module);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)] delegate int Registration();
    public static int Invoke(string path, bool remove) {
        IntPtr module = LoadLibraryEx(path, IntPtr.Zero, 0x100 | 0x1000);
        if (module == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            IntPtr address = GetProcAddress(module, remove ? "DllUnregisterServer" : "DllRegisterServer");
            if (address == IntPtr.Zero) throw new Win32Exception(Marshal.GetLastWin32Error());
            return ((Registration)Marshal.GetDelegateForFunctionPointer(address, typeof(Registration)))();
        } finally { FreeLibrary(module); }
    }
}
'@
    if ($job.mode -eq 'Register') {
        if (-not $newDll) { throw 'Missing registration DLL' }
        # Own partial registration before executing DllRegisterServer. Recovery can then
        # undo a failed/aborted registration without touching an unrelated installation.
        $key = [Microsoft.Win32.Registry]::LocalMachine.CreateSubKey($script:BetaOwnerKey)
        try { $key.SetValue('OwnerSid', $job.ownerSid); $key.SetValue('DllPath', $newDll); $key.Flush() } finally { $key.Dispose() }
        $hr = [WufanBetaRegistrar]::Invoke($newDll, $false)
        if ($hr -lt 0) { throw ('DllRegisterServer failed: HRESULT 0x{0:X8}' -f $hr) }
        if ((Get-ComDll) -ne $newDll) { throw 'Registered COM path does not match the requested version.' }
    } elseif ($job.mode -eq 'Unregister') {
        if (-not $ownerSid -and ($tipExists -or $currentDll)) { throw 'No owned machine registration to remove.' }
        $dll = if ($newDll) { $newDll } else { $oldDll }
        $hr = [WufanBetaRegistrar]::Invoke($dll, $true)
        # DllUnregisterServer can report a missing subkey after a partial failed install.
        # Success is determined by the postcondition as well as the normal return code.
        $tip = [Microsoft.Win32.Registry]::LocalMachine.OpenSubKey("SOFTWARE\Microsoft\CTF\TIP\$script:BetaClsid")
        $tipExists = $null -ne $tip
        if ($tip) { $tip.Dispose() }
        if ($tipExists -or (Get-ComDll)) { throw ('Unregistration incomplete: HRESULT 0x{0:X8}' -f $hr) }
        [Microsoft.Win32.Registry]::LocalMachine.DeleteSubKeyTree($script:BetaOwnerKey, $false)
    } else { throw 'Unknown registration operation' }
    Write-Json $resultPath @{ ok = $true; message = 'Registration operation completed'; ownerSid = Get-UserSid }
} catch {
    Write-Json $resultPath @{ ok = $false; message = $_.Exception.Message; ownerSid = Get-UserSid }
    exit 1
} finally {
    if ($lockHeld) { $registrationLock.ReleaseMutex() }
    if ($registrationLock) { $registrationLock.Dispose() }
}
