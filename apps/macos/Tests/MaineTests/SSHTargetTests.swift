import Foundation
import Maine
#if canImport(Darwin)
import Darwin
#endif
import Testing

/// The rules that keep `~/.ssh/config` in control of the parts the user left
/// unset. These are what make existing keys, agents and ProxyJump work.
@Suite
struct SSHTargetTests {
    private let address = Host(name: "Web", hostname: "example.com")

    @Test
    func commandUsesTheSystemOpenSSHClient() {
        #expect(SSHTarget.executablePath == "/usr/bin/ssh")
    }

    @Test
    func defaultHostGetsNoOptions() {
        #expect(SSHTarget.make(for: address).arguments == ["example.com"])
    }

    @Test
    func nonDefaultPortIsPassed() {
        let host = Host(name: "Build", hostname: "example.com", port: 2222)
        #expect(SSHTarget.make(for: host).arguments == ["-p", "2222", "example.com"])
    }

    @Test
    func usernameIsPassedOnlyWhenSet() {
        let host = Host(name: "Web", hostname: "example.com", username: "deploy")
        #expect(SSHTarget.make(for: host).arguments == ["-l", "deploy", "example.com"])
    }

    @Test
    func bothOptionsArePassedInAConventionalOrder() {
        let host = Host(name: "Web", hostname: "example.com", port: 2222, username: "deploy")
        #expect(SSHTarget.make(for: host).arguments == ["-p", "2222", "-l", "deploy", "example.com"])
    }

    @Test
    func commandLineIsShownForCopying() {
        let host = Host(name: "Web", hostname: "example.com", port: 2222, username: "deploy")
        #expect(SSHTarget.make(for: host).commandLine == "/usr/bin/ssh -p 2222 -l deploy example.com")
    }

    @Test
    func resolutionFillsInTheLocalUserWhenNothingIsSet() {
        let resolution = SSHTarget.make(for: address).resolved(using: SSHConfigFile())

        #expect(resolution.hostname == "example.com")
        #expect(resolution.port == Host.defaultPort)
        #expect(resolution.username == NSUserName())
        #expect(resolution.configPattern == nil)
    }

    @Test
    func resolutionTakesValuesFromASshConfigStanza() {
        let config = SSHConfigFile(entries: [
            SSHConfigEntry(pattern: "example.com", hostname: "10.1.2.3", user: "svc", port: 2222)
        ])
        let resolution = SSHTarget.make(for: address).resolved(using: config)

        #expect(resolution.hostname == "10.1.2.3")
        #expect(resolution.username == "svc")
        #expect(resolution.port == 2222)
        #expect(resolution.configPattern == "example.com")
        #expect(resolution.target == "svc@10.1.2.3:2222")
    }

    @Test
    func explicitPortWinsOverTheConfig() {
        let config = SSHConfigFile(entries: [SSHConfigEntry(pattern: "example.com", port: 2222)])
        let host = Host(name: "Web", hostname: "example.com", port: 2200)
        let resolution = SSHTarget.make(for: host).resolved(using: config)

        #expect(resolution.port == 2200)
    }

    @Test
    func defaultPortLetsTheConfigDecide() {
        let config = SSHConfigFile(entries: [SSHConfigEntry(pattern: "example.com", port: 2222)])
        let resolution = SSHTarget.make(for: address).resolved(using: config)

        #expect(resolution.port == 2222)
    }

    @Test
    func explicitUsernameWinsOverTheConfig() {
        let config = SSHConfigFile(entries: [SSHConfigEntry(pattern: "example.com", user: "svc")])
        let host = Host(name: "Web", hostname: "example.com", username: "deploy")
        let resolution = SSHTarget.make(for: host).resolved(using: config)

        #expect(resolution.username == "deploy")
    }
}

@Suite
struct TerminalEnvironmentTests {
    private static var variables: [String: String] {
        TerminalEnvironment.variables.reduce(into: [:]) { values, line in
            // Parse by hand: `split` drops a trailing empty piece, so an
            // inherited variable with an empty value (KEY=) would crash it.
            guard let separator = line.firstIndex(of: "=") else { return }
            values[String(line[..<separator])] = String(line[line.index(after: separator)...])
        }
    }

    @Test
    func terminalVariablesAreSet() {
        let values = Self.variables

        #expect(values["TERM"] == "xterm-256color")
        #expect(values["COLORTERM"] == "truecolor")
        #expect(values.keys.contains("LC_ALL") || values.keys.contains("LANG"))
    }

    @Test
    func parentEnvironmentIsInherited() {
        // Replacing the environment would break ssh-agent, PATH, proxies and
        // locale, so the child keeps whatever the user's shell had.
        let values = Self.variables
        let parent = ProcessInfo.processInfo.environment

        for key in ["PATH", "HOME", "SSH_AUTH_SOCK", "https_proxy"] {
            if let expected = parent[key] {
                #expect(values[key] == expected)
            }
        }
    }

    @Test
    func termOverrideWinsOverAnInheritedValue() {
        setenv("TERM", "vt100", 1)
        defer { unsetenv("TERM") }

        #expect(Self.variables["TERM"] == "xterm-256color")
    }
}
