import AgentKit
import AppKit
import SwiftUI

// A task key's hovercard in a terminal (ov-299). ⌘ underlines and opens a link,
// as it did; resting the pointer on a key, ⌘ or not, shows what the task is.
// Which cells are a key is the link hit-testing's own answer
// (`TerminalRenderView.link(atRow:column:)`), so the card shows exactly where
// ⌘-click would open a task, and nowhere a URL would win.

/// One pane's hovercard: the key under the pointer, the wait for the system's
/// hover delay, and the popover.
@MainActor
final class TerminalKeyHover {
    /// The key under the pointer, shown or waiting to be.
    private(set) var key: String?
    /// The card on screen, or nil.
    private(set) var shown: TaskKeyCard?
    /// How long the pointer rests before the card shows; nil is the
    /// system's (`TaskKeyHoverDelay`). A test sets zero.
    var delay: Duration?
    private var waiting: Task<Void, Never>?
    /// Whether a card is waiting out the delay, for a test.
    var isWaiting: Bool { waiting != nil }
    private var popover: NSPopover?

    /// The pointer moved over `view`: follow it onto or off a key.
    func moved(_ view: TerminalRenderView, _ event: NSEvent) {
        let cell = view.cellForTesting(event)
        let x = view.convert(event.locationInWindow, from: nil).x
        hover(view, row: cell.row, column: cell.column, x: x)
    }

    /// The pointer over cell (`row`, `column`), at `x` in the view.
    func hover(_ view: TerminalRenderView, row: Int, column: Int, x: CGFloat) {
        let found = view.taskKeyCard(atRow: row, column: column)
        guard found?.card.key != key else { return }
        close()
        key = found?.card.key
        guard let found else { return }
        let anchor = NSRect(x: x - 1, y: view.rowRect(found.row).minY, width: 2, height: view.rowRect(found.row).height)
        let wait = delay ?? TaskKeyHoverDelay.current()
        waiting = Task { [weak self, weak view] in
            if wait > .zero { try? await Task.sleep(for: wait) }
            guard let self, let view, !Task.isCancelled, self.key == found.card.key else { return }
            self.show(found.card, at: anchor, in: view)
            self.waiting = nil
        }
    }

    /// The pointer left the pane, or the pane is going.
    func exited() {
        close()
        key = nil
    }

    private func close() {
        waiting?.cancel()
        waiting = nil
        popover?.close()
        popover = nil
        shown = nil
    }

    private func show(_ card: TaskKeyCard, at anchor: NSRect, in view: TerminalRenderView) {
        // A popover shown from a view outside a window raises, and an
        // exception thrown through AppKit's event loop is a crash later.
        guard view.window != nil else { return }
        let popover = NSPopover()
        popover.behavior = .semitransient
        popover.animates = false
        let content = NSHostingController(rootView: TaskKeyCardView(card: card))
        // Its size before it's shown: a popover placed at a hosting view's
        // first guess, then shrunk to fit, keeps its bottom edge and ends
        // up rows below the key.
        content.sizingOptions = .preferredContentSize
        popover.contentViewController = content
        popover.contentSize = content.view.fittingSize
        popover.show(relativeTo: anchor, of: view, preferredEdge: .maxY)
        self.popover = popover
        shown = card
    }
}

extension TerminalRenderView {
    /// The card for the task key at cell (`row`, `column`), with the row its
    /// key ends on, or nil: no link there, a URL, or a key without a card.
    func taskKeyCard(atRow row: Int, column: Int) -> (card: TaskKeyCard, row: Int)? {
        guard let found = link(atRow: row, column: column), let url = URL(string: found.url),
            let card = taskKeyLinker.card(for: url)
        else { return nil }
        return (card, Int(found.span.end_row))
    }
}
