using System.ComponentModel;
using System.IO;
using System.Text;
using System.Text.Json;
using System.Windows;
using System.Windows.Controls;
using System.Windows.Data;
using System.Windows.Input;
using Maine.Core;
using Microsoft.Web.WebView2.Wpf;

namespace Maine.App;

public partial class MainWindow : Window
{
    private readonly VaultStore _vault;
    private readonly SshSessionManager _sessions;
    private readonly MainViewModel _viewModel;
    private readonly Dictionary<Guid, WebView2> _terminals = new();
    private readonly HashSet<Guid> _subscribed = new();
    private readonly string _terminalHtmlPath;

    public MainWindow()
    {
        InitializeComponent();
        _vault = new VaultStore(VaultPaths.VaultFile, new DpapiFileKeyStore(), VaultPaths.LegacyHostsFile);
        _sessions = new SshSessionManager(_vault);
        _viewModel = new MainViewModel(_vault, _sessions);
        DataContext = _viewModel;
        _terminalHtmlPath = Path.Combine(AppContext.BaseDirectory, "Terminal", "terminal.html");

        Loaded += (_, _) => Dispatcher.BeginInvoke(new Action(FirstUnlock));
        Closing += OnClosing;
        KeyDown += OnKeyDown;
        Sidebar.ContextMenu = BuildSidebarMenu();
    }

    // MARK: - Vault gate: master password on every launch, no "remember me".

    private void FirstUnlock()
    {
        if (PromptUnlock(firstRun: !_vault.Exists))
            _viewModel.SetUnlocked(true);
        else
            _viewModel.SetUnlocked(false);
        _viewModel.ReloadHosts();
        if (_viewModel.IsUnlocked)
        {
            _sessions.Restore(FindHost);
            foreach (var session in _sessions.Sessions)
                Subscribe(session);
            _viewModel.RefreshSidebar();
        }
    }

    /// <summary>Returns true when the vault is unlocked afterwards.</summary>
    private bool PromptUnlock(bool firstRun)
    {
        var dialog = new VaultUnlockWindow(firstRun);
        if (dialog.ShowDialog() != true)
            return false;
        try
        {
            _vault.Unlock(dialog.MasterPassword);
            return true;
        }
        catch (VaultException ex)
        {
            MessageBox.Show(ex.Message, "Cannot unlock vault",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return false;
        }
    }

    private Host? FindHost(Guid id) => _viewModel.Hosts.FirstOrDefault(h => h.Id == id);

    private void OnUnlock(object sender, RoutedEventArgs e)
    {
        if (_viewModel.IsUnlocked)
            return;
        if (PromptUnlock(firstRun: false))
        {
            _viewModel.SetUnlocked(true);
            _viewModel.ReloadHosts();
            _sessions.Restore(FindHost);
            foreach (var session in _sessions.Sessions)
                Subscribe(session);
            _viewModel.RefreshSidebar();
        }
    }

    private void OnLock(object sender, RoutedEventArgs e)
    {
        _sessions.Checkpoint();
        try { _vault.Lock(); } catch { }
        _viewModel.SetUnlocked(false);
        _viewModel.ReloadHosts();
    }

    private void OnClosing(object? sender, CancelEventArgs e)
    {
        // Quit only stops processes and keeps records.
        _sessions.Dispose();
        try { _vault.Lock(); } catch { }
    }

    private void OnShowVault(object sender, RoutedEventArgs e)
    {
        try
        {
            System.Diagnostics.Process.Start(new System.Diagnostics.ProcessStartInfo
            {
                FileName = Path.GetDirectoryName(VaultPaths.VaultFile),
                UseShellExecute = true,
            });
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot show vault",
                MessageBoxButton.OK, MessageBoxImage.Error);
        }
    }

    // MARK: - Hosts

    private void OnNewHost(object sender, RoutedEventArgs e)
    {
        if (!_viewModel.IsUnlocked)
        {
            MessageBox.Show("Unlock the password vault first.",
                "Vault is locked", MessageBoxButton.OK, MessageBoxImage.Information);
            return;
        }
        var dialog = new HostEditorWindow(Host.Blank(), password: "", configPath: SshConfigFile.DefaultPath);
        if (dialog.ShowDialog() != true)
            return;
        try
        {
            _vault.SaveHost(dialog.Host, dialog.Password);
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot save host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        _viewModel.ReloadHosts();
    }

    private void EditHost(Host host)
    {
        string existing;
        try { existing = _vault.GetPassword(host.Id) ?? ""; }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot edit host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        var dialog = new HostEditorWindow(host, existing, SshConfigFile.DefaultPath);
        if (dialog.ShowDialog() != true)
            return;
        try
        {
            _vault.SaveHost(dialog.Host, dialog.Password);
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot save host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        _viewModel.ReloadHosts();
    }

    private void DeleteHost(Host host)
    {
        var answer = MessageBox.Show(
            $"Delete \"{host.DisplayName}\" and close all of its sessions?",
            "Delete host", MessageBoxButton.YesNo, MessageBoxImage.Warning);
        if (answer != MessageBoxResult.Yes)
            return;
        try
        {
            // Deleting a saved host closes all of its sessions; the vault
            // write removes hosts, passwords and session records atomically.
            _vault.DeleteHost(host.Id);
        }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot delete host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        _sessions.CloseAllForHost(host.Id);
        _viewModel.ReloadHosts();
    }

    private void DuplicateHost(Host host)
    {
        var copy = new Host(Guid.NewGuid(), host.Name + " (copy)",
            host.Hostname, host.Port, host.Username);
        string password;
        try { password = _vault.GetPassword(host.Id) ?? ""; }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot duplicate host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        if (string.IsNullOrEmpty(password))
        {
            MessageBox.Show("Duplicating also copies the saved password, which requires an unlocked vault.",
                "Cannot duplicate host", MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        try { _vault.SaveHost(copy, password); }
        catch (Exception ex)
        {
            MessageBox.Show(ex.Message, "Cannot duplicate host",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        _viewModel.ReloadHosts();
    }

    // MARK: - Sidebar

    private void OnSearchChanged(object sender, TextChangedEventArgs e)
    {
        var view = CollectionViewSource.GetDefaultView(Sidebar.ItemsSource);
        string query = SearchBox.Text.Trim();
        view.Filter = string.IsNullOrEmpty(query)
            ? null
            : item => item is SidebarItem row &&
                (row.Title.Contains(query, StringComparison.OrdinalIgnoreCase) ||
                 row.Detail.Contains(query, StringComparison.OrdinalIgnoreCase));
    }

    private void OnSelectionChanged(object sender, SelectionChangedEventArgs e)
    {
        if (_viewModel.SelectedItem is null)
            return;
        if (_viewModel.SelectedItem.IsSession)
        {
            var session = _sessions.Session(_viewModel.SelectedItem.Id);
            if (session is not null)
                ShowSession(session);
        }
        else
        {
            var host = FindHost(_viewModel.SelectedItem.HostId);
            if (host is not null)
            {
                // Single click connects: reveal the existing connection, if any.
                var session = _sessions.OpenOrReveal(host);
                ShowSession(session);
                SelectSidebarRow(session.Id);
            }
        }
    }

    private void OnSidebarDoubleClick(object sender, MouseButtonEventArgs e)
    {
        if (_viewModel.SelectedItem is { IsSession: true } item &&
            _sessions.Session(item.Id) is SshSession session)
            RenameSession(session);
    }

    private ContextMenu BuildSidebarMenu()
    {
        var menu = new ContextMenu();
        menu.Opened += (_, _) =>
        {
            menu.Items.Clear();
            var item = (SidebarItem?)Sidebar.SelectedItem;
            if (item is null)
                return;
            if (item.IsSession)
            {
                menu.Items.Add(MenuEntry("Reconnect", (_, _) => ReconnectSelected()));
                menu.Items.Add(MenuEntry("Rename…", (_, _) =>
                {
                    if (_sessions.Session(item.Id) is SshSession s)
                        RenameSession(s);
                }));
                menu.Items.Add(MenuEntry("Close Session", (_, _) =>
                {
                    if (_sessions.Session(item.Id) is SshSession s)
                        CloseSession(s);
                }));
            }
            else if (FindHost(item.HostId) is Host host)
            {
                menu.Items.Add(MenuEntry("New Session", (_, _) =>
                {
                    var created = _sessions.OpenNew(host);
                    ShowSession(created);
                    SelectSidebarRow(created.Id);
                }));
                menu.Items.Add(new Separator());
                menu.Items.Add(MenuEntry("Edit…", (_, _) => EditHost(host)));
                menu.Items.Add(MenuEntry("Duplicate", (_, _) => DuplicateHost(host)));
                menu.Items.Add(MenuEntry("Delete", (_, _) => DeleteHost(host)));
            }
        };
        return menu;
    }

    private static MenuItem MenuEntry(string header, RoutedEventHandler onClick)
    {
        var entry = new MenuItem { Header = header };
        entry.Click += onClick;
        return entry;
    }

    private void RenameSession(SshSession session)
    {
        var dialog = new PromptDialog("Rename session", "Name:", session.Name);
        if (dialog.ShowDialog() != true || string.IsNullOrWhiteSpace(dialog.Answer))
            return;
        _sessions.Rename(session, dialog.Answer.Trim());
    }

    // MARK: - Sessions

    private void OnNewSession(object sender, RoutedEventArgs e)
    {
        if (_viewModel.SelectedHost() is Host host)
        {
            var session = _sessions.OpenNew(host);
            ShowSession(session);
            SelectSidebarRow(session.Id);
        }
    }

    private void SelectSidebarRow(Guid sessionId)
    {
        _viewModel.RefreshSidebar();
        _viewModel.SelectedItem = _viewModel.SidebarItems.FirstOrDefault(i => i.Id == sessionId);
    }

    private void OnReconnect(object sender, RoutedEventArgs e) => ReconnectSelected();

    private void ReconnectSelected()
    {
        if (_viewModel.SelectedSession is SshSession session &&
            FindHost(session.HostId) is Host host)
        {
            _sessions.Reconnect(session, host);
            // Reconnect starts from a clean screen.
            if (_terminals.TryGetValue(session.Id, out var view) && view.CoreWebView2 is not null)
            {
                try { view.CoreWebView2.PostWebMessageAsJson(Message("reset", "")); }
                catch { }
            }
            ShowSession(session);
        }
    }

    private void OnCloseSession(object sender, RoutedEventArgs e)
    {
        if (_viewModel.SelectedSession is SshSession session)
            CloseSession(session);
    }

    private void CloseSession(SshSession session)
    {
        string? error = _sessions.Close(session, explicitClose: true);
        if (error is not null)
        {
            // A failed delete keeps the session visible and reports an error.
            MessageBox.Show(error, "Cannot close session",
                MessageBoxButton.OK, MessageBoxImage.Error);
            return;
        }
        if (_terminals.Remove(session.Id, out var view))
            TerminalHost.Children.Remove(view);
        _viewModel.RefreshSidebar();
    }

    private void Subscribe(SshSession session)
    {
        if (!_subscribed.Add(session.Id))
            return;
        session.Changed += () => Dispatcher.BeginInvoke(new Action(
            () => _viewModel.RefreshSidebar()));
        session.Output += chunk => Dispatcher.BeginInvoke(new Action(() =>
        {
            try
            {
                if (_terminals.TryGetValue(session.Id, out var view) && view.CoreWebView2 is not null)
                    view.CoreWebView2.PostWebMessageAsJson(Message("output", Convert.ToBase64String(chunk)));
            }
            catch { /* terminal not ready yet; scrollback restore covers it */ }
        }));
    }

    private void ShowSession(SshSession session)
    {
        Subscribe(session);
        if (!_terminals.TryGetValue(session.Id, out var view))
        {
            view = new WebView2();
            _terminals[session.Id] = view;
            TerminalHost.Children.Add(view);
            view.WebMessageReceived += (_, args) => OnTerminalMessage(session, args.WebMessageAsJson);
            view.NavigationCompleted += (_, _) =>
            {
                // Restored output is scrollback, never replayed as commands.
                string history;
                lock (session) history = session.History.ToString();
                if (history.Length > 0)
                    view.CoreWebView2?.PostWebMessageAsJson(Message("restore",
                        Convert.ToBase64String(Encoding.UTF8.GetBytes(
                            SshSessionManager.StripForScrollback(history)))));
            };
            _ = view.EnsureCoreWebView2Async().ContinueWith(_ =>
                Dispatcher.BeginInvoke(new Action(() =>
                    view.Source = new Uri(_terminalHtmlPath))), TaskScheduler.Default);
        }
        foreach (var (id, terminal) in _terminals)
            terminal.Visibility = id == session.Id ? Visibility.Visible : Visibility.Collapsed;
        _viewModel.RefreshSidebar();
    }

    private void OnTerminalMessage(SshSession session, string json)
    {
        try
        {
            using var doc = JsonDocument.Parse(json);
            var root = doc.RootElement;
            string type = root.GetProperty("type").GetString() ?? "";
            switch (type)
            {
                case "input":
                    session.Process?.Write(Convert.FromBase64String(
                        root.GetProperty("data").GetString() ?? ""));
                    break;
                case "resize":
                    session.Process?.Resize(
                        (short)root.GetProperty("cols").GetInt32(),
                        (short)root.GetProperty("rows").GetInt32());
                    break;
            }
        }
        catch { /* a malformed terminal message is never fatal */ }
    }

    private static string Message(string type, string data) =>
        JsonSerializer.Serialize(new { type, data });

    private void OnKeyDown(object sender, KeyEventArgs e)
    {
        bool ctrl = Keyboard.Modifiers.HasFlag(ModifierKeys.Control);
        bool shift = Keyboard.Modifiers.HasFlag(ModifierKeys.Shift);
        if (ctrl && e.Key == Key.N) { OnNewHost(this, new RoutedEventArgs()); e.Handled = true; }
        else if (ctrl && e.Key == Key.F) { SearchBox.Focus(); e.Handled = true; }
        else if (ctrl && shift && e.Key == Key.R) { ReconnectSelected(); e.Handled = true; }
        else if (ctrl && shift && e.Key == Key.W)
        {
            if (_viewModel.SelectedSession is SshSession s)
                CloseSession(s);
            e.Handled = true;
        }
    }
}
