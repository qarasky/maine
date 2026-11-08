import Foundation
@testable import Maine
import Testing

/// Shell-integration marks (`OSC 133`) a remote shell may already emit.
@Suite
struct OSC133Tests {
    private func marks(_ text: String, split: Int? = nil) -> [OSC133Detector.Mark] {
        let bytes = Array(text.utf8)
        var detector = OSC133Detector()
        let cut = split ?? bytes.count
        return (detector.scan(ArraySlice(bytes[..<cut])) + detector.scan(ArraySlice(bytes[cut...]))).map(\.mark)
    }

    @Test
    func recognisesPromptOutputAndDoneMarks() {
        #expect(marks("\u{1B}]133;A\u{07}$ \u{1B}]133;B\u{07}") == [.promptStart, .promptEnd])
        #expect(marks("\u{1B}]133;C\u{07}hi\n\u{1B}]133;D;3\u{07}") == [.outputStart, .commandDone(3)])
        #expect(marks("\u{1B}]133;D\u{07}") == [.commandDone(nil)])
        #expect(marks("\u{1B}]133;D;0;aid=1\u{07}") == [.commandDone(0)])
    }

    @Test
    func stringTerminatorWorksToo() {
        #expect(marks("\u{1B}]133;D;7\u{1B}\\") == [.commandDone(7)])
    }

    @Test
    func marksSurviveAnyChunkSplit() {
        let text = "x\u{1B}]133;C\u{07}out\u{1B}]133;D;12\u{1B}\\y"
        for index in 0...text.utf8.count {
            #expect(marks(text, split: index) == [.outputStart, .commandDone(12)], "split at \(index)")
        }
    }

    @Test
    func ignoresOtherSequencesAndGarbage() {
        #expect(marks("\u{1B}]7770;ab;0\u{07}\u{1B}]133\u{07}\u{1B}]1337;File=x\u{07}").isEmpty)
        #expect(marks("\u{1B}]133;D;notanumber\u{07}") == [.commandDone(nil)])
        #expect(marks("\u{1B}]133;" + String(repeating: "9", count: 500) + "\u{07}").isEmpty)
        #expect(marks("\u{1B}]133;D;1\u{1B}[0m").isEmpty)
        #expect(marks("\u{1B}]133;D;4\u{07}") == [.commandDone(4)])
    }

    @Test
    func reportsWhereEachMarkEnds() {
        var detector = OSC133Detector()
        let bytes = Array("ab\u{1B}]133;D;1\u{07}cd".utf8)
        let found = detector.scan(ArraySlice(bytes))
        #expect(found.count == 1)
        #expect(String(decoding: bytes[found[0].end...], as: UTF8.self) == "cd")
    }
}
