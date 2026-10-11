using System.IO;
using System.Security.Cryptography;
using Maine.Core;

namespace Maine.App;

/// <summary>
/// Well-known on-disk locations. Mirrors the macOS layout:
/// the vault lives under roaming AppData (like ~/Library/Application Support),
/// the session helper key under local AppData.
/// </summary>
public static class VaultPaths
{
    public static string VaultFile =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "Maine", "passwords.maine");

    public static string LegacyHostsFile =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData),
            "Maine", "hosts.json");

    public static string KeyDirectory =>
        Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "Maine", "vault-keys");
}

/// <summary>
/// <see cref="IKeyStore"/> backed by a DPAPI-protected file per vault, scoped
/// to the current Windows user. The file is deleted on lock and quit, so the
/// derived key only exists while the app session is unlocked — the same
/// lifetime as the macOS Keychain entry. Only the unlock key lives here; the
/// SSH askpass helper (a separate process) reads it back to serve saved
/// passwords to OpenSSH.
/// </summary>
public sealed class DpapiFileKeyStore : IKeyStore
{
    private readonly string _directory;

    public DpapiFileKeyStore(string? directory = null)
    {
        _directory = directory ?? VaultPaths.KeyDirectory;
    }

    private string PathFor(Guid vaultId) =>
        Path.Combine(_directory, vaultId.ToString("D").ToUpperInvariant() + ".bin");

    public byte[]? GetKey(Guid vaultId)
    {
        string path = PathFor(vaultId);
        byte[] protectedBytes;
        try { protectedBytes = File.ReadAllBytes(path); }
        catch { return null; }
        try
        {
            return ProtectedData.Unprotect(protectedBytes, null, DataProtectionScope.CurrentUser);
        }
        catch
        {
            return null;
        }
    }

    public void SaveKey(Guid vaultId, byte[] key)
    {
        Directory.CreateDirectory(_directory);
        byte[] protectedBytes = ProtectedData.Protect(key, null, DataProtectionScope.CurrentUser);
        string path = PathFor(vaultId);
        string temp = path + ".tmp";
        File.WriteAllBytes(temp, protectedBytes);
        File.Move(temp, path, overwrite: true);
        VaultStore.RestrictToCurrentUser(path);
    }

    public void DeleteKey(Guid vaultId)
    {
        try { File.Delete(PathFor(vaultId)); } catch { }
    }
}
