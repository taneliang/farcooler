import AppKit
import Testing

/// What a capture must be before it is kept (ov-476): the size that was asked
/// for, and a window whose traffic lights are round. A capture drawn at a
/// different size from the window's, or stretched on one axis, fails here and
/// not in front of the person reading the website.
enum CaptureCheck {
    /// The three traffic lights' boxes in pixels, found as what differs from
    /// the title bar beside them, left of the toolbar's first item. Nil when
    /// there aren't three.
    static func lightBoxes(_ rep: NSBitmapImageRep) -> [(width: Int, height: Int)]? {
        let scale = max(1, rep.lookScale)
        // Lights sit in the first 84 pt, below the window's top edge and above its toolbar's middle.
        let (x0, x1, y0, y1) = (6 * scale, 84 * scale, 4 * scale, 40 * scale)
        guard rep.pixelsWide >= x1, rep.pixelsHigh >= y1, let ground = rep.colorAt(x: 4 * scale, y: 20 * scale)?.usingColorSpace(.sRGB) else {
            return nil
        }
        func differs(_ x: Int, _ y: Int) -> Bool {
            guard let c = rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB) else { return false }
            let d = abs(c.redComponent - ground.redComponent) + abs(c.greenComponent - ground.greenComponent) + abs(c.blueComponent - ground.blueComponent)
            // An unfocused window's lights are gray on gray, about 0.1 from the bar (a focused one's are colored, over 0.5).
            return d > 0.05
        }
        let columns = (x0..<x1).map { x in (y0..<y1).contains { differs(x, $0) } }
        var boxes: [(width: Int, height: Int)] = []
        var x = 0
        while x < columns.count {
            guard columns[x] else { x += 1; continue }
            var end = x
            while end < columns.count, columns[end] { end += 1 }
            let rows = (y0..<y1).filter { y in (x + x0..<end + x0).contains { differs($0, y) } }
            if let top = rows.first, let bottom = rows.last { boxes.append((end - x, bottom - top + 1)) }
            x = end
        }
        return boxes.count == 3 ? boxes : nil
    }

    /// Why this capture should be thrown away, or nil when it is fine.
    static func problem(_ rep: NSBitmapImageRep, expectedPixels: (width: Int, height: Int)?) -> String? {
        if let want = expectedPixels, rep.pixelsWide != want.width || rep.pixelsHigh != want.height {
            return "capture is \(rep.pixelsWide)x\(rep.pixelsHigh) px, asked for \(want.width)x\(want.height): the window isn't the size it was set to"
        }
        guard let boxes = lightBoxes(rep) else { return "capture's traffic lights weren't found" }
        for box in boxes where abs(box.width - box.height) > 1 {
            return "capture is stretched: a traffic light is \(box.width)x\(box.height) px, not round"
        }
        return nil
    }

    static func rep(png: URL) throws -> NSBitmapImageRep {
        try #require(NSBitmapImageRep(data: try Data(contentsOf: png)))
    }
}

struct CaptureCheckTests {
    static let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Fixtures/traffic-lights-dark-2x.png")
    /// The corner of a real capture of an unfocused window on macOS 27: gray lights, as the harness takes it without FOCUS.
    static let unfocused = URL(fileURLWithPath: #filePath).deletingLastPathComponent().appendingPathComponent("Fixtures/traffic-lights-unfocused-light-2x.png")

    /// The title bar's corner of the real hero capture: round lights, 28 px at 2x.
    @Test func aRealCapturesRoundLightsPass() throws {
        let rep = try CaptureCheck.rep(png: Self.fixture)
        rep.size = NSSize(width: rep.pixelsWide / 2, height: rep.pixelsHigh / 2)
        #expect(CaptureCheck.lightBoxes(rep)?.map(\.width) == [28, 28, 28])
        #expect(CaptureCheck.problem(rep, expectedPixels: nil) == nil)
    }

    @Test func anUnfocusedCapturesGrayLightsPass() throws {
        let rep = try CaptureCheck.rep(png: Self.unfocused)
        rep.size = NSSize(width: rep.pixelsWide / 2, height: rep.pixelsHigh / 2)
        #expect(CaptureCheck.lightBoxes(rep)?.map(\.width) == [28, 28, 28])
        #expect(CaptureCheck.problem(rep, expectedPixels: nil) == nil)
    }

    /// A flat bar, and the same corner with the lights painted over: no lights, so no file.
    @Test(arguments: [false, true])
    func aCaptureWithoutLightsFails(paintedOver: Bool) throws {
        let src = try CaptureCheck.rep(png: Self.unfocused)
        let out = try #require(
            NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: src.pixelsWide, pixelsHigh: src.pixelsHigh, bitsPerSample: 8, samplesPerPixel: 4,
                hasAlpha: true, isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
        if paintedOver { src.draw(in: NSRect(x: 0, y: 0, width: out.pixelsWide, height: out.pixelsHigh)) }
        NSColor(srgbRed: 240 / 255, green: 240 / 255, blue: 240 / 255, alpha: 1).setFill()
        // Flat: the whole image; painted over: the strip the lights sit in (AppKit's origin is the bottom).
        let strip = paintedOver ? NSRect(x: 0, y: out.pixelsHigh - 90, width: 170, height: 70) : NSRect(x: 0, y: 0, width: out.pixelsWide, height: out.pixelsHigh)
        strip.fill()
        NSGraphicsContext.restoreGraphicsState()
        out.size = NSSize(width: out.pixelsWide / 2, height: out.pixelsHigh / 2)
        #expect(CaptureCheck.problem(out, expectedPixels: nil)?.contains("weren't found") == true)
    }

    /// The same corner drawn 1.17x too wide, or too short: what a stretched capture does to it.
    @Test(arguments: [(1.17, 1.0), (1.0, 0.86)])
    func aStretchedCaptureFails(sx: Double, sy: Double) throws {
        let src = try CaptureCheck.rep(png: Self.fixture)
        let (w, h) = (Int(Double(src.pixelsWide) * sx), Int(Double(src.pixelsHigh) * sy))
        let out = try #require(
            NSBitmapImageRep(
                bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true,
                isPlanar: false, colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0))
        NSGraphicsContext.saveGraphicsState()
        NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: out)
        NSGraphicsContext.current?.imageInterpolation = .high
        src.draw(in: NSRect(x: 0, y: 0, width: w, height: h))
        NSGraphicsContext.restoreGraphicsState()
        // Kept at 2x: the lights' own span is what's measured.
        out.size = NSSize(width: w / 2, height: h / 2)
        #expect(CaptureCheck.problem(out, expectedPixels: nil)?.contains("stretched") == true)
    }

    @Test func aCaptureOfTheWrongSizeFails() throws {
        let rep = try CaptureCheck.rep(png: Self.fixture)
        rep.size = NSSize(width: rep.pixelsWide / 2, height: rep.pixelsHigh / 2)
        #expect(CaptureCheck.problem(rep, expectedPixels: (3200, 2000))?.contains("asked for 3200x2000") == true)
    }
}
