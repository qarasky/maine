import Foundation
import Security

/// The text typed for an agent's `run`: the command in a `{ … }` group, then
/// a `printf` that emits `ESC ] 7770 ; <nonce> ; <exit> BEL`. The terminal
/// ignores that OSC; the detector finds it in the raw PTY bytes. The echoed
/// line contains the characters `\033`, never an ESC byte, so the echo can't
/// match, and a remote program can't guess the per-command nonce.
struct CommandWrapper: Sendable {
    let nonce: String
    let typedText: String

    init(command: String, nonce: String = CommandWrapper.makeNonce()) {
        self.nonce = nonce
        typedText = "{ \(command)\n}; printf '\\033]7770;\(nonce);%d\\007' \"$?\"\n"
    }

    /// 16 random bytes as lowercase hex.
    static func makeNonce() -> String {
        var bytes = [UInt8](repeating: 0, count: 16)
        let status = SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes)
        precondition(status == errSecSuccess, "SecRandomCopyBytes failed")
        return bytes.map { String(format: "%02x", $0) }.joined()
    }
}

/// Streams raw PTY bytes and reports the exit code once, when the marker for
/// its nonce has fully arrived. The marker may be split across chunks.
struct CompletionDetector: Sendable {
    private let prefix: [UInt8]
    private var window: [UInt8] = []
    private var finished = false

    init(nonce: String) {
        prefix = Array("\u{1B}]7770;\(nonce);".utf8)
    }

    mutating func scan(_ bytes: ArraySlice<UInt8>) -> Int32? {
        scanForEnd(bytes)?.code
    }

    /// Like `scan`, also returning the index just past the marker's BEL, so
    /// output after the marker (the next prompt) can be told apart.
    mutating func scanForEnd(_ bytes: ArraySlice<UInt8>) -> (code: Int32, end: ArraySlice<UInt8>.Index)? {
        guard !finished else { return nil }
        let limit = prefix.count + 12
        for index in bytes.indices {
            let byte = bytes[index]
            window.append(byte)
            if window.count > limit { window.removeFirst(window.count - limit) }
            guard byte == 0x07, let code = match() else { continue }
            finished = true
            return (code, bytes.index(after: index))
        }
        return nil
    }

    /// The window ends in BEL: does it end in `prefix digits BEL`?
    private func match() -> Int32? {
        var digits: [UInt8] = []
        var index = window.count - 2
        while index >= 0, (0x30...0x39).contains(window[index]) {
            digits.insert(window[index], at: 0)
            index -= 1
        }
        guard !digits.isEmpty, digits.count <= 10, index + 1 >= prefix.count,
              Array(window[(index + 1 - prefix.count)...index]) == prefix else { return nil }
        return Int32(String(decoding: digits, as: UTF8.self))
    }
}

enum PromptHeuristics {
    /// A line that ends like an interactive shell prompt (`$`, `#`, `%`, `>`),
    /// used only by the quiet-period fallback for shells without the marker.
    static func looksLikeShellPrompt(_ line: String) -> Bool {
        let trimmed = line.trimmingCharacters(in: .whitespaces)
        guard let last = trimmed.last, "$#%>".contains(last), trimmed.count > 1 else { return false }
        if last == "%", let previous = trimmed.dropLast().last, previous.isNumber { return false }
        return true
    }
}

/// Finds the shell-integration marks (`OSC 133`) a remote shell may already
/// emit in the raw PTY bytes: `A` prompt start, `B` prompt end, `C` command
/// output start, `D[;exit]` command done. Either BEL or ST ends a mark, and a
/// mark may be split across chunks. Unlike Maine's own marker these carry no
/// nonce, so a program's output could fake one; that can only end a `run`
/// early, never reach a host or a secret.
struct OSC133Detector: Sendable {
    enum Mark: Equatable, Sendable {
        case promptStart, promptEnd, outputStart
        case commandDone(Int32?)
    }

    private static let prefix: [UInt8] = [0x1B, 0x5D, 0x31, 0x33, 0x33, 0x3B] // ESC ] 1 3 3 ;
    private static let bodyLimit = 32

    private var matched = 0
    private var body: [UInt8] = []
    private var inBody = false
    private var sawEscape = false

    mutating func scan(_ bytes: ArraySlice<UInt8>) -> [(mark: Mark, end: ArraySlice<UInt8>.Index)] {
        var found: [(mark: Mark, end: ArraySlice<UInt8>.Index)] = []
        for index in bytes.indices {
            let byte = bytes[index]
            guard inBody else {
                if byte == Self.prefix[matched] {
                    matched += 1
                    if matched == Self.prefix.count { inBody = true; matched = 0; body = []; sawEscape = false }
                } else {
                    matched = byte == Self.prefix[0] ? 1 : 0
                }
                continue
            }
            if sawEscape {
                if byte == 0x5C, let mark = Self.parse(body) { found.append((mark, bytes.index(after: index))) }
                reset(retryingEscape: byte == 0x1B)
            } else if byte == 0x07 {
                if let mark = Self.parse(body) { found.append((mark, bytes.index(after: index))) }
                reset()
            } else if byte == 0x1B {
                sawEscape = true
            } else if body.count >= Self.bodyLimit {
                reset()
            } else {
                body.append(byte)
            }
        }
        return found
    }

    private mutating func reset(retryingEscape: Bool = false) {
        inBody = false
        sawEscape = false
        body = []
        matched = retryingEscape ? 1 : 0
    }

    private static func parse(_ body: [UInt8]) -> Mark? {
        let parts = String(decoding: body, as: UTF8.self).split(separator: ";", omittingEmptySubsequences: false)
        switch parts.first {
        case "A": return .promptStart
        case "B": return .promptEnd
        case "C": return .outputStart
        case "D": return .commandDone(parts.count > 1 ? Int32(parts[1]) : nil)
        default: return nil
        }
    }
}
