using System.Text.Json;

namespace Maine.Core;

/// <summary>
/// The on-disk host list: the exact JSON Maine writes for the legacy
/// plaintext file and for the hosts inside the encrypted vault.
/// The format is defined by <c>packages/schema/hosts.schema.json</c>.
/// Mirrors <c>HostConfiguration.swift</c> in the macOS app.
/// </summary>
public sealed class HostConfiguration : IEquatable<HostConfiguration>
{
    /// <summary>The format version this build of Maine writes and understands.</summary>
    public const int CurrentSchemaVersion = 1;

    public int SchemaVersion { get; set; } = CurrentSchemaVersion;
    public List<Host> Hosts { get; set; } = new();

    public HostConfiguration() { }

    public HostConfiguration(List<Host> hosts, int schemaVersion = CurrentSchemaVersion)
    {
        Hosts = hosts;
        SchemaVersion = schemaVersion;
    }

    /// <summary>The version this build understands, used to detect files
    /// written by a newer client.</summary>
    public bool IsSupportedByThisVersion => SchemaVersion <= CurrentSchemaVersion;

    public static HostConfiguration Decode(string json)
    {
        JsonDocument doc;
        try
        {
            doc = JsonDocument.Parse(json);
        }
        catch (JsonException ex)
        {
            throw VaultException.MalformedFile($"the file is not valid JSON ({ex.Message}).");
        }
        using (doc)
        {
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
                throw VaultException.MalformedFile("the host list should hold an object.");

            if (!root.TryGetProperty("schemaVersion", out var versionEl) ||
                versionEl.ValueKind != JsonValueKind.Number ||
                !versionEl.TryGetInt32(out int version))
                throw VaultException.MalformedFile("\"schemaVersion\" is missing.");

            var config = new HostConfiguration { SchemaVersion = version };
            if (!config.IsSupportedByThisVersion)
                throw VaultException.UnsupportedVersion(version);

            if (!root.TryGetProperty("hosts", out var hostsEl) ||
                hostsEl.ValueKind != JsonValueKind.Array)
                throw VaultException.MalformedFile("\"hosts\" is missing.");

            int index = 0;
            foreach (var hostEl in hostsEl.EnumerateArray())
            {
                config.Hosts.Add(Host.FromElement(hostEl, $"hosts.{index}"));
                index++;
            }
            return config;
        }
    }

    /// <summary>The JSON to write to disk: every required field is always
    /// emitted, even defaults (<c>"port": 22</c>, <c>"username": ""</c>).</summary>
    public string Encode()
    {
        using var stream = new System.IO.MemoryStream();
        using (var writer = new Utf8JsonWriter(stream, new JsonWriterOptions { Indented = true }))
        {
            writer.WriteStartObject();
            writer.WriteNumber("schemaVersion", SchemaVersion);
            writer.WriteStartArray("hosts");
            foreach (var host in Hosts)
            {
                writer.WriteStartObject();
                writer.WriteString("id", VaultFormat.FormatGuid(host.Id));
                writer.WriteString("name", host.Name);
                writer.WriteString("hostname", host.Hostname);
                writer.WriteNumber("port", host.Port);
                writer.WriteString("username", host.Username);
                writer.WriteEndObject();
            }
            writer.WriteEndArray();
            writer.WriteEndObject();
        }
        return System.Text.Encoding.UTF8.GetString(stream.ToArray());
    }

    public bool Equals(HostConfiguration? other) =>
        other is not null && SchemaVersion == other.SchemaVersion &&
        Hosts.SequenceEqual(other.Hosts);

    public override bool Equals(object? obj) => Equals(obj as HostConfiguration);
    public override int GetHashCode() => HashCode.Combine(SchemaVersion, Hosts.Count);
}
