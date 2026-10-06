import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The runners' toolbar item ("carl offline", "Update available") keeps the
/// padding the system gives a toolbar button's label (ov-350). It had been
/// `.borderless` and `.fixedSize()`, which strip it, so the words ran to the
/// edge of the item's capsule. Measured in a real titled window, as the
/// capture harness draws one, on the production `TrailingToolbar`.
@MainActor
@Suite(.serialized)
struct RunnerItemPaddingTests {
    private struct Bar: View {
        let troubles: [RunnerStatusItem.Trouble]
        let stale: [String]
        var body: some View {
            Color.clear.toolbar {
                TrailingToolbar(
                    troubles: troubles, stale: stale, updates: [], needsYou: 1, needsYouSelected: false,
                    onNeedsYou: {}, perform: { _ in })
            }
        }
    }

    /// The label's room inside its toolbar item, left and right, in points.
    private static func padding(troubles: [RunnerStatusItem.Trouble], stale: [String]) async throws -> (left: CGFloat, right: CGFloat) {
        let window = try await TitleBarHarness.window(Bar(troubles: troubles, stale: stale), width: 900, height: 200)
        defer { window.close() }
        var anchor: NSView?
        func find(_ view: NSView) {
            if view is PullDownAnchorView { anchor = view }
            view.subviews.forEach(find)
        }
        window.contentView?.superview.map(find)
        let label = try #require(anchor, "no runner item drawn")
        var item = label.superview
        while let current = item, !String(describing: type(of: current)).hasPrefix("ToolbarItemHostingView") {
            item = current.superview
        }
        let host = try #require(item, "the label has no toolbar item around it")
        let inLabel = label.convert(label.bounds, to: nil)
        let inItem = host.convert(host.bounds, to: nil)
        return (inLabel.minX - inItem.minX, inItem.maxX - inLabel.maxX)
    }

    /// Room a system toolbar button leaves each side of its label: well over
    /// a point or two, never none. Needs You beside it sets the standard.
    static let leastRoom: CGFloat = 6

    @Test("An offline runner's label has room each side inside its item, even to a point")
    func offlineHasEvenPadding() async throws {
        let (left, right) = try await Self.padding(troubles: [.init(host: "carl", problem: .offline)], stale: [])
        #expect(left >= Self.leastRoom, "left \(left)")
        #expect(right >= Self.leastRoom, "right \(right)")
        #expect(abs(left - right) <= 1, "left \(left), right \(right)")
    }

    @Test("An update's label has the same room")
    func updateHasEvenPadding() async throws {
        let (left, right) = try await Self.padding(troubles: [], stale: ["carl"])
        #expect(left >= Self.leastRoom, "left \(left)")
        #expect(right >= Self.leastRoom, "right \(right)")
        #expect(abs(left - right) <= 1, "left \(left), right \(right)")
    }
}
