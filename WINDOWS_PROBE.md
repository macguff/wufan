# Windows TSF probe

The current DLL is an **integration probe**. It registers a Simplified Chinese TSF keyboard profile called **Wufan TSF Probe**, implements the COM class factory, TSF activation/key sink, a language-bar mode button, and Chinese/English compartment updates. Text composition and commit are not wired to RuntimeCore/Broker, so ordinary keys still pass through to the host. Do not treat it as a usable IME release.

## Current limitations

- Ordinary keys are passed through: `OnTestKeyDown`/`OnKeyDown` return `BOOL(0)` and no composition or commit is created, so Chinese text input is not implemented.
- The language bar exposes a 中/英 button and a right-click mode menu. Ctrl+Space toggles `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` and `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`. Mode is persisted per host executable under the user's `LOCALAPPDATA`; a background worker performs file writes away from key callbacks. Real synchronization with the Windows indicator, other TSFs, and Word/Chromium/Win32 hosts remains unverified.
- There is no candidate window and no IPC to a broker.

These are covered by gates **G9** (language bar item & menu) and **G10** (mode compartment & per-application memory) in [ADR-006](3.md); see the Slice 1 deliverables in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Build and inspect on Windows x64

Use a Windows 10/11 x64 machine with Visual Studio 2022 C++ build tools, Windows SDK, and the pinned Rust toolchain. Open an elevated PowerShell session as the account that will test the profile, then run from the repository root:

```powershell
.\build.ps1 ci
cargo build --locked -p ime-windows-tsf --release --target x86_64-pc-windows-msvc
$dll = (Resolve-Path .\target\x86_64-pc-windows-msvc\release\ime_windows_tsf.dll).Path
$register = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', "`"$dll`"") -Wait -PassThru
if ($register.ExitCode -ne 0) { throw "registration failed: $($register.ExitCode)" }
```

The profile should appear under the Simplified Chinese input methods after signing out and back in. Select it in a plain text editor and check that Latin keys still pass through. Test activation and deactivation in a disposable Windows account. The DLL must remain at the registered path while the profile is installed.

Remove the probe with:

```powershell
$unregister = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', '/u', "`"$dll`"") -Wait -PassThru
if ($unregister.ExitCode -ne 0) { throw "unregistration failed: $($unregister.ExitCode)" }
```

The Windows CI job compiles the native DLL, runs tests, then performs 100 registration, COM activation, object-release, and unregistration cycles through `scripts/tsf_lifecycle.ps1`. Each cycle checks `DllCanUnloadNow` and the probe's COM/TSF registry keys. The G1 lifecycle gate passed on commit `ba9866e`. A CI green build does not establish Word/Chromium/Win32 host behavior, callback timing, edit-session safety, or packaging.
