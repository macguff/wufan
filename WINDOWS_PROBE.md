# Windows TSF probe

The current DLL is an **early integration build**. It registers a Simplified Chinese TSF keyboard profile called **Wufan TSF Probe**, implements the COM class factory, TSF activation/key sink, a language-bar mode button, and Chinese/English compartment updates. Chinese-mode letters are wired to a small local phrase table; inline preedit and commit are implemented through asynchronous TSF edit sessions. The DLL compiles and links, but this host path has not yet been exercised in a real editor. It is not the planned Broker/librime implementation or a release-quality IME.

## Current limitations

- Ordinary keys still use the RuntimeCore synchronous reply adapter. Chinese-mode letters and composition controls use the isolated `ime-pinyin-engine` bootstrap engine: type pinyin, inspect the inline candidate labels, press Space/Enter for the first result or 1–9 for a listed result. The built-in phrase table currently covers common entries such as `nihao` → `你好`, `zhongguo` → `中国`, and `woaini` → `我爱你`; unknown syllables fall back to their pinyin spelling on Space.
- Composition start/update/end and commit are applied by asynchronous `ITfEditSession` requests. The key callback does not wait for those edit sessions. If an edit request cannot be queued, the key is passed through. Real callback timing, edit-session acceptance, and TestKey/Key pairing still need observation in Word, Chromium, and a Win32 host.
- The adapter state is thread-local and the engine/TSF edit path is currently local to the text service process. There is no Broker, IPC, Rime dictionary, candidate window, or RuntimeCore commit lifecycle integration yet.
- Key, focus, and preserved-key callback bodies contain Rust unwinding in unwind-enabled builds. A panic in a key callback returns Pass; focus/preserved-key callbacks return a safe no-op result. The release profile uses `panic = "abort"`, so process recovery there is a fresh activation rather than in-process unwinding.
- The language bar exposes a 中/英 button and a right-click mode menu. Ctrl+Space toggles `GUID_COMPARTMENT_KEYBOARD_OPENCLOSE` and `GUID_COMPARTMENT_KEYBOARD_INPUTMODE_CONVERSION`. Mode is persisted per host executable under the user's `LOCALAPPDATA`; a background worker performs file writes away from key callbacks. Real synchronization with the Windows indicator, other TSFs, and Word/Chromium/Win32 hosts remains unverified.
- There is no candidate window and no IPC to a broker.

These are covered by gates **G9** (language bar item & menu) and **G10** (mode compartment & per-application memory) in [ADR-006](3.md); see the Slice 1 deliverables in [IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md).

## Build and inspect on Windows x64

Use a Windows 10/11 x64 machine with Visual Studio 2022 C++ build tools, Windows SDK, and the pinned Rust toolchain. In PowerShell under the account that will test the profile, run from the repository root:

```powershell
.\build.ps1 ci
cargo build --locked -p ime-windows-tsf --release --target x86_64-pc-windows-msvc
$dll = (Resolve-Path .\target\x86_64-pc-windows-msvc\release\ime_windows_tsf.dll).Path
$register = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', "`"$dll`"") -Wait -PassThru
if ($register.ExitCode -ne 0) { throw "registration failed: $($register.ExitCode)" }
```

For an automated local smoke test, use an elevated Windows PowerShell 5.1 session. The script builds the DLL, registers it temporarily, opens a visible WPF text box, activates the Wufan profile, types `nihao ` through Windows keyboard input, checks that the control contains `你好`, and unregisters in `finally`:

```powershell
powershell.exe -NoProfile -STA -ExecutionPolicy Bypass -File .\scripts\tsf_input_smoke.ps1
```

To compile the smoke host without changing TSF registration, add `-CompileOnly`. On the current development desktop, the smoke host compiles, but `ITfInputProcessorProfiles::Register` returns `E_ACCESSDENIED` even when elevated; registration rolls back and the COM/TSF registry locations are clean. This is an environment limitation, so actual text entry remains unverified here. The script prints the registration trace when that happens.

When registration succeeds, the profile should appear under Simplified Chinese input methods after signing out and back in. Select it in a plain text editor, switch to Chinese mode, type `nihao`, then Space; the expected committed text is `你好`. Type `ni`, then `2`; the expected candidate is `呢`. Check that English mode and ordinary keys still pass through. Test activation and deactivation in a disposable Windows account. The DLL must remain at the registered path while the profile is installed.

Remove the probe with:

```powershell
$unregister = Start-Process -FilePath "$env:WINDIR\System32\regsvr32.exe" -ArgumentList @('/s', '/u', "`"$dll`"") -Wait -PassThru
if ($unregister.ExitCode -ne 0) { throw "unregistration failed: $($unregister.ExitCode)" }
```

The Windows CI job compiles the native DLL, runs tests, then performs 100 registration, COM activation, object-release, and unregistration cycles through `scripts/tsf_lifecycle.ps1`. Each cycle checks `DllCanUnloadNow` and the probe's COM/TSF registry keys. The G1 lifecycle gate passed on commit `ba9866e`. A CI green build does not establish Word/Chromium/Win32 host behavior, callback timing, edit-session safety, or packaging.
