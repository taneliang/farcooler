// Renders the shared Far Cooler app-icon master into a macOS .iconset.
//
// Run via: swift Tools/make-icon.swift <output-iconset-dir> [output-icns] [source-png]

import AppKit
import CoreGraphics
import Foundation

let outDir = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.iconset"
let icnsPath = CommandLine.arguments.count > 2 ? CommandLine.arguments[2] : nil
let sourcePath =
    CommandLine.arguments.count > 3
    ? CommandLine.arguments[3]
    : "../shared/Assets.xcassets/AppIcon.appiconset/AppIcon.png"
try? FileManager.default.createDirectory(
    atPath: outDir, withIntermediateDirectories: true)

guard
    let sourceImage = NSImage(contentsOfFile: sourcePath),
    let source = sourceImage.cgImage(forProposedRect: nil, context: nil, hints: nil)
else {
    FileHandle.standardError.write(
        "could not load icon master at \(sourcePath)\n".data(using: .utf8)!)
    exit(1)
}

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

    // iOS applies its own icon mask. macOS does not, so place the same master
    // inside a padded rounded tile and leave the outer corners transparent.
    let radius = size * 0.2237  // macOS icon corner ratio
    let rect = CGRect(x: size * 0.06, y: size * 0.06, width: size * 0.88, height: size * 0.88)
    let path = CGPath(
        roundedRect: rect, cornerWidth: radius, cornerHeight: radius, transform: nil)

    // A restrained baked shadow keeps the ivory tile legible on pale desktops.
    ctx.saveGState()
    ctx.setShadow(
        offset: CGSize(width: 0, height: -size * 0.012),
        blur: size * 0.025,
        color: CGColor(gray: 0, alpha: 0.22))
    ctx.setFillColor(CGColor(gray: 1, alpha: 1))
    ctx.addPath(path)
    ctx.fillPath()
    ctx.restoreGState()

    ctx.saveGState()
    ctx.addPath(path)
    ctx.clip()
    ctx.interpolationQuality = .high
    ctx.draw(source, in: rect)
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

// iconutil on macOS 26.5 rejects even an iconset it just unpacked. An ICNS
// container is only a big-endian header plus typed PNG chunks, so write the
// modern Retina chunk set directly and keep the build independent of that bug.
if let icnsPath {
    let chunks: [(type: String, filename: String)] = [
        ("ic07", "icon_128x128.png"),
        ("ic08", "icon_256x256.png"),
        ("ic09", "icon_512x512.png"),
        ("ic10", "icon_512x512@2x.png"),
        ("ic11", "icon_16x16@2x.png"),
        ("ic12", "icon_32x32@2x.png"),
        ("ic13", "icon_128x128@2x.png"),
        ("ic14", "icon_256x256@2x.png"),
    ]

    func fourCC(_ value: String) -> Data {
        precondition(value.utf8.count == 4)
        return Data(value.utf8)
    }

    func bigEndian(_ value: Int) -> Data {
        var encoded = UInt32(value).bigEndian
        return Data(bytes: &encoded, count: MemoryLayout<UInt32>.size)
    }

    let payloads: [(type: String, data: Data)] = try chunks.map { chunk in
        let url = URL(fileURLWithPath: "\(outDir)/\(chunk.filename)")
        return (chunk.type, try Data(contentsOf: url))
    }

    var tableOfContents = Data()
    for payload in payloads {
        tableOfContents.append(fourCC(payload.type))
        tableOfContents.append(bigEndian(payload.data.count + 8))
    }

    var encodedChunks = Data()
    encodedChunks.append(fourCC("TOC "))
    encodedChunks.append(bigEndian(tableOfContents.count + 8))
    encodedChunks.append(tableOfContents)
    for payload in payloads {
        encodedChunks.append(fourCC(payload.type))
        encodedChunks.append(bigEndian(payload.data.count + 8))
        encodedChunks.append(payload.data)
    }

    var icns = Data()
    icns.append(fourCC("icns"))
    icns.append(bigEndian(encodedChunks.count + 8))
    icns.append(encodedChunks)
    try icns.write(to: URL(fileURLWithPath: icnsPath))
    print("wrote \(icnsPath)")
}
