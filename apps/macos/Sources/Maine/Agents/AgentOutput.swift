import Foundation

/// The plain-text view of an agent session's output: PTY bytes with escape
/// sequences stripped, `\r` and backspace applied to the current line, kept in
/// a bounded ring. The bytes the terminal renders are untouched; this is a copy.
///
/// The current, unfinished line is readable (prompts have no newline) and can
/// still be rewritten by `\r` until it ends. Masks (saved passwords that were
/// typed) are replaced when a line is rendered; passwords never span lines.
///
/// Agents read by cursor, a position in the stream that only ever grows, and
/// grows whenever the text changes. For plain output it is the cleaned byte
/// count. A line redrawn in place (a progress bar or spinner behind `\r`) does
/// not get longer, so every change to it also advances the cursor: otherwise a
/// reader waiting for new output would see none, and a narrower redraw would
/// move the cursor backwards. Such a line therefore spans more cursor positions
/// than bytes; `anchors` record where cursor and byte offset diverge.
struct AgentOutput: Sendable {
    static let maskReplacement = "••••••"
    static let maxLineLength = 65_536
    var currentLineLength: Int { line.count }

    let capacity: Int
    var masks: [String] = []
    private(set) var isAlternateScreen = false

    /// Committed lines; `ringStart` is the byte offset of its first byte.
    private var ring: [UInt8] = []
    private var ringStart = 0
    /// Cursor at the end of the committed lines (where the current line starts).
    private var committedCursor = 0
    /// Cursor positions the current line has used: grows by the bytes of every
    /// scalar that changes it, so it never shrinks while the line is open.
    private var lineSpan = 0
    /// Cursor ↔ byte offset pairs, ascending. From each anchor on, both advance
    /// together until the next one. Every committed line that spanned more
    /// positions than bytes adds one at its end.
    private var anchors: [Anchor] = [Anchor(offset: 0, cursor: 0)]
    private var line: [Unicode.Scalar] = []
    private var column = 0
    private var parser = Parser.ground
    private var pendingUTF8: [UInt8] = []
    private var expectedUTF8 = 0
    private var csiParameters: [UInt8] = []

    private struct Anchor: Sendable {
        var offset: Int
        var cursor: Int
    }

    private enum Parser: Sendable {
        case ground, escape, escapeIntermediate, csi, osc, oscEscape, string, stringEscape
    }

    init(capacity: Int = 2 * 1024 * 1024) {
        self.capacity = capacity
    }

    /// The current end of the stream, including the current line.
    var cursor: Int { committedCursor + max(lineSpan, renderedLine.utf8.count) }

    mutating func append(_ bytes: ArraySlice<UInt8>) {
        for byte in bytes { consume(byte) }
        trimRing()
    }

    /// Output after `since`. A cursor older than the ring starts at the ring
    /// and reports `truncated`; long results are cut in the middle. A cursor
    /// inside a line delivers that whole line again: `\r` may have rewritten it.
    func text(since: Int, upTo limit: Int? = nil, maxBytes: Int = 65_536) -> (text: String, cursor: Int, truncated: Bool) {
        let rendered = renderedLine
        let streamEnd = committedCursor + max(lineSpan, rendered.utf8.count)
        let stop = min(limit ?? streamEnd, streamEnd)
        guard since < stop else { return ("", stop, false) }
        var all = ring + Array(rendered.utf8)
        if stop < streamEnd {
            var cut = min(max(0, offset(of: stop, lineBytes: rendered.utf8.count) - ringStart), all.count)
            while cut < all.count, all[cut] & 0xC0 == 0x80 { cut += 1 }
            all = Array(all[..<cut])
        }
        var truncated = false
        var start: Int
        if since < cursor(atOffset: ringStart) {
            start = 0
            truncated = true
        } else if since >= committedCursor {
            start = ring.count // the current line
        } else {
            start = offset(of: since, lineBytes: rendered.utf8.count) - ringStart
        }
        start = min(start, all.count)
        if start < all.count {
            while start > 0, all[start - 1] != 0x0A { start -= 1 }
        }
        while start < all.count, all[start] & 0xC0 == 0x80 { start += 1 }
        let text = String(decoding: all[start...], as: UTF8.self)
        let (cut, didCut) = Self.middleTruncated(text, maxBytes: maxBytes)
        return (cut, stop, truncated || didCut)
    }

    /// The byte offset for `cursor`. Inside a line that spans more positions
    /// than bytes it lands within that line, never in a later one.
    private func offset(of cursor: Int, lineBytes: Int) -> Int {
        if cursor >= committedCursor {
            return ringStart + ring.count + min(cursor - committedCursor, lineBytes)
        }
        let index = lastAnchor { $0.cursor <= cursor }
        var offset = anchors[index].offset + (cursor - anchors[index].cursor)
        if index + 1 < anchors.count { offset = min(offset, anchors[index + 1].offset - 1) }
        return offset
    }

    private func cursor(atOffset offset: Int) -> Int {
        let index = lastAnchor { $0.offset <= offset }
        return anchors[index].cursor + (offset - anchors[index].offset)
    }

    /// The last anchor matching `isAtOrBefore` (a prefix of `anchors`); the first always does.
    private func lastAnchor(_ isAtOrBefore: (Anchor) -> Bool) -> Int {
        var low = 0
        var high = anchors.count - 1
        while low < high {
            let middle = (low + high + 1) / 2
            if isAtOrBefore(anchors[middle]) { low = middle } else { high = middle - 1 }
        }
        return low
    }

    static func middleTruncated(_ text: String, maxBytes: Int) -> (String, Bool) {
        let bytes = Array(text.utf8)
        guard bytes.count > maxBytes else { return (text, false) }
        let markerBudget = "…[\(bytes.count) bytes omitted]…".utf8.count
        let keep = max(0, maxBytes - markerBudget)
        var headEnd = keep / 2
        while headEnd > 0, bytes[headEnd] & 0xC0 == 0x80 { headEnd -= 1 }
        var tailStart = bytes.count - (keep - keep / 2)
        while tailStart < bytes.count, bytes[tailStart] & 0xC0 == 0x80 { tailStart += 1 }
        let omitted = tailStart - headEnd
        let head = String(decoding: bytes[..<headEnd], as: UTF8.self)
        let tail = String(decoding: bytes[tailStart...], as: UTF8.self)
        return (head + "…[\(omitted) bytes omitted]…" + tail, true)
    }

    // MARK: - Parsing

    private var renderedLine: String {
        var text = String(String.UnicodeScalarView(line))
        for mask in masks where !mask.isEmpty {
            text = text.replacingOccurrences(of: mask, with: Self.maskReplacement)
        }
        return text
    }

    private mutating func consume(_ byte: UInt8) {
        switch parser {
        case .ground:
            ground(byte)
        case .escape:
            switch byte {
            case UInt8(ascii: "["): csiParameters = []; parser = .csi
            case UInt8(ascii: "]"): parser = .osc
            case UInt8(ascii: "P"), UInt8(ascii: "X"), UInt8(ascii: "^"), UInt8(ascii: "_"): parser = .string
            case 0x20...0x2F: parser = .escapeIntermediate
            default: parser = .ground
            }
        case .escapeIntermediate:
            parser = .ground
        case .csi:
            if (0x40...0x7E).contains(byte) {
                finishCSI(final: byte)
                parser = .ground
            } else if byte == 0x1B {
                parser = .escape
            } else {
                csiParameters.append(byte)
            }
        case .osc:
            if byte == 0x07 { parser = .ground } else if byte == 0x1B { parser = .oscEscape }
        case .oscEscape:
            parser = byte == UInt8(ascii: "\\") ? .ground : .osc
        case .string:
            if byte == 0x1B { parser = .stringEscape }
        case .stringEscape:
            parser = byte == UInt8(ascii: "\\") ? .ground : .string
        }
    }

    private mutating func ground(_ byte: UInt8) {
        if byte >= 0x80 {
            utf8(byte)
            return
        }
        pendingUTF8 = []
        switch byte {
        case 0x1B: parser = .escape
        case 0x0A: commitLine()
        case 0x0D: column = 0
        case 0x08: column = max(0, column - 1)
        case 0x09: put("\t")
        case 0x20...0x7E: put(Unicode.Scalar(byte))
        default: break
        }
    }

    private mutating func utf8(_ byte: UInt8) {
        if byte & 0xC0 == 0x80 {
            guard !pendingUTF8.isEmpty else { return }
            pendingUTF8.append(byte)
        } else {
            pendingUTF8 = [byte]
            expectedUTF8 = byte >= 0xF0 ? 4 : byte >= 0xE0 ? 3 : byte >= 0xC0 ? 2 : 0
            if expectedUTF8 == 0 { pendingUTF8 = []; return }
        }
        guard pendingUTF8.count == expectedUTF8 else { return }
        for scalar in String(decoding: pendingUTF8, as: UTF8.self).unicodeScalars where scalar != "\u{FFFD}" {
            put(scalar)
        }
        pendingUTF8 = []
    }

    private mutating func put(_ scalar: Unicode.Scalar) {
        // Output without newlines (minified files, base64) must stay bounded:
        // a full line is committed as if it had wrapped.
        if column >= Self.maxLineLength { commitLine() }
        if column < line.count {
            // Reprinting the same text (a shell redrawing its prompt) is no change.
            if line[column] != scalar {
                line[column] = scalar
                lineSpan += UTF8.width(scalar)
            }
        } else {
            line.append(scalar)
            lineSpan += UTF8.width(scalar)
        }
        column += 1
    }

    private mutating func commitLine() {
        let bytes = Array((renderedLine + "\n").utf8)
        let span = max(lineSpan, bytes.count - 1) + 1
        ring.append(contentsOf: bytes)
        committedCursor += span
        if span != bytes.count {
            anchors.append(Anchor(offset: ringStart + ring.count, cursor: committedCursor))
        }
        line = []
        column = 0
        lineSpan = 0
    }

    private mutating func finishCSI(final: UInt8) {
        guard final == UInt8(ascii: "h") || final == UInt8(ascii: "l"),
              csiParameters.first == UInt8(ascii: "?") else { return }
        let modes = String(decoding: csiParameters.dropFirst(), as: UTF8.self).split(separator: ";")
        if modes.contains(where: { ["1049", "1047", "47"].contains(String($0)) }) {
            isAlternateScreen = final == UInt8(ascii: "h")
        }
    }

    private mutating func trimRing() {
        guard ring.count > capacity else { return }
        var drop = ring.count - capacity
        while drop < ring.count, ring[drop] & 0xC0 == 0x80 { drop += 1 }
        ring.removeFirst(drop)
        ringStart += drop
        // Keep the anchor that still governs the ring's first byte.
        while anchors.count > 1, anchors[1].offset <= ringStart { anchors.removeFirst() }
    }
}
