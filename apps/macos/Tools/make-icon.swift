// Renders the Far Cooler app icon and writes an .iconset.
//
// Run via: swift Tools/make-icon.swift <output-iconset-dir>
// Then: iconutil -c icns <output-iconset-dir>
//
// A crescent over a terminal prompt: night, plus a shell.

import AppKit
import CoreGraphics
import Foundation

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.iconset"
try? FileManager.default.createDirectory(
    atPath: outDir, withIntermediateDirectories: true)

/// Sizes macOS expects in an iconset.
let specs: [(name: String, px: Int)] = [
    ("icon_16x16", 16), ("icon_16x16@2x", 32),
    ("icon_32x32", 32), ("icon_32x32@2x", 64),
    ("icon_128x128", 128), ("icon_128x128@2x", 256),
    ("icon_256x256", 256), ("icon_256x256@2x", 512),
    ("icon_512x512", 512), ("icon_512x512@2x", 1024),
]

func render(px: Int) -> Data? {
    let size = CGFloat(px)
    guard
        let ctx = CGContext(
            data: nil, width: px, height: px, bitsPerComponent: 8, bytesPerRow: 0,
            space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
    else { return nil }

    // Rounded squircle background, deep indigo to near-black.
    let radius = size * 0.2237  // macOS icon corner ratio
    let rect = CGRect(x: size * 0.06, y: size * 0.06, width: size * 0.88, height: size * 0.88)
    let path = CGPath(
        roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)

    ctx.saveGState()
    ctx.addPath(path)
    ctx.clip()

    let colors =
        [
            CGColor(red: 0.16, green: 0.17, blue: 0.31, alpha: 1),
            CGColor(red: 0.06, green: 0.06, blue: 0.12, alpha: 1),
        ] as CFArray
    if let grad = CGGradient(
        colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: colors, locations: [0, 1])
    {
        ctx.drawLinearGradient(
            grad, start: CGPoint(x: 0, y: size), end: CGPoint(x: size, y: 0), options: [])
    }

    // Crescent moon, upper right.
    let moonR = size * 0.15
    let moonC = CGPoint(x: size * 0.68, y: size * 0.70)
    ctx.setFillColor(CGColor(red: 0.96, green: 0.93, blue: 0.78, alpha: 1))
    ctx.addArc(
        center: moonC, radius: moonR, startAngle: 0, endAngle: .pi * 2, clockwise: false)
    ctx.fillPath()
    // Bite out of it, in the background colour, to make the crescent.
    ctx.setBlendMode(.destinationOut)
    ctx.addArc(
        center: CGPoint(x: moonC.x - moonR * 0.45, y: moonC.y + moonR * 0.30),
        radius: moonR * 0.92, startAngle: 0, endAngle: .pi * 2, clockwise: false)
    ctx.fillPath()
    ctx.setBlendMode(.normal)

    // Terminal prompt: a chevron and a cursor bar.
    let lw = max(size * 0.045, 1.5)
    ctx.setStrokeColor(CGColor(red: 0.55, green: 0.85, blue: 0.62, alpha: 1))
    ctx.setLineWidth(lw)
    ctx.setLineCap(.round)
    ctx.setLineJoin(.round)

    let bx = size * 0.26, by = size * 0.36, arm = size * 0.11
    ctx.move(to: CGPoint(x: bx, y: by + arm))
    ctx.addLine(to: CGPoint(x: bx + arm * 0.9, y: by))
    ctx.addLine(to: CGPoint(x: bx, y: by - arm))
    ctx.strokePath()

    ctx.setFillColor(CGColor(red: 0.55, green: 0.85, blue: 0.62, alpha: 1))
    ctx.fill(
        CGRect(
            x: bx + arm * 1.5, y: by - arm, width: size * 0.22, height: lw * 1.1))

    ctx.restoreGState()

    guard let image = ctx.makeImage() else { return nil }
    let rep = NSBitmapImageRep(cgImage: image)
    return rep.representation(using: .png, properties: [:])
}

for spec in specs {
    guard let data = render(px: spec.px) else {
        FileHandle.standardError.write("failed to render \(spec.name)\n".data(using: .utf8)!)
        exit(1)
    }
    let path = "\(outDir)/\(spec.name).png"
    try data.write(to: URL(fileURLWithPath: path))
}

print("wrote \(specs.count) icon sizes to \(outDir)")
