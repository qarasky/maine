using System.Windows;
using Maine.Core;

namespace Maine.App;

/// <summary>
/// OpenSSH runs the app executable again as its <c>SSH_ASKPASS</c> helper.
/// Only the host ID travels in the environment; the helper decrypts the
/// portable vault. Mirrors the helper-mode entry point in the macOS app.
/// </summary>
public static class AskPassHelper
{
    public static bool IsHelperMode =>
        Environment.GetEnvironmentVariable(SshAskPass.HelperFlagVariable) == "1";

    /// <summary>Runs the helper and returns the process exit code.</summary>
    public static int Run()
    {
        string[] args = Environment.GetCommandLineArgs();
        string prompt = args.Length > 1 ? args[1] : "";
        string? rawId = Environment.GetEnvironmentVariable(SshAskPass.HostIdVariable);
        if (!Guid.TryParse(rawId, out Guid hostId))
            return 1;

        try
        {
            if (SshAskPass.IsPasswordPrompt(prompt))
            {
                var vault = new VaultStore(VaultPaths.VaultFile, new DpapiFileKeyStore(),
                    VaultPaths.LegacyHostsFile);
                string? password = vault.GetPassword(hostId);
                if (password is null)
                    return 1;
                Console.WriteLine(password);
                return 0;
            }
        }
        catch
        {
            return 1;
        }

        // Never reuse a login password for key passphrases, MFA, or host
        // trust. Unknown-host approval remains an explicit user decision.
        bool confirmation = string.Equals(
                Environment.GetEnvironmentVariable("SSH_ASKPASS_PROMPT"), "confirm",
                StringComparison.OrdinalIgnoreCase)
            || prompt.Contains("yes/no", StringComparison.OrdinalIgnoreCase);

        string? answer = null;
        if (confirmation)
        {
            var result = MessageBox.Show(prompt + "\n\nTrust this host?",
                "SSH authentication", MessageBoxButton.YesNo, MessageBoxImage.Warning);
            if (result == MessageBoxResult.Yes)
                answer = "yes";
        }
        else
        {
            var dialog = new PromptDialog("SSH authentication", prompt);
            if (dialog.ShowDialog() == true)
                answer = dialog.Answer;
        }
        if (answer is null)
            return 1;
        Console.WriteLine(answer);
        return 0;
    }
}
