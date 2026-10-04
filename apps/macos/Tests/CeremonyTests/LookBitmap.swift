import AppKit

/// The scales every pixel test runs at: CI's headless Mac renders at 1x and a
/// Retina Mac at 2x, so a test that holds at only one of them fails on the
/// other machine (ov-280).
let lookScales: [Int] = [1, 2]

extension NSView {
    /// This view drawn into a bitmap allocated at an explicit scale, so a test
    /// sees 1x on a Retina Mac and 2x on CI alike. The bitmap is
    /// `bounds.size × scale` pixels and its `size` is the view's points, which is
    /// what `color(_:_:_:)` reads the scale back from.
    func lookBitmap(scale: Int) -> NSBitmapImageRep? {
        guard
            let rep = NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: Int(bounds.width) * scale, pixelsHigh: Int(bounds.height) * scale,
                bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)
        else { return nil }
        rep.size = bounds.size
        cacheDisplay(in: bounds, to: rep)
        return rep
    }
}

extension NSBitmapImageRep {
    /// Pixels per point: 1 or 2.
    var lookScale: Int { max(1, pixelsWide / Int(size.width.rounded())) }

    /// The sRGB color at a point (not a pixel), in the bitmap's own scale.
    func color(atPoint x: Int, _ y: Int) -> NSColor {
        colorAt(x: x * lookScale, y: y * lookScale)!.usingColorSpace(.sRGB)!
    }
}
