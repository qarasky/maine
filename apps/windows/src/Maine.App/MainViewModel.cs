using System.Collections.ObjectModel;
using System.ComponentModel;
using Maine.Core;

namespace Maine.App;

/// <summary>
/// One row in the sidebar: either a host or one of its sessions.
/// Sessions render as indented child rows beneath their host.
/// </summary>
public sealed class SidebarItem
{
    public bool IsSession { get; init; }
    public Guid Id { get; init; }
    public Guid HostId { get; init; }
    public string Title { get; init; } = "";
    public string Detail { get; init; } = "";
    public int Indent { get; init; }
    public SessionState State { get; init; } = SessionState.Idle;
}

/// <summary>
/// Window-level UI state shared by menus and views. Host list data lives in
/// the vault; open sessions live in the manager; the item being edited lives
/// here — so menus and views can never disagree about what is selected.
/// Mirrors <c>AppState.swift</c> in the macOS app.
/// </summary>
public sealed class MainViewModel : INotifyPropertyChanged
{
    private readonly VaultStore _vault;
    private readonly SshSessionManager _sessions;

    public event PropertyChangedEventHandler? PropertyChanged;

    public ObservableCollection<SidebarItem> SidebarItems { get; } = new();
    public ObservableCollection<Host> Hosts { get; } = new();

    private SidebarItem? _selectedItem;
    public SidebarItem? SelectedItem
    {
        get => _selectedItem;
        set
        {
            _selectedItem = value;
            OnChanged(nameof(SelectedItem));
            OnChanged(nameof(SelectedSession));
            OnChanged(nameof(HeaderEndpoint));
            OnChanged(nameof(HeaderStatus));
            OnChanged(nameof(BannerText));
            OnChanged(nameof(IsBannerVisible));
        }
    }

    public SshSession? SelectedSession =>
        SelectedItem is { IsSession: true } item ? _sessions.Session(item.Id) : null;

    public string HeaderEndpoint
    {
        get
        {
            var host = SelectedHost();
            if (host is null)
                return "Maine";
            var target = SshTarget.Make(host, SshTarget.ResolveExecutable());
            return $"{host.DisplayName} — {target.Resolved(new SshConfigFile(), null).Target}";
        }
    }

    public string HeaderStatus => SelectedSession switch
    {
        { State: SessionState.Connecting } => "Connecting…",
        { State: SessionState.Connected } => "Connected",
        { State: SessionState.Closed } => "Closed",
        { State: SessionState.Disconnected, ExitCode: { } code } => $"Disconnected (exit {code})",
        { State: SessionState.Failed, Error: { } error } => error,
        _ => SelectedHost() is null ? "Select a host to connect." : "Ready to connect.",
    };

    public string? BannerText => SelectedSession switch
    {
        { State: SessionState.Disconnected, Error: { } error } => error,
        { State: SessionState.Failed, Error: { } error } => error,
        _ => null,
    };

    public bool IsBannerVisible => BannerText is not null;

    public bool IsUnlocked { get; private set; }

    public MainViewModel(VaultStore vault, SshSessionManager sessions)
    {
        _vault = vault;
        _sessions = sessions;
        _sessions.SessionsChanged += RefreshSidebar;
    }

    public Host? SelectedHost()
    {
        if (SelectedItem is null)
            return null;
        return Hosts.FirstOrDefault(h => h.Id == SelectedItem.HostId);
    }

    public void SetUnlocked(bool unlocked)
    {
        IsUnlocked = unlocked;
        if (!unlocked)
            Hosts.Clear();
        RefreshSidebar();
        OnChanged(nameof(IsUnlocked));
    }

    public void ReloadHosts()
    {
        Hosts.Clear();
        try
        {
            foreach (var host in _vault.GetHosts())
                Hosts.Add(host);
        }
        catch (VaultException)
        {
            // Locked: the host list stays empty and editors stay disabled.
        }
        RefreshSidebar();
    }

    public void RefreshSidebar()
    {
        App.Current?.Dispatcher.Invoke(() =>
        {
            Guid? selectedId = SelectedItem?.Id;
            SidebarItems.Clear();
            foreach (var host in Hosts)
            {
                SidebarItems.Add(new SidebarItem
                {
                    IsSession = false,
                    Id = host.Id,
                    HostId = host.Id,
                    Title = host.DisplayName,
                    Detail = host.Subtitle,
                });
                var open = _sessions.SessionsFor(host.Id);
                foreach (var session in open)
                {
                    SidebarItems.Add(new SidebarItem
                    {
                        IsSession = true,
                        Id = session.Id,
                        HostId = host.Id,
                        Title = $"  {session.Name}",
                        Detail = session.State.ToString().ToLowerInvariant(),
                        Indent = 1,
                        State = session.State,
                    });
                }
            }
            if (selectedId is Guid id)
                SelectedItem = SidebarItems.FirstOrDefault(i => i.Id == id);
            OnChanged(nameof(HeaderEndpoint));
            OnChanged(nameof(HeaderStatus));
            OnChanged(nameof(BannerText));
            OnChanged(nameof(IsBannerVisible));
        });
    }

    private void OnChanged(string name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
