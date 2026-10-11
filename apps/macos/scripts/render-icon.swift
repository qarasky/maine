#!/usr/bin/env swift
// Renders Maine's application icon master (1024x1024 PNG).
// Usage: swift apps/macos/scripts/render-icon.swift /path/to/AppIcon-1024.png
//
// To rebuild Resources/AppIcon.icns from the master:
//   mkdir -p AppIcon.iconset
//   for s in 16 32 128 256 512; do
//     sips -z $s $s AppIcon-1024.png --out AppIcon.iconset/icon_${s}x${s}.png
//     sips -z $((s*2)) $((s*2)) AppIcon-1024.png --out AppIcon.iconset/icon_${s}x${s}@2x.png
//   done
//   iconutil -c icns AppIcon.iconset -o apps/macos/Resources/AppIcon.icns
import AppKit
import Foundation

let size: CGFloat = 1024
let output = CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : FileManager.default.temporaryDirectory.appendingPathComponent("AppIcon-1024.png").path

let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()

// Transparent surround; macOS masks the tile itself.
NSColor.clear.setFill()
NSRect(x: 0, y: 0, width: size, height: size).fill()

// Dark tile, same family as the terminal background, lifted for Dock contrast.
let tile = NSBezierPath(roundedRect: NSRect(x: 0, y: 0, width: size, height: size),
                        xRadius: 232, yRadius: 232)
let gradient = NSGradient(
    colors: [NSColor(calibratedWhite: 0.21, alpha: 1.0),
             NSColor(calibratedWhite: 0.08, alpha: 1.0)],
    atLocations: [0.0, 1.0], colorSpace: .deviceRGB)!
gradient.draw(in: tile, angle: 90)

// Monospace prompt: light ">" plus a green block cursor (the app's connected tint).
let font = NSFont(name: "Menlo-Bold", size: 430)
    ?? NSFont.monospacedSystemFont(ofSize: 430, weight: .bold)
let glyph = NSAttributedString(string: ">", attributes: [
    .font: font, .foregroundColor: NSColor(calibratedWhite: 0.93, alpha: 1.0),
])
let glyphWidth = glyph.size().width
let cursorWidth: CGFloat = 152
let cursorHeight: CGFloat = 296
let gap: CGFloat = 58
let totalWidth = glyphWidth + gap + cursorWidth
let x = (size - totalWidth) / 2

// Center the line-height box optically (cap-height mass sits slightly high).
let lineHeight = glyph.size().height
glyph.draw(in: NSRect(x: x, y: (size - lineHeight) / 2 + 8, width: glyphWidth, height: lineHeight))

let cursor = NSBezierPath(roundedRect: NSRect(x: x + glyphWidth + gap,
                                             y: (size - cursorHeight) / 2,
                                             width: cursorWidth, height: cursorHeight),
                          xRadius: 28, yRadius: 28)
NSColor(calibratedRed: 0.20, green: 0.78, blue: 0.35, alpha: 1.0).setFill()
cursor.fill()

image.unlockFocus()

guard let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fputs("render-icon: failed to encode PNG\n", stderr)
    exit(1)
}
do {
    try png.write(to: URL(fileURLWithPath: output))
    print("Wrote \(output)")
} catch {
    fputs("render-icon: \(error)\n", stderr)
    exit(1)
}
