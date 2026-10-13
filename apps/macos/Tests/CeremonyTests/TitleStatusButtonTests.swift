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

    /// The plate `id` draws when held at `plate`, and its label, at `width`.
    static func measure(
        _ id: String, plate: TitleStatusPlate, width: CGFloat, count: Int, appearance: NSAppearance.Name
    ) async throws -> Measure? {
        let rest = try await Kit.shot(width: width, count: count, appearance: appearance, forced: nil)
        let held = try await Kit.shot(width: width, count: count, appearance: appearance, forced: (id, plate))
        // Each pixel is read at the bitmap's own scale (1x on CI, 2x here).
        let scale = CGFloat(rest.rep.lookScale)
        let fromX = Int(rest.status.minX * scale), toX = Int(rest.status.maxX * scale)
        var plateBox = Extent()
        for y in 0..<rest.rep.pixelsHigh {
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
            for plate in [TitleStatusPlate.hover, .pressed] {
                var heights: [CGFloat] = []
                var rooms: [CGFloat] = []
                for id in ids {
                    let m = try #require(
                        try await Self.measure(id, plate: plate, width: width, count: 8, appearance: .aqua),
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
