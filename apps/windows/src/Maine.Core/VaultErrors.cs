using System.Runtime.CompilerServices;

[assembly: InternalsVisibleTo("Maine.Core.Tests")]

namespace Maine.Core;

public enum VaultErrorKind
{
    Locked,
    InvalidPassword,
    InvalidFile,
    WeakPassword,
}

/// <summary>
/// Vault failures with the same user-facing messages as the macOS app, so
/// both clients explain problems identically. Mirrors
/// <c>PasswordStore.VaultError</c>.
/// </summary>
public sealed class VaultException : Exception
{
    public VaultErrorKind Kind { get; }

    public VaultException(VaultErrorKind kind, string message) : base(message)
    {
        Kind = kind;
    }

    public static VaultException Locked() => new(
        VaultErrorKind.Locked,
        "The password vault is locked. Choose Host → Unlock Password Vault to enter its master password.");

    public static VaultException InvalidPassword() => new(
        VaultErrorKind.InvalidPassword,
        "The master password is incorrect, or the vault has been damaged.");

    public static VaultException MalformedFile(string detail) => new(
        VaultErrorKind.InvalidFile,
        $"This password vault is damaged or uses an unsupported format. It has not been changed. ({detail})");

    public static VaultException InvalidFile() => new(
        VaultErrorKind.InvalidFile,
        "This password vault is damaged or uses an unsupported format. It has not been changed.");

    public static VaultException WeakPassword() => new(
        VaultErrorKind.WeakPassword,
        "Use a master password of at least 12 characters.");

    public static VaultException UnsupportedVersion(int version) => new(
        VaultErrorKind.InvalidFile,
        "This host list was written with a newer version of the Maine " +
        $"configuration format (version {version}). Update Maine to read it.");
}

/// <summary>
/// Byte-shape helpers shared by the vault and the host list. Swift's
/// <c>JSONEncoder</c> writes UUIDs uppercase and <c>Data</c> as standard
/// base64; dates are seconds since 1970 as doubles. The C# client must match
/// exactly, otherwise a vault written on one OS will not open on the other.
/// These shapes are proven by <c>swift-vault-v2.json</c> in the test fixtures,
/// which was produced by CryptoKit and is decrypted by the test suite.
/// </summary>
public static class VaultFormat
{
    /// <summary>Uppercase hyphenated, exactly like Swift's <c>UUID.uuidString</c>.</summary>
    public static string FormatGuid(Guid id) => id.ToString("D").ToUpperInvariant();
}
