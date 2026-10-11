using System.Security.Cryptography;
using System.Text;
using System.Text.Json;
using Maine.Core;
using Xunit;

namespace Maine.Core.Tests;

public sealed class VaultStoreTests : IDisposable
{
    private readonly string _directory =
        Path.Combine(Path.GetTempPath(), $"maine-vault-tests-{Guid.NewGuid():D}");
    private readonly List<(VaultStore Store, string Dir)> _cleanup = new();

    public VaultStoreTests() => Directory.CreateDirectory(_directory);

    public void Dispose()
    {
        foreach (var (store, _) in _cleanup)
            try { store.Lock(); } catch { }
        try { Directory.Delete(_directory, recursive: true); } catch { }
    }

    private VaultStore NewStore(string name = "passwords.maine")
    {
        var store = new VaultStore(
            Path.Combine(_directory, Guid.NewGuid().ToString("D"), name),
            new InMemoryKeyStore());
        _cleanup.Add((store, _directory));
        return store;
    }

    [Fact]
    public void PasswordsAreStoredUpdatedAndRemovedPerHost()
    {
        var store = NewStore();
        var first = Guid.NewGuid();
        var second = Guid.NewGuid();

        Assert.Null(store.GetPassword(first));
        store.Unlock("a long test master password");
        store.SavePassword("test secret with spaces 🔐", first);
        store.SavePassword("another test secret", second);
        Assert.Equal("test secret with spaces 🔐", store.GetPassword(first));

        string text = File.ReadAllText(store.FilePath);
        Assert.DoesNotContain("test secret", text);
        Assert.DoesNotContain("a long test master password", text);

        store.SavePassword("changed secret", first);
        Assert.Equal("changed secret", store.GetPassword(first));
        Assert.Equal("another test secret", store.GetPassword(second));
        store.SavePassword("", first);
        Assert.Null(store.GetPassword(first));
        store.DeletePassword(first);
    }

    [Fact]
    public void CopiedVaultUnlocksWithMasterPasswordOnAnotherInstallation()
    {
        var original = NewStore("original.maine");
        var copyPath = Path.Combine(_directory, Guid.NewGuid().ToString("D"), "copy.maine");
        var copy = new VaultStore(copyPath, new InMemoryKeyStore());
        _cleanup.Add((copy, _directory));

        var hostId = Guid.NewGuid();
        var host = new Host(hostId, "Private server", "private.example", 22, "private-user");
        original.Unlock("portable master password");
        original.SaveHost(host, "shared test password");
        var saved = new SavedSession(hostId, "Private session", "Sensitive terminal output");
        original.SaveSession(saved);

        string text = File.ReadAllText(original.FilePath);
        foreach (var value in new[] { host.Name, host.Hostname, host.Username,
                     VaultFormat.FormatGuid(hostId), "shared test password", saved.Name, saved.History })
            Assert.DoesNotContain(value, text);

        Directory.CreateDirectory(Path.GetDirectoryName(copyPath)!);
        File.Copy(original.FilePath, copyPath);
        Assert.Throws<VaultException>(() => copy.GetPassword(hostId));
        byte[] before = File.ReadAllBytes(copyPath);
        Assert.Throws<VaultException>(() => copy.Unlock("incorrect master password"));
        Assert.Equal(before, File.ReadAllBytes(copyPath));

        copy.Unlock("portable master password");
        Assert.Equal(new List<Host> { host }, copy.GetHosts());
        Assert.Equal(new List<SavedSession> { saved }, copy.GetSavedSessions());
        Assert.Equal("shared test password", copy.GetPassword(hostId));

        copy.Lock();
        Assert.Throws<VaultException>(() => copy.GetHosts());
        Assert.Throws<VaultException>(() => copy.GetSavedSessions());
        Assert.Throws<VaultException>(() => copy.GetPassword(hostId));
        Assert.Throws<VaultException>(() => copy.SavePassword("replacement", hostId));
        Assert.Equal(before, File.ReadAllBytes(copyPath));
    }

    [Fact]
    public void WeakMasterPasswordIsRefusedForNewVaults() =>
        Assert.Throws<VaultException>(() => NewStore().Unlock("too short"));

    [Fact]
    public void TamperedPayloadIsRejected()
    {
        var store = NewStore();
        var first = Guid.NewGuid();
        var second = Guid.NewGuid();
        store.Unlock("test integrity master password");
        store.SavePassword("first secret", first);
        store.SavePassword("second secret", second);

        using var doc = JsonDocument.Parse(File.ReadAllText(store.FilePath));
        var root = doc.RootElement;
        byte[] ciphertext = Convert.FromBase64String(root.GetProperty("payload").GetString()!);
        ciphertext[^1] ^= 1;
        var tampered = new Dictionary<string, object?>();
        foreach (var prop in root.EnumerateObject())
            tampered[prop.Name] = prop.Name == "payload"
                ? Convert.ToBase64String(ciphertext)
                : JsonSerializer.Deserialize<object>(prop.Value.GetRawText());
        File.WriteAllText(store.FilePath, JsonSerializer.Serialize(tampered));

        Assert.Throws<VaultException>(() => store.GetPassword(second));
        Assert.Throws<VaultException>(() => store.Unlock("test integrity master password"));
    }

    [Fact]
    public void LegacyVaultMigratesOnlyAfterAuthenticationAndPreservesOriginals()
    {
        string dir = Path.Combine(_directory, Guid.NewGuid().ToString("D"));
        Directory.CreateDirectory(dir);
        string hostsPath = Path.Combine(dir, "hosts.json");
        string vaultPath = Path.Combine(dir, "passwords.maine");
        const string master = "legacy test master password";

        var host = new Host(Guid.NewGuid(), "Legacy host", "legacy.example");
        File.WriteAllText(hostsPath, new HostConfiguration(new List<Host> { host }).Encode());

        var id = Guid.NewGuid();
        byte[] salt = Enumerable.Repeat((byte)7, 32).ToArray();
        byte[] key = VaultStore.Derive(master, salt, VaultStore.Iterations);
        byte[] context = Encoding.UTF8.GetBytes(
            $"maine-vault:1:{VaultFormat.FormatGuid(id)}:{Convert.ToBase64String(salt)}:600000");
        byte[] verification = VaultStore.Seal(Encoding.UTF8.GetBytes(VaultStore.Marker), key, context);
        byte[] secret = VaultStore.Seal(Encoding.UTF8.GetBytes("legacy secret"), key,
            Combine(context, Encoding.UTF8.GetBytes(VaultFormat.FormatGuid(host.Id))));
        File.WriteAllText(vaultPath, JsonSerializer.Serialize(new Dictionary<string, object>
        {
            ["version"] = 1,
            ["id"] = VaultFormat.FormatGuid(id),
            ["salt"] = Convert.ToBase64String(salt),
            ["iterations"] = 600000,
            ["verification"] = Convert.ToBase64String(verification),
            ["passwords"] = new Dictionary<string, string>
                { [VaultFormat.FormatGuid(host.Id)] = Convert.ToBase64String(secret) },
        }));
        byte[] originalBytes = File.ReadAllBytes(vaultPath);

        var vault = new VaultStore(vaultPath, new InMemoryKeyStore(), hostsPath);
        _cleanup.Add((vault, _directory));
        Assert.Throws<VaultException>(() => vault.Unlock("wrong master password"));
        Assert.Equal(originalBytes, File.ReadAllBytes(vaultPath));

        File.WriteAllText(hostsPath, "not JSON");
        Assert.ThrowsAny<Exception>(() => vault.Unlock(master));
        Assert.Equal(originalBytes, File.ReadAllBytes(vaultPath));

        File.WriteAllText(hostsPath, new HostConfiguration(new List<Host> { host }).Encode());
        vault.Unlock(master);
        Assert.Equal(new List<Host> { host }, vault.GetHosts());
        Assert.Equal("legacy secret", vault.GetPassword(host.Id));
        Assert.Equal(originalBytes, File.ReadAllBytes(vaultPath + ".v1-backup"));
        // Migration must not re-import stale plaintext on subsequent unlocks.
        vault.DeleteHost(host.Id);
        vault.Lock();
        vault.Unlock(master);
        Assert.Empty(vault.GetHosts());
    }

    [Fact]
    public void SwiftProducedVaultOpensWithTheSameMasterPassword()
    {
        // Fixture generated by CryptoKit on macOS (see Fixtures/ + vector scripts).
        // This is the cross-platform proof: a vault written by Swift opens here.
        string fixture = Path.Combine(AppContext.BaseDirectory, "Fixtures", "swift-vault-v2.json");
        Assert.True(File.Exists(fixture), "Swift fixture missing");
        string dir = Path.Combine(_directory, Guid.NewGuid().ToString("D"));
        Directory.CreateDirectory(dir);
        string vaultPath = Path.Combine(dir, "passwords.maine");
        File.Copy(fixture, vaultPath);

        var store = new VaultStore(vaultPath, new InMemoryKeyStore());
        _cleanup.Add((store, _directory));
        Assert.Throws<VaultException>(() => store.Unlock("wrong master password"));
        store.Unlock("vector master password 123");

        var hostId = Guid.Parse("11111111-2222-3333-4444-555555555555");
        var hosts = store.GetHosts();
        Assert.Single(hosts);
        Assert.Equal(hostId, hosts[0].Id);
        Assert.Equal("Vector host", hosts[0].Name);
        Assert.Equal("vector.example", hosts[0].Hostname);
        Assert.Equal(2222, hosts[0].Port);
        Assert.Equal("deploy", hosts[0].Username);
        Assert.Equal("s3cr3t 🔐", store.GetPassword(hostId));

        var sessions = store.GetSavedSessions();
        Assert.Single(sessions);
        Assert.Equal(Guid.Parse("AAAAAAAA-BBBB-CCCC-DDDD-EEEEEEEEEEEE"), sessions[0].Id);
        Assert.Equal(hostId, sessions[0].HostId);
        Assert.Equal("sess 1", sessions[0].Name);
        Assert.Equal("hello\nworld", sessions[0].History);
        Assert.Equal(DateTimeOffset.UnixEpoch.AddSeconds(1700000000.5).ToUnixTimeSeconds(),
            sessions[0].SavedAt.ToUnixTimeSeconds());
        Assert.Equal(true, sessions[0].UsesDefaultName);
        Assert.Equal(2, sessions[0].Number);
    }

    [Fact]
    public void AskPassEnvironmentContainsOnlyAReferenceNotTheSecret()
    {
        var hostId = Guid.NewGuid();
        var environment = SshAskPass.EnvironmentFor(hostId,
            new[] { "TERM=xterm", "SSH_AUTH_SOCK=/agent", "SSH_ASKPASS=/old", "SSH_ASKPASS_REQUIRE=never" },
            @"C:\Program Files\Maine\Maine.exe");
        Assert.Contains("TERM=xterm", environment);
        Assert.Contains("SSH_AUTH_SOCK=/agent", environment);
        Assert.Contains($"{SshAskPass.HostIdVariable}={VaultFormat.FormatGuid(hostId)}", environment);
        Assert.Contains("SSH_ASKPASS_REQUIRE=force", environment);
        Assert.Single(environment, e => e.StartsWith("SSH_ASKPASS=", StringComparison.Ordinal));
        Assert.DoesNotContain("SSH_ASKPASS_REQUIRE=never", environment);
    }

    [Theory]
    [InlineData("pi@raspberrypi's password:", true)]
    [InlineData("Password:", true)]
    [InlineData("Enter passphrase for key '/Users/me/.ssh/id_ed25519':", false)]
    [InlineData("Verification password:", false)]
    [InlineData("One-time password:", false)]
    [InlineData("Are you sure you want to continue connecting (yes/no/[fingerprint])?", false)]
    public void SavedPasswordIsNotUsedForTrustPassphrasesOrMfa(string prompt, bool expected) =>
        Assert.Equal(expected, SshAskPass.IsPasswordPrompt(prompt));

    private static byte[] Combine(byte[] a, byte[] b)
    {
        var result = new byte[a.Length + b.Length];
        Buffer.BlockCopy(a, 0, result, 0, a.Length);
        Buffer.BlockCopy(b, 0, result, a.Length, b.Length);
        return result;
    }
}
