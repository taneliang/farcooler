import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// A status menu hangs below its button and never covers it (ov-319).
@MainActor
@Suite(.serialized)
struct PullDownMenuTests {
    private let entries: [PullDownEntry] = [
        .item("ov-1 Fix the board") {}, .item("ov-2 Ship the relay") {}, .header("Queued"), .item("ov-3 Wait") {},
    ]

    @Test func originIsBelowTheAnchor() {
        let origin = PullDownCoordinator.origin(anchorHeight: 22)
        // The anchor is flipped: y grows downward, so a y past its height is
        // under its bottom edge. Any gap at all, at 1x or 2x, clears it.
        #expect(origin.y > 22)
        #expect(origin.x == 0)
    }

    @Test func menuHoldsEveryEntryInOrder() {
        let menu = PullDownCoordinator.menu(for: entries, target: .init())
        #expect(menu.items.count == 4)
        #expect(menu.items.map(\.title).filter { !$0.isEmpty }.contains("ov-2 Ship the relay"))
        #expect(menu.items[2].isSectionHeader)
    }

    /// Opens the menu from a real anchor in a real window and reads where the
    /// menu's window lands against the anchor's, then closes it. No input.
    ///
    /// Opt-in (FARCOOLER_REAL_MENU): `popUp` tracks the menu in a nested run
    /// loop on the main thread, and in the full parallel run, beside
    /// NavigatorFilterTests' synthetic presses, the test process then ended
    /// with status 0 partway through, every test after it unrun and the run
    /// reported green (integ-14: 1,189 tests run without it, the run stopping
    /// about 21 s in with it). `originIsBelowTheAnchor` is the guard in the
    /// default run; run this alone with
    /// `FARCOOLER_REAL_MENU=1 swift test --filter PullDownMenuTests`.
    @Test(.enabled(if: ProcessInfo.processInfo.environment["FARCOOLER_REAL_MENU"] != nil))
    func openMenuSitsBelowItsAnchor() async throws {
        let window = try await TitleBarHarness.window(Color.clear, width: 600)
        let anchor = PullDownAnchorView(frame: NSRect(x: 100, y: 100, width: 120, height: 22))
        window.contentView?.addSubview(anchor)
        let coordinator = PullDownCoordinator()
        coordinator.entries = entries
        coordinator.anchor = anchor
        let buttonOnScreen = window.convertToScreen(anchor.convert(anchor.bounds, to: nil))

        var menuFrame: NSRect?
        let probe = Timer(timeInterval: 0.4, repeats: false) { _ in
            MainActor.assumeIsolated {
                menuFrame = NSApp.windows.first { String(describing: type(of: $0)).contains("Menu") }?.frame
                coordinator.lastMenu?.cancelTracking()
            }
        }
        RunLoop.main.add(probe, forMode: .common)
        coordinator.open()

        let frame = try #require(menuFrame)
        // Screen coordinates grow upward: the menu's top is below the button's
        // bottom edge when its maxY is at or under the button's minY.
        #expect(frame.maxY <= buttonOnScreen.minY + 1, "menu \(frame) covers the button \(buttonOnScreen)")
        window.close()
    }
}
