using System;
using System.IO;
using System.Runtime.InteropServices;
using System.Threading;
using System.Threading.Tasks;



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

[ComImport]
[Guid("71C6E74C-0F28-11D8-A82A-00065B84435C")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface ITfInputProcessorProfileMgrSmoke
{
    void ActivateProfile(uint profileType, ushort language, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
    void DeactivateProfile(uint profileType, ushort language, ref Guid clsid, ref Guid profile, IntPtr hkl, uint flags);
    void GetProfile(uint profileType, ushort language, ref Guid clsid, ref Guid profile, IntPtr hkl, IntPtr profileInfo);
}

[ComImport]
[Guid("AA80E801-2021-11D2-93E0-0060B067B86E")]
[InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
interface ITfThreadMgrSmoke
{
    void Activate(out uint clientId);
    void Deactivate();
}

public static class WufanTsfInputSmoke
{
    [DllImport("user32.dll")]
    static extern void keybd_event(byte key, byte scan, uint flags, UIntPtr extraInfo);
    [DllImport("user32.dll")]
    static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")]
    static extern bool SetForegroundWindow(IntPtr window);
    [DllImport("user32.dll")]
    static extern uint GetWindowThreadProcessId(IntPtr window, out uint processId);
    [DllImport("kernel32.dll")]
    static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")]
    static extern bool AttachThreadInput(uint from, uint to, bool attach);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern IntPtr FindWindowEx(IntPtr parent, IntPtr after, string className, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    static extern int GetClassName(IntPtr window, System.Text.StringBuilder name, int capacity);
    [DllImport("user32.dll")]
    static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
    // Acceptance only: suspend the process owned by this harness, never a host.
    [DllImport("ntdll.dll")]
    static extern int NtSuspendProcess(IntPtr process);
    [DllImport("ntdll.dll")]
    static extern int NtResumeProcess(IntPtr process);
    static IntPtr hostWindow;

    const uint KEYEVENTF_KEYUP = 0x0002;
    static int resultCode = 1;
    static string repeatEvidence = "";
    static string faultEvidence = "";
    static int keyDelay = int.Parse(Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_KEY_DELAY") ?? "80");

    static long Probe(uint message)
    {
        IntPtr after = IntPtr.Zero;
        while ((after = FindWindowEx(new IntPtr(-3), after, null, null)) != IntPtr.Zero) {
            uint process;
            if (GetWindowThreadProcessId(after, out process) != GetCurrentThreadId()) continue;
            var name = new System.Text.StringBuilder(256);
            GetClassName(after, name, name.Capacity);
            if (name.ToString().StartsWith("Wufan.BrokerPump.", StringComparison.Ordinal))
                return SendMessage(after, message, IntPtr.Zero, IntPtr.Zero).ToInt64();
        }
        return -1;
    }

    static async Task WaitForIdle(long accepted = -1)
    {
        DateTime deadline = DateTime.UtcNow.AddSeconds(5);
        while (Probe(0x8574) != 3 || (accepted >= 0 && Probe(0x8575) < accepted)) {
            if (DateTime.UtcNow >= deadline) throw new Exception("TSF input session did not become ready and idle");
            await Task.Delay(50);
        }
    }

    static void FocusHost(SmokeHost window)
    {
        uint foregroundProcess;
        uint foregroundThread = GetWindowThreadProcessId(GetForegroundWindow(), out foregroundProcess);
        uint testThread = GetCurrentThreadId();
        bool attached = foregroundThread != 0 && foregroundThread != testThread && AttachThreadInput(testThread, foregroundThread, true);
        try { SetForegroundWindow(hostWindow); window.Activate(); }
        finally { if (attached) AttachThreadInput(testThread, foregroundThread, false); }
    }

    static async Task TapKey(byte key)
    {
        if (GetForegroundWindow() != hostWindow) throw new Exception("Smoke host is not foreground; keyboard injection stopped.");
        keybd_event(key, 0, 0, UIntPtr.Zero);
        keybd_event(key, 0, KEYEVENTF_KEYUP, UIntPtr.Zero);
        // Yield to the host so TSF callbacks and asynchronous sessions can run.
        await Task.Delay(keyDelay);
    }

    static async Task TypeText(string text)
    {
        long accepted = Probe(0x8575);
        if (accepted < 0) throw new Exception("TSF apartment probe window not found");
        foreach (char character in text)
            await TapKey(character == ' ' ? (byte)0x20 : (byte)char.ToUpperInvariant(character));
        await WaitForIdle(accepted + text.Length);
    }

    static async Task Check(SmokeInput input, string expected, string scenario)
    {
        DateTime deadline = DateTime.UtcNow.AddSeconds(5);
        while (input.Text != expected && DateTime.UtcNow < deadline) await Task.Delay(50);
        if (input.Text != expected)
            throw new Exception(scenario + ": expected [" + expected + "], got [" + input.Text + "]");
    }

    static async Task CheckPreedit(SmokeInput input, string scenario)
    {
        string preedit = input.Preedit;
        DateTime deadline = DateTime.UtcNow.AddSeconds(5);
        while ((!preedit.StartsWith("ni", StringComparison.Ordinal) || !preedit.Contains("[1.")) && DateTime.UtcNow < deadline) {
            await Task.Delay(50);
            preedit = input.Preedit;
        }
        if (!preedit.StartsWith("ni", StringComparison.Ordinal) || !preedit.Contains("[1."))
            throw new Exception(scenario + ": expected active Rime ni preedit, got [" + preedit + "] (committed text [" + input.Text + "])");
    }

    static async Task RepeatKey(SmokeHost host, byte key, int count)
    {
        long accepted = Probe(0x8575);
        long repeated = Probe(0x8576);
        int rawRepeated = host.RepeatedDowns;
        try {
            for (int i = 0; i < count; i++) {
                if (GetForegroundWindow() != hostWindow) throw new Exception("Smoke host is not foreground; repeat injection stopped.");
                // Multiple down events without release exercise the OS previous
                // key-state flag. Always release the key, even on interruption.
                keybd_event(key, 0, 0, UIntPtr.Zero);
                await Task.Delay(30);
            }
        } finally { keybd_event(key, 0, KEYEVENTF_KEYUP, UIntPtr.Zero); }
        await WaitForIdle(accepted + count);
        long tsfRepeated = Probe(0x8576) - repeated;
        int visibleRaw = host.RepeatedDowns - rawRepeated;
        // Native EDIT's TSF preprocessing can consume WM_KEYDOWN before the
        // WinForms filter sees it. WPF exposes the raw message but normalizes
        // LPARAM for TSF. Either observer must prove the injected repeat flag.
        if (Math.Max(visibleRaw, tsfRepeated) < count - 1)
            throw new Exception("Repeat injection produced no verified repeat observations");
        repeatEvidence = "repeat observations raw-visible=" + visibleRaw + " TSF=" + tsfRepeated;
        Console.WriteLine(repeatEvidence);
    }

    static async Task WaitForDisconnected(SmokeInput input)
    {
        DateTime deadline = DateTime.UtcNow.AddSeconds(5);
        while ((Probe(0x8574) & 1) != 0 || input.Preedit.Length != 0 || input.Text.Length != 0) {
            if (DateTime.UtcNow >= deadline) throw new Exception("Broker loss did not invalidate session and clear preedit");
            await Task.Delay(50);
        }
        if (Probe(0x8574) < 0) throw new Exception("TSF probe disappeared after Broker loss");
    }

    static System.Diagnostics.Process RestartBroker(string fault = null)
    {
        var start = new System.Diagnostics.ProcessStartInfo {
            FileName = Environment.GetEnvironmentVariable("WUFAN_BROKER_EXE"),
            Arguments = "--dev-client \"" + Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_HOST_EXE") + "\"",
            UseShellExecute = false, CreateNoWindow = true,
            RedirectStandardOutput = true, RedirectStandardError = true
        };
        if (fault != null) {
            string directory = Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_FAULT_DIR");
            File.Delete(Path.Combine(directory, fault + ".entered"));
            start.Arguments += " --dev-fault " + fault + " --dev-fault-dir \"" + directory + "\"";
        }
        var broker = System.Diagnostics.Process.Start(start);
        // Drain both redirected streams asynchronously so neither can block it.
        broker.OutputDataReceived += (sender, args) => { };
        broker.ErrorDataReceived += (sender, args) => { };
        broker.BeginOutputReadLine();
        broker.BeginErrorReadLine();
        return broker;
    }

    static void StopBroker(System.Diagnostics.Process broker)
    {
        if (broker == null) return;
        try {
            if (!broker.HasExited) {
                broker.Kill();
                if (!broker.WaitForExit(2000)) throw new Exception("Owned Broker did not exit");
            }
        } finally { broker.Dispose(); }
    }

    static async Task WaitForMarker(string fault)
    {
        string marker = Path.Combine(Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_FAULT_DIR"), fault + ".entered");
        DateTime deadline = DateTime.UtcNow.AddSeconds(5);
        while (!File.Exists(marker)) {
            if (DateTime.UtcNow >= deadline) throw new Exception("Broker did not reach fault stage " + fault);
            await Task.Delay(25);
        }
    }

    static async Task WaitForEpochChange(long epoch)
    {
        var clock = System.Diagnostics.Stopwatch.StartNew();
        long previous = 0;
        long maximumGap = 0;
        while (Probe(0x8577) == epoch) {
            if (clock.ElapsedMilliseconds > 5000) throw new Exception("Failed request did not invalidate its transport epoch");
            await Task.Delay(50);
            long now = clock.ElapsedMilliseconds;
            maximumGap = Math.Max(maximumGap, now - previous);
            previous = now;
        }
        if (Probe(0x8577) < 0) throw new Exception("Apartment probe lost after fault");
        if (maximumGap > 500) throw new Exception("Host message pump stalled during Broker fault: " + maximumGap + " ms");
        Console.WriteLine("Fault recovery epoch changed; maximum UI sampling gap=" + maximumGap + " ms");
    }

    public static int Run()
    {
        var managerType = Type.GetTypeFromCLSID(new Guid("529A9E6B-6587-4F23-AB9E-9C7D683E3C50"), true);
        var threadManager = (ITfThreadMgrSmoke)Activator.CreateInstance(managerType);
        uint probeClientId;
        threadManager.Activate(out probeClientId);
        var window = new SmokeHost();
        var input = window.Input;
        var otherInput = window.OtherInput;
        window.Run(async () =>
        {
            System.Diagnostics.Process restartedBroker = null;
            string resultPath = Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_RESULT");
            try
            {
                window.Activate();
                hostWindow = window.Handle;
                FocusHost(window);
                input.Focus();
                await Task.Delay(500);

                Guid clsid = new Guid("58F68769-239A-4DCA-854E-57AA887E979B");
                Guid profile = new Guid("9DE41EC9-BCC0-4DD8-90C8-975F03C3EF73");
                var service = Activator.CreateInstance(Type.GetTypeFromCLSID(clsid, true));
                Marshal.ReleaseComObject(service); // Prove the registered COM DLL can load.
                var profileType = Type.GetTypeFromCLSID(new Guid("33C53A50-F456-4884-B049-85FD643ECFED"), true);
                var profiles = (ITfInputProcessorProfileMgrSmoke)Activator.CreateInstance(profileType);
                // Activate only this process and allow a different current input language.
                try {
                    IntPtr profileInfo = Marshal.AllocHGlobal(256);
                    try {
                        profiles.GetProfile(1, 0x0804, ref clsid, ref profile, IntPtr.Zero, profileInfo);
                        Console.WriteLine("Profile flags=" + Marshal.ReadInt32(profileInfo, 80));
                    } finally { Marshal.FreeHGlobal(profileInfo); }
                    profiles.ActivateProfile(1, 0x0804, ref clsid, ref profile, IntPtr.Zero, 0x10000005);
                }
                finally { Marshal.ReleaseComObject(profiles); }
                await Task.Delay(500); // Let the asynchronous Broker session become ready.
                // Profile activation can change the foreground window. Restore
                // this host once before the scenario; TapKey still stops on any
                // later foreground change instead of sending keys elsewhere.
                FocusHost(window);
                input.Focus();
                await Task.Delay(100);
                await WaitForIdle();

                await TypeText("nihao woaini ");
                await Check(input, "\u4f60\u597d\u6211\u7231\u4f60", "consecutive commits and caret");

                input.Clear();
                await Task.Delay(300); // Settle programmatic text store changes.
                await WaitForIdle();
                await TypeText("ni2");
                await Check(input, "\u62df", "librime numeric candidate selection");

                input.Clear();
                await Task.Delay(300); // Settle programmatic text store changes.
                await WaitForIdle();
                await TypeText("ni");
                await CheckPreedit(input, "preedit before Escape");
                await TapKey(0x1B);
                await TypeText("hao ");
                await Check(input, "\u597d", "Escape and fresh composition");

                input.Clear();
                await Task.Delay(300); // Settle programmatic text store changes.
                await WaitForIdle();
                await TypeText("ni");
                await CheckPreedit(input, "preedit before focus loss");
                otherInput.Focus();
                await Task.Delay(300);
                await WaitForIdle();
                await TypeText("hao ");
                await Check(otherInput, "\u597d", "new context starts fresh");
                await Check(input, "", "focus loss clears old preedit");
                if (input.Preedit.Length != 0) throw new Exception("focus loss leaves old preedit: [" + input.Preedit + "]");

                otherInput.Clear();
                await Task.Delay(300);
                await WaitForIdle();
                await TypeText("nihao");
                await CheckPreedit(otherInput, "preedit before repeated Backspace");
                await RepeatKey(window, 0x08, 3);
                await CheckPreedit(otherInput, "preedit after repeated Backspace");
                await TypeText(" ");
                await Check(otherInput, "\u4f60", "repeat edits nihao to ni and commits once");

                otherInput.Clear();
                await Task.Delay(300);
                await WaitForIdle();
                await TypeText("ni");
                await CheckPreedit(otherInput, "preedit before Broker loss");
                // Only terminate the exact Broker created by this smoke script.
                using (var ownedBroker = System.Diagnostics.Process.GetProcessById(
                    int.Parse(Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_BROKER_PID")))) {
                    if (!string.Equals(ownedBroker.MainModule.FileName,
                        Environment.GetEnvironmentVariable("WUFAN_BROKER_EXE"), StringComparison.OrdinalIgnoreCase))
                        throw new Exception("Smoke Broker process identity changed");
                    ownedBroker.Kill();
                    if (!ownedBroker.WaitForExit(2000)) throw new Exception("Smoke Broker did not exit");
                }
                await WaitForDisconnected(otherInput);
                long beforeFallback = Probe(0x8575);
                foreach (char letter in "abc") await TapKey((byte)char.ToUpperInvariant(letter));
                await Check(otherInput, "abc", "Broker unavailable passes through Latin input");
                if (Probe(0x8575) != beforeFallback) throw new Exception("Unavailable Broker accepted fallback keys");
                restartedBroker = RestartBroker();
                await WaitForIdle();
                await TypeText("hao ");
                await Check(otherInput, "abc\u597d", "Broker restart begins a fresh composition without old ni replay");

                if (Environment.GetEnvironmentVariable("WUFAN_TSF_SMOKE_FAULTS") == "1") {
                    otherInput.Clear();
                    await Task.Delay(300);
                    await WaitForIdle();
                    long suspendedEpoch = Probe(0x8577);
                    if (NtSuspendProcess(restartedBroker.Handle) != 0) throw new Exception("Could not suspend owned Broker");
                    try {
                        await TapKey(0x4E);
                        await WaitForEpochChange(suspendedEpoch);
                        await WaitForDisconnected(otherInput);
                        long accepted = Probe(0x8575);
                        foreach (char letter in "abc") await TapKey((byte)char.ToUpperInvariant(letter));
                        await Check(otherInput, "abc", "suspended Broker passes through Latin input after deadline");
                        if (Probe(0x8575) != accepted) throw new Exception("Suspended Broker accepted fallback input");
                    } finally {
                        if (NtResumeProcess(restartedBroker.Handle) != 0) throw new Exception("Could not resume owned Broker");
                    }
                    await WaitForIdle();
                    await TypeText("hao ");
                    await Check(otherInput, "abc\u597d", "resumed Broker starts fresh without replaying stalled key");

                    // Stall a real engine reply until the client's 2 s deadline.
                    // Then repeat the stage and kill while that request is in flight.
                    foreach (bool killInFlight in new bool[] { false, true }) {
                        otherInput.Clear();
                        await Task.Delay(300);
                        await WaitForIdle();
                        StopBroker(restartedBroker);
                        restartedBroker = null;
                        await WaitForDisconnected(otherInput);
                        restartedBroker = RestartBroker("key-reply-stall");
                        await WaitForIdle();
                        long epoch = Probe(0x8577);
                        await TapKey(0x4E);
                        await WaitForMarker("key-reply-stall");
                        if (killInFlight) {
                            StopBroker(restartedBroker);
                            restartedBroker = null;
                        }
                        await WaitForEpochChange(epoch);
                        if (killInFlight) {
                            await WaitForDisconnected(otherInput);
                            restartedBroker = RestartBroker();
                        }
                        await WaitForIdle();
                        await Check(otherInput, "", "failed in-flight key creates no host text");
                        await TypeText("hao ");
                        await Check(otherInput, "\u597d", "fresh request after fault does not replay old n");
                    }

                    // Host has already committed when the Broker withholds the
                    // wire Acknowledged reply. Never retry that host mutation.
                    otherInput.Clear();
                    await Task.Delay(300);
                    await WaitForIdle();
                    StopBroker(restartedBroker);
                    restartedBroker = null;
                    await WaitForDisconnected(otherInput);
                    restartedBroker = RestartBroker("commit-ack-stall");
                    await WaitForIdle();
                    long ackEpoch = Probe(0x8577);
                    foreach (char letter in "nihao ")
                        await TapKey(letter == ' ' ? (byte)0x20 : (byte)char.ToUpperInvariant(letter));
                    await WaitForMarker("commit-ack-stall");
                    await Check(otherInput, "\u4f60\u597d", "host commit exists before wire ACK failure");
                    await WaitForEpochChange(ackEpoch);
                    await WaitForIdle();
                    await Check(otherInput, "\u4f60\u597d", "ACK failure preserves the already applied commit exactly once");
                    await TypeText("hao ");
                    await Check(otherInput, "\u4f60\u597d\u597d", "fresh input after ACK failure does not replay the commit");
                    faultEvidence = "; process suspend/resume deadline, key-reply deadline, in-flight Broker kill, host-applied ACK deadline";
                }

                File.WriteAllText(resultPath, "PASS (" + window.Kind + "): consecutive commits, candidate selection, Escape, context switch, repeated Backspace, Broker loss/fallback/restart; " + repeatEvidence + faultEvidence);
                resultCode = 0;
            }
            catch (Exception error)
            {
                File.WriteAllText(resultPath, "FAIL: " + error.ToString());
                resultCode = 1;
            }
            finally {
                try { StopBroker(restartedBroker); }
                finally { window.Close(); }
            }
        });
        window.Dispose();
        threadManager.Deactivate();
        Marshal.ReleaseComObject(threadManager);
        return resultCode;
    }
}
