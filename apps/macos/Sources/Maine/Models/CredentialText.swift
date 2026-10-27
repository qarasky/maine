import Foundation

/// The plain-text credential block copied for a host: one labelled field per
/// line, and only the fields that exist.
///
/// ```
/// IP: 10.0.0.4
/// port: 2222          (only when the port is not 22)
/// username: root
/// password: hunter2   (only when a saved password was unlocked)
/// ```
public enum CredentialText {
    public static func make(host: Host, password: String?) -> String {
        var lines = ["IP: \(host.hostname)"]
        if host.port != Host.defaultPort {
            lines.append("port: \(host.port)")
        }
        if !host.username.isEmpty {
            lines.append("username: \(host.username)")
        }
        if let password, !password.isEmpty {
            lines.append("password: \(password)")
        }
        return lines.joined(separator: "\n")
    }
}
