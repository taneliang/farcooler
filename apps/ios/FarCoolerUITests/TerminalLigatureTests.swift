import XCTest

/// The terminal must never join two characters into one.
///
/// Iosevka has coding ligatures and the app bundles it deliberately, so a
/// renderer that hands CoreText a run of text to shape will draw `a != b` as
/// `a ≠ b` and `->` as `→`. In a prose view that is a feature. In a terminal
/// it is a lie about what a program printed, and the kind of lie that reads
/// as correct until somebody looks at a diff and cannot find the character
/// they typed.
///
/// The renderer places glyphs by index — `CTFontGetGlyphsForCharacters` into a
/// cache, `CTFontDrawGlyphs` at cell positions — so there is no shaper in the
/// path and nothing that *could* substitute. This test is the proof that the
/// path stayed that way.
///
/// ## Why it photographs the screen
///
/// Every cheaper check here is a check that cannot fail. Asserting about a
/// helper in the test bundle tests a copy of the logic, not the renderer;
/// asking the app to report on itself is the app marking its own homework. The
/// only evidence that the *drawing* did not ligate is the drawing.
///
/// So the app draws a fixture — `!=` at columns 0–1, and the same `!` and `=`
/// alone at columns 4 and 6 — and this compares the pixels of the pair against
/// the pixels of the singletons. A ligature changes the glyph in BOTH cells of
/// the pair (Iosevka's `≠` is two half-glyphs, one per cell, which is exactly
/// why the total width stays right and the eye is the only thing that catches
/// it), so either comparison fails.
///
/// ## Watching it fail
///
/// `-bench-row-text` makes the harness draw each row as one `Text`, which is
/// the obvious batching and the wrong one. Under it the fixture renders `≠`
/// and `→`, and both assertions below fail with a difference of about a third
/// of the cell. That is the check being exercised, not assumed:
///
///     -terminal-ligature                     ✅ ! and = , - and >
///     -terminal-ligature -bench-row-text     ❌ ≠ and →
///
/// Note also what does NOT work, and cost an afternoon to establish:
/// `kCTLigatureAttributeName = 0` does not disable these. Iosevka Nerd Font
/// Mono carries them in `calt`, contextual alternates, and has no `liga` table
/// at all — `CTLineCreateWithAttributedString` on `"!="` returns the two
/// halves of `≠` with the attribute set to zero just as it does without it.
final class TerminalLigatureTests: XCTestCase {
    /// The columns the fixture puts each character in. See
    /// `TerminalBenchHarness.ligatureGrid`.
    private enum Column {
        static let bangOfPair = 0
        static let equalsOfPair = 1
        static let bangAlone = 4
        static let equalsAlone = 6
        static let dashOfPair = 8
        static let angleOfPair = 9
        static let dashAlone = 12
        static let angleAlone = 14
    }

    func testThePairDrawsTheSameGlyphsAsTheSingletons() throws {
        let app = XCUIApplication()
        // No runner, no fleet, no host: the fixture is a grid built in the app
        // itself, so this test cannot skip for want of a demo daemon and
        // cannot pass because one was missing.
        app.launchArguments += ["-terminal-ligature"]
        app.launch()

        let fixture = app.otherElements["terminal-ligature-fixture"]
        XCTAssertTrue(
            fixture.waitForExistence(timeout: 30),
            "the ligature fixture never appeared; the harness did not launch")

        let value = (fixture.value as? String) ?? ""
        // By name, like `terminal-surface`'s, and for the same reason: a string
        // read by position is a string nobody can add a field to.
        let cell = try XCTUnwrap(field(value, "cell"), "no cell width in \(value)")
        let line = try XCTUnwrap(field(value, "line"), "no line height in \(value)")
        let pad = try XCTUnwrap(field(value, "pad"), "no padding in \(value)")
        XCTAssertGreaterThan(cell, 4, "a cell this narrow has no glyph in it to compare")

        let shot = Bitmap(fixture.screenshot(), pointWidth: fixture.frame.width)
        XCTAssertGreaterThan(shot.scale, 0, "the fixture screenshot had no size")

        func box(_ column: Int) -> CGRect {
            CGRect(x: pad + CGFloat(column) * cell, y: pad, width: cell, height: line)
        }

        // Sanity first, and it is not ceremony: if the fixture drew nothing at
        // all then every comparison below would pass on two blank rectangles,
        // which is precisely the shape of failure this repo keeps finding.
        for column in [
            Column.bangOfPair, Column.equalsOfPair, Column.bangAlone, Column.equalsAlone,
            Column.dashOfPair, Column.angleOfPair, Column.dashAlone, Column.angleAlone,
        ] {
            XCTAssertGreaterThan(
                shot.ink(in: box(column)), 0.005,
                "column \(column) of the fixture is blank; nothing was drawn to compare")
        }

        assertSameGlyph(shot, box(Column.bangOfPair), box(Column.bangAlone), "! of !=", "! alone")
        assertSameGlyph(
            shot, box(Column.equalsOfPair), box(Column.equalsAlone), "= of !=", "= alone")
        assertSameGlyph(shot, box(Column.dashOfPair), box(Column.dashAlone), "- of ->", "- alone")
        assertSameGlyph(
            shot, box(Column.angleOfPair), box(Column.angleAlone), "> of ->", "> alone")
    }

    /// Two cells drawn from the same character land on the same pixels.
    ///
    /// The tolerance is for antialiasing only, and it is set from measurement
    /// rather than from taste. Both cells sit at a whole number of points from
    /// the canvas origin and the grid is monospaced, so the two rasterisations
    /// are of the same glyph at the same subpixel phase: on the healthy side
    /// all four comparisons come back at exactly 0.0. Under `-bench-row-text`
    /// they come back at 2.1%, 4.2%, 6.2% and 21.7%. Half a percent sits four
    /// times below the closest failure and infinitely above the healthy one.
    private func assertSameGlyph(
        _ shot: Bitmap, _ left: CGRect, _ right: CGRect, _ leftName: String,
        _ rightName: String, file: StaticString = #filePath, line: UInt = #line
    ) {
        let difference = shot.difference(left, right)
        // Printed on the way past, so a passing run says how much margin it
        // had. A threshold nobody has seen the healthy side of is a threshold
        // chosen by hope.
        print("ligature-margin \(leftName) vs \(rightName): \(difference)")
        let percent = Int(difference * 100)
        var message = leftName + " is drawn differently from " + rightName
        message += ": \(percent)% of the cell disagrees."
        message += " Two characters were joined into a ligature."
        XCTAssertLessThan(difference, 0.005, message, file: file, line: line)
    }

    private func field(_ value: String, _ key: String) -> CGFloat? {
        let prefix = key + "="
        for part in value.split(separator: " ") where part.hasPrefix(prefix) {
            let text = String(part.dropFirst(prefix.count))
            guard let number = Double(text) else { return nil }
            return CGFloat(number)
        }
        return nil
    }

    /// A screenshot with its pixels in hand, addressed in the points the app
    /// laid the grid out in.
    private struct Bitmap {
        let width: Int
        let height: Int
        let scale: CGFloat
        private let bytes: [UInt8]

        init(_ screenshot: XCUIScreenshot, pointWidth: CGFloat) {
            guard let image = screenshot.image.cgImage, pointWidth > 0 else {
                width = 0
                height = 0
                scale = 0
                bytes = []
                return
            }
            let pixelWidth = image.width
            let pixelHeight = image.height
            width = pixelWidth
            height = pixelHeight
            // Measured, not assumed to be `UIScreen.main.scale`: what comes
            // back is the element's own image, and the ratio between its pixel
            // width and the frame the app reported is the only number that
            // relates the two.
            scale = CGFloat(image.width) / pointWidth
            // Locals throughout, never `self`: a closure that reaches for a
            // property here captures a `self` whose `bytes` is the thing being
            // initialised, which the compiler refuses outright.
            var raw = [UInt8](repeating: 0, count: pixelWidth * pixelHeight * 4)
            raw.withUnsafeMutableBytes { buffer in
                guard
                    let context = CGContext(
                        data: buffer.baseAddress, width: pixelWidth, height: pixelHeight,
                        bitsPerComponent: 8, bytesPerRow: pixelWidth * 4,
                        space: CGColorSpaceCreateDeviceRGB(),
                        bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
                else { return }
                let box = CGRect(x: 0, y: 0, width: pixelWidth, height: pixelHeight)
                context.draw(image, in: box)
            }
            bytes = raw
        }

        private func luminance(_ x: Int, _ y: Int) -> Int {
            guard x >= 0, y >= 0, x < width, y < height else { return 0 }
            let i = (y * width + x) * 4
            let red: Int = Int(bytes[i])
            let green: Int = Int(bytes[i + 1])
            let blue: Int = Int(bytes[i + 2])
            return (red * 30 + green * 59 + blue * 11) / 100
        }

        private func pixels(_ rect: CGRect) -> (x: Int, y: Int, w: Int, h: Int) {
            let x: Int = Int((rect.minX * scale).rounded())
            let y: Int = Int((rect.minY * scale).rounded())
            let w: Int = Int((rect.width * scale).rounded())
            let h: Int = Int((rect.height * scale).rounded())
            return (x, y, w, h)
        }

        /// The fraction of a cell that is not the colour of its own corner.
        ///
        /// The corner rather than a fixed colour, so this reads the same on a
        /// black fixture and on whatever ground a theme puts behind it.
        func ink(in rect: CGRect) -> Double {
            let box = pixels(rect)
            guard box.w > 0, box.h > 0 else { return 0 }
            let ground = luminance(box.x, box.y)
            var inked = 0
            for y in box.y..<(box.y + box.h) {
                for x in box.x..<(box.x + box.w) where abs(luminance(x, y) - ground) > 32 {
                    inked += 1
                }
            }
            return Double(inked) / Double(box.w * box.h)
        }

        /// The fraction of two cells' pixels that disagree.
        func difference(_ left: CGRect, _ right: CGRect) -> Double {
            let a = pixels(left)
            let b = pixels(right)
            let w = min(a.w, b.w)
            let h = min(a.h, b.h)
            guard w > 0, h > 0 else { return 1 }
            var differing = 0
            for y in 0..<h {
                for x in 0..<w
                where abs(luminance(a.x + x, a.y + y) - luminance(b.x + x, b.y + y)) > 32 {
                    differing += 1
                }
            }
            return Double(differing) / Double(w * h)
        }
    }
}
