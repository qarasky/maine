namespace Maine.Core;

/// <summary>
/// Session-scoped storage for the derived vault key. Only the unlock key
/// lives here; passwords live in the vault file. The Windows app backs this
/// with a DPAPI-protected file that is deleted on lock and quit, mirroring
/// the macOS Keychain entry. Tests use <see cref="InMemoryKeyStore"/>.
/// </summary>
public interface IKeyStore
{
    byte[]? GetKey(Guid vaultId);
    void SaveKey(Guid vaultId, byte[] key);
    void DeleteKey(Guid vaultId);
}

/// <summary>Non-persisted key store for tests.</summary>
public sealed class InMemoryKeyStore : IKeyStore
{
    private readonly Dictionary<Guid, byte[]> _keys = new();

    public byte[]? GetKey(Guid vaultId) =>
        _keys.TryGetValue(vaultId, out var key) ? (byte[])key.Clone() : null;

    public void SaveKey(Guid vaultId, byte[] key) => _keys[vaultId] = (byte[])key.Clone();

    public void DeleteKey(Guid vaultId) => _keys.Remove(vaultId);
}
