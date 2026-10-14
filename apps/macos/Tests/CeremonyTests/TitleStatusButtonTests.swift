import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// Every status button's hover and pressed plate contains its icon and its
/// count, with the same room each side, and every button has the same plate
/// (ov-511). Measured on the real title bar: the plate is what appears when a
/// button's plate is held, found by differencing the bitmaps, and the label is
/// the ink the resting bar has inside it.
@MainActor
@Suite(.serialized)
struct TitleStatusButtonTests {
    typealias Kit = TitleStatusButtonKit

    struct Extent {
        var minX = Int.max, maxX = Int.min, minY = Int.max, maxY = Int.min
        mutating func add(_ x: Int, _ y: Int) {
            minX = min(minX, x); maxX = max(maxX, x); minY = min(minY, y); maxY = max(maxY, y)
        }
        var isEmpty: Bool { minX > maxX }
    }

    /// The plate's extent and the label's, in points.
    struct Measure {
        let plate: CGRect
        let ink: CGRect
        var left: CGFloat { ink.minX - plate.minX }
        var right: CGFloat { plate.maxX - ink.maxX }
    }

    private static func differs(_ a: NSColor, _ b: NSColor, by limit: CGFloat) -> Bool {
        abs(a.redComponent - b.redComponent) > limit || abs(a.greenComponent - b.greenComponent) > limit
            || abs(a.blueComponent - b.blueComponent) > limit
    }

    /// Whether row `y` of the two bitmaps holds the same bytes from pixel
    /// `from` to `to`: true only when both are laid out alike and equal.
    private static func sameBytes(_ a: NSBitmapImageRep, _ b: NSBitmapImageRep, row y: Int, from: Int, to: Int) -> Bool {
        guard a.bitsPerPixel == b.bitsPerPixel, a.bytesPerRow == b.bytesPerRow, a.pixelsHigh == b.pixelsHigh,
            let pa = a.bitmapData, let pb = b.bitmapData, to > from
        else { return false }
        let bytes = a.bitsPerPixel / 8
        return memcmp(pa + y * a.bytesPerRow + from * bytes, pb + y * b.bytesPerRow + from * bytes, (to - from) * bytes) == 0
    }

    /// The plate `id` draws when held at `plate`, and its label, at `width`.
    ///
    /// `rest` is the bar with nothing held, which is the same for every
    /// button and plate at one width: the caller draws it once (ov-537).
    static func measure(
        _ id: String, plate: TitleStatusPlate, width: CGFloat, count: Int, appearance: NSAppearance.Name,
        rest: Kit.Shot
    ) async throws -> Measure? {
        let held = try await Kit.shot(width: width, count: count, appearance: appearance, forced: (id, plate))
        // Each pixel is read at the bitmap's own scale (1x on CI, 2x here).
        let scale = CGFloat(rest.rep.lookScale)
        let fromX = Int(rest.status.minX * scale), toX = Int(rest.status.maxX * scale)
        var plateBox = Extent()
        for y in 0..<rest.rep.pixelsHigh {
            // A row the two draw alike byte for byte has no plate in it.
            if Self.sameBytes(rest.rep, held.rep, row: y, from: fromX, to: min(toX, rest.rep.pixelsWide)) { continue }
            for x in fromX..<min(toX, rest.rep.pixelsWide) {
                if let a = rest.rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB),
                    let b = held.rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB), Self.differs(a, b, by: 0.012)
                {
                    plateBox.add(x, y)
                }
            }
        }
        guard !plateBox.isEmpty else { return nil }
        // The bar's own color, just outside the plate on its middle row.
        let midY = (plateBox.minY + plateBox.maxY) / 2
        let background = try #require(rest.rep.colorAt(x: plateBox.minX - 2, y: midY)?.usingColorSpace(.sRGB))
        var inkBox = Extent()
        for y in plateBox.minY...plateBox.maxY {
            for x in plateBox.minX...plateBox.maxX {
                if let c = rest.rep.colorAt(x: x, y: y)?.usingColorSpace(.sRGB), Self.differs(c, background, by: 0.15) {
                    inkBox.add(x, y)
                }
            }
        }
        guard !inkBox.isEmpty else { return nil }
        func rect(_ e: Extent) -> CGRect {
            CGRect(
                x: CGFloat(e.minX) / scale, y: CGFloat(e.minY) / scale, width: CGFloat(e.maxX - e.minX + 1) / scale,
                height: CGFloat(e.maxY - e.minY + 1) / scale)
        }
        return Measure(plate: rect(plateBox), ink: rect(inkBox))
    }

    /// The buttons at each width (Activity only where the wide form has it).
    static let cases: [(width: CGFloat, ids: [String])] = [
        (900, Array(TitleStatusButtonKit.ids.dropFirst())), (1400, TitleStatusButtonKit.ids),
    ]

    @Test("A hover or pressed plate holds the icon and the count with even room, on every button")
    func plateContainsLabel() async throws {
        for (width, ids) in Self.cases {
            let rest = try await Kit.shot(width: width, count: 8, appearance: .aqua, forced: nil)
            for plate in [TitleStatusPlate.hover, .pressed] {
                var heights: [CGFloat] = []
                var rooms: [CGFloat] = []
                for id in ids {
                    let m = try #require(
                        try await Self.measure(id, plate: plate, width: width, count: 8, appearance: .aqua, rest: rest),
                        "\(id) draws no \(plate) plate at \(width)")
                    // At least a few points each side: the symbol's and the
                    // digit's own side bearings are well under 2 pt, and the
                    // plate pads by 8, so 6 holds at 1x and 2x.
                    #expect(m.left >= 6, "\(id) \(plate) at \(width): \(m.left) pt left of the label")
                    #expect(m.right >= 6, "\(id) \(plate) at \(width): \(m.right) pt right of the label")
                    #expect(abs(m.left - m.right) <= 2.5, "\(id) \(plate) at \(width): \(m.left) left, \(m.right) right")
                    #expect(m.plate.height >= m.ink.height + 6, "\(id) \(plate): the label fills the plate's height")
                    heights.append(m.plate.height)
                    rooms.append(m.plate.width - m.ink.width)
                }
                // One hit target: the same height and padding on every button.
                #expect((heights.max() ?? 0) - (heights.min() ?? 0) <= 1, "heights \(heights) at \(width)")
                #expect((rooms.max() ?? 0) - (rooms.min() ?? 0) <= 2.5, "padding \(rooms) at \(width)")
            }
        }
    }
}
