using System.Text.Json;

namespace Maine.Core;

/// <summary>
/// A single saved SSH host. Maps 1:1 onto the <c>host</c> object defined by
/// <c>packages/schema/hosts.schema.json</c>, the portable format shared by
/// every Maine client. Mirrors <c>Host.swift</c> in the macOS app.
/// </summary>
public sealed class Host : IEquatable<Host>
{
    /// <summary>
    /// The port OpenSSH uses unless told otherwise. Storing 22 means
    /// "default": Maine then leaves the <c>-p</c> flag off so a matching
    /// <c>~/.ssh/config</c> stanza can pick the real port.
    /// </summary>
    public const int DefaultPort = 22;

    /// <summary>A host with empty fields, the starting point for the "New host" editor.</summary>
    public static Host Blank() => new(Guid.NewGuid(), "", "");

    public Guid Id { get; set; } = Guid.NewGuid();
    public string Name { get; set; } = "";
    /// <summary>An IP address, a DNS name, or a <c>Host</c> alias from the user's ssh config.</summary>
    public string Hostname { get; set; } = "";
    public int Port { get; set; } = DefaultPort;
    /// <summary>Empty means "not specified"; OpenSSH then takes the user from
    /// ssh config, the key that authenticates, or the local user name.</summary>
    public string Username { get; set; } = "";

    public Host() { }

    public Host(Guid id, string name, string hostname, int port = DefaultPort, string username = "")
    {
        Id = id;
        Name = name;
        Hostname = hostname;
        Port = port;
        Username = username;
    }

    /// <summary>Lenient reading: an absent or null field falls back to its
    /// default so one odd host cannot stop the whole list from loading. A
    /// field with the wrong JSON type still fails.</summary>
    public static Host FromElement(JsonElement el, string path)
    {
        if (el.ValueKind != JsonValueKind.Object)
            throw VaultException.MalformedFile($"\"{path}\" should hold an object.");

        Guid id = Guid.NewGuid();
        if (el.TryGetProperty("id", out var idEl) && idEl.ValueKind != JsonValueKind.Null)
        {
            if (idEl.ValueKind != JsonValueKind.String || !Guid.TryParse(idEl.GetString(), out id))
                throw VaultException.MalformedFile($"\"{path}.id\" should hold a UUID string.");
        }

        string name = ReadString(el, "name", path) ?? "";
        string hostname = ReadString(el, "hostname", path) ?? "";
        string username = ReadString(el, "username", path) ?? "";

        int port = DefaultPort;
        if (el.TryGetProperty("port", out var portEl) && portEl.ValueKind != JsonValueKind.Null)
        {
            if (portEl.ValueKind != JsonValueKind.Number || !portEl.TryGetInt32(out port))
                throw VaultException.MalformedFile($"\"{path}.port\" should hold an integer.");
        }

        return new Host(id, name, hostname, port, username);
    }

    private static string? ReadString(JsonElement el, string key, string path)
    {
        if (!el.TryGetProperty(key, out var prop) || prop.ValueKind == JsonValueKind.Null)
            return null;
        if (prop.ValueKind != JsonValueKind.String)
            throw VaultException.MalformedFile($"\"{path}.{key}\" should hold a string.");
        return prop.GetString() ?? "";
    }

    /// <summary>The trimmed name used for display.</summary>
    public string DisplayName =>
        string.IsNullOrWhiteSpace(Name) ? Hostname : Name.Trim();

    /// <summary><c>user@hostname</c>, or just <c>hostname</c> when no user is set.</summary>
    public string Target => string.IsNullOrEmpty(Username) ? Hostname : $"{Username}@{Hostname}";

    /// <summary>The port, only shown when it is not the default.</summary>
    public string? PortDescription => Port == DefaultPort ? null : $":{Port}";

    /// <summary>A one line summary used in the sidebar.</summary>
    public string Subtitle => Target + (PortDescription ?? "");

    /// <summary>Problems that stop this host from being used. Mirrors the
    /// constraints in the shared schema. Messages match macOS exactly.</summary>
    public List<string> Issues
    {
        get
        {
            var issues = new List<string>();

            if (string.IsNullOrWhiteSpace(Name))
                issues.Add("A name is required.");

            var address = Hostname.Trim();
            if (address.Length == 0)
                issues.Add("A hostname or IP address is required.");
            else if (address.Any(char.IsWhiteSpace))
                issues.Add("The hostname cannot contain spaces.");
            else if (address.StartsWith("-"))
                issues.Add("The hostname cannot start with \"-\", it would be read as an option by ssh.");
            else if (address.Length > 253)
                issues.Add("The hostname cannot be longer than 253 characters.");

            if (Port is < 1 or > 65535)
                issues.Add("The port must be between 1 and 65535.");

            if (Username.Any(char.IsWhiteSpace))
                issues.Add("The username cannot contain spaces.");
            else if (Username.Length > 128)
                issues.Add("The username cannot be longer than 128 characters.");

            return issues;
        }
    }

    public bool IsValid => Issues.Count == 0;

    /// <summary>Builds a host from an ssh config entry, filling in what the
    /// entry leaves out. Null when the pattern is not a single literal host name.</summary>
    public static Host? FromSshConfigEntry(SshConfigEntry entry)
    {
        if (!SshConfigEntry.IsConnectable(entry.Pattern))
            return null;
        return new Host(Guid.NewGuid(), entry.Pattern, entry.Pattern,
            entry.Port ?? DefaultPort, entry.User ?? "");
    }

    public bool Equals(Host? other) =>
        other is not null && Id == other.Id && Name == other.Name &&
        Hostname == other.Hostname && Port == other.Port && Username == other.Username;

    public override bool Equals(object? obj) => Equals(obj as Host);
    public override int GetHashCode() => HashCode.Combine(Id, Name, Hostname, Port, Username);
}
