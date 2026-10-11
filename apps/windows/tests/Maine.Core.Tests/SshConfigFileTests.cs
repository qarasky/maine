using Maine.Core;
using Xunit;

namespace Maine.Core.Tests;

public sealed class SshConfigFileTests : IDisposable
{
    private readonly string _directory =
        Path.Combine(Path.GetTempPath(), $"maine-config-tests-{Guid.NewGuid():D}");

    public SshConfigFileTests() => Directory.CreateDirectory(_directory);
    public void Dispose()
    {
        try { Directory.Delete(_directory, recursive: true); } catch { }
    }

    private string Write(string name, string contents)
    {
        var path = Path.Combine(_directory, name);
        File.WriteAllText(path, contents);
        return path;
    }

    [Fact]
    public void MissingFileIsNotAnError()
    {
        var config = SshConfigFile.Load(Path.Combine(_directory, "nonexistent", "config"));
        Assert.Empty(config.Entries);
        Assert.Null(config.EntryFor("anything"));
    }

    [Fact]
    public void ReadsHostNameUserAndPort()
    {
        var main = Write("config", "# my hosts\nHost web\n    HostName example.com\n    User deploy\n    Port 2222\n\nHost db\n    HostName db.example.com\n");
        var config = SshConfigFile.Load(main);
        Assert.Equal(2, config.Entries.Count);
        var web = config.EntryFor("web");
        Assert.NotNull(web);
        Assert.Equal("example.com", web.Hostname);
        Assert.Equal("deploy", web.User);
        Assert.Equal(2222, web.Port);
        var db = config.EntryFor("db");
        Assert.NotNull(db);
        Assert.Equal("db.example.com", db.Hostname);
    }

    [Fact]
    public void IgnoresDirectivesMaineDoesNotModel()
    {
        var main = Write("config", "Host withkeys\n    IdentityFile ~/.ssh/id_ed25519\n    ProxyJump bastion\n");
        var entry = SshConfigFile.Load(main).EntryFor("withkeys");
        Assert.NotNull(entry);
        // Maine records nothing here on purpose: OpenSSH applies these itself.
        Assert.Null(entry.Hostname);
        Assert.Null(entry.User);
        Assert.Null(entry.Port);
    }

    [Fact]
    public void IgnoresWildcardEntriesForImporting()
    {
        var main = Write("config", "Host *.internal example.com\n    User me\n\nHost *\n    Compression yes\n");
        var entries = SshConfigFile.Load(main).Entries;
        // Three patterns on two Host lines, but only one names a single host.
        Assert.Equal(3, entries.Count);
        Assert.Equal(new[] { "example.com" },
            entries.Where(e => SshConfigEntry.IsConnectable(e.Pattern)).Select(e => e.Pattern));
    }

    [Fact]
    public void DirectivesAreCaseInsensitiveAndCommentsAreDropped()
    {
        var main = Write("config", "# leading comment\nhost build # trailing comment\nHOSTNAME build.example.com\nUsER builder\nport 22\n");
        var entry = SshConfigFile.Load(main).EntryFor("build");
        Assert.NotNull(entry);
        Assert.Equal("build.example.com", entry.Hostname);
        Assert.Equal("builder", entry.User);
        Assert.Equal(22, entry.Port);
    }

    [Fact]
    public void KeywordEqualsValueIsUnderstood()
    {
        var main = Write("config", "Host eq\nHostName=eq.example.com\nPort=2022\n");
        var entry = SshConfigFile.Load(main).EntryFor("eq");
        Assert.NotNull(entry);
        Assert.Equal("eq.example.com", entry.Hostname);
        Assert.Equal(2022, entry.Port);
    }

    [Fact]
    public void FirstValueWinsForARepeatedKeyword()
    {
        var main = Write("config", "Host repeat\n    User first\n    User second\n");
        var repeat = SshConfigFile.Load(main).EntryFor("repeat");
        Assert.NotNull(repeat);
        Assert.Equal("first", repeat.User);
    }

    [Fact]
    public void ALaterBlockCanSupplyAValueAnEarlierOneLeftOut()
    {
        var main = Write("config", "Host split\n    HostName split.example.com\n\nHost split\n    User later\n");
        var entry = SshConfigFile.Load(main).EntryFor("split");
        Assert.NotNull(entry);
        Assert.Equal("split.example.com", entry.Hostname);
        Assert.Equal("later", entry.User);
    }

    [Fact]
    public void IncludeBringsInOtherFiles()
    {
        Write("included.conf", "Host extra\n    HostName extra.example.com\n");
        var main = Write("config", "Include included.conf\nHost main\n    HostName main.example.com\n");
        var config = SshConfigFile.Load(main);
        var mainEntry = config.EntryFor("main");
        Assert.NotNull(mainEntry);
        Assert.Equal("main.example.com", mainEntry.Hostname);
        var extra = config.EntryFor("extra");
        Assert.NotNull(extra);
        Assert.Equal("extra.example.com", extra.Hostname);
    }

    [Fact]
    public void RealConfigurationFileLoadsWithoutCrashing()
    {
        // Reading the user's own file must always be safe, whatever it holds.
        _ = SshConfigFile.Load();
    }
}
