using System.Windows;

namespace Maine.App;

public partial class App : Application
{
    protected override void OnStartup(StartupEventArgs e)
    {
        // The OpenSSH askpass helper runs before any UI starts: it prints the
        // saved password (or shows a prompt dialog) and exits.
        if (AskPassHelper.IsHelperMode)
        {
            Shutdown(AskPassHelper.Run());
            return;
        }
        base.OnStartup(e);
    }
}
