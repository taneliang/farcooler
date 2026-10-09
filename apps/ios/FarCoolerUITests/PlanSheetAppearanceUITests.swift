import XCTest

/// The Plan sheet keeps one appearance while it expands and collapses (ov-444).
///
/// The sheet opens from the strip over the orchestrator, whose ground forces
/// the terminal theme's scheme on everything under it. A sheet took that value
/// into its content while its bars and background followed the phone's own
/// Light or Dark, so a dark theme on a light phone (and the reverse) drew half
/// the sheet in each, and the mix changed with the detent. These open it at the
/// medium detent, pull it to large and back, and read the pixels of the sheet
/// at each stop: the same scheme every time, and the phone's.
///
/// Both pairings of theme and appearance that disagree, and the two that
/// agree, since the fix must not move the ones that were right. Each stop is
/// kept as a screenshot, and written to `FARCOOLER_CAPTURE_OUT` when that is
/// set (as `TEST_RUNNER_FARCOOLER_CAPTURE_OUT`), for a review.
final class PlanSheetAppearanceUITests: XCTestCase {
    override func setUp() {
        continueAfterFailure = false
    }

    /// The simulator is shared, so the next test finds it as it was.
    override func tearDown() {
        XCUIDevice.shared.appearance = .light
    }

    private static var fixture: String {
        var root = URL(fileURLWithPath: #filePath)
        for _ in 0..<4 { root.deleteLastPathComponent() }
        return root.appendingPathComponent("test/fixtures/plan-seeded.json").path
    }

    /// Billing on its orchestrator, under `theme`, on a phone set to `system`.
    private func openSheet(system: String, theme: String) -> XCUIApplication {
        // The device's own appearance: the `-AppleInterfaceStyle` launch
        // argument changes what SwiftUI reads but not what the system's glass
        // sheet draws with, and a test that used it saw a dark phone with a
        // light sheet at the medium detent whatever the app did.
        XCUIDevice.shared.appearance = system == "Dark" ? .dark : .light
        let app = XCUIApplication.phoneHarness([
            "-phone-empty-inbox", "-phone-billing-led", "-phone-plan", "-phone-plan-file", Self.fixture,
            "-app.theme", theme,
        ])
        let billing = app.buttons["workspace-row-Billing"]
        XCTAssertTrue(billing.waitForExistence(timeout: 30), "no Billing row")
        billing.tap()
        let strip = app.buttons["plan-strip"]
        XCTAssertTrue(strip.waitForExistence(timeout: 30), "no strip")
        strip.tap()
        XCTAssertTrue(app.navigationBars["Plan"].waitForExistence(timeout: 30), "no sheet")
        return app
    }

    // MARK: Pixels

    /// What the sheet looks like at one stop: the luminance (0 black, 1 white)
    /// of the middle of its pixels under the navigation bar (its ground, which
    /// is only the sheet's own once it is large: at the medium detent the
    /// sheet is glass over the pane), and of the orchestrator card's ground and
    /// ink, which are the sheet's at every stop.
    private struct Look {
        var contentGround: Double
        var cardGround: Double
        var cardInk: Double
    }

    private func look(_ app: XCUIApplication, _ name: String) -> Look {
        let shot = app.screenshot()
        let attachment = XCTAttachment(screenshot: shot)
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
        if let out = ProcessInfo.processInfo.environment["FARCOOLER_CAPTURE_OUT"] {
            try? shot.pngRepresentation.write(to: URL(fileURLWithPath: out).appendingPathComponent("plan-sheet-\(name).png"))
        }
        let bar = app.navigationBars["Plan"].frame
        let width = app.windows.firstMatch.frame.width
        let bandTop = bar.maxY + 10
        let content = Self.luminances(shot, x: width * 0.05..<width * 0.95, y: bandTop..<(bandTop + 150))
        let card = app.descendants(matching: .any)["plan-sheet-orchestrator"].frame
        let cell = Self.luminances(
            shot, x: (card.minX + 8)..<(card.minX + card.width * 0.6), y: (card.minY + 4)..<(card.maxY - 4))
        let ground = Self.median(cell)
        return Look(contentGround: Self.median(content), cardGround: ground, cardInk: Self.farthest(cell, from: ground))
    }

    /// The luminance of every pixel in the rectangle, in points.
    private static func luminances(_ shot: XCUIScreenshot, x: Range<CGFloat>, y: Range<CGFloat>) -> [Double] {
        guard let cg = shot.image.cgImage else { return [] }
        let scale = CGFloat(cg.width) / shot.image.size.width
        let width = cg.width, height = cg.height
        var pixels = [UInt8](repeating: 0, count: width * height * 4)
        guard
            let context = CGContext(
                data: &pixels, width: width, height: height, bitsPerComponent: 8, bytesPerRow: width * 4,
                space: CGColorSpaceCreateDeviceRGB(), bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return [] }
        context.draw(cg, in: CGRect(x: 0, y: 0, width: width, height: height))
        var out: [Double] = []
        let rows = max(0, Int(y.lowerBound * scale))..<min(height, Int(y.upperBound * scale))
        let columns = max(0, Int(x.lowerBound * scale))..<min(width, Int(x.upperBound * scale))
        for row in rows {
            for column in columns {
                let at = (row * width + column) * 4
                out.append((0.2126 * Double(pixels[at]) + 0.7152 * Double(pixels[at + 1]) + 0.0722 * Double(pixels[at + 2])) / 255)
            }
        }
        return out
    }

    private static func median(_ values: [Double]) -> Double {
        values.isEmpty ? 0.5 : values.sorted()[values.count / 2]
    }

    /// The pixel furthest in luminance from `ground`: the ink of the text.
    private static func farthest(_ values: [Double], from ground: Double) -> Double {
        values.max { abs($0 - ground) < abs($1 - ground) } ?? ground
    }

    // MARK: Detents

    /// Pull the sheet by its bar to `fraction` of the window's height, and
    /// wait for it to settle.
    private func pull(_ app: XCUIApplication, to fraction: CGFloat) {
        let bar = app.navigationBars["Plan"]
        let start = bar.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: 0.5))
        let end = app.windows.firstMatch.coordinate(withNormalizedOffset: CGVector(dx: 0.3, dy: fraction))
        start.press(forDuration: 0.1, thenDragTo: end)
        var last = -1.0
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            let top = Double(bar.frame.minY)
            if abs(top - last) < 0.5 { return }
            last = top
            Thread.sleep(forTimeInterval: 0.4)
        }
    }

    private func isLarge(_ app: XCUIApplication) -> Bool {
        app.navigationBars["Plan"].frame.minY < app.windows.firstMatch.frame.height * 0.2
    }

    /// The sheet at medium, large, medium and large again, each the phone's
    /// own scheme.
    private func checkOneAppearance(system: String, theme: String, name: String) {
        let dark = system == "Dark"
        let app = openSheet(system: system, theme: theme)
        var stops: [(String, Look)] = []
        stops.append(("medium", look(app, "\(name)-1-medium")))
        pull(app, to: 0.02)
        XCTAssertTrue(isLarge(app), "the sheet didn't expand")
        stops.append(("large", look(app, "\(name)-2-large")))
        pull(app, to: 0.6)
        XCTAssertFalse(isLarge(app), "the sheet didn't collapse")
        stops.append(("medium again", look(app, "\(name)-3-medium-again")))
        pull(app, to: 0.02)
        XCTAssertTrue(isLarge(app), "the sheet didn't expand again")
        stops.append(("large again", look(app, "\(name)-4-large-again")))

        for (stop, seen) in stops {
            let where_ = "\(name), \(stop): content \(seen.contentGround), card \(seen.cardGround), ink \(seen.cardInk)"
            // The words are light on a dark sheet and dark on a light one,
            // and read against their card.
            XCTAssertEqual(seen.cardInk > seen.cardGround, dark, "the sheet's words aren't the phone's \(system): \(where_)")
            XCTAssertGreaterThan(abs(seen.cardInk - seen.cardGround), 0.3, "the sheet's words don't contrast: \(where_)")
            if stop.hasPrefix("large") {
                XCTAssertEqual(seen.contentGround < 0.5, dark, "the sheet's ground isn't the phone's \(system): \(where_)")
            }
        }
    }

    // MARK: Tests

    /// The reported case: a dark terminal theme on a light phone.
    func testADarkThemeOnALightPhoneKeepsALightSheet() {
        checkOneAppearance(system: "Light", theme: "Nord", name: "light-phone-dark-theme")
    }

    /// And the reverse.
    func testALightThemeOnADarkPhoneKeepsADarkSheet() {
        checkOneAppearance(system: "Dark", theme: "Solarized Light", name: "dark-phone-light-theme")
    }

    func testADarkThemeOnADarkPhoneKeepsADarkSheet() {
        checkOneAppearance(system: "Dark", theme: "Nord", name: "dark-phone-dark-theme")
    }

    func testALightThemeOnALightPhoneKeepsALightSheet() {
        checkOneAppearance(system: "Light", theme: "Solarized Light", name: "light-phone-light-theme")
    }
}
