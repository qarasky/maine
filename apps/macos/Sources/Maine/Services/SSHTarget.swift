import Foundation

/// The exact `ssh` command Maine runs for a host.
///
/// Maine adds as little as possible to the invocation so that everything the
/// user already set up keeps working:
///
/// * keys in `~/.ssh`, `ssh-agent`, `ProxyJump`, `ForwardAgent`, `Match` and every
///   other directive are applied by OpenSSH itself;
/// * `-l <user>` is only passed when the host has a username;
/// * `-p <port>` is only passed when the port is not 22, so a `~/.ssh/config`
///   stanza that matches a `Host` alias can still choose the real port.
///
/// Maine never passes a password, and never reads a private key.
public struct SSHTarget: Sendable, Equatable {
    /// The system OpenSSH client. Maine never ships its own.
    public static let executablePath = "/usr/bin/ssh"

    public static var isSSHAvailable: Bool {
        FileManager.default.isExecutableFile(atPath: executablePath)
    }

    public var executable: String
    public var arguments: [String]

    public init(executable: String = SSHTarget.executablePath, arguments: [String]) {
        self.executable = executable
        self.arguments = arguments
    }

    /// The command line Maine runs, for display and for copying to a terminal.
    public var commandLine: String {
        ([executable as String] + arguments).joined(separator: " ")
    }

    public static func make(for host: Host) -> SSHTarget {
        var arguments: [String] = []

        if host.port != Host.defaultPort {
            arguments += ["-p", String(host.port)]
        }
        if !host.username.isEmpty {
            arguments += ["-l", host.username]
        }

        arguments.append(host.hostname)
        return SSHTarget(executable: SSHTarget.executablePath, arguments: arguments)
    }

    // MARK: - What OpenSSH will actually connect to

    /// The connection as OpenSSH resolves it, after `~/.ssh/config` is applied.
    public struct Resolution: Sendable, Equatable {
        public var hostname: String
        public var username: String
        public var port: Int
        /// The `Host` stanza that contributed these values, when there is one.
        public var configPattern: String?

        public var target: String { "\(username)@\(hostname):\(port)" }

        public init(hostname: String, username: String, port: Int, configPattern: String? = nil) {
            self.hostname = hostname
            self.username = username
            self.port = port
            self.configPattern = configPattern
        }
    }

    /// Best effort resolution for display. Maine lets OpenSSH do the real work;
    /// this only explains what will happen, so it deliberately reports plain
    /// values when `~/.ssh/config` has nothing to say.
    public func resolved(using config: SSHConfigFile) -> Resolution {
        guard let address = arguments.last else {
            return Resolution(
                hostname: "",
                username: NSUserName(),
                port: Host.defaultPort
            )
        }

        let entry = config.entry(for: address)

        var username = NSUserName()
        if let user = firstValue(of: "-l") ?? entry?.user {
            username = user
        }

        var port = Host.defaultPort
        if let explicit = firstValue(of: "-p") {
            port = Int(explicit) ?? Host.defaultPort
        } else if let configured = entry?.port {
            port = configured
        }

        return Resolution(
            hostname: entry?.hostname ?? address,
            username: username,
            port: port,
            configPattern: entry?.pattern
        )
    }

    /// The value that follows `flag`, or nil when the flag is absent or truncated.
    private func firstValue(of flag: String) -> String? {
        guard let index = arguments.firstIndex(of: flag), index + 1 < arguments.count else {
            return nil
        }
        return arguments[index + 1]
    }
}

/// The environment the child `ssh` process runs with.
///
/// The parent environment is inherited — not replaced — so `SSH_AUTH_SOCK`
/// (agent), `PATH`, proxies and locale keep working — and only the terminal
/// related variables are set.
public enum TerminalEnvironment {
    public static let term = "xterm-256color"

    public static var variables: [String] {
        var environment = ProcessInfo.processInfo.environment
        environment["TERM"] = term
        environment["COLORTERM"] = "truecolor"
        if environment["LANG"]?.isEmpty != false {
            environment["LANG"] = "en_US.UTF-8"
        }
        return environment.map { "\($0.key)=\($0.value)" }
    }
}
