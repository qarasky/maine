import AppKit

/// OpenSSH invokes this executable again to request authentication. Only the
/// host ID travels in the environment; the helper decrypts the portable vault.
enum SSHAskPass {
    static func environment(for hostID: Host.ID, base: [String], executable: String) -> [String] {
        let overrides = [
            "SSH_ASKPASS": executable,
            "SSH_ASKPASS_REQUIRE": "force",
            "MAINE_SSH_ASKPASS": "1",
            "MAINE_SSH_HOST_ID": hostID.uuidString
        ]
        return base.filter { entry in
            let key = String(entry.prefix { $0 != "=" })
            return overrides[key] == nil
        } + overrides.map { "\($0.key)=\($0.value)" }
    }

    static func isPasswordPrompt(_ prompt: String) -> Bool {
        let lower = prompt.lowercased()
        return lower.contains("password:") && !lower.contains("passphrase")
            && !lower.contains("one-time") && !lower.contains("verification")
    }

    @MainActor
    static func run() -> Int32 {
        guard let rawID = ProcessInfo.processInfo.environment["MAINE_SSH_HOST_ID"],
              let hostID = UUID(uuidString: rawID), let prompt = CommandLine.arguments.dropFirst().first else {
            return 1
        }
        do {
            if isPasswordPrompt(prompt), let password = try PasswordStore.shared.password(for: hostID) {
                print(password)
                return 0
            }
        } catch {
            return 1
        }

        // Never reuse a login password for key passphrases, MFA, or host trust.
        // Unknown-host approval remains an explicit user decision.
        let app = NSApplication.shared
        app.setActivationPolicy(.accessory)
        app.activate(ignoringOtherApps: true)
        let alert = NSAlert()
        alert.messageText = "SSH authentication"
        alert.informativeText = prompt
        let confirmation = ProcessInfo.processInfo.environment["SSH_ASKPASS_PROMPT"] == "confirm"
            || prompt.contains("yes/no")
        alert.addButton(withTitle: confirmation ? "Trust Host" : "Continue")
        alert.addButton(withTitle: "Cancel")
        let input = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 320, height: 24))
        if !confirmation {
            alert.accessoryView = input
            alert.window.initialFirstResponder = input
        }
        guard alert.runModal() == .alertFirstButtonReturn else { return 1 }
        print(confirmation ? "yes" : input.stringValue)
        return 0
    }
}
