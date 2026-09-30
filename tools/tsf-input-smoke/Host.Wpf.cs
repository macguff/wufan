// WPF text store adapter; Scenarios.cs owns the shared acceptance contract.
public sealed class SmokeInput
{
    internal readonly System.Windows.Controls.TextBox Control =
        new System.Windows.Controls.TextBox { FontSize = 24, MinHeight = 60 };
    public string Text { get { return Control.Text; } }
    public string Preedit { get { return Control.Text; } }
    public void Clear() { Control.Clear(); }
    public void Focus() { Control.Focus(); }
}

public sealed class SmokeHost : IDisposable
{
    public readonly SmokeInput Input = new SmokeInput();
    public readonly SmokeInput OtherInput = new SmokeInput();
    readonly System.Windows.Application app = new System.Windows.Application();
    readonly System.Windows.Window window;
    public int RepeatedDowns { get; private set; }
    void ObserveKey(ref System.Windows.Interop.MSG message, ref bool handled)
    {
        if (message.message == 0x100 && message.wParam.ToInt64() == 0x08 &&
            (message.lParam.ToInt64() & (1L << 30)) != 0) RepeatedDowns++;
    }
    public string Kind { get { return "WPF"; } }
    public IntPtr Handle { get { return new System.Windows.Interop.WindowInteropHelper(window).Handle; } }
    public SmokeHost()
    {
        var panel = new System.Windows.Controls.StackPanel();
        panel.Children.Add(Input.Control);
        panel.Children.Add(OtherInput.Control);
        window = new System.Windows.Window {
            Title = "Wufan TSF WPF acceptance", Width = 560, Height = 220,
            Content = panel, WindowStartupLocation = System.Windows.WindowStartupLocation.CenterScreen
        };
    }
    public void Activate() { window.Activate(); }
    public void Close() { window.Close(); }
    public void Run(Func<Task> scenario)
    {
        window.Loaded += async (sender, args) => await scenario();
        System.Windows.Interop.ComponentDispatcher.ThreadFilterMessage += ObserveKey;
        try { app.Run(window); }
        finally { System.Windows.Interop.ComponentDispatcher.ThreadFilterMessage -= ObserveKey; }
    }
    public void Dispose() { }
}
