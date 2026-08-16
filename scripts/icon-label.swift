#!/usr/bin/env swift
// Draw a channel's banner across the app icon.
//
// One source icon, four apps. Without this they are the same bear, and the only
// way to tell a canary from the build you depend on is to open it and read
// Settings.
//
// AppKit rather than ImageMagick because every caller is already a macOS runner
// where Swift exists, and adding a `brew install` to a path that needs none
// buys nothing.
//
//   swift scripts/icon-label.swift canary in.png out.png
import AppKit
import Foundation

struct Banner {
    let text: String
    let fill: NSColor
    let ink: NSColor
}

func rgb(_ hex: UInt32) -> NSColor {
    NSColor(
        srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
        green: CGFloat((hex >> 8) & 0xFF) / 255,
        blue: CGFloat(hex & 0xFF) / 255,
        alpha: 1
    )
}

// Colors are per channel and carry more than the word does: at 32 points the
// text is unreadable and the color is the whole signal.
let banners: [String: Banner] = [
    "canary": Banner(text: "CANARY", fill: rgb(0xE8A21C), ink: rgb(0x16130B)),
    "preview": Banner(text: "PREVIEW", fill: rgb(0x3B6FD4), ink: rgb(0xFFFFFF)),
    "local": Banner(text: "LOCAL", fill: rgb(0x6E6E73), ink: rgb(0xFFFFFF)),
]

func fail(_ message: String, _ code: Int32) -> Never {
    FileHandle.standardError.write(Data((message + "\n").utf8))
    exit(code)
}

let args = CommandLine.arguments
guard args.count == 4 else {
    fail("usage: icon-label.swift <channel> <input.png> <output.png>", 2)
}
let channel = args[1]
let input = args[2]
let output = args[3]

// Stable is not labeled, and is not re-encoded either: it is the source asset
// every channel shares, and rewriting its bytes would churn it for nothing.
if channel == "stable" {
    try? FileManager.default.removeItem(atPath: output)
    do {
        try FileManager.default.copyItem(atPath: input, toPath: output)
    } catch {
        fail("could not copy \(input) to \(output): \(error)", 1)
    }
    exit(0)
}

guard let banner = banners[channel] else {
    fail("unknown channel: \(channel)", 1)
}
guard let data = FileManager.default.contents(atPath: input),
    let source = NSBitmapImageRep(data: data)
else {
    fail("could not read an image from \(input)", 1)
}

let side = CGFloat(source.pixelsWide)
guard source.pixelsHigh == source.pixelsWide else {
    fail("the icon must be square, got \(source.pixelsWide)x\(source.pixelsHigh)", 1)
}

guard
    let canvas = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: source.pixelsWide, pixelsHigh: source.pixelsHigh,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    )
else { fail("could not allocate a \(source.pixelsWide)px canvas", 1) }

NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: canvas)
source.draw(in: CGRect(x: 0, y: 0, width: side, height: side))

let context = NSGraphicsContext.current!.cgContext
context.saveGState()
// Rotate about the center so the band crosses the bottom-right corner at 45°.
context.translateBy(x: side / 2, y: side / 2)
context.rotate(by: .pi / 4)

let bandHeight = side * 0.17
// Overlong on purpose: the band must run past both edges of the icon after
// rotation, or its ends appear as cut corners inside the artwork.
let band = CGRect(x: -side, y: -side * 0.60, width: side * 2, height: bandHeight)
banner.fill.setFill()
context.fill(band)

let font = NSFont.systemFont(ofSize: bandHeight * 0.58, weight: .black)
let attributes: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: banner.ink,
    .kern: bandHeight * 0.05,
]
let text = banner.text as NSString
let measured = text.size(withAttributes: attributes)
text.draw(
    at: CGPoint(x: -measured.width / 2, y: band.midY - measured.height / 2),
    withAttributes: attributes
)

context.restoreGState()
NSGraphicsContext.restoreGraphicsState()

guard let png = canvas.representation(using: .png, properties: [:]) else {
    fail("could not encode a PNG", 1)
}
do {
    try png.write(to: URL(fileURLWithPath: output))
} catch {
    fail("could not write \(output): \(error)", 1)
}
