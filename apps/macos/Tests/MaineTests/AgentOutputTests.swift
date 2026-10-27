import Foundation
@testable import Maine
import Testing

/// What agents read: PTY bytes turned into plain text, bounded and masked.
@Suite
struct AgentOutputTests {
    private func cleaned(_ chunks: [String], capacity: Int = 2 * 1024 * 1024) -> String {
        var output = AgentOutput(capacity: capacity)
        for chunk in chunks { output.append(ArraySlice(Array(chunk.utf8))) }
        return output.text(since: 0).text
    }

    @Test
    func escapeSequencesAreStripped() {
        #expect(cleaned(["\u{1B}[1;32mok\u{1B}[0m\r\n\u{1B}]0;title\u{07}done"]) == "ok\ndone")
        #expect(cleaned(["a\u{1B}P1;2|xyz\u{1B}\\b\u{1B}(Bc\n"]) == "abc\n")
    }

    @Test
    func carriageReturnAndBackspaceRewriteTheCurrentLine() {
        #expect(cleaned(["50%\r100%\n"]) == "100%\n")
        #expect(cleaned(["abc\u{08}\u{08}X\n"]) == "aXc\n")
        #expect(cleaned(["one\r\ntwo\r\n"]) == "one\ntwo\n")
    }

    @Test
    func utf8AndEscapesSplitAcrossChunksSurvive() {
        let bytes = Array("é✓\u{1B}[31mred".utf8)
        for index in 0...bytes.count {
            var output = AgentOutput()
            output.append(ArraySlice(bytes[0..<index]))
            output.append(ArraySlice(bytes[index...]))
            let text = output.text(since: 0).text
            #expect(text == "é✓red", "split at \(index)")
            #expect(!text.contains("\u{FFFD}"))
        }
    }

    @Test
    func cursorsAreMonotonicAndRingIsBounded() {
        var output = AgentOutput(capacity: 1_024)
        for _ in 0..<5_000 { output.append(ArraySlice(Array("x\n".utf8))) }
        #expect(output.cursor == 10_000)
        let all = output.text(since: 0)
        #expect(all.truncated)
        #expect(all.text.utf8.count <= 1_024)
        #expect(all.cursor == 10_000)
        let tail = output.text(since: 9_996)
        #expect(tail.text == "x\nx\n")
        #expect(!tail.truncated)
        #expect(output.text(since: 10_000).text.isEmpty)
    }

    @Test
    func theCurrentUnfinishedLineIsReadable() {
        var output = AgentOutput()
        output.append(ArraySlice(Array("[sudo] password for alice: ".utf8)))
        #expect(output.text(since: 0).text == "[sudo] password for alice: ")
        #expect(output.cursor == 27)
    }

    @Test
    func longResultsAreCutInTheMiddle() {
        let (text, truncated) = AgentOutput.middleTruncated(String(repeating: "a", count: 100_000), maxBytes: 65_536)
        #expect(truncated)
        #expect(text.utf8.count <= 65_536)
        #expect(text.contains("…[") && text.contains("bytes omitted]…"))
        let (short, cut) = AgentOutput.middleTruncated("short", maxBytes: 65_536)
        #expect(short == "short" && !cut)
    }

    @Test
    func alternateScreenIsTracked() {
        var output = AgentOutput()
        output.append(ArraySlice(Array("\u{1B}[?1049h".utf8)))
        #expect(output.isAlternateScreen)
        output.append(ArraySlice(Array("\u{1B}[?1049l".utf8)))
        #expect(!output.isAlternateScreen)
        output.append(ArraySlice(Array("\u{1B}[?47h".utf8)))
        #expect(output.isAlternateScreen)
    }

    @Test
    func masksAreReplacedIncludingAcrossChunks() {
        var output = AgentOutput()
        output.masks = ["hunter2"]
        output.append(ArraySlice(Array("pw: hun".utf8)))
        output.append(ArraySlice(Array("ter2\n".utf8)))
        let text = output.text(since: 0).text
        #expect(text.contains("••••••"))
        #expect(!text.contains("hunter2"))
    }

    // MARK: - Final review fixes

    @Test
    func aLineWithoutNewlinesStaysBounded() {
        var output = AgentOutput(capacity: 2 * 1024 * 1024)
        let chunk = ArraySlice(Array(String(repeating: "a", count: 4_096).utf8))
        for _ in 0..<256 { output.append(chunk) } // 1 MiB, no newline
        #expect(output.currentLineLength <= AgentOutput.maxLineLength)
        // 16 full lines; the last stays open until the next character arrives.
        #expect(output.cursor == 1_048_576 + 1_048_576 / AgentOutput.maxLineLength - 1)
        #expect(output.text(since: 0).truncated) // longer than one result
    }

    @Test
    func readingInsideARewrittenLineRedeliversTheWholeLine() {
        var output = AgentOutput()
        output.append(ArraySlice(Array("Downloading 5%".utf8)))
        let cursor = output.text(since: 0).cursor
        output.append(ArraySlice(Array("\rDownloading 100%\n".utf8)))
        #expect(output.text(since: cursor).text == "Downloading 100%\n")
        // A cursor at a committed boundary still reads only what is new.
        let end = output.cursor
        output.append(ArraySlice(Array("next\n".utf8)))
        #expect(output.text(since: end).text == "next\n")
    }
}
