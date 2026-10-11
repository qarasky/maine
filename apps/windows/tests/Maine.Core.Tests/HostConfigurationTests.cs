using Maine.Core;
using Xunit;

namespace Maine.Core.Tests;

public sealed class HostConfigurationTests
{
    private static readonly Host WebHost = new(
        Guid.Parse("0F8FAD5B-D9CB-469F-A165-70867728950E"),
        "Web server", "203.0.113.10", 22, "");

    [Fact]
    public void RoundTripKeepsEveryField()
    {
        var configuration = new HostConfiguration(new List<Host> { WebHost });
        var decoded = HostConfiguration.Decode(configuration.Encode());
        Assert.Equal(HostConfiguration.CurrentSchemaVersion, decoded.SchemaVersion);
        Assert.Equal(new List<Host> { WebHost }, decoded.Hosts);
    }

    [Fact]
    public void EncodedFileHasTheFieldsTheSchemaRequires()
    {
        using var doc = System.Text.Json.JsonDocument.Parse(
            new HostConfiguration(new List<Host> { WebHost }).Encode());
        var root = doc.RootElement;
        Assert.Equal(1, root.GetProperty("schemaVersion").GetInt32());
        var entry = root.GetProperty("hosts").EnumerateArray().First();
        foreach (var key in new[] { "id", "name", "hostname", "port", "username" })
            Assert.True(entry.TryGetProperty(key, out _), $"{key} is required by the schema");
        Assert.Equal(22, entry.GetProperty("port").GetInt32());
    }

    [Fact]
    public void MissingOptionalsFallBackToTheirDefaults()
    {
        var decoded = HostConfiguration.Decode(
            "{\"schemaVersion\":1,\"hosts\":[{\"id\":\"0F8FAD5B-D9CB-469F-A165-70867728950E\"," +
            "\"name\":\"Web\",\"hostname\":\"example.com\"}]}");
        Assert.Single(decoded.Hosts);
        Assert.Equal(Host.DefaultPort, decoded.Hosts[0].Port);
        Assert.Equal("", decoded.Hosts[0].Username);
    }

    [Fact]
    public void EmptyListIsValid()
    {
        var decoded = HostConfiguration.Decode(new HostConfiguration(new List<Host>()).Encode());
        Assert.Empty(decoded.Hosts);
    }

    [Fact]
    public void FutureSchemaVersionIsRefused()
    {
        var ex = Assert.Throws<VaultException>(
            () => HostConfiguration.Decode("{\"schemaVersion\":2,\"hosts\":[]}"));
        Assert.Equal(VaultErrorKind.InvalidFile, ex.Kind);
        Assert.Contains("version 2", ex.Message);
    }

    [Fact]
    public void WrongFieldTypeIsRefusedWithAReadableMessage()
    {
        var ex = Assert.Throws<VaultException>(() => HostConfiguration.Decode(
            "{\"schemaVersion\":1,\"hosts\":[{\"id\":\"0F8FAD5B-D9CB-469F-A165-70867728950E\"," +
            "\"name\":\"Web\",\"hostname\":\"example.com\",\"port\":\"22\"}]}"));
        Assert.Equal(VaultErrorKind.InvalidFile, ex.Kind);
        Assert.Contains("port", ex.Message);
    }

    [Fact]
    public void InvalidJsonIsRefused() =>
        Assert.Throws<VaultException>(() => HostConfiguration.Decode("not json"));
}
