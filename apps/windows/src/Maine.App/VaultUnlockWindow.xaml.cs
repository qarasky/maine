using System.Windows;

namespace Maine.App;

public partial class VaultUnlockWindow : Window
{
    public string MasterPassword { get; private set; } = "";

    public VaultUnlockWindow(bool firstRun)
    {
        InitializeComponent();
        PromptText.Text = firstRun
            ? "Create the password vault with a master password of at least 12 characters. " +
              "Maine asks for it on every launch; a forgotten master password cannot be recovered."
            : "Enter the master password to unlock hosts, saved passwords and sessions.";
        OkButton.Content = firstRun ? "Create" : "Unlock";
        Loaded += (_, _) => PasswordBox.Focus();
    }

    private void OnOk(object sender, RoutedEventArgs e)
    {
        MasterPassword = PasswordBox.Password;
        DialogResult = true;
    }
}
