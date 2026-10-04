import AgentKit
import AppKit
import SwiftUI

// Mark as Read asks first (ov-210). The owner: "mark all as read should have
// a confirmation step, too easy to hit by accident, especially the keyboard
// shortcut".
//
// Every way in comes through `MarkReadConfirmation.confirm`: Unread's header
// button on hover, its VoiceOver action, its context menu, and Board ▸ Mark
// All as Read (⇧⌘K). Reading every task on a board can't be undone, and the
// one call that does it (`TaskBoardStore.markAllRead`) takes a
// `MarkReadGrant`, which only a confirmation hands out, so no new way in can
// skip the question.

/// What a Mark as Read asks: which one, and how many tasks it reads.
struct MarkReadRequest: Equatable, Sendable {
    /// "Mark These as Read", under the navigator's filter; else "Mark All
    /// as Read".
    var filtering: Bool
    /// How many tasks it reads.
    var tasks: Int
    /// Whether it reads on every device: the runner keeps this board's
    /// state and this Mac's grant can write it (ov-254).
    var everywhere = false

    /// "Mark All as Read?", or "Mark These as Read?" under a filter.
    var title: String { "\(MarkAllReadButton.title(filtering: filtering))?" }

    /// "68 tasks will be marked as read on all your devices.", "1 task will
    /// be marked as read on this Mac."
    var message: String {
        let what = tasks == 1 ? "1 task will be marked as read" : "\(tasks) tasks will be marked as read"
        return "\(what) \(everywhere ? "on all your devices" : "on this Mac")."
    }

    static let confirmTitle = "Mark as Read"
    static let cancelTitle = "Cancel"

    /// How many tasks `summary` lists: a task under Finished and Activity
    /// both counts once.
    static func tasks(in summary: BoardSummary) -> Int {
        Set((summary.finished + summary.moved + summary.created).map(\.taskID) + summary.activity.map(\.taskID)).count
    }
}

/// Leave to read every task on a board: handed out only by
/// `MarkReadConfirmation.confirm`, once the person said yes.
struct MarkReadGrant {
    fileprivate init() {}
}

/// Where a Mark as Read asks, and the one place it's asked from. The app's
/// is `alert`; tests answer for the person.
struct MarkReadConfirmation: Sendable {
    /// Ask `request`, then say whether the person confirmed.
    let ask: @MainActor (MarkReadRequest, @escaping @MainActor (Bool) -> Void) -> Void

    /// Ask, and `perform` only on a yes. With nothing to read there's
    /// nothing to ask, and nothing done.
    @MainActor
    func confirm(_ request: MarkReadRequest, then perform: @escaping @MainActor (MarkReadGrant) -> Void) {
        guard request.tasks > 0 else { return }
        ask(request) { confirmed in
            if confirmed { perform(MarkReadGrant()) }
        }
    }

    /// The app's: an alert, as a sheet on the key window.
    static var alert: MarkReadConfirmation {
        MarkReadConfirmation { request, done in present(request, on: NSApp.keyWindow ?? NSApp.mainWindow, done: done) }
    }

    /// The alert, in Apple's style: the question as its title, the count
    /// as its message, Cancel to the left of Mark as Read.
    ///
    /// No button is the default, so Return after ⇧⌘K can't confirm: the
    /// HIG's advice for an alert the person should read rather than press
    /// Return through, and the reason it gives against making Cancel the
    /// default. Esc and Cancel cancel.
    @MainActor
    static func makeAlert(_ request: MarkReadRequest) -> NSAlert {
        let alert = NSAlert()
        alert.messageText = request.title
        alert.informativeText = request.message
        let confirm = alert.addButton(withTitle: MarkReadRequest.confirmTitle)
        let cancel = alert.addButton(withTitle: MarkReadRequest.cancelTitle)
        confirm.keyEquivalent = ""
        cancel.keyEquivalent = "\u{1b}"
        return alert
    }

    /// Whether `response` from `makeAlert`'s alert is a yes.
    static func confirmed(_ response: NSApplication.ModalResponse) -> Bool {
        response == .alertFirstButtonReturn
    }

    /// Show `request` as a sheet on `window`, or with no window, on its
    /// own. The alert shown, for tests to answer.
    @MainActor @discardableResult
    static func present(
        _ request: MarkReadRequest, on window: NSWindow?, done: @escaping @MainActor (Bool) -> Void
    ) -> NSAlert {
        let alert = makeAlert(request)
        if let window {
            alert.beginSheetModal(for: window) { response in done(confirmed(response)) }
        } else {
            done(confirmed(alert.runModal()))
        }
        return alert
    }
}

extension EnvironmentValues {
    /// Where Unread's Mark as Read asks first.
    @Entry var markReadConfirmation: MarkReadConfirmation = .alert
}

extension TaskBoardStore {
    /// Whether a Mark as Read here clears every device: the runner keeps
    /// this board's state, and this Mac's grant isn't Read-only.
    var readsEverywhere: Bool { runnerKeepsReads && offersWrites }

    /// Mark All as Read, once asked: everything on the board, however the
    /// navigator is filtered (Board ▸ Mark All as Read, ⇧⌘K), or Unread's
    /// header unfiltered. The count is the tasks Unread lists unfiltered.
    func askToMarkAllRead(_ confirmation: MarkReadConfirmation, animation: Animation? = nil) {
        let summary = BoardSummaryStrip.summary(store: self, reads: reads, filter: "")
        let request = MarkReadRequest(
            filtering: false, tasks: MarkReadRequest.tasks(in: summary), everywhere: readsEverywhere)
        confirmation.confirm(request) { [weak self] grant in
            withAnimation(animation) { self?.markAllRead(grant) }
        }
    }
}
