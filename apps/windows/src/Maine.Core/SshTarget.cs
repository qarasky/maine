namespace Maine.Core;

/// <summary>
/// The exact <c>ssh</c> command Maine runs for a host. Maine adds as little as
/// possible to the invocation so that everything the user already set up keeps
/// working: keys, <c>ssh-agent</c>, <c>ProxyJump</c>, <c>Match</c> and every
/// other directive are applied by OpenSSH itself. Maine never passes a
/// password, and never reads a private key. Mirrors <c>SSHTarget.swift</c>.
/// </summary>
public sealed class SshTarget : IEquatable<SshTarget>
{
    /// <summary>Well-known install location of the system OpenSSH client on Windows 10+.</summary>
    public const string WindowsSshPath = @"C:\Windows\System32\OpenSSH\ssh.exe";

    public string Executable { get; set; } = "ssh.exe";
    public List<string> Arguments { get; set; } = new();

    public SshTarget() { }

    public SshTarget(string executable, List<string> arguments)
    {
        Executable = executable;
        Arguments = arguments;
    }

    /// <summary>Locates the system OpenSSH client: first on <c>PATH</c>, then
    /// at the well-known install location, otherwise the bare name so the
    /// child process search can resolve it.</summary>
    public static string ResolveExecutable()
    {
        var path = Environment.GetEnvironmentVariable("PATH") ?? "";
        foreach (var dir in path.Split(Path.PathSeparator))
        {
            if (string.IsNullOrWhiteSpace(dir))
                continue;
            var candidate = Path.Combine(dir.Trim().Trim('"'), "ssh.exe");
            if (File.Exists(candidate))
                return candidate;
        }
        if (File.Exists(WindowsSshPath))
            return WindowsSshPath;
        return "ssh.exe";
    }

    /// <summary>The command line Maine runs, for display and for copying to a terminal.</summary>
    public string CommandLine
    {
        get
        {
            var parts = new List<string> { QuoteIfNeeded(Executable) };
            parts.AddRange(Arguments);
            return string.Join(" ", parts);
        }
    }

    private static string QuoteIfNeeded(string value) =>
        value.Contains(' ') || value.Contains('\t') ? $"\"{value}\"" : value;

    public static SshTarget Make(Host host, string? executable = null)
    {
        var arguments = new List<string>();
        if (host.Port != Host.DefaultPort)
        {
            arguments.Add("-p");
            arguments.Add(host.Port.ToString());
        }
        if (!string.IsNullOrEmpty(host.Username))
        {
            arguments.Add("-l");
            arguments.Add(host.Username);
        }
        arguments.Add(host.Hostname);
        return new SshTarget(executable ?? ResolveExecutable(), arguments);
    }

    /// <summary>The connection as OpenSSH resolves it, after the ssh config is
    /// applied. Best effort, for display only.</summary>
    public sealed class Resolution : IEquatable<Resolution>
    {
        public string Hostname { get; set; } = "";
        public string Username { get; set; } = "";
        public int Port { get; set; } = Host.DefaultPort;
        /// <summary>The <c>Host</c> stanza that contributed these values, when there is one.</summary>
        public string? ConfigPattern { get; set; }

        public string Target => $"{Username}@{Hostname}:{Port}";

        public bool Equals(Resolution? other) =>
            other is not null && Hostname == other.Hostname && Username == other.Username &&
            Port == other.Port && ConfigPattern == other.ConfigPattern;

        public override bool Equals(object? obj) => Equals(obj as Resolution);
        public override int GetHashCode() => HashCode.Combine(Hostname, Username, Port);
    }

    public Resolution Resolved(SshConfigFile config, string? localUsername = null)
    {
        localUsername ??= Environment.UserName;
        if (Arguments.Count == 0)
            return new Resolution { Hostname = "", Username = localUsername, Port = Host.DefaultPort };

        string address = Arguments[^1];
        var entry = config.EntryFor(address);

        string username = localUsername;
        string? configuredUser = FirstValueOf("-l") ?? entry?.User;
        if (configuredUser is not null)
            username = configuredUser;

        int port = Host.DefaultPort;
        if (FirstValueOf("-p") is string explicitPort)
            port = int.TryParse(explicitPort, out int p) ? p : Host.DefaultPort;
        else if (entry?.Port is int configured)
            port = configured;

        return new Resolution
        {
            Hostname = entry?.Hostname ?? address,
            Username = username,
            Port = port,
            ConfigPattern = entry?.Pattern,
        };
    }

    /// <summary>The value that follows <c>flag</c>, or null when the flag is absent or truncated.</summary>
    private string? FirstValueOf(string flag)
    {
        int index = Arguments.IndexOf(flag);
        if (index < 0 || index + 1 >= Arguments.Count)
            return null;
        return Arguments[index + 1];
    }

    public bool Equals(SshTarget? other) =>
        other is not null && Executable == other.Executable &&
        Arguments.SequenceEqual(other.Arguments);

    public override bool Equals(object? obj) => Equals(obj as SshTarget);
    public override int GetHashCode() => HashCode.Combine(Executable, Arguments.Count);
}

/// <summary>
/// The environment the child <c>ssh</c> process runs with. The parent
/// environment is inherited — not replaced — so <c>SSH_AUTH_SOCK</c> (agent),
/// <c>PATH</c>, proxies and locale keep working — and only the terminal
/// related variables are set.
/// </summary>
public static class TerminalEnvironment
{
    public const string Term = "xterm-256color";

    public static Dictionary<string, string> Variables()
    {
        var environment = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (System.Collections.DictionaryEntry kv in Environment.GetEnvironmentVariables())
        {
            if (kv.Key is string k && kv.Value is string v)
                environment[k] = v;
        }
        environment["TERM"] = Term;
        environment["COLORTERM"] = "truecolor";
        if (!environment.TryGetValue("LANG", out var lang) || string.IsNullOrEmpty(lang))
            environment["LANG"] = "en_US.UTF-8";
        return environment;
    }
}

/// <summary>
/// OpenSSH invokes the app executable again to request authentication. Only
/// the host ID travels in the environment; the helper decrypts the portable
/// vault. Mirrors <c>SSHAskPass.swift</c> in the macOS app.
/// </summary>
public static class SshAskPass
{
    public const string HostIdVariable = "MAINE_SSH_HOST_ID";
    public const string HelperFlagVariable = "MAINE_SSH_ASKPASS";

    public static List<string> EnvironmentFor(Guid hostId, IEnumerable<string> baseEnv, string executable)
    {
        var overrides = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["SSH_ASKPASS"] = executable,
            ["SSH_ASKPASS_REQUIRE"] = "force",
            [HelperFlagVariable] = "1",
            [HostIdVariable] = VaultFormat.FormatGuid(hostId),
        };
        var result = baseEnv
            .Where(entry => !overrides.ContainsKey(entry.Split('=', 2)[0]))
            .ToList();
        result.AddRange(overrides.Select(kv => $"{kv.Key}={kv.Value}"));
        return result;
    }

    public static bool IsPasswordPrompt(string prompt)
    {
        var lower = prompt.ToLowerInvariant();
        return lower.Contains("password:") && !lower.Contains("passphrase")
            && !lower.Contains("one-time") && !lower.Contains("verification");
    }
}
