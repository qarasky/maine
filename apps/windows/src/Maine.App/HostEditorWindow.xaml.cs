using System.Windows;
using Maine.Core;

namespace Maine.App;

public partial class HostEditorWindow : Window
{
    public Host Host { get; private set; }
    public string Password { get; private set; }

    public HostEditorWindow(Host host, string password, string configPath)
    {
        InitializeComponent();
        Host = host;
        Password = password;
        NameBox.Text = host.Name;
        HostnameBox.Text = host.Hostname;
        UsernameBox.Text = host.Username;
        PortBox.Text = host.Port.ToString();
        PasswordBox.Password = password;

        // Offer the user's existing ssh config entries, like the macOS importer.
        var entry = SshConfigFile.Load(configPath).EntryFor(host.Hostname);
        if (entry?.Hostname is not null)
            KnownHosts.Text = $"ssh config: {entry.Pattern} → {entry.Hostname}" +
                (entry.User is not null ? $" as {entry.User}" : "") +
                (entry.Port is int port ? $" :{port}" : "");
        else
            KnownHosts.Visibility = Visibility.Collapsed;

        Loaded += (_, _) => (string.IsNullOrEmpty(host.Name) ? NameBox : HostnameBox).Focus();
    }

    private void OnSave(object sender, RoutedEventArgs e)
    {
        if (!int.TryParse(PortBox.Text.Trim(), out int port))
            port = 0; // Surfaces as a validation issue below.
        var candidate = new Host(Host.Id, NameBox.Text, HostnameBox.Text.Trim(),
            port, UsernameBox.Text.Trim());
        var issues = candidate.Issues;
        string password = PasswordBox.Password;
        if (issues.Count > 0)
        {
            ErrorText.Text = string.Join(" ", issues);
            ErrorText.Visibility = Visibility.Visible;
            return;
        }
        if (password.Length == 0)
        {
            ErrorText.Text = "A password is required for adding or saving a host. " +
                "Maine keeps it in the encrypted vault and supplies it to OpenSSH.";
            ErrorText.Visibility = Visibility.Visible;
            return;
        }
        Host = candidate;
        Password = password;
        DialogResult = true;
    }
}
