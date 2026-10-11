using System.Text.Json;

namespace Maine.Core;

/// <summary>
/// A portable session snapshot, never a running shell or a command replay.
/// Mirrors <c>SavedSession.swift</c> in the macOS app.
/// </summary>
public sealed class SavedSession : IEquatable<SavedSession>
{
    public Guid Id { get; set; } = Guid.NewGuid();
    public Guid HostId { get; set; }
    public string Name { get; set; } = "";
    public string History { get; set; } = "";
    public DateTimeOffset SavedAt { get; set; } = DateTimeOffset.UtcNow;
    public bool? UsesDefaultName { get; set; }
    public int? Number { get; set; }

    public SavedSession() { }

    public SavedSession(Guid hostId, string name, string history,
        Guid id = default, DateTimeOffset savedAt = default,
        bool? usesDefaultName = null, int? number = null)
    {
        Id = id == default ? Guid.NewGuid() : id;
        HostId = hostId;
        Name = name;
        History = history;
        SavedAt = savedAt == default ? DateTimeOffset.UtcNow : savedAt;
        UsesDefaultName = usesDefaultName;
        Number = number;
    }

    public static SavedSession FromElement(JsonElement el, string path)
    {
        Guid id = ReadGuid(el, "id", path);
        Guid hostId = ReadGuid(el, "hostID", path);
        string name = ReadRequiredString(el, "name", path);
        string history = ReadRequiredString(el, "history", path);
        if (!el.TryGetProperty("savedAt", out var savedAtEl) ||
            savedAtEl.ValueKind != JsonValueKind.Number)
            throw VaultException.MalformedFile($"\"{path}.savedAt\" should hold a number.");
        var savedAt = DateTimeOffset.UnixEpoch.AddSeconds(savedAtEl.GetDouble());

        bool? usesDefaultName = null;
        if (el.TryGetProperty("usesDefaultName", out var udn) && udn.ValueKind == JsonValueKind.True)
            usesDefaultName = true;
        else if (el.TryGetProperty("usesDefaultName", out var udn2) && udn2.ValueKind == JsonValueKind.False)
            usesDefaultName = false;

        int? number = null;
        if (el.TryGetProperty("number", out var num) && num.ValueKind == JsonValueKind.Number &&
            num.TryGetInt32(out int n))
            number = n;

        return new SavedSession(hostId, name, history, id, savedAt, usesDefaultName, number);
    }

    private static Guid ReadGuid(JsonElement el, string key, string path)
    {
        if (!el.TryGetProperty(key, out var prop) || prop.ValueKind != JsonValueKind.String ||
            !Guid.TryParse(prop.GetString(), out var guid))
            throw VaultException.MalformedFile($"\"{path}.{key}\" should hold a UUID string.");
        return guid;
    }

    private static string ReadRequiredString(JsonElement el, string key, string path)
    {
        if (!el.TryGetProperty(key, out var prop) || prop.ValueKind != JsonValueKind.String)
            throw VaultException.MalformedFile($"\"{path}.{key}\" should hold a string.");
        return prop.GetString() ?? "";
    }

    public bool Equals(SavedSession? other) =>
        other is not null && Id == other.Id && HostId == other.HostId &&
        Name == other.Name && History == other.History &&
        SavedAt.ToUnixTimeMilliseconds() == other.SavedAt.ToUnixTimeMilliseconds() &&
        UsesDefaultName == other.UsesDefaultName && Number == other.Number;

    public override bool Equals(object? obj) => Equals(obj as SavedSession);
    public override int GetHashCode() => HashCode.Combine(Id, HostId, Name);
}
