import Foundation
@testable import Maine
import Testing

/// Which mode the executable starts in. The askpass environment wins: its
/// argument is a prompt the remote server chose, which may be anything.
@Suite
struct EntryPointTests {
    @Test
    func askpassWinsOverAnArgumentThatLooksLikeTheMCPFlag() {
        #expect(MaineEntryPoint.mode(arguments: ["Maine", "--mcp"], environment: ["MAINE_SSH_ASKPASS": "1"]) == .askPass)
        #expect(MaineEntryPoint.mode(arguments: ["Maine", "--mcp"], environment: [:]) == .mcpBridge)
        #expect(MaineEntryPoint.mode(arguments: ["Maine"], environment: [:]) == .app)
        #expect(MaineEntryPoint.mode(arguments: ["Maine", "--debug"], environment: [:]) == .app)
    }
}
