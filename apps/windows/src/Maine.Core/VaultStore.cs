using System.Security.AccessControl;
using System.Security.Cryptography;
using System.Text;
using System.Text.Json;

namespace Maine.Core;

/// <summary>
/// Portable AES-256-GCM hosts and credentials protected by a master password.
/// The file format is byte-compatible with the macOS vault: a vault written
/// on one OS opens on the other with the same master password. The master
/// password never goes on disk. The <see cref="IKeyStore"/> remembers the
/// derived key on this machine only, so the SSH askpass helper can reuse
/// saved credentials. Mirrors <c>PasswordStore.swift</c> in the macOS app.
///
/// File shape (JSON):
/// <code>
/// {
///   "version": 2, "id": "UPPERCASE-UUID", "salt": "base64-32-bytes",
///   "iterations": 600000, "verification": "base64(nonce|ct|tag)",
///   "passwords": {}, "payload": "base64(nonce|ct|tag)"
/// }
/// </code>
/// The verification marker plaintext is <c>"Maine password vault v1"</c>.
/// The AES-GCM additional data is
/// <c>"maine-vault:{version}:{UPPERCASE-UUID}:{base64-salt}:{iterations}"</c>.
/// </summary>
public sealed class VaultStore
{
    public const int Iterations = 600_000;
    public const int SaltLength = 32;
    public const int KeyLength = 32;
    public const string Marker = "Maine password vault v1";

    public string FilePath { get; }
    public string? LegacyHostsPath { get; }
    private readonly IKeyStore _keys;

    public VaultStore(string filePath, IKeyStore keyStore, string? legacyHostsPath = null)
    {
        FilePath = filePath;
        _keys = keyStore;
        LegacyHostsPath = legacyHostsPath;
    }

    public static string DefaultFilePath
    {
        get
        {
            var appData = Environment.GetFolderPath(Environment.SpecialFolder.ApplicationData);
            return Path.Combine(appData, "Maine", "passwords.maine");
        }
    }

    public static string LegacyHostsPathFor(string vaultPath) =>
        Path.Combine(Path.GetDirectoryName(vaultPath) ?? "", "hosts.json");

    public bool Exists => File.Exists(FilePath);

    /// <summary>Creates a vault on first use, or verifies and unlocks an existing copy.</summary>
    public void Unlock(string masterPassword)
    {
        var vault = Read();
        if (vault is null)
        {
            CreateNew(masterPassword);
            return;
        }

        byte[] key;
        try
        {
            key = Derive(masterPassword, vault.Salt, vault.Iterations);
        }
        catch
        {
            throw VaultException.InvalidPassword();
        }

        // Authenticate the master key. Every failure here means the password
        // is wrong or the vault is damaged — same as the macOS client.
        VaultContents contents;
        try
        {
            if (!ConstantTimeEquals(Open(vault.Verification, key, vault.Context), Encoding.UTF8.GetBytes(Marker)))
                throw VaultException.InvalidPassword();
            contents = new VaultContents();
            // Verify every entry before remembering a key for this vault.
            foreach (var (hostId, data) in vault.PasswordsV1)
            {
                byte[] plain = Open(data, key, Concat(vault.Context, Encoding.UTF8.GetBytes(hostId)));
                contents.Passwords[hostId] = StrictUtf8(plain);
            }
            if (vault.Version == 2)
                contents = DecodeContents(vault, key);
        }
        catch
        {
            CryptographicOperations.ZeroMemory(key);
            throw VaultException.InvalidPassword();
        }

        try
        {
            if (vault.Version == 1)
                MigrateV1(vault, key, contents);
            _keys.SaveKey(vault.Id, key);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(key);
        }
    }

    private void MigrateV1(VaultFile vault, byte[] key, VaultContents contents)
    {
        // Read legacy metadata only after authenticating the master key.
        // Keep the originals as backups; never overwrite malformed input.
        // Errors here propagate raw, like the macOS client.
        if (LegacyHostsPath is not null && File.Exists(LegacyHostsPath))
            contents.Hosts = HostConfiguration.Decode(File.ReadAllText(LegacyHostsPath)).Hosts;
        string backup = FilePath + ".v1-backup";
        if (!File.Exists(backup))
            File.Copy(FilePath, backup);
        vault.Version = 2;
        vault.PasswordsV1 = new Dictionary<string, byte[]>();
        vault.Verification = Seal(Encoding.UTF8.GetBytes(Marker), key, vault.Context);
        vault.Payload = Seal(EncodeContents(contents), key, vault.Context);
        Write(vault);
    }

    private void CreateNew(string masterPassword)
    {
        if (masterPassword.Length < 12)
            throw VaultException.WeakPassword();
        byte[] salt = RandomNumberGenerator.GetBytes(SaltLength);
        var fresh = new VaultFile(Guid.NewGuid(), salt, Iterations, Array.Empty<byte>(),
            new Dictionary<string, byte[]>(), null, Version: 2);
        byte[] freshKey = Derive(masterPassword, salt, fresh.Iterations);
        try
        {
            fresh.Verification = Seal(Encoding.UTF8.GetBytes(Marker), freshKey, fresh.Context);
            var freshContents = new VaultContents();
            if (LegacyHostsPath is not null && File.Exists(LegacyHostsPath))
                freshContents.Hosts = HostConfiguration.Decode(File.ReadAllText(LegacyHostsPath)).Hosts;
            fresh.Payload = Seal(EncodeContents(freshContents), freshKey, fresh.Context);
            Write(fresh);
            _keys.SaveKey(fresh.Id, freshKey);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(freshKey);
        }
    }

    /// <summary>Decodes UTF-8 strictly: invalid bytes fail instead of turning
    /// into replacement characters (mirrors Swift's failable String init).</summary>
    private static string StrictUtf8(byte[] bytes)
    {
        var strict = new UTF8Encoding(encoderShouldEmitUTF8Identifier: false, throwOnInvalidBytes: true);
        try
        {
            return strict.GetString(bytes);
        }
        catch (DecoderFallbackException)
        {
            throw VaultException.InvalidFile();
        }
    }

    public void Lock()
    {
        var vault = Read();
        if (vault is not null)
            _keys.DeleteKey(vault.Id);
    }

    public string? GetPassword(Guid hostId)
    {
        var vault = Read();
        if (vault is null)
            return null;
        return DecodeContents(vault, UnlockedKey(vault))
            .Passwords.GetValueOrDefault(VaultFormat.FormatGuid(hostId));
    }

    public void SavePassword(string password, Guid hostId)
    {
        if (password.Length == 0)
        {
            DeletePassword(hostId);
            return;
        }
        UpdateContents(c => c.Passwords[VaultFormat.FormatGuid(hostId)] = password);
    }

    public void DeletePassword(Guid hostId)
    {
        if (!Exists)
            return;
        UpdateContents(c => c.Passwords.Remove(VaultFormat.FormatGuid(hostId)));
    }

    public List<Host> GetHosts()
    {
        var vault = Read();
        if (vault is null)
            return new List<Host>();
        return DecodeContents(vault, UnlockedKey(vault)).Hosts;
    }

    public void SaveHosts(List<Host> hosts) =>
        UpdateContents(c => c.Hosts = new List<Host>(hosts));

    /// <summary>Commits metadata and its credential together in one atomic replacement.</summary>
    public void SaveHost(Host host, string password)
    {
        if (!host.IsValid || password.Length == 0)
            throw VaultException.InvalidFile();
        UpdateContents(contents =>
        {
            int index = contents.Hosts.FindIndex(h => h.Id == host.Id);
            if (index >= 0)
                contents.Hosts[index] = host;
            else
                contents.Hosts.Add(host);
            contents.Passwords[VaultFormat.FormatGuid(host.Id)] = password;
        });
    }

    public void DeleteHost(Guid hostId) =>
        UpdateContents(contents =>
        {
            contents.Hosts.RemoveAll(h => h.Id == hostId);
            contents.Passwords.Remove(VaultFormat.FormatGuid(hostId));
            contents.Sessions?.RemoveAll(s => s.HostId == hostId);
        });

    public List<SavedSession> GetSavedSessions()
    {
        var vault = Read();
        if (vault is null)
            return new List<SavedSession>();
        return DecodeContents(vault, UnlockedKey(vault)).Sessions ?? new List<SavedSession>();
    }

    public void SaveSession(SavedSession session)
    {
        if (string.IsNullOrWhiteSpace(session.Name))
            throw VaultException.InvalidFile();
        UpdateContents(contents =>
        {
            contents.Sessions ??= new List<SavedSession>();
            int index = contents.Sessions.FindIndex(s => s.Id == session.Id);
            if (index >= 0)
                contents.Sessions[index] = session;
            else
                contents.Sessions.Add(session);
        });
    }

    public void SaveSessions(List<SavedSession> sessions) =>
        UpdateContents(contents =>
        {
            contents.Sessions ??= new List<SavedSession>();
            foreach (var session in sessions)
            {
                int index = contents.Sessions.FindIndex(s => s.Id == session.Id);
                if (index >= 0)
                    contents.Sessions[index] = session;
                else
                    contents.Sessions.Add(session);
            }
        });

    public void DeleteSavedSession(Guid id) =>
        UpdateContents(contents => contents.Sessions?.RemoveAll(s => s.Id == id));

    private void UpdateContents(Action<VaultContents> update)
    {
        var vault = Read() ?? throw VaultException.Locked();
        byte[] key = UnlockedKey(vault);
        try
        {
            var contents = DecodeContents(vault, key);
            update(contents);
            vault.Payload = Seal(EncodeContents(contents), key, vault.Context);
            Write(vault);
        }
        finally
        {
            CryptographicOperations.ZeroMemory(key);
        }
    }

    private VaultContents DecodeContents(VaultFile vault, byte[] key)
    {
        if (vault.Version != 2 || vault.Payload is null)
            throw VaultException.InvalidFile();
        byte[] plain;
        try
        {
            plain = Open(vault.Payload, key, vault.Context);
        }
        catch
        {
            throw VaultException.InvalidFile();
        }
        JsonDocument doc;
        try
        {
            doc = JsonDocument.Parse(plain);
        }
        catch
        {
            throw VaultException.InvalidFile();
        }
        using (doc)
        {
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
                throw VaultException.InvalidFile();
            var contents = new VaultContents();

            if (root.TryGetProperty("hosts", out var hostsEl))
            {
                if (hostsEl.ValueKind != JsonValueKind.Array)
                    throw VaultException.InvalidFile();
                int index = 0;
                try
                {
                    foreach (var hostEl in hostsEl.EnumerateArray())
                    {
                        contents.Hosts.Add(Host.FromElement(hostEl, $"hosts.{index}"));
                        index++;
                    }
                }
                catch (VaultException)
                {
                    throw VaultException.InvalidFile();
                }
                // Refuse to save a host list that would not load back.
                HostConfiguration.Decode(new HostConfiguration(contents.Hosts).Encode());
            }

            if (root.TryGetProperty("passwords", out var passwordsEl))
            {
                if (passwordsEl.ValueKind != JsonValueKind.Object)
                    throw VaultException.InvalidFile();
                foreach (var prop in passwordsEl.EnumerateObject())
                {
                    if (!Guid.TryParse(prop.Name, out _) ||
                        prop.Value.ValueKind != JsonValueKind.String)
                        throw VaultException.InvalidFile();
                    contents.Passwords[prop.Name.ToUpperInvariant()] = prop.Value.GetString() ?? "";
                }
            }

            if (root.TryGetProperty("sessions", out var sessionsEl) &&
                sessionsEl.ValueKind != JsonValueKind.Null)
            {
                if (sessionsEl.ValueKind != JsonValueKind.Array)
                    throw VaultException.InvalidFile();
                contents.Sessions = new List<SavedSession>();
                int index = 0;
                try
                {
                    foreach (var sessionEl in sessionsEl.EnumerateArray())
                    {
                        contents.Sessions.Add(SavedSession.FromElement(sessionEl, $"sessions.{index}"));
                        index++;
                    }
                }
                catch (VaultException)
                {
                    throw VaultException.InvalidFile();
                }
            }
            return contents;
        }
    }

    private byte[] UnlockedKey(VaultFile vault)
    {
        byte[]? data = _keys.GetKey(vault.Id);
        if (data is null || data.Length != KeyLength)
            throw VaultException.Locked();
        try
        {
            if (!ConstantTimeEquals(Open(vault.Verification, data, vault.Context),
                    Encoding.UTF8.GetBytes(Marker)))
                throw VaultException.InvalidFile();
        }
        catch (VaultException)
        {
            throw;
        }
        catch
        {
            throw VaultException.InvalidFile();
        }
        return data;
    }

    private VaultFile? Read()
    {
        if (!Exists)
            return null;
        try
        {
            using var doc = JsonDocument.Parse(File.ReadAllBytes(FilePath));
            var root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object)
                throw VaultException.InvalidFile();

            int version = root.TryGetProperty("version", out var vEl) &&
                vEl.ValueKind == JsonValueKind.Number && vEl.TryGetInt32(out int v) ? v : -1;
            if (version is not (1 or 2))
                throw VaultException.InvalidFile();

            if (!root.TryGetProperty("id", out var idEl) || idEl.ValueKind != JsonValueKind.String ||
                !Guid.TryParse(idEl.GetString(), out var id))
                throw VaultException.InvalidFile();

            if (!root.TryGetProperty("salt", out var saltEl) || saltEl.ValueKind != JsonValueKind.String)
                throw VaultException.InvalidFile();
            byte[] salt;
            try { salt = Convert.FromBase64String(saltEl.GetString() ?? ""); }
            catch { throw VaultException.InvalidFile(); }
            if (salt.Length != SaltLength)
                throw VaultException.InvalidFile();

            if (!root.TryGetProperty("iterations", out var itEl) ||
                itEl.ValueKind != JsonValueKind.Number || !itEl.TryGetInt32(out int iterations) ||
                iterations != Iterations)
                throw VaultException.InvalidFile();

            if (!root.TryGetProperty("verification", out var verEl) ||
                verEl.ValueKind != JsonValueKind.String)
                throw VaultException.InvalidFile();
            byte[] verification;
            try { verification = Convert.FromBase64String(verEl.GetString() ?? ""); }
            catch { throw VaultException.InvalidFile(); }

            var passwords = new Dictionary<string, byte[]>();
            if (root.TryGetProperty("passwords", out var pwEl))
            {
                if (pwEl.ValueKind != JsonValueKind.Object)
                    throw VaultException.InvalidFile();
                foreach (var prop in pwEl.EnumerateObject())
                {
                    if (!Guid.TryParse(prop.Name, out _) ||
                        prop.Value.ValueKind != JsonValueKind.String)
                        throw VaultException.InvalidFile();
                    // Version-1 keys keep their original spelling: the key
                    // string is part of the entry's AES-GCM additional data,
                    // so case must round-trip exactly.
                    try { passwords[prop.Name] = Convert.FromBase64String(prop.Value.GetString() ?? ""); }
                    catch { throw VaultException.InvalidFile(); }
                }
            }

            byte[]? payload = null;
            if (root.TryGetProperty("payload", out var payloadEl) &&
                payloadEl.ValueKind != JsonValueKind.Null)
            {
                if (payloadEl.ValueKind != JsonValueKind.String)
                    throw VaultException.InvalidFile();
                try { payload = Convert.FromBase64String(payloadEl.GetString() ?? ""); }
                catch { throw VaultException.InvalidFile(); }
            }

            if (version == 2 && (payload is null || passwords.Count != 0))
                throw VaultException.InvalidFile();

            return new VaultFile(id, salt, iterations, verification, passwords, payload, version);
        }
        catch (VaultException)
        {
            throw VaultException.InvalidFile();
        }
        catch
        {
            throw VaultException.InvalidFile();
        }
    }

    private void Write(VaultFile vault)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream, new JsonWriterOptions { Indented = true }))
        {
            writer.WriteStartObject();
            writer.WriteNumber("version", vault.Version);
            writer.WriteString("id", VaultFormat.FormatGuid(vault.Id));
            writer.WriteString("salt", Convert.ToBase64String(vault.Salt));
            writer.WriteNumber("iterations", vault.Iterations);
            writer.WriteString("verification", Convert.ToBase64String(vault.Verification));
            writer.WriteStartObject("passwords");
            foreach (var (hostId, data) in vault.PasswordsV1.OrderBy(kv => kv.Key, StringComparer.Ordinal))
            {
                writer.WriteString(hostId.ToUpperInvariant(), Convert.ToBase64String(data));
            }
            writer.WriteEndObject();
            if (vault.Payload is not null)
                writer.WriteString("payload", Convert.ToBase64String(vault.Payload));
            else
                writer.WriteNull("payload");
            writer.WriteEndObject();
        }

        string? folder = Path.GetDirectoryName(FilePath);
        if (!string.IsNullOrEmpty(folder))
            Directory.CreateDirectory(folder);
        // Atomic replacement keeps a crash from leaving a half written file.
        string temp = Path.Combine(folder ?? Path.GetTempPath(), Path.GetRandomFileName());
        File.WriteAllBytes(temp, stream.ToArray());
        try
        {
            File.Move(temp, FilePath, overwrite: true);
        }
        catch
        {
            try { File.Delete(temp); } catch { }
            throw;
        }
        RestrictToCurrentUser(FilePath);
    }

    public static void RestrictToCurrentUser(string path)
    {
        if (OperatingSystem.IsWindows())
        {
            var security = new FileSecurity();
            security.SetAccessRuleProtection(isProtected: true, preserveInheritance: false);
            var user = System.Security.Principal.WindowsIdentity.GetCurrent().User;
            if (user is not null)
            {
                security.AddAccessRule(new FileSystemAccessRule(
                    user,
                    FileSystemRights.FullControl,
                    AccessControlType.Allow));
                new FileInfo(path).SetAccessControl(security);
            }
        }
        else
        {
            try
            {
                File.SetUnixFileMode(path,
                    UnixFileMode.UserRead | UnixFileMode.UserWrite);
            }
            catch { /* best effort on exotic mounts */ }
        }
    }

    internal static byte[] Derive(string password, byte[] salt, int iterations) =>
        Rfc2898DeriveBytes.Pbkdf2(
            Encoding.UTF8.GetBytes(password), salt, iterations,
            HashAlgorithmName.SHA256, KeyLength);

    internal static byte[] Seal(byte[] data, byte[] key, byte[] context)
    {
        byte[] nonce = RandomNumberGenerator.GetBytes(12);
        byte[] ciphertext = new byte[data.Length];
        byte[] tag = new byte[16];
        using (var gcm = new AesGcm(key, 16))
            gcm.Encrypt(nonce, data, ciphertext, tag, context);
        var combined = new byte[12 + ciphertext.Length + 16];
        Buffer.BlockCopy(nonce, 0, combined, 0, 12);
        Buffer.BlockCopy(ciphertext, 0, combined, 12, ciphertext.Length);
        Buffer.BlockCopy(tag, 0, combined, 12 + ciphertext.Length, 16);
        return combined;
    }

    internal static byte[] Open(byte[] combined, byte[] key, byte[] context)
    {
        if (combined.Length < 12 + 16)
            throw new CryptographicException("Sealed box too short.");
        byte[] nonce = combined[..12];
        byte[] tag = combined[^16..];
        byte[] ciphertext = combined[12..^16];
        byte[] plaintext = new byte[ciphertext.Length];
        using (var gcm = new AesGcm(key, 16))
            gcm.Decrypt(nonce, ciphertext, tag, plaintext, context);
        return plaintext;
    }

    private static byte[] Concat(byte[] a, byte[] b)
    {
        var result = new byte[a.Length + b.Length];
        Buffer.BlockCopy(a, 0, result, 0, a.Length);
        Buffer.BlockCopy(b, 0, result, a.Length, b.Length);
        return result;
    }

    private static bool ConstantTimeEquals(byte[] a, byte[] b) =>
        CryptographicOperations.FixedTimeEquals(a, b);

    private static byte[] EncodeContents(VaultContents contents)
    {
        using var stream = new MemoryStream();
        using (var writer = new Utf8JsonWriter(stream))
        {
            writer.WriteStartObject();
            writer.WriteStartArray("hosts");
            foreach (var host in contents.Hosts)
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
            writer.WriteStartObject("passwords");
            foreach (var (hostId, password) in contents.Passwords)
                writer.WriteString(hostId.ToUpperInvariant(), password);
            writer.WriteEndObject();
            if (contents.Sessions is null)
            {
                writer.WriteNull("sessions");
            }
            else
            {
                writer.WriteStartArray("sessions");
                foreach (var session in contents.Sessions)
                {
                    writer.WriteStartObject();
                    writer.WriteString("id", VaultFormat.FormatGuid(session.Id));
                    writer.WriteString("hostID", VaultFormat.FormatGuid(session.HostId));
                    writer.WriteString("name", session.Name);
                    writer.WriteString("history", session.History);
                    writer.WriteNumber("savedAt",
                        (session.SavedAt - DateTimeOffset.UnixEpoch).TotalSeconds);
                    if (session.UsesDefaultName is bool udn)
                        writer.WriteBoolean("usesDefaultName", udn);
                    else
                        writer.WriteNull("usesDefaultName");
                    if (session.Number is int number)
                        writer.WriteNumber("number", number);
                    else
                        writer.WriteNull("number");
                    writer.WriteEndObject();
                }
                writer.WriteEndArray();
            }
            writer.WriteEndObject();
        }
        return stream.ToArray();
    }

    private sealed class VaultContents
    {
        public List<Host> Hosts { get; set; } = new();
        public Dictionary<string, string> Passwords { get; } = new(StringComparer.OrdinalIgnoreCase);
        public List<SavedSession>? Sessions { get; set; }
    }

    private sealed class VaultFile
    {
        public int Version;
        public Guid Id;
        public byte[] Salt;
        public int Iterations;
        public byte[] Verification;
        public Dictionary<string, byte[]> PasswordsV1;
        public byte[]? Payload;

        public VaultFile(Guid id, byte[] salt, int iterations, byte[] verification,
            Dictionary<string, byte[]> passwordsV1, byte[]? payload, int Version)
        {
            Id = id;
            Salt = salt;
            Iterations = iterations;
            Verification = verification;
            PasswordsV1 = passwordsV1;
            Payload = payload;
            this.Version = Version;
        }

        public byte[] Context => Encoding.UTF8.GetBytes(
            $"maine-vault:{Version}:{VaultFormat.FormatGuid(Id)}" +
            $":{Convert.ToBase64String(Salt)}:{Iterations}");
    }
}
