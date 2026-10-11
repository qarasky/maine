#!/usr/bin/env swift
// Renders Maine's mascot icon master (1024x1024 PNG): a chibi bust leaning
// into the fan meme — huge traps, tiny head, red cyber slits, red scarf.
//
// Usage: swift apps/macos/scripts/render-mascot.swift /path/to/Mascot-1024.png
//
// To rebuild Resources/Mascot.icns from the master, use the same
// sips + iconutil steps documented in render-icon.swift.
import AppKit
import Foundation

let size: CGFloat = 1024
let output = CommandLine.arguments.count > 1
    ? CommandLine.arguments[1]
    : FileManager.default.temporaryDirectory.appendingPathComponent("Mascot-1024.png").path

let skin = NSColor(calibratedRed: 0.66, green: 0.44, blue: 0.25, alpha: 1.0)
let skinShade = NSColor(calibratedRed: 0.52, green: 0.34, blue: 0.19, alpha: 1.0)
let hair = NSColor(calibratedRed: 0.91, green: 0.75, blue: 0.40, alpha: 1.0)
let scarfRed = NSColor(calibratedRed: 0.83, green: 0.18, blue: 0.18, alpha: 1.0)
let eyeRed = NSColor(calibratedRed: 1.0, green: 0.27, blue: 0.22, alpha: 1.0)
let vest = NSColor(calibratedWhite: 0.15, alpha: 1.0)
let vestPanel = NSColor(calibratedWhite: 0.10, alpha: 1.0)
let edge = NSColor(calibratedWhite: 0.24, alpha: 1.0)

func rounded(_ rect: NSRect, _ radius: CGFloat) -> NSBezierPath {
    NSBezierPath(roundedRect: rect, xRadius: radius, yRadius: radius)
}

let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
NSColor.clear.setFill()
NSRect(x: 0, y: 0, width: size, height: size).fill()

// Tile, same family as the prompt icon.
let tile = rounded(NSRect(x: 0, y: 0, width: size, height: size), 232)
NSGradient(colors: [NSColor(calibratedWhite: 0.21, alpha: 1.0),
                    NSColor(calibratedWhite: 0.08, alpha: 1.0)],
           atLocations: [0.0, 1.0], colorSpace: .deviceRGB)!
    .draw(in: tile, angle: 90)

let cx: CGFloat = 512

// Massive bare traps (his arms/shoulders are bare skin in the reference),
// deliberately much wider than the tiny head.
let shoulders = rounded(NSRect(x: cx - 365, y: -10, width: 730, height: 475), 150)
skinShade.setFill()
shoulders.fill()
NSColor(calibratedRed: 0.36, green: 0.23, blue: 0.12, alpha: 1.0).setStroke()
shoulders.lineWidth = 7
shoulders.stroke()

// Vest panel + signature red rings.
let panel = NSBezierPath()
panel.move(to: NSPoint(x: cx - 245, y: -10))
panel.line(to: NSPoint(x: cx - 125, y: 470))
panel.line(to: NSPoint(x: cx + 125, y: 470))
panel.line(to: NSPoint(x: cx + 245, y: -10))
panel.close()
vestPanel.setFill()
panel.fill()
for dx: CGFloat in [-78, 78] {
    let ring = NSBezierPath(ovalIn: NSRect(x: cx + dx - 27, y: 235, width: 54, height: 54))
    scarfRed.setStroke()
    ring.lineWidth = 13
    ring.stroke()
}

// Neck + ears + head (deliberately small).
skinShade.setFill()
NSBezierPath(rect: NSRect(x: cx - 32, y: 490, width: 64, height: 90)).fill()
for dx: CGFloat in [-1, 1] {
    rounded(NSRect(x: cx + dx * 127 - 13, y: 610, width: 26, height: 62), 13).fill(with: skin)
}
let head = rounded(NSRect(x: cx - 118, y: 535, width: 236, height: 268), 105)
skin.setFill()
head.fill()

// Blonde buzz cut: clip to the head, cap the top.
NSGraphicsContext.saveGraphicsState()
head.addClip()
hair.setFill()
NSRect(x: cx - 118, y: 700, width: 236, height: 110).fill()
NSGraphicsContext.restoreGraphicsState()

// Heavy brow bars.
for dx: CGFloat in [-1, 1] {
    rounded(NSRect(x: cx + dx * 59 - 36, y: 688, width: 72, height: 15), 7).fill(with: skinShade)
}

// Red cyber slits with a neon glow.
let glow = NSShadow()
glow.shadowColor = eyeRed
glow.shadowBlurRadius = 26
glow.set()
for dx: CGFloat in [-1, 1] {
    rounded(NSRect(x: cx + dx * 59 - 34, y: 636, width: 68, height: 27), 13).fill(with: eyeRed)
}
NSShadow().set()

// Flat neutral mouth.
rounded(NSRect(x: cx - 28, y: 578, width: 56, height: 12), 6).fill(with: skinShade)

// Red scarf across the neck.
rounded(NSRect(x: cx - 178, y: 428, width: 356, height: 96), 44).fill(with: scarfRed)

image.unlockFocus()

guard let tiff = image.tiffRepresentation,
      let bitmap = NSBitmapImageRep(data: tiff),
      let png = bitmap.representation(using: .png, properties: [:]) else {
    fputs("render-mascot: failed to encode PNG\n", stderr)
    exit(1)
}
do {
    try png.write(to: URL(fileURLWithPath: output))
    print("Wrote \(output)")
} catch {
    fputs("render-mascot: \(error)\n", stderr)
    exit(1)
}

extension NSBezierPath {
    func fill(with color: NSColor) {
        color.setFill()
        fill()
    }
}
