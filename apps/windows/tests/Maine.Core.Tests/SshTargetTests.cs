using Maine.Core;
using Xunit;

namespace Maine.Core.Tests;

public sealed class SshTargetTests
{
    private static Host Address() => new(Guid.NewGuid(), "Web", "example.com");

    [Fact]
    public void DefaultHostGetsNoOptions() =>
        Assert.Equal(new List<string> { "example.com" }, SshTarget.Make(Address()).Arguments);

    [Fact]
    public void NonDefaultPortIsPassed()
    {
        var host = new Host(Guid.NewGuid(), "Build", "example.com", 2222);
        Assert.Equal(new List<string> { "-p", "2222", "example.com" },
            SshTarget.Make(host, "ssh").Arguments);
    }

    [Fact]
    public void UsernameIsPassedOnlyWhenSet()
    {
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 22, "deploy");
        Assert.Equal(new List<string> { "-l", "deploy", "example.com" },
            SshTarget.Make(host, "ssh").Arguments);
    }

    [Fact]
    public void BothOptionsArePassedInAConventionalOrder()
    {
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 2222, "deploy");
        Assert.Equal(new List<string> { "-p", "2222", "-l", "deploy", "example.com" },
            SshTarget.Make(host, "ssh").Arguments);
    }

    [Fact]
    public void CommandLineIsShownForCopying()
    {
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 2222, "deploy");
        Assert.Equal("/usr/bin/ssh -p 2222 -l deploy example.com",
            new SshTarget("/usr/bin/ssh", SshTarget.Make(host, "ssh").Arguments).CommandLine);
    }

    [Fact]
    public void ResolutionFillsInTheLocalUserWhenNothingIsSet()
    {
        var resolution = SshTarget.Make(Address(), "ssh").Resolved(new SshConfigFile(), "someuser");
        Assert.Equal("example.com", resolution.Hostname);
        Assert.Equal(Host.DefaultPort, resolution.Port);
        Assert.Equal("someuser", resolution.Username);
        Assert.Null(resolution.ConfigPattern);
    }

    [Fact]
    public void ResolutionTakesValuesFromASshConfigStanza()
    {
        var config = new SshConfigFile(new[]
            { new SshConfigEntry("example.com", "10.1.2.3", "svc", 2222) });
        var resolution = SshTarget.Make(Address(), "ssh").Resolved(config, "someuser");
        Assert.Equal("10.1.2.3", resolution.Hostname);
        Assert.Equal("svc", resolution.Username);
        Assert.Equal(2222, resolution.Port);
        Assert.Equal("example.com", resolution.ConfigPattern);
        Assert.Equal("svc@10.1.2.3:2222", resolution.Target);
    }

    [Fact]
    public void ExplicitPortWinsOverTheConfig()
    {
        var config = new SshConfigFile(new[] { new SshConfigEntry("example.com", port: 2222) });
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 2200);
        Assert.Equal(2200, SshTarget.Make(host, "ssh").Resolved(config, "someuser").Port);
    }

    [Fact]
    public void DefaultPortLetsTheConfigDecide()
    {
        var config = new SshConfigFile(new[] { new SshConfigEntry("example.com", port: 2222) });
        Assert.Equal(2222, SshTarget.Make(Address(), "ssh").Resolved(config, "someuser").Port);
    }

    [Fact]
    public void ExplicitUsernameWinsOverTheConfig()
    {
        var config = new SshConfigFile(new[] { new SshConfigEntry("example.com", user: "svc") });
        var host = new Host(Guid.NewGuid(), "Web", "example.com", 22, "deploy");
        Assert.Equal("deploy", SshTarget.Make(host, "ssh").Resolved(config, "someuser").Username);
    }
}

public sealed class TerminalEnvironmentTests
{
    [Fact]
    public void TerminalVariablesAreSet()
    {
        var values = TerminalEnvironment.Variables();
        Assert.Equal("xterm-256color", values["TERM"]);
        Assert.Equal("truecolor", values["COLORTERM"]);
        Assert.True(values.ContainsKey("LANG"));
    }

    [Fact]
    public void ParentEnvironmentIsInherited()
    {
        var values = TerminalEnvironment.Variables();
        foreach (var key in new[] { "PATH", "HOME", "SSH_AUTH_SOCK" })
        {
            var expected = Environment.GetEnvironmentVariable(key);
            if (expected is not null)
                Assert.Equal(expected, values[key]);
        }
    }
}
