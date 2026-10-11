using Maine.Core;
using Xunit;

namespace Maine.Core.Tests;

public sealed class HostValidationTests
{
    [Fact]
    public void NewHostDraftsHaveDistinctIDs() =>
        Assert.NotEqual(Host.Blank().Id, Host.Blank().Id);

    [Fact]
    public void CompleteHostIsValid()
    {
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 2222, "deploy");
        Assert.True(host.IsValid);
        Assert.Empty(host.Issues);
    }

    [Fact]
    public void NameIsRequired() =>
        Assert.False(new Host(Guid.NewGuid(), "   ", "example.com").IsValid);

    [Fact]
    public void HostnameIsRequired() =>
        Assert.False(new Host(Guid.NewGuid(), "Web", "").IsValid);

    [Fact]
    public void HostnameCannotContainSpaces() =>
        Assert.False(new Host(Guid.NewGuid(), "Web", "example com").IsValid);

    [Fact]
    public void HostnameCannotLookLikeAnOption() =>
        Assert.False(new Host(Guid.NewGuid(), "Web", "-oProxyCommand=...").IsValid);

    [Theory]
    [InlineData(0, false)]
    [InlineData(65536, false)]
    [InlineData(65535, true)]
    public void PortMustBeInRange(int port, bool valid) =>
        Assert.Equal(valid, new Host(Guid.NewGuid(), "Web", "example.com", port).IsValid);

    [Fact]
    public void UsernameCannotContainSpaces() =>
        Assert.False(new Host(Guid.NewGuid(), "Web", "example.com", 22, "two words").IsValid);

    [Fact]
    public void BlankUsernameIsAllowedSoSshConfigCanDecide() =>
        Assert.True(new Host(Guid.NewGuid(), "Web", "example.com", 22, "").IsValid);

    [Fact]
    public void SubtitleShowsPortOnlyWhenItIsNotTheDefault()
    {
        Assert.Equal("me@example.com",
            new Host(Guid.NewGuid(), "Web", "example.com", 22, "me").Subtitle);
        Assert.Equal("me@example.com:2222",
            new Host(Guid.NewGuid(), "Web", "example.com", 2222, "me").Subtitle);
        Assert.Equal("example.com",
            new Host(Guid.NewGuid(), "Web", "example.com").Subtitle);
    }

    [Fact]
    public void DisplayNameFallsBackToHostname() =>
        Assert.Equal("example.com", new Host(Guid.NewGuid(), "  ", "example.com").DisplayName);

    [Fact]
    public void HostCanBeBuiltFromAnSshConfigEntry()
    {
        var host = Host.FromSshConfigEntry(
            new SshConfigEntry("homelab", "10.0.0.5", "admin", 2222));
        Assert.NotNull(host);
        Assert.Equal("homelab", host.Name);
        Assert.Equal("homelab", host.Hostname);
        Assert.Equal("admin", host.Username);
        Assert.Equal(2222, host.Port);
    }

    [Fact]
    public void HostsCannotBeBuiltFromWildcardPatterns()
    {
        Assert.Null(Host.FromSshConfigEntry(new SshConfigEntry("*.internal")));
        Assert.False(SshConfigEntry.IsConnectable("*"));
        Assert.False(SshConfigEntry.IsConnectable("two words"));
        Assert.True(SshConfigEntry.IsConnectable("homelab"));
    }
}
