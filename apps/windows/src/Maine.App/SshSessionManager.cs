using System.Diagnostics;
using System.Text;
using Maine.Core;

namespace Maine.App;

public enum SessionState
{
    Idle,
    Connecting,
    Connected,
    Closed,
    Disconnected,
    Failed,
}

/// <summary>
/// One live connection: a ConPTY child running <c>ssh.exe</c>, keyed by
/// session UUID (never by host UUID, so one host can own many sessions).
/// Mirrors <c>TerminalSession</c> + <c>SessionManager</c> in the macOS app.
/// </summary>
public sealed class SshSession
{
    public Guid Id { get; } = Guid.NewGuid();
    public Guid HostId { get; }
    public string Name { get; set; }
    public int Number { get; }
    public bool UsesDefaultName { get; set; }
    public SessionState State { get; set; } = SessionState.Idle;
    public int? ExitCode { get; set; }
    public string? Error { get; set; }
    public ConPtySession? Process { get; set; }

    /// <summary>Retained plain-text buffer for snapshots (output only, capped at 1 MiB).</summary>
    public readonly StringBuilder History = new();

    public event Action<byte[]>? Output;
    public event Action? Changed;

    public SshSession(Guid hostId, string name, int number, bool usesDefaultName)
        : this(Guid.NewGuid(), hostId, name, number, usesDefaultName)
    {
    }

    public SshSession(Guid id, Guid hostId, string name, int number, bool usesDefaultName)
    {
        Id = id;
        HostId = hostId;
        Name = name;
        Number = number;
        UsesDefaultName = usesDefaultName;
    }

    public void NotifyChanged() => Changed?.Invoke();
    public void NotifyOutput(byte[] chunk) => Output?.Invoke(chunk);
}

/// <summary>
/// Owns every open session. Views never spawn processes; they ask the
/// manager, which publishes status back. Sessions stay mounted while
/// switching, and reconnect replaces the process in place.
/// </summary>
public sealed class SshSessionManager : IDisposable
{
    public const int MaxHistoryBytes = 1024 * 1024;
    public const int SnapshotIntervalMs = 2000;

    private readonly VaultStore _vault;
    private readonly Func<string> _executablePath;
    private readonly System.Timers.Timer _snapshotTimer;
    private readonly object _gate = new();
    private bool _disposed;

    public event Action? SessionsChanged;

    public List<SshSession> Sessions { get; } = new();

    public SshSessionManager(VaultStore vault, Func<string>? executablePath = null)
    {
        _vault = vault;
        _executablePath = executablePath ?? (() => Process.GetCurrentProcess().MainModule?.FileName ?? "Maine.exe");
        _snapshotTimer = new System.Timers.Timer(SnapshotIntervalMs);
        _snapshotTimer.Elapsed += (_, _) => Checkpoint();
        _snapshotTimer.AutoReset = true;
        _snapshotTimer.Start();
    }

    public SshSession? Session(Guid id)
    {
        lock (_gate) return Sessions.FirstOrDefault(s => s.Id == id);
    }

    public List<SshSession> SessionsFor(Guid hostId)
    {
        lock (_gate) return Sessions.Where(s => s.HostId == hostId).ToList();
    }

    /// <summary>Opening an already-connected host reveals its session;
    /// <see cref="OpenNew"/> always creates another independent connection.</summary>
    public SshSession OpenOrReveal(Host host)
    {
        lock (_gate)
        {
            var existing = Sessions.FirstOrDefault(s => s.HostId == host.Id);
            if (existing is not null)
                return existing;
        }
        return OpenNew(host);
    }

    public SshSession OpenNew(Host host)
    {
        int number;
        lock (_gate)
            number = SessionsFor(host.Id).Select(s => s.Number).DefaultIfEmpty(0).Max() + 1;
        var session = new SshSession(host.Id,
            SessionsFor(host.Id).Count == 0 ? host.DisplayName : $"{host.DisplayName} {number}",
            number, usesDefaultName: true);
        lock (_gate) Sessions.Add(session);
        Connect(session, host, restoredHistory: null);
        SessionsChanged?.Invoke();
        return session;
    }

    public void Reconnect(SshSession session, Host host)
    {
        StopProcess(session);
        // Reconnect starts from a clean screen, like the macOS client.
        session.State = SessionState.Connecting;
        session.ExitCode = null;
        session.Error = null;
        Connect(session, host, restoredHistory: null);
        session.NotifyChanged();
        SessionsChanged?.Invoke();
    }

    /// <summary>Explicit close deletes the encrypted session record first;
    /// a failed delete keeps the session visible and reports an error.</summary>
    public string? Close(SshSession session, bool explicitClose)
    {
        if (explicitClose)
        {
            try
            {
                _vault.DeleteSavedSession(session.Id);
            }
            catch (Exception ex)
            {
                return ex.Message;
            }
        }
        StopProcess(session);
        lock (_gate) Sessions.Remove(session);
        session.State = SessionState.Closed;
        SessionsChanged?.Invoke();
        return null;
    }

    public void CloseAllForHost(Guid hostId)
    {
        foreach (var session in SessionsFor(hostId).ToList())
            Close(session, explicitClose: false);
    }

    public void Rename(SshSession session, string name)
    {
        session.Name = name;
        session.UsesDefaultName = false;
        PersistSession(session);
        session.NotifyChanged();
        SessionsChanged?.Invoke();
    }

    private void Connect(SshSession session, Host host, string? restoredHistory)
    {
        session.State = SessionState.Connecting;
        var target = SshTarget.Make(host);
        var environment = TerminalEnvironment.Variables();
        string? savedPassword = null;
        try { savedPassword = _vault.GetPassword(host.Id); } catch { /* locked: prompt in terminal */ }
        if (savedPassword is not null)
        {
            var baseEnv = environment.Select(kv => $"{kv.Key}={kv.Value}");
            foreach (var entry in SshAskPass.EnvironmentFor(host.Id, baseEnv, _executablePath()))
            {
                int eq = entry.IndexOf('=');
                environment[entry[..eq]] = entry[(eq + 1)..];
            }
        }

        var process = new ConPtySession();
        session.Process = process;
        process.DataReceived += chunk =>
        {
            lock (_gate)
            {
                AppendHistory(session, chunk);
            }
            session.NotifyOutput(chunk);
            if (session.State == SessionState.Connecting)
            {
                session.State = SessionState.Connected;
                session.NotifyChanged();
            }
        };
        process.Exited += code =>
        {
            bool live;
            lock (_gate) live = Sessions.Contains(session);
            if (!live)
                return; // Closed explicitly or with its host; records already handled.
            session.ExitCode = code;
            if (code == 0)
            {
                session.State = SessionState.Closed;
                session.Error = null;
            }
            else if (session.State == SessionState.Connecting)
            {
                session.State = SessionState.Failed;
                session.Error = $"ssh exited with code {code} before the terminal was ready.";
            }
            else
            {
                // Unexpected non-zero exits raise a banner; the terminal keeps
                // whatever ssh printed. (Exit 255 from termination only
                // happens after an explicit close, which is not live anymore.)
                session.State = SessionState.Disconnected;
                session.Error = $"ssh exited with code {code}.";
            }
            PersistSession(session);
            session.NotifyChanged();
            SessionsChanged?.Invoke();
        };

        try
        {
            process.Start(target.Executable, target.Arguments, environment, cols: 80, rows: 24);
        }
        catch (Exception ex)
        {
            session.State = SessionState.Failed;
            session.Error = ex.Message;
            session.NotifyChanged();
        }
        PersistSession(session, restoredHistory);
    }

    private static void AppendHistory(SshSession session, byte[] chunk)
    {
        // Keep plain text only; control characters are stripped at snapshot time.
        string text = Encoding.UTF8.GetString(chunk);
        session.History.Append(text);
        if (session.History.Length > MaxHistoryBytes)
            session.History.Remove(0, session.History.Length - MaxHistoryBytes);
    }

    /// <summary>Strips control characters for scrollback restore. Restored
    /// output is fed only to the renderer, never to a PTY.</summary>
    public static string StripForScrollback(string text)
    {
        var sb = new StringBuilder(text.Length);
        foreach (char c in text)
        {
            if (c is '\n' or '\r' or '\t' || !char.IsControl(c))
                sb.Append(c);
        }
        string stripped = sb.ToString();
        if (Encoding.UTF8.GetByteCount(stripped) > MaxHistoryBytes)
            stripped = stripped[^MaxHistoryBytes..];
        return stripped;
    }

    private void PersistSession(SshSession session, string? restoredHistory = null)
    {
        try
        {
            string history;
            lock (_gate) history = session.History.ToString();
            _vault.SaveSession(new SavedSession(session.HostId, session.Name,
                StripForScrollback(restoredHistory ?? history),
                session.Id, DateTimeOffset.UtcNow, session.UsesDefaultName, session.Number));
        }
        catch { /* while locked, the last checkpoint is retained */ }
    }

    /// <summary>Periodic + quit/lock checkpoint.</summary>
    public void Checkpoint()
    {
        List<SshSession> snapshot;
        lock (_gate) snapshot = Sessions.ToList();
        // Quit keeps records; only explicit close deletes them, so every live
        // session is checkpointed unconditionally here.
        foreach (var session in snapshot)
            PersistSession(session);
    }

    /// <summary>After unlock, restores saved snapshots as scrollback and
    /// reconnects automatically. Already-mounted IDs are skipped.</summary>
    public void Restore(Func<Guid, Host?> findHost)
    {
        List<SavedSession> saved;
        try { saved = _vault.GetSavedSessions(); }
        catch { return; }
        foreach (var record in saved)
        {
            lock (_gate)
            {
                if (Sessions.Any(s => s.Id == record.Id))
                    continue;
            }
            var host = findHost(record.HostId);
            if (host is null)
                continue;
            var session = new SshSession(record.Id, record.HostId, record.Name,
                record.Number ?? 1, record.UsesDefaultName ?? true);
            lock (_gate) Sessions.Add(session);
            Connect(session, host, restoredHistory: record.History);
        }
        SessionsChanged?.Invoke();
    }

    /// <summary>Stops processes without deleting records (quit / lock).</summary>
    public void StopAll()
    {
        Checkpoint();
        List<SshSession> snapshot;
        lock (_gate) snapshot = Sessions.ToList();
        foreach (var session in snapshot)
            StopProcess(session);
    }

    private static void StopProcess(SshSession session)
    {
        try { session.Process?.Kill(); } catch { }
        try { session.Process?.Dispose(); } catch { }
        session.Process = null;
    }

    public void Dispose()
    {
        if (_disposed)
            return;
        _disposed = true;
        _snapshotTimer.Stop();
        _snapshotTimer.Dispose();
        StopAll();
    }
}
