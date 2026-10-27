import Foundation
@testable import Maine
import Testing

/// Knowing an agent's command finished, and recognising prompts.
@Suite
struct CommandCompletionTests {
    @Test
    func wrapperTextIsExact() {
        let wrapper = CommandWrapper(command: "ls -la", nonce: "ab12")
        #expect(wrapper.typedText == "{ ls -la\n}; printf '\\033]7770;ab12;%d\\007' \"$?\"\n")
        #expect(wrapper.nonce == "ab12")
    }

    @Test
    func markerIsFoundAcrossChunkBoundaries() {
        let marker = Array("out\u{1B}]7770;ab12;3\u{07}more".utf8)
        for index in 0...marker.count {
            var detector = CompletionDetector(nonce: "ab12")
            let first = detector.scan(ArraySlice(marker[0..<index]))
            let second = detector.scan(ArraySlice(marker[index...]))
            #expect([first, second].compactMap { $0 } == [3], "split at \(index)")
            #expect(detector.scan(ArraySlice(marker)) == nil, "latched after the first match")
        }
    }

    @Test
    func echoedWrapperAndWrongNoncesNeverMatch() {
        var detector = CompletionDetector(nonce: "ab12")
        #expect(detector.scan(ArraySlice(Array(CommandWrapper(command: "true", nonce: "ab12").typedText.utf8))) == nil)
        #expect(detector.scan(ArraySlice(Array("\u{1B}]7770;ffff;0\u{07}".utf8))) == nil)
        #expect(detector.scan(ArraySlice(Array("\u{1B}]7770;ab12;x\u{07}".utf8))) == nil)
        #expect(detector.scan(ArraySlice(Array("\u{1B}]7770;ab12;127\u{07}".utf8))) == 127)
    }

    @Test
    func nonceIsRandomHex() {
        let first = CommandWrapper.makeNonce(), second = CommandWrapper.makeNonce()
        #expect(first.count == 32)
        #expect(first.allSatisfy { "0123456789abcdef".contains($0) })
        #expect(first != second)
    }

    @Test
    func passwordPromptsIncludeSudo() {
        for prompt in ["[sudo] password for alice: ", "Password:", "Password for alice@web: "] {
            #expect(SSHAskPass.isPasswordPrompt(prompt), "\(prompt)")
        }
        for prompt in ["Enter passphrase for key '/x':", "One-time password:", "Verification code:"] {
            #expect(!SSHAskPass.isPasswordPrompt(prompt), "\(prompt)")
        }
    }

    @Test
    func shellPromptHeuristic() {
        for line in ["alice@web:~$ ", "root@web:/# ", "web% ", "fish> "] {
            #expect(PromptHeuristics.looksLikeShellPrompt(line), "\(line)")
        }
        for line in ["Downloading 40%", "", "done"] {
            #expect(!PromptHeuristics.looksLikeShellPrompt(line), "\(line)")
        }
    }
}
