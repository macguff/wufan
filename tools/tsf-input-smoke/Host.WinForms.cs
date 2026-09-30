// WinForms TextBox uses the native Win32 EDIT control, with a different text store.
public sealed class SmokeInput
{
    [DllImport("imm32.dll")]
    static extern IntPtr ImmGetContext(IntPtr window);
    [DllImport("imm32.dll")]
    static extern bool ImmReleaseContext(IntPtr window, IntPtr context);
    [DllImport("imm32.dll", CharSet = CharSet.Unicode)]
    static extern int ImmGetCompositionStringW(IntPtr context, uint index, byte[] buffer, uint length);
    internal readonly System.Windows.Forms.TextBox Control = new System.Windows.Forms.TextBox();
    public string Text { get { return Control.Text; } }
    // Native EDIT exposes committed text through WM_GETTEXT; ongoing preedit
    // is delivered separately by the TSF/IMM bridge as GCS_COMPSTR.
    public string Preedit {
        get {
            IntPtr context = ImmGetContext(Control.Handle);
            if (context == IntPtr.Zero) return "";
            try {
                int length = ImmGetCompositionStringW(context, 8, null, 0);
                if (length <= 0 || length > 65536 || length % 2 != 0) return "";
                var buffer = new byte[length];
                int copied = ImmGetCompositionStringW(context, 8, buffer, (uint)buffer.Length);
                if (copied < 0 || copied > buffer.Length || copied % 2 != 0) return "";
                return System.Text.Encoding.Unicode.GetString(buffer, 0, copied);
            } finally { ImmReleaseContext(Control.Handle, context); }
        }
    }
    public void Clear() { Control.Clear(); }
    public void Focus() { Control.Focus(); }
}

public sealed class SmokeHost : IDisposable, System.Windows.Forms.IMessageFilter
{
    public readonly SmokeInput Input = new SmokeInput();
    public readonly SmokeInput OtherInput = new SmokeInput();
    readonly System.Windows.Forms.Form window = new System.Windows.Forms.Form();
    readonly System.Drawing.Font font = new System.Drawing.Font("Microsoft YaHei", 24);
    public int RepeatedDowns { get; private set; }
    public bool PreFilterMessage(ref System.Windows.Forms.Message message)
    {
        if (message.Msg == 0x100 && message.WParam.ToInt64() == 0x08 &&
            (message.LParam.ToInt64() & (1L << 30)) != 0) RepeatedDowns++;
        return false;
    }
    public string Kind { get { return "WinForms/Win32 EDIT"; } }
    public IntPtr Handle { get { return window.Handle; } }
    public SmokeHost()
    {
        window.Text = "Wufan TSF Win32 EDIT acceptance";
        window.ClientSize = new System.Drawing.Size(560, 180);
        window.StartPosition = System.Windows.Forms.FormStartPosition.CenterScreen;
        Input.Control.SetBounds(16, 16, 528, 60);
        OtherInput.Control.SetBounds(16, 90, 528, 60);
        Input.Control.Font = font;
        OtherInput.Control.Font = font;
        window.Controls.Add(Input.Control);
        window.Controls.Add(OtherInput.Control);
    }
    public void Activate() { window.Activate(); }
    public void Close() { window.Close(); }
    public void Run(Func<Task> scenario)
    {
        window.Shown += async (sender, args) => await scenario();
        System.Windows.Forms.Application.AddMessageFilter(this);
        try { System.Windows.Forms.Application.Run(window); }
        finally { System.Windows.Forms.Application.RemoveMessageFilter(this); }
    }
    public void Dispose() { window.Dispose(); font.Dispose(); }
}
