import Maine
import Testing

@Suite
struct HostValidationTests {
    @Test
    func newHostDraftsHaveDistinctIDs() {
        #expect(Host.blank.id != Host.blank.id)
    }

    @Test
    func completeHostIsValid() {
        let host = Host(name: "Web", hostname: "example.com", port: 2222, username: "deploy")
        #expect(host.isValid)
        #expect(host.issues == [])
    }

    @Test
    func nameIsRequired() {
        #expect(Host(name: "   ", hostname: "example.com").isValid == false)
    }

    @Test
    func hostnameIsRequired() {
        #expect(Host(name: "Web", hostname: "").isValid == false)
    }

    @Test
    func hostnameCannotContainSpaces() {
        #expect(Host(name: "Web", hostname: "example com").isValid == false)
    }

    @Test
    func hostnameCannotLookLikeAnOption() {
        #expect(Host(name: "Web", hostname: "-oProxyCommand=...").isValid == false)
    }

    @Test
    func portMustBeInRange() {
        #expect(Host(name: "Web", hostname: "example.com", port: 0).isValid == false)
        #expect(Host(name: "Web", hostname: "example.com", port: 65536).isValid == false)
        #expect(Host(name: "Web", hostname: "example.com", port: 65535).isValid)
    }

    @Test
    func usernameCannotContainSpaces() {
        #expect(Host(name: "Web", hostname: "example.com", username: "two words").isValid == false)
    }

    @Test
    func blankUsernameIsAllowedSoSSHConfigCanDecide() {
        #expect(Host(name: "Web", hostname: "example.com", username: "").isValid)
    }

    @Test
    func subtitleShowsPortOnlyWhenItIsNotTheDefault() {
        #expect(Host(name: "Web", hostname: "example.com", port: 22, username: "me").subtitle == "me@example.com")
        #expect(Host(name: "Web", hostname: "example.com", port: 2222, username: "me").subtitle == "me@example.com:2222")
        #expect(Host(name: "Web", hostname: "example.com").subtitle == "example.com")
    }

    @Test
    func displayNameFallsBackToHostname() {
        #expect(Host(name: "  ", hostname: "example.com").displayName == "example.com")
    }

    @Test
    func hostCanBeBuiltFromAnSSHConfigEntry() {
        let entry = SSHConfigEntry(pattern: "homelab", hostname: "10.0.0.5", user: "admin", port: 2222)
        let host = Host(sshConfigEntry: entry)

        #expect(host?.name == "homelab")
        #expect(host?.hostname == "homelab")
        #expect(host?.username == "admin")
        #expect(host?.port == 2222)
    }

    @Test
    func hostsCannotBeBuiltFromWildcardPatterns() {
        #expect(Host(sshConfigEntry: SSHConfigEntry(pattern: "*.internal")) == nil)
        #expect(SSHConfigEntry.isConnectable("*") == false)
        #expect(SSHConfigEntry.isConnectable("two words") == false)
        #expect(SSHConfigEntry.isConnectable("homelab"))
    }
}
