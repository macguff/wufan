# Windows TSF probe

The current DLL is an **integration probe**. It registers a Simplified Chinese TSF keyboard profile called **Wufan TSF Probe**, implements the COM class factory and TSF activation/key sink, and passes every key to the host. It does not yet produce Chinese text or connect to RuntimeCore/Broker. Do not treat it as a usable IME release.

## Build and inspect on Windows x64

Use a Windows 10/11 x64 machine with Visual Studio 2022 C++ build tools, Windows SDK, and the pinned Rust toolchain. Open an elevated PowerShell session as the account that will test the profile, then run from the repository root:

```powershell
.\build.ps1 ci
cargo build --locked -p ime-windows-tsf --release --target x86_64-pc-windows-msvc
$dll = (Resolve-Path .\target\x86_64-pc-windows-msvc\release\ime_windows_tsf.dll).Path
& "$env:WINDIR\System32\regsvr32.exe" /s $dll
if ($LASTEXITCODE -ne 0) { throw "registration failed: $LASTEXITCODE" }
```

The profile should appear under the Simplified Chinese input methods after signing out and back in. Select it in a plain text editor and check that Latin keys still pass through. Test activation and deactivation in a disposable Windows account. The DLL must remain at the registered path while the profile is installed.

Remove the probe with:

```powershell
& "$env:WINDIR\System32\regsvr32.exe" /s /u $dll
if ($LASTEXITCODE -ne 0) { throw "unregistration failed: $LASTEXITCODE" }
```

The Windows CI job compiles the native DLL, runs tests, then checks COM registration and removal. A CI green build does not establish Word/Chromium/Win32 host behavior, callback timing, edit-session safety, or packaging.
