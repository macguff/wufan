param(
    [Parameter(Mandatory = $true)]
    [string] $DllPath,
    [ValidateRange(1, 1000)]
    [int] $Cycles = 100
)

$ErrorActionPreference = 'Stop'
$dll = (Resolve-Path -LiteralPath $DllPath).Path
$clsid = '{58F68769-239A-4DCA-854E-57AA887E979B}'
$comKey = "Registry::HKEY_CURRENT_USER\Software\Classes\CLSID\$clsid"
$tipKeys = @(
    "Registry::HKEY_CURRENT_USER\Software\Microsoft\CTF\TIP\$clsid",
    "Registry::HKEY_LOCAL_MACHINE\Software\Microsoft\CTF\TIP\$clsid"
)

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class WufanProbeNative {
    [DllImport("kernel32.dll", CharSet = CharSet.Unicode, SetLastError = true)]
    public static extern IntPtr LoadLibrary(string path);
    [DllImport("kernel32.dll", CharSet = CharSet.Ansi, SetLastError = true)]
    public static extern IntPtr GetProcAddress(IntPtr module, string name);
    [DllImport("kernel32.dll", SetLastError = true)]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool FreeLibrary(IntPtr module);
    [UnmanagedFunctionPointer(CallingConvention.StdCall)]
    public delegate int CanUnloadNow();
}
'@
$module = [WufanProbeNative]::LoadLibrary($dll)
if ($module -eq [IntPtr]::Zero) { throw "cannot load probe DLL: $dll" }
$entry = [WufanProbeNative]::GetProcAddress($module, 'DllCanUnloadNow')
if ($entry -eq [IntPtr]::Zero) { throw 'probe DLL has no DllCanUnloadNow export' }
$canUnload = [Runtime.InteropServices.Marshal]::GetDelegateForFunctionPointer(
    $entry, [type][WufanProbeNative+CanUnloadNow]
)

function Invoke-Registration([bool] $Remove) {
    $arguments = @('/s')
    if ($Remove) { $arguments += '/u' }
    $arguments += "`"$dll`""
    $process = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList $arguments -Wait -PassThru
    if ($process.ExitCode -ne 0) {
        if ($env:WUFAN_TSF_PROBE_TRACE -and (Test-Path -LiteralPath $env:WUFAN_TSF_PROBE_TRACE)) {
            Get-Content -LiteralPath $env:WUFAN_TSF_PROBE_TRACE
        }
        $action = if ($Remove) { 'unregistration' } else { 'registration' }
        throw "TSF $action failed with exit code $($process.ExitCode)"
    }
}

try {
    for ($cycle = 1; $cycle -le $Cycles; $cycle++) {
        Invoke-Registration $false
        try {
            $inprocKey = Join-Path $comKey 'InprocServer32'
            $registered = (Get-Item -LiteralPath $inprocKey).GetValue('')
            if ($registered -ne $dll) {
                throw "cycle $cycle registered an unexpected COM path: $registered"
            }
            $comType = [type]::GetTypeFromCLSID([guid]$clsid)
            $instance = [Activator]::CreateInstance($comType)
            if ($null -eq $instance) { throw "cycle $cycle COM activation returned null" }
            [void][Runtime.InteropServices.Marshal]::ReleaseComObject($instance)
            $instance = $null
            [GC]::Collect()
            [GC]::WaitForPendingFinalizers()
            if ($canUnload.Invoke() -ne 0) {
                throw "cycle $cycle left a live COM object or server lock"
            }
        } finally {
            Invoke-Registration $true
        }

        foreach ($key in @($comKey) + $tipKeys) {
            if (Test-Path -LiteralPath $key) {
                throw "cycle $cycle left a registration key: $key"
            }
        }
        if ($cycle % 10 -eq 0 -or $cycle -eq $Cycles) {
            Write-Host "TSF registration lifecycle: $cycle/$Cycles clean cycles"
        }
    }
} finally {
    [void][WufanProbeNative]::FreeLibrary($module)
}
