#!/usr/bin/env swift
// Logo-direction marks for Maine: the terminal prompt rebuilt from the
// universe's two most ownable brand details — Cyberpunk yellow (#FCEE0C)
// and the chromatic-aberration glitch (cyan #03D8F3 / red #FF003C ghosts).
// Nothing is traced or copied; only palette + treatment are borrowed.
//
// Usage: swift apps/macos/scripts/render-mark.swift <glitch|monogram> <output.png>
import AppKit
import Foundation

guard CommandLine.arguments.count == 3,
      ["glitch", "monogram"].contains(CommandLine.arguments[1]) else {
    fputs("usage: render-mark.swift <glitch|monogram> <output.png>\n", stderr)
    exit(2)
}
let variant = CommandLine.arguments[1]
let output = CommandLine.arguments[2]

let size: CGFloat = 1024
let yellow = NSColor(calibratedRed: 0.988, green: 0.933, blue: 0.047, alpha: 1.0) // #FCEE0C
let cyan = NSColor(calibratedRed: 0.012, green: 0.847, blue: 0.953, alpha: 1.0)   // #03D8F3
let red = NSColor(calibratedRed: 1.0, green: 0.0, blue: 0.235, alpha: 1.0)        // #FF003C

let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
NSColor.clear.setFill()
NSRect(x: 0, y: 0, width: size, height: size).fill()

let tile = NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: size, height: size),
                        xRadius: 232, yRadius: 232)
NSGradient(colors: [NSColor(calibratedWhite: 0.10, alpha: 1.0),
                    NSColor.black],
           atLocations: [0.0, 1.0], colorSpace: .deviceRGB)!
    .draw(in: tile, angle: 90)

if variant == "glitch" {
    // The product truth (shell prompt) in brand yellow, split into RGB ghosts.
    let font = NSFont(name: "Menlo-Bold", size: 430)
        ?? NSFont.monospacedSystemFont(ofSize: 430, weight: .bold)
    let glyph = ">"
    let gw = NSAttributedString(string: glyph, attributes: [.font: font]).size().width
    let cursorW: CGFloat = 152, cursorH: CGFloat = 296, gap: CGFloat = 58
    let x = (size - (gw + gap + cursorW)) / 2
    let lineH = NSAttributedString(string: glyph, attributes: [.font: font]).size().height
    let textY = (size - lineH) / 2 + 8
    let cursorRect = NSRect(x: x + gw + gap, y: (size - cursorH) / 2,
                            width: cursorW, height: cursorH)
    // Ghosts first: cyan shifted left, red shifted right.
    for (color, dx) in [(cyan, -16.0), (red, 16.0)] {
        NSAttributedString(string: glyph, attributes: [.font: font, .foregroundColor: color])
            .draw(in: NSRect(x: x + dx, y: textY, width: gw + 40, height: lineH))
        color.setFill()
        NSBezierPath(roundedRect: cursorRect.offsetBy(dx: dx, dy: 0), xRadius: 28, yRadius: 28).fill()
    }
    NSAttributedString(string: glyph, attributes: [.font: font, .foregroundColor: yellow])
        .draw(in: NSRect(x: x, y: textY, width: gw + 40, height: lineH))
    yellow.setFill()
    NSBezierPath(roundedRect: cursorRect, xRadius: 28, yRadius: 28).fill()
} else {
    // Angular M monogram: one chamfered polyline, sharp Trigger-style vertices.
    // Bottom-left -> top-left -> low center -> top-right -> bottom-right.
    let w: CGFloat = 540, h: CGFloat = 430
    let ox = (size - w) / 2, oy = (size - h) / 2 + 10
    let path = NSBezierPath()
    path.move(to: NSPoint(x: ox, y: oy))
    path.line(to: NSPoint(x: ox, y: oy + h))
    path.line(to: NSPoint(x: ox + w / 2, y: oy + 105))
    path.line(to: NSPoint(x: ox + w, y: oy + h))
    path.line(to: NSPoint(x: ox + w, y: oy))
    path.lineJoinStyle = .bevel
    path.lineCapStyle = .square
    path.lineWidth = 112
    for (color, dx) in [(cyan, -15.0), (red, 15.0)] {
        color.setStroke()
        NSGraphicsContext.saveGraphicsState()
        NSBezierPath.translate(dx, 0)
        path.stroke()
        NSGraphicsContext.restoreGraphicsState()
    }
    yellow.setStroke()
    path.stroke()
}

image.unlockFocus()

guard let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fputs("render-mark: failed to encode PNG\n", stderr)
    exit(1)
}
do {
    try png.write(to: URL(fileURLWithPath: output))
    print("Wrote \(output)")
} catch {
    fputs("render-mark: \(error)\n", stderr)
    exit(1)
}

extension NSBezierPath {
    static func translate(_ dx: CGFloat, _ dy: CGFloat) {
        let t = NSAffineTransform()
        t.translateX(by: dx, yBy: dy)
        t.concat()
    }
}
