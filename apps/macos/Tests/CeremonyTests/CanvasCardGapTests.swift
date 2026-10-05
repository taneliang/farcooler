import AppKit
import SwiftUI
import Testing

@testable import Far_Cooler

/// The canvas's card and the chat's card are one gutter apart (integ-13b
/// ruling): `Gutter.window`, as between tiled panes' cards and between a card
/// and the window's edge. Each was drawn with its own gutter on its own side,
/// which put two gutters and the divider between them, 13 pt on macOS 27.
@MainActor
@Suite(.serialized)
struct CanvasCardGapTests {
    @Test("The canvas's card and the chat's card are one gutter apart", arguments: lookScales)
    func oneGutterBetweenCards(scale: Int) async throws {
        let size = CGSize(width: 1600, height: 400)
        // The real `contentCard()`, filled with a color of its own so each
        // card's pixels are told from the plane's and from each other's.
        let view = WorkspaceView(
            opened: nil as String?, hasConversation: true, cell: WorkspaceColumns.defaultCell, focused: false,
            navigatorWidth: .constant(320), conversation: { Color.blue.contentCard() }, navigator: { Color.clear },
            breadcrumb: { _ in Color.clear }, detail: { _, _ in Color.clear }, split: true,
            home: { AnyView(Color.red.contentCard()) })
            .frame(width: size.width, height: size.height)
        let host = NSHostingView(rootView: view)
        host.frame = CGRect(origin: .zero, size: size)
        for _ in 0..<5 {
            host.layoutSubtreeIfNeeded()
            try await Task.sleep(for: .milliseconds(20))
        }
        let rep = try #require(host.lookBitmap(scale: scale))
        let pixels = rep.pixelsWide / Int(size.width)
        let y = rep.pixelsHigh / 2
        func isRed(_ c: NSColor) -> Bool { c.redComponent > 0.5 && c.blueComponent < 0.5 }
        func isBlue(_ c: NSColor) -> Bool { c.blueComponent > 0.5 && c.redComponent < 0.5 }
        let row = (0..<rep.pixelsWide).map { rep.colorAt(x: $0, y: y)!.usingColorSpace(.sRGB)! }
        let lastRed = try #require(row.lastIndex(where: isRed), "no canvas card drawn")
        let firstBlue = try #require(row.firstIndex(where: isBlue), "no chat card drawn")
        let gap = firstBlue - lastRed - 1
        let want = Int(Pane.inset) * pixels
        // Each card's edge can land between two pixels (the chat's leading
        // edge is a whole number of terminal cells from the window's, and a
        // cell isn't a whole point), and its antialiased pixel is counted on
        // either side: one pixel per edge at 1x or 2x. The double gutter this
        // guards against is 7 pt wider, 7 pixels even at 1x.
        #expect(abs(gap - want) <= 2, "\(gap) px between the cards at \(scale)x, want \(want)")
    }
}
