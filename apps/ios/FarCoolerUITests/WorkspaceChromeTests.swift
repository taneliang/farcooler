import UIKit
import XCTest

/// The workspace screen's chrome over the terminal's ground (ov-342): its
/// navigation title, its segment titles and its plan strip's line.
///
/// The orchestrator's pane paints the terminal theme's background, dark or
/// light whatever the phone is set to, so the text that sits on it has to
/// follow that ground and not the system's appearance. With Light Mode and
/// the dark Nord theme the title and segments drew black on #2E3440, about
/// 1.6:1. The test measures what is on screen: for each of the two system
/// appearances and the two terminal themes, the contrast between an element's
/// background (the commonest color in its frame) and its text (the pixel
/// farthest from that background in luminance) must be at least 4.5:1, the
/// AA ratio for body text.
///
/// Over the canned runner (`-phone-harness`), so nothing here skips.
final class WorkspaceChromeTests: XCTestCase {
    override func setUp() {
        continueAfterFailure = true
    }

    override func tearDown() {
        XCUIDevice.shared.appearance = .light
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    func testChromeReadsOverTheDarkNordTheme() throws {
        try check(appearance: .light, theme: "Nord")
    }

    func testChromeReadsOverTheLightSolarizedTheme() throws {
        try check(appearance: .dark, theme: "Solarized Light")
    }

    func testChromeReadsInTheSystemsOwnPairings() throws {
        try check(appearance: .dark, theme: "Nord")
        try check(appearance: .light, theme: "Solarized Light")
    }

    private func check(appearance: XCUIDevice.Appearance, theme: String) throws {
        XCUIDevice.shared.appearance = appearance
        let app = XCUIApplication.phoneHarness([
            "-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file",
            Self.fixture, "-app.theme", theme,
        ])
        defer { app.terminateRetrying() }
        let row = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(row.waitForExistence(timeout: 30), "no Billing row")
        row.tap()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 15), "no plan strip")
        XCTAssertTrue(app.buttons["segment-tree"].exists)
        let name = "\(appearance == .light ? "light" : "dark")-\(theme.replacingOccurrences(of: " ", with: "-"))"
        let shot = XCTAttachment(screenshot: app.screenshot())
        shot.name = "chrome-\(name)"
        shot.lifetime = .keepAlways
        add(shot)

        let title = app.navigationBars.firstMatch.staticTexts["Billing"]
        XCTAssertTrue(title.exists, "no navigation title")
        let screen = app.screenshot()
        for (what, element) in [
            ("the navigation title", title), ("the Themes segment", app.buttons["segment-tree"]),
            ("the plan strip", strip),
        ] {
            let ratio = Self.contrast(in: element.frame, of: screen)
            print("chrome-contrast \(name) \(what): \(String(format: "%.2f", ratio)):1")
            XCTAssertGreaterThanOrEqual(
                ratio, 4.5,
                "\(what) is \(String(format: "%.2f", ratio)):1 over its ground with \(name)")
        }
    }

    /// The contrast between the commonest color in `frame` (the ground) and
    /// the pixel farthest from it in luminance (the text, at its fullest).
    ///
    /// The frame is in points; the scale is read off the bitmap rather than
    /// assumed, since CI renders at 1x and this Mac at 2x or 3x. At 1x a
    /// glyph's stems are one pixel wide, and the farthest pixel of a title
    /// still reaches the text's own color, so no tolerance is needed.
    static func contrast(in frame: CGRect, of screenshot: XCUIScreenshot) -> Double {
        guard let cg = screenshot.image.cgImage else { return 0 }
        let scale = Double(cg.width) / Double(screenshot.image.size.width)
        let box = CGRect(
            x: frame.minX * scale, y: frame.minY * scale,
            width: frame.width * scale, height: frame.height * scale
        ).integral.intersection(CGRect(x: 0, y: 0, width: cg.width, height: cg.height))
        guard !box.isEmpty, let crop = cg.cropping(to: box) else { return 0 }
        let width = crop.width
        let height = crop.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard let context = CGContext(
            data: &pixels, width: width, height: height, bitsPerComponent: 8,
            bytesPerRow: width * 4, space: CGColorSpaceCreateDeviceRGB(),
            bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return 0 }
        context.draw(crop, in: CGRect(x: 0, y: 0, width: width, height: height))
        var counts: [UInt32: Int] = [:]
        for i in 0..<(width * height) {
            let key = UInt32(pixels[i * 4]) << 16 | UInt32(pixels[i * 4 + 1]) << 8 | UInt32(pixels[i * 4 + 2])
            counts[key, default: 0] += 1
        }
        guard let ground = counts.max(by: { $0.value < $1.value })?.key else { return 0 }
        let groundLuminance = luminance(ground)
        var best = 1.0
        for key in counts.keys {
            let other = luminance(key)
            let hi = max(groundLuminance, other)
            let lo = min(groundLuminance, other)
            best = max(best, (hi + 0.05) / (lo + 0.05))
        }
        return best
    }

    /// WCAG relative luminance of a packed 0xRRGGBB.
    private static func luminance(_ packed: UInt32) -> Double {
        func channel(_ shift: UInt32) -> Double {
            let v = Double((packed >> shift) & 0xFF) / 255
            return v <= 0.03928 ? v / 12.92 : pow((v + 0.055) / 1.055, 2.4)
        }
        return 0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
    }
}
