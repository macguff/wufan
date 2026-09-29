param([switch]$CompileOnly)

$ErrorActionPreference = 'Stop'

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$target = 'x86_64-pc-windows-msvc'
$dll = Join-Path $repoRoot "target\$target\release\ime_windows_tsf.dll"
$resultPath = Join-Path $env:TEMP 'wufan-tsf-input-smoke.txt'
$tracePath = Join-Path $env:TEMP 'wufan-tsf-registration.trace'
$registered = $false

Push-Location $repoRoot
try {
    cargo build --locked -p ime-windows-tsf --release --target $target
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed: $LASTEXITCODE" }

    $source = @'
using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;
using System.Windows;
using System.Windows.Controls;

[ComImport]
[Guid("1F02B6C5-7842-4EE6-8A0B-9A24183A95CA")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface ITfInputProcessorProfilesSmoke
{
    void Register(ref Guid clsid);
    void Unregister(ref Guid clsid);
    void AddLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile, string description, uint descriptionLength, string iconFile, uint iconLength, uint iconIndex);
    void RemoveLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile);
    void EnumInputProcessorInfo(out IntPtr enumerator);
    void GetDefaultLanguageProfile(ushort langid, ref Guid category, out Guid clsid, out Guid profile);
    void SetDefaultLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile);
    void ActivateLanguageProfile(ref Guid clsid, ushort langid, ref Guid profile);
}

public static class WufanTsfInputSmoke
{
    [DllImport("user32.dll")]
    static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);

    const uint KEYEVENTF_KEYUP = 0x0002;
    static int resultCode = 1;

    static void Tap(char character)
    {
        ushort key = character == ' ' ? (ushort)0x20 : (ushort)char.ToUpperInvariant(character);
        keybd_event((byte)key, 0, 0, UIntPtr.Zero);
        keybd_event((byte)key, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        Thread.Sleep(80);
    }

    public static int Run()
    {
        var app = new Application();
        var input = new TextBox { FontSize = 24, MinHeight = 60, AcceptsReturn = false };
        var window = new Window
        {
            Title = "Wufan TSF input smoke test",
            Width = 560,
            Height = 150,
            Content = input,
            WindowStartupLocation = WindowStartupLocation.CenterScreen
        };

        window.Loaded += async (sender, args) =>
        {
            window.Activate();
            input.Focus();
            await Task.Delay(500);

            Guid clsid = new Guid("58F68769-239A-4DCA-854E-57AA887E979B");
            Guid profile = new Guid("9DE41EC9-BCC0-4DD8-90C8-975F03C3EF73");
            var profileType = Type.GetTypeFromCLSID(new Guid("33C53A50-F456-4884-B049-85FD643ECFED"), true);
            var profiles = (ITfInputProcessorProfilesSmoke)Activator.CreateInstance(profileType);
            profiles.ActivateLanguageProfile(ref clsid, 0x0804, ref profile);

            foreach (char character in "nihao ") Tap(character);
            await Task.Delay(1200);

            string actual = input.Text;
            string resultPath = Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_RESULT");
            File.WriteAllText(resultPath, actual);
            resultCode = actual == "你好" ? 0 : 1;
            window.Close();
        };

        app.Run(window);
        return resultCode;
    }
}
'@

    Add-Type -TypeDefinition $source -Language CSharp -ReferencedAssemblies @(
        'PresentationFramework', 'PresentationCore', 'WindowsBase', 'System.Xaml'
    )
    if ($CompileOnly) {
        Write-Host 'TSF WPF smoke host compiled successfully.'
        return
    }

    $env:WUFAN_TSF_PROBE_TRACE = $tracePath
    Remove-Item -LiteralPath $tracePath -Force -ErrorAction SilentlyContinue
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

    $env:WUFAN_TSF_SMOKE_RESULT = $resultPath
    Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue

    $smokeExitCode = [WufanTsfInputSmoke]::Run()
    $actualText = if (Test-Path -LiteralPath $resultPath) {
        Get-Content -LiteralPath $resultPath -Raw
    } else {
        '<no result returned>'
    }
    if ($smokeExitCode -ne 0) {
        throw "TSF input smoke failed. TextBox contained: [$actualText]"
    }
    Write-Host "TSF input smoke passed: [$actualText]"
}
finally {
    if ($registered) {
        $unregister = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" `
            -ArgumentList @('/s', '/u', "`"$dll`"") -WindowStyle Hidden -Wait -PassThru
        if ($unregister.ExitCode -ne 0) {
            Write-Warning "DLL unregistration failed: $($unregister.ExitCode)"
        }
    }
    Remove-Item Env:\WUFAN_TSF_SMOKE_RESULT -ErrorAction SilentlyContinue
    Remove-Item Env:\WUFAN_TSF_PROBE_TRACE -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath $resultPath -Force -ErrorAction SilentlyContinue
    Pop-Location
}
